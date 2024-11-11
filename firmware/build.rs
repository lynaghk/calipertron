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
    // TODO: a full sine/cosine table is wasteful, we can save memory by just
    // storing a quarter sine wave and flipping/reversing the values as needed.

    let mut output = String::new();
    output.push_str("pub const SINE_COSINE_TABLE: [(i16, i16); ");
    output.push_str(&num_samples.to_string());
    output.push_str("] = [\n");

    let adc_bits = 12;
    let max_adc = (1 << adc_bits) - 1;
    let amplitude =
        (i32::MAX as f64 * std::f64::consts::PI) / (max_adc as f64 * num_samples as f64);
    // Using this amplitude can cause overflows, probably due to rounding
    // and because our table isn't exactly one wavelength.
    // We can fix it by scaling down a bit.
    let amplitude = amplitude * 0.999;

    let f_to_i16 = |x: f64| (x * amplitude).round() as i16;

    // Just checking the max values, not the min values.
    // It's probably fine.
    let mut max_sine: i64 = 0;
    let mut max_cosine: i64 = 0;

    for i in 0..num_samples {
        let angle = 2.0 * PI * signal_frequency * (i as f64 * (1.0 / sampling_frequency));
        let sine = f_to_i16(angle.sin());
        let cosine = f_to_i16(angle.cos());
        output.push_str(&format!("    ({:?}, {:?}),\n", sine, cosine));

        max_sine += sine as i64 * (if sine > 0 { max_adc as i64 } else { 0 });
        max_cosine += cosine as i64 * (if cosine > 0 { max_adc as i64 } else { 0 });
    }

    assert!(max_sine <= i32::MAX as i64);
    assert!(max_cosine <= i32::MAX as i64);
    // We also want to make sure we are using as much of the range as we can.
    let ninetynine_percent = (i32::MAX as i64 * 99) / 100;
    assert!(max_sine >= ninetynine_percent);
    assert!(max_cosine >= ninetynine_percent);

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

    let pdm_frequency: u32 = 222_000;
    f.write_all(format!("pub const PDM_FREQUENCY: u32 = {:?};\n", pdm_frequency).as_bytes())
        .unwrap();

    let pdm_length = 128;
    let num_samples = 128;

    let signal_frequency = pdm_frequency as f64 / pdm_length as f64;
    let adc_frequency = 12_000_000.;
    // let adc_sample_cycles = 239.5;
    // let adc_sample_cycles = 71.5;
    let adc_sample_cycles = 41.5;
    let adc_sample_overhead_cycles = 12.5; // see reference manual section 11.6
    let sampling_frequency = adc_frequency / (adc_sample_cycles + adc_sample_overhead_cycles);

    f.write_all(
        generate_sine_cosine_table(signal_frequency, sampling_frequency, num_samples).as_bytes(),
    )
    .unwrap();

    f.write_all(generate_pdm_bsrr(pdm_length).as_bytes())
        .unwrap();

    // Tell Cargo to rerun this script if the source file changes
    println!("cargo:rerun-if-changed=build.rs");
}
