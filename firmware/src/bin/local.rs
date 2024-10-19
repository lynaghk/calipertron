#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::dma::*;
use embassy_stm32::gpio::{Flex, Level, Output, Speed};
use embassy_stm32::time::Hertz;
use embassy_stm32::{adc, Config};

use num_traits::Float;

use {defmt_rtt as _, panic_probe as _};

const PDM_LENGTH: usize = 132;
const NUM_SAMPLES: usize = SINE_COSINE_TABLE.len();

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
    let p = embassy_stm32::init(config);

    info!("Hello World!");

    ////////////////////////
    // Signal emission setup

    let _pins = [
        Output::new(p.PA0, Level::Low, Speed::Low),
        Output::new(p.PA1, Level::Low, Speed::Low),
        Output::new(p.PA2, Level::Low, Speed::Low),
        Output::new(p.PA3, Level::Low, Speed::Low),
        Output::new(p.PA4, Level::Low, Speed::Low),
        Output::new(p.PA5, Level::Low, Speed::Low),
        Output::new(p.PA6, Level::Low, Speed::Low),
        Output::new(p.PA7, Level::Low, Speed::Low),
    ];

    let tim = embassy_stm32::timer::low_level::Timer::new(p.TIM2);
    let timer_registers = tim.regs_gp16();
    timer_registers
        .cr2()
        .modify(|w| w.set_ccds(embassy_stm32::pac::timer::vals::Ccds::ONUPDATE));
    timer_registers.dier().modify(|w| {
        // Enable update DMA request
        w.set_ude(true);
        // Enable update interrupt request
        w.set_uie(true);
    });

    tim.set_frequency(Hertz(100_000));

    let start_pdm = || unsafe {
        let mut opts = TransferOptions::default();
        opts.circular = true;

        let dma_ch = embassy_stm32::Peripheral::clone_unchecked(&p.DMA1_CH2);
        let request = embassy_stm32::timer::UpDma::request(&dma_ch);

        tim.reset();

        let t = Transfer::new_write(
            dma_ch,
            request,
            &SIGNAL,
            embassy_stm32::pac::GPIOA.bsrr().as_ptr() as *mut u32,
            opts,
        );

        tim.start();
        t
    };

    ////////////////////////
    // ADC + DMA setup

    let start_adc = |sample_buf| unsafe {
        let dma_ch = embassy_stm32::Peripheral::clone_unchecked(&p.DMA1_CH1);
        let request = embassy_stm32::adc::RxDma::request(&dma_ch);
        let opts = TransferOptions::default();

        let t = Transfer::new_read(
            dma_ch,
            request,
            embassy_stm32::pac::ADC1.dr().as_ptr() as *mut u16,
            sample_buf,
            opts,
        );

        // Start ADC conversions
        embassy_stm32::pac::ADC1.cr2().modify(|w| w.set_adon(true));
        t
    };

    // just need this to power on ADC
    let _adc = adc::Adc::new(p.ADC1);

    // Configure ADC for continuous conversion with DMA
    let adc = embassy_stm32::pac::ADC1;

    adc.cr1().modify(|w| {
        w.set_scan(true);
        w.set_eocie(true);
    });

    adc.cr2().modify(|w| {
        w.set_dma(true);
        w.set_cont(true);
    });

    // Configure channel and sampling time
    adc.sqr1().modify(|w| w.set_l(0)); // one conversion.

    // TODO: this may not be necessary
    let mut pb1 = Flex::new(p.PB1);
    pb1.set_as_analog();

    const PIN_CHANNEL: u8 = 9; // PB1 is on channel 9 for STM32F103
    adc.sqr3().modify(|w| w.set_sq(0, PIN_CHANNEL));
    adc.smpr2().modify(|w| {
        w.set_smp(
            PIN_CHANNEL as usize,
            adc::SampleTime::CYCLES239_5,
            //adc::SampleTime::CYCLES71_5,
        )
    });

    let fut_main = async {
        loop {
            // TODO: I'd rather this be local, but Transfer requires the buffer have the same lifetime as the DMA channel for some reason.
            static mut ADC_BUF: [u16; NUM_SAMPLES] = [0u16; NUM_SAMPLES];

            let adc_buf = unsafe { &mut ADC_BUF[..] };
            let adc_transfer = start_adc(adc_buf);
            let mut pdm_transfer = start_pdm();
            // wait for all of the samples to be taken
            adc_transfer.await;
            pdm_transfer.request_stop();

            let mut sum_sine: f32 = 0.0;
            let mut sum_cosine: f32 = 0.0;

            let adc_buf = unsafe { &ADC_BUF[..] };

            for i in 0..NUM_SAMPLES {
                let (sine, cosine) = SINE_COSINE_TABLE[i];
                sum_sine += adc_buf[i] as f32 * sine;
                sum_cosine += adc_buf[i] as f32 * cosine;
            }
            let phase = sum_sine.atan2(sum_cosine);
            info!("Phase: {}", phase);

            // make sure everything is reset before we continue
            pdm_transfer.await;
        }
    };

    fut_main.await
}

include!(concat!(env!("OUT_DIR"), "/sine_cosine_table.rs"));

