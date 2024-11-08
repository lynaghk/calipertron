#![no_std]
#![no_main]

use defmt::{panic, *};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::time::Hertz;
use embassy_stm32::usb::{Driver, Instance};
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::Builder;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => usb::InterruptHandler<peripherals::USB>;
});

bind_interrupts!(struct AdcIrqs {
    ADC1_2 => adc::InterruptHandler<ADC1>;
});

const MAX_PACKET_SIZE: u8 = 64;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: Hertz(8_000_000),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            src: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL9,
        });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
    }
    let mut p = embassy_stm32::init(config);

    info!("Hello World!");

    {
        // Board has a pull-up resistor on the D+ line; pull it down to send a RESET condition to the USB bus.
        // This forced reset is needed only for development, without it host will not reset your device when you upload new firmware.
        let _dp = Output::new(&mut p.PA12, Level::Low, Speed::Low);
        Timer::after_millis(10).await;
    }

    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let (vid, pid) = (0xc0de, 0xcafe);
    let mut config = embassy_usb::Config::new(vid, pid);
    config.max_packet_size_0 = MAX_PACKET_SIZE;

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 7];

    let mut state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );

    let mut class = CdcAcmClass::new(&mut builder, &mut state, MAX_PACKET_SIZE as u16);
    let mut usb = builder.build();
    let usb_fut = usb.run();

    let mut adc = Adc::new(p.ADC1);
    let mut pin = p.PB1;

    let fut = async {
        loop {
            class.wait_connection().await;
            info!("Connected");
            //let _ = echo(&mut class).await;
            let _ = stream_adc(&mut class, &mut adc, &mut pin).await;
            info!("Disconnected");
        }
    };

    // Run everything concurrently.
    // If we had made everything `'static` above instead, we could do this using separate tasks instead.
    join(usb_fut, fut).await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn echo<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
    let mut buf = [0; MAX_PACKET_SIZE as usize];
    loop {
        let n = class.read_packet(&mut buf).await?;
        let data = &buf[..n];
        info!("data: {:x}", data);
        class.write_packet(data).await?;
    }
}

use embassy_stm32::adc;
use embassy_stm32::adc::Adc;
use embassy_stm32::peripherals::ADC1;

async fn stream_adc<'d, T: Instance + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T>>,
    adc: &mut Adc<'d, ADC1>,
    pin: &mut impl embassy_stm32::adc::AdcChannel<ADC1>,
) -> Result<(), Disconnected> {
    let mut vrefint = adc.enable_vref();
    let vrefint_sample = adc.read(&mut vrefint).await;
    let convert_to_millivolts = |sample| {
        const VREFINT_MV: u32 = 1200;
        (u32::from(sample) * VREFINT_MV / u32::from(vrefint_sample)) as u16
    };

    let mut buf = [0u8; MAX_PACKET_SIZE as usize];
    let samples_per_packet = (MAX_PACKET_SIZE as usize) / 2; // 2 bytes per sample

    loop {
        for i in 0..samples_per_packet {
            let v = adc.read(pin).await;
            let mv = convert_to_millivolts(v);
            buf[i * 2] = (mv >> 8) as u8;
            buf[i * 2 + 1] = mv as u8;
        }
        class.write_packet(&buf).await?;
    }
}
