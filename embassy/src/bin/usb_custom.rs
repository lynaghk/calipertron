#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_stm32::adc::{Adc, RxDma};
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::peripherals::ADC1;
use embassy_stm32::time::Hertz;
use embassy_stm32::{adc, bind_interrupts, peripherals, usb, Config};
use embassy_time::Timer;
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};
use embassy_usb::Builder;

use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => usb::InterruptHandler<peripherals::USB>;
});

bind_interrupts!(struct AdcIrqs {
    ADC1_2 => adc::InterruptHandler<ADC1>;
});

const MAX_PACKET_SIZE: u8 = 64;
const SAMPLES_PER_PACKET: usize = (MAX_PACKET_SIZE as usize) / 2; // 2 bytes per sample
pub const USB_CLASS_CUSTOM: u8 = 0xFF;
const USB_SUBCLASS_CUSTOM: u8 = 0x00;
const USB_PROTOCOL_CUSTOM: u8 = 0x00;

pub struct CustomClass<'d, D: Driver<'d>> {
    read_ep: D::EndpointOut,
    write_ep: D::EndpointIn,
}

impl<'d, D: Driver<'d>> CustomClass<'d, D> {
    pub fn new(builder: &mut Builder<'d, D>) -> Self {
        let mut func = builder.function(USB_CLASS_CUSTOM, USB_SUBCLASS_CUSTOM, USB_PROTOCOL_CUSTOM);
        let mut iface = func.interface();
        let mut iface_alt = iface.alt_setting(
            USB_CLASS_CUSTOM,
            USB_SUBCLASS_CUSTOM,
            USB_PROTOCOL_CUSTOM,
            None,
        );
        let read_ep = iface_alt.endpoint_bulk_out(MAX_PACKET_SIZE as u16);
        let write_ep = iface_alt.endpoint_bulk_in(MAX_PACKET_SIZE as u16);

        CustomClass { read_ep, write_ep }
    }

    pub async fn write_packet(&mut self, data: &[u8]) -> Result<(), EndpointError> {
        self.write_ep.write(data).await
    }

    pub async fn read_packet(&mut self, data: &mut [u8]) -> Result<usize, EndpointError> {
        self.read_ep.read(data).await
    }

    pub async fn wait_connection(&mut self) {
        self.read_ep.wait_enabled().await;
    }
}

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

    ////////////////////////
    // USB Setup

    let driver = embassy_stm32::usb::Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let (vid, pid) = (0xc0de, 0xcafe);
    let mut config = embassy_usb::Config::new(vid, pid);
    config.max_packet_size_0 = MAX_PACKET_SIZE;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );

    let mut custom = CustomClass::new(&mut builder);
    let mut usb = builder.build();

    let fut_usb = usb.run();

    ////////////////////////
    // ADC + DMA setup

    let mut adc_buffer = [0; 2 * SAMPLES_PER_PACKET];
    let mut adc_rb = unsafe {
        use embassy_stm32::dma::*;
        let request = p.DMA1_CH1.request();
        let mut opts = TransferOptions::default();
        opts.half_transfer_ir = true;
        ReadableRingBuffer::new(
            p.DMA1_CH1,
            request,
            embassy_stm32::pac::ADC1.dr().as_ptr() as *mut u16,
            &mut adc_buffer,
            opts,
        )
    };

    let mut adc = Adc::new(p.ADC1);

    let vrefint_sample = {
        let mut vrefint = adc.enable_vref();

        // give vref some time to warm up
        embassy_time::block_for(embassy_time::Duration::from_micros(100));

        adc.read(&mut vrefint).await as u32
    };

    let convert_to_millivolts = |sample| (sample as u32 * adc::VREF_INT / vrefint_sample) as u16;

    // Configure ADC for continuous conversion with DMA
    let adc = embassy_stm32::pac::ADC1;

    adc.cr1().modify(|w| {
        w.set_scan(true);
    });

    adc.cr2().modify(|w| {
        w.set_dma(true);
        w.set_cont(true)
    });

    // Configure channel and sampling time
    const PIN_CHANNEL: u8 = 9; // PB1 is on channel 9 for STM32F103

    adc.sqr3().modify(|w| w.set_sq(0, PIN_CHANNEL));
    adc.smpr2()
        .modify(|w| w.set_smp(PIN_CHANNEL as usize, adc::SampleTime::CYCLES239_5));

    // Start ADC conversions
    adc.cr2().modify(|w| w.set_adon(true));

    ////////////////////////
    // Main loop

    let fut_main = async {
        loop {
            custom.wait_connection().await;

            info!("Connected");

            // Start handling DMA requests from ADC
            adc_rb.start();

            let mut buf = [0; SAMPLES_PER_PACKET];

            loop {
                let r = adc_rb.read_exact(&mut buf).await;

                if r.is_err() {
                    error!("ADC_RB error: {:?}", r);
                    break;
                }

                // Process and send the data
                for i in 0..SAMPLES_PER_PACKET {
                    buf[i] = convert_to_millivolts(buf[i]);
                }

                let r = custom.write_packet(bytemuck::cast_slice(&buf)).await;

                if r.is_err() {
                    error!("USB Error: {:?}", r);
                    break;
                }
            }

            adc_rb.stop().await;
            adc_rb.clear();
        }
    };

    // Run everything concurrently.
    // If we had made everything `'static` above instead, we could do this using separate tasks instead.
    join(fut_usb, fut_main).await;
}