static SIGNAL: [u32; PDM_LENGTH] = [
    0b00000000010101010000000010101010,
    0b00000000010101010000000010101010,
    0b00000000011010100000000010010101,
    0b00000000011010100000000010010101,
    0b00000000010101010000000010101010,
    0b00000000100101010000000001101010,
    0b00000000011010100000000010010101,
    0b00000000011010100000000010010101,
    0b00000000010101010000000010101010,
    0b00000000100101010000000001101010,
    0b00000000011010100000000010010101,
    0b00000000011010100000000010010101,
    0b00000000100101010000000001101010,
    0b00000000100101010000000001101010,
    0b00000000010110100000000010100101,
    0b00000000011010100000000010010101,
    0b00000000100101010000000001101010,
    0b00000000100101010000000001101010,
    0b00000000010110100000000010100101,
    0b00000000010110100000000010100101,
    0b00000000100101010000000001101010,
    0b00000000101001010000000001011010,
    0b00000000010110100000000010100101,
    0b00000000010110100000000010100101,
    0b00000000100101010000000001101010,
    0b00000000101001010000000001011010,
    0b00000000010110100000000010100101,
    0b00000000010110100000000010100101,
    0b00000000101001010000000001011010,
    0b00000000101001010000000001011010,
    0b00000000010101100000000010101001,
    0b00000000010110100000000010100101,
    0b00000000101001010000000001011010,
    0b00000000101001010000000001011010,
    0b00000000010101100000000010101001,
    0b00000000010101100000000010101001,
    0b00000000101001010000000001011010,
    0b00000000101010010000000001010110,
    0b00000000010101100000000010101001,
    0b00000000010101100000000010101001,
    0b00000000101001010000000001011010,
    0b00000000101010010000000001010110,
    0b00000000010101100000000010101001,
    0b00000000010101100000000010101001,
    0b00000000101010010000000001010110,
    0b00000000101010010000000001010110,
    0b00000000010101010000000010101010,
    0b00000000010101100000000010101001,
    0b00000000101010010000000001010110,
    0b00000000101010010000000001010110,
    0b00000000010101010000000010101010,
    0b00000000010101010000000010101010,
    0b00000000101010010000000001010110,
    0b00000000101010100000000001010101,
    0b00000000010101010000000010101010,
    0b00000000010101010000000010101010,
    0b00000000101010010000000001010110,
    0b00000000101010100000000001010101,
    0b00000000010101010000000010101010,
    0b00000000010101010000000010101010,
    0b00000000101010100000000001010101,
    0b00000000101010100000000001010101,
    0b00000000010101010000000010101010,
    0b00000000010101010000000010101010,
    0b00000000101010100000000001010101,
    0b00000000101010100000000001010101,
    0b00000000100101010000000001101010,
    0b00000000010101010000000010101010,
    0b00000000101010100000000001010101,
    0b00000000101010100000000001010101,
    0b00000000100101010000000001101010,
    0b00000000100101010000000001101010,
    0b00000000101010100000000001010101,
    0b00000000011010100000000010010101,
    0b00000000100101010000000001101010,
    0b00000000100101010000000001101010,
    0b00000000101010100000000001010101,
    0b00000000011010100000000010010101,
    0b00000000100101010000000001101010,
    0b00000000100101010000000001101010,
    0b00000000011010100000000010010101,
    0b00000000011010100000000010010101,
    0b00000000101001010000000001011010,
    0b00000000100101010000000001101010,
    0b00000000011010100000000010010101,
    0b00000000011010100000000010010101,
    0b00000000101001010000000001011010,
    0b00000000101001010000000001011010,
    0b00000000011010100000000010010101,
    0b00000000010110100000000010100101,
    0b00000000101001010000000001011010,
    0b00000000101001010000000001011010,
    0b00000000011010100000000010010101,
    0b00000000010110100000000010100101,
    0b00000000101001010000000001011010,
    0b00000000101001010000000001011010,
    0b00000000010110100000000010100101,
    0b00000000010110100000000010100101,
    0b00000000101010010000000001010110,
    0b00000000101001010000000001011010,
    0b00000000010110100000000010100101,
    0b00000000010110100000000010100101,
    0b00000000101010010000000001010110,
    0b00000000101010010000000001010110,
    0b00000000010110100000000010100101,
    0b00000000010101100000000010101001,
    0b00000000101010010000000001010110,
    0b00000000101010010000000001010110,
    0b00000000010110100000000010100101,
    0b00000000010101100000000010101001,
    0b00000000101010010000000001010110,
    0b00000000101010010000000001010110,
    0b00000000010101100000000010101001,
    0b00000000010101100000000010101001,
    0b00000000101010100000000001010101,
    0b00000000101010010000000001010110,
    0b00000000010101100000000010101001,
    0b00000000010101100000000010101001,
    0b00000000101010100000000001010101,
    0b00000000101010100000000001010101,
    0b00000000010101100000000010101001,
    0b00000000010101010000000010101010,
    0b00000000101010100000000001010101,
    0b00000000101010100000000001010101,
    0b00000000010101100000000010101001,
    0b00000000010101010000000010101010,
    0b00000000101010100000000001010101,
    0b00000000101010100000000001010101,
    0b00000000010101010000000010101010,
    0b00000000010101010000000010101010,
    0b00000000011010100000000010010101,
    0b00000000101010100000000001010101,
];
