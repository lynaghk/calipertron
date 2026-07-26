use std::f64::consts::PI;
use std::fs::File;
use std::io::Write;

fn generate_pdm_bsrr(n_samples: usize) -> String {
    let mut output = String::new();
    output.push_str("pub const PDM_SIGNAL: [u32; ");
    output.push_str(&n_samples.to_string());
    output.push_str("] = [\n");

    let n_waves = 8;

    // in PCB schematic v1.1 the pins PA0--PA7 are wired up for signal idx 0,4, 1,5, 2,6, 3,7
    let mut wave_idx_to_pin = [0; 8];
    for (pin_idx, wave_idx) in [0, 4, 1, 5, 2, 6, 3, 7].into_iter().enumerate() {
        wave_idx_to_pin[wave_idx] = pin_idx;
    }

    let mut errors = vec![0.0; n_waves];
    for sample in 0..n_samples {
        let mut bsrr = 0u32;
        for wave in 0..n_waves {
            let phase_offset = 2.0 * PI * (wave as f64) / (n_waves as f64);
            let angle = 2.0 * PI * (sample as f64 / n_samples as f64) + phase_offset;
            let cosine = angle.cos() as f32;
            let normalized_signal = (cosine + 1.0) / 2.0;

            // rescale
            let scale = 1.0;
            let normalized_signal = (1.0 - scale) / 2.0 + scale * normalized_signal;

            if normalized_signal > errors[wave] {
                bsrr |= 1 << wave_idx_to_pin[wave]; // set bit
                errors[wave] += 1.0 - normalized_signal;
            } else {
                bsrr |= 1 << (wave_idx_to_pin[wave] + 16); // reset bit
                errors[wave] -= normalized_signal;
            }
        }
        output.push_str(&format!("    {:#034b},\n", bsrr));
    }

    output.push_str("];\n");
    output
}

fn generate_sine_cosine_table(
    signal_frequency: f64,
    sampling_frequency: f64,
    num_samples: usize,
) -> String {
    let mut output = String::new();
    output.push_str("pub const SINE_COSINE_TABLE: [(f32, f32); ");
    output.push_str(&num_samples.to_string());
    output.push_str("] = [\n");

    for i in 0..num_samples {
        let angle = 2.0 * PI * signal_frequency * (i as f64 * (1.0 / sampling_frequency));
        // Hann-weight the correlation so interference at frequencies other than the signal
        // integrates to near zero instead of leaking into the phase estimate. Periodic
        // window (denominator N, like scipy's hann(sym=False)), not the symmetric N-1
        // variant: its response nulls land exactly on the coherent bin frequencies, so the
        // signal's own harmonics stay fully rejected.
        let hann = 0.5 - 0.5 * (2.0 * PI * i as f64 / num_samples as f64).cos();
        let sine = (hann * angle.sin()) as f32;
        let cosine = (hann * angle.cos()) as f32;
        output.push_str(&format!("    ({:?}, {:?}),\n", sine, cosine));
    }

    output.push_str("];\n");
    output
}

fn main() {
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = std::path::Path::new(&out_dir).join("constants.rs");
    let mut f = File::create(&dest_path).unwrap();

    // The PDM timer can only divide the 72 MHz timer clock by an integer, so the divider is
    // the source of truth and all frequencies derive from it. With divider 324, table length
    // 128, and the ADC at CYCLES41_5 (12 MHz / 54 = 222222.2 Hz) everything is coherent:
    // the ADC takes exactly one sample per PDM tick and 128 samples per signal cycle, so a
    // window integrates whole cycles and DC offset can't leak into the phase.
    let timer_clock: f64 = 72_000_000.;
    let pdm_timer_divider: u32 = 324;
    let pdm_frequency = timer_clock / pdm_timer_divider as f64; // 222222.2 Hz

    // Emitted as integer Hz for embassy's set_frequency, which floor-divides the timer
    // clock: 72e6 / 222222 = 324.0003 -> divider 324, reproducing the intended rate.
    let pdm_frequency_hz = pdm_frequency.floor() as u32;
    assert_eq!(
        timer_clock as u32 / pdm_frequency_hz,
        pdm_timer_divider,
        "rounded PDM_FREQUENCY must floor-divide back to the intended timer divider"
    );
    f.write_all(format!("pub const PDM_FREQUENCY: u32 = {:?};\n", pdm_frequency_hz).as_bytes())
        .unwrap();

    let pdm_length = 128;
    // Two coherent signal cycles per demodulation window: correlating over more samples
    // averages down random phase noise, and the resulting 868 Hz update rate is still far
    // faster than the motion being measured.
    let num_samples = 256;

    let signal_frequency = pdm_frequency / pdm_length as f64; // 1736.1 Hz
    let adc_frequency = 12_000_000.;
    // let adc_sample_cycles = 239.5;
    // let adc_sample_cycles = 71.5;
    let adc_sample_cycles = 41.5;
    let adc_sample_overhead_cycles = 12.5; // see reference manual section 11.6
    let sampling_frequency = adc_frequency / (adc_sample_cycles + adc_sample_overhead_cycles);

    let samples_per_cycle = sampling_frequency / signal_frequency;
    assert!(
        (samples_per_cycle - samples_per_cycle.round()).abs() < 1e-9,
        "ADC sampling is not coherent with the signal: {} samples per cycle",
        samples_per_cycle
    );

    f.write_all(
        generate_sine_cosine_table(signal_frequency, sampling_frequency, num_samples).as_bytes(),
    )
    .unwrap();

    f.write_all(generate_pdm_bsrr(pdm_length).as_bytes())
        .unwrap();

    // Tell Cargo to rerun this script if the source file changes
    println!("cargo:rerun-if-changed=build.rs");
}
