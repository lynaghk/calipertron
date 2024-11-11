#![no_std]
#![no_main]

use calipertron_core::*;

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::dma::*;
use embassy_stm32::gpio::{Flex, Input, Level, Output, Speed};
use embassy_stm32::peripherals::{DMA1_CH1, DMA1_CH2, TIM2};
use embassy_stm32::time::Hertz;
use embassy_stm32::timer::low_level::Timer;
use embassy_stm32::{adc, Config};

//use embassy_time::Duration;
use num_traits::Float;

use {defmt_rtt as _, panic_probe as _};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));
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
    let mut p = embassy_stm32::init(config);

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

    let mut tim = embassy_stm32::timer::low_level::Timer::new(p.TIM2);
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

    tim.set_frequency(Hertz(PDM_FREQUENCY));

    fn start_pdm<'a>(dma_ch: &'a mut DMA1_CH2, tim: &'_ mut Timer<'_, TIM2>) -> Transfer<'a> {
        unsafe {
            let mut opts = TransferOptions::default();
            opts.circular = true;

            tim.reset();

            let t = Transfer::new_write(
                dma_ch,
                (),
                &PDM_SIGNAL,
                embassy_stm32::pac::GPIOA.bsrr().as_ptr() as *mut u32,
                opts,
            );

            tim.start();
            t
        }
    }

    ////////////////////////
    // ADC + DMA setup

    fn start_adc<'a>(sample_buf: &'a mut [u16], dma_ch: &'a mut DMA1_CH1) -> Transfer<'a> {
        unsafe {
            let opts = TransferOptions::default();

            let t = Transfer::new_read(
                dma_ch,
                (),
                embassy_stm32::pac::ADC1.dr().as_ptr() as *mut u16,
                sample_buf,
                opts,
            );

            // Start ADC conversions
            embassy_stm32::pac::ADC1.cr2().modify(|w| w.set_adon(true));
            t
        }
    }

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
    adc.smpr2()
        .modify(|w| w.set_smp(PIN_CHANNEL as usize, adc::SampleTime::CYCLES41_5));

    let user_button = Input::new(p.PB14, embassy_stm32::gpio::Pull::None);

    let mut phase_accumulator = PhaseAccumulator::new(0.0, 0.1);

    // 9.4mm spacing across all 8 emission pads on the v1.1 PCB Mitko sent me.
    let distance_per_phase_cycle = 9.4;

    let adc_buf = &mut [0; NUM_SAMPLES];

    loop {
        let adc_transfer = start_adc(adc_buf, &mut p.DMA1_CH1);
        let mut pdm_transfer = start_pdm(&mut p.DMA1_CH2, &mut tim);
        // wait for all of the samples to be taken
        adc_transfer.await;
        pdm_transfer.request_stop();

        let mut sum_sine: i32 = 0;
        let mut sum_cosine: i32 = 0;

        for (&sample, (sine, cosine)) in adc_buf.iter().zip(SINE_COSINE_TABLE) {
            sum_sine += sample as i32 * sine as i32;
            sum_cosine += sample as i32 * cosine as i32;
        }

        let sum_sine = sum_sine as f32;
        let sum_cosine = sum_cosine as f32;

        let phase = sum_sine.atan2(sum_cosine);

        phase_accumulator.update(phase);
        info!(
            //"Phase: {:06.2} Position: {:06.2}",
            "Position: {}mm, Phase: {} ",
            phase_accumulator.unwrapped_phase
                * (distance_per_phase_cycle / (2.0 * core::f32::consts::PI)),
            phase,
        );

        // make sure everything is reset before we continue
        pdm_transfer.await;

        ///////////////////////
        // handle button press

        if user_button.is_low() {
            info!("Button pressed, zeroing");
            phase_accumulator.unwrapped_phase = 0.;
        }
    }
}
