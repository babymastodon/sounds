// Additional deterministic sources built from spectrally controlled noise.

use std::f64::consts::PI;

use super::{Biquad, Rng, smooth};

/// Dense, Gaussian-windowed bands of noise whose centers sweep exponentially.
pub(super) fn chirplet_dust(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let rate = sample_rate as f64;
    let lowest = 55.0;
    // Leave room for each chirplet's baseband width and for later preparation.
    let highest = (rate * 0.40).min(16_000.0);
    if highest <= lowest * 1.5 {
        let mut rng = Rng::new(0xc417_d057_4eaf_1901);
        for sample in &mut output {
            *sample = rng.bipolar() as f32 * 0.1;
        }
        return output;
    }

    // Parameter and excitation streams are separate so that the event pattern
    // remains stable when the same source is rendered at another sample rate.
    let mut parameter_rng = Rng::new(0xc417_d057_4eaf_1901);
    let mut noise_rng = Rng::new(0x9d35_72b1_0cc4_a681);
    let available_octaves = (highest / lowest).log2();
    let mut start_frame = 0_usize;

    while start_frame < frames {
        let duration = 0.040 + 0.080 * parameter_rng.unit();
        let length = (duration * rate).round().max(8.0) as usize;
        let span_octaves = (0.5 + 1.5 * parameter_rng.unit()).min(available_octaves * 0.72);
        let half_ratio = 2.0_f64.powf(span_octaves * 0.5);
        let minimum_midpoint = lowest * half_ratio;
        let maximum_midpoint = highest / half_ratio;
        let midpoint =
            minimum_midpoint * (maximum_midpoint / minimum_midpoint).powf(parameter_rng.unit());
        let (start_frequency, end_frequency) = if parameter_rng.unit() < 0.5 {
            (midpoint / half_ratio, midpoint * half_ratio)
        } else {
            (midpoint * half_ratio, midpoint / half_ratio)
        };

        // Quadrature modulation of two low-passed random streams gives a true
        // noise band around the moving center rather than a disguised tone.
        let bandwidth = (midpoint * (0.045 + 0.040 * parameter_rng.unit())).clamp(18.0, 280.0);
        let smoothing = 1.0 - (-2.0 * PI * bandwidth / rate).exp();
        let noise_normalizer = (3.0 * (2.0 - smoothing) / smoothing).sqrt();
        let mut in_phase = 0.0_f64;
        let mut quadrature = 0.0_f64;
        let mut phase = 2.0 * PI * parameter_rng.unit();
        let frequency_ratio = end_frequency / start_frequency;
        let sigma = 0.16_f64;
        let window_floor = (-0.5_f64 * (0.5 / sigma).powi(2)).exp();

        for grain_index in 0..length {
            let target = start_frame + grain_index;
            if target >= frames {
                break;
            }
            let position = grain_index as f64 / (length - 1) as f64;
            let centered = (position - 0.5) / sigma;
            let gaussian = (-0.5 * centered * centered).exp();
            let window = (gaussian - window_floor) / (1.0 - window_floor);
            let frequency = start_frequency * frequency_ratio.powf(position);

            in_phase += smoothing * (noise_rng.bipolar() - in_phase);
            quadrature += smoothing * (noise_rng.bipolar() - quadrature);
            phase += 2.0 * PI * frequency / rate;
            let band_noise = in_phase * phase.cos() - quadrature * phase.sin();
            output[target] += (0.095 * window * noise_normalizer * band_noise) as f32;
        }

        // 12--35 ms hops yield several simultaneous chirplets without turning
        // their diagonal time-frequency traces into a single stationary wash.
        let hop = 0.012 + 0.023 * parameter_rng.unit();
        start_frame = start_frame.saturating_add((hop * rate).round().max(1.0) as usize);
    }

    output
}

/// Narrow noise bands on a lattice that morphs between three ratio systems.
pub(super) fn noisy_harmonic_sieve(sample_rate: u32, frames: usize) -> Vec<f32> {
    const BANDS: usize = 18;
    const PRIMES: [f64; BANDS] = [
        2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0, 37.0, 41.0, 43.0, 47.0, 53.0,
        59.0, 61.0,
    ];

    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let rate = sample_rate as f64;
    let cutoff = rate * 0.43;
    let mut filters = [Biquad::default(); BANDS];
    let mut rngs = [Rng::new(1); BANDS];
    let mut gains = [0.0_f64; BANDS];
    for (band, rng) in rngs.iter_mut().enumerate() {
        let salt = (band as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        *rng = Rng::new(0x519e_7e11_9a2d_403b ^ salt);
    }

    for (index, sample) in output.iter_mut().enumerate() {
        if index % 64 == 0 {
            let time = index as f64 / rate;
            let lattice_position = time / 6.7;
            let left_mode = lattice_position.floor() as usize % 3;
            let right_mode = (left_mode + 1) % 3;
            let mode_mix = smooth(lattice_position.fract());
            // Moving the fundamental lets the same ratio lattices inspect low,
            // middle, and high spectral regions over the full source.
            let register = 0.5 - 0.5 * (2.0 * PI * time / 19.7).cos();
            let fundamental = 55.0 * 7.5_f64.powf(register);

            for band in 0..BANDS {
                let harmonic = band as f64 + 1.0;
                let stretched = harmonic.powf(1.16);
                let prime = PRIMES[band] * 0.5;
                let ratios = [harmonic, stretched, prime];
                // Log interpolation keeps every trajectory positive and makes
                // the perceptual movement even across large ratio changes.
                let ratio = (ratios[left_mode].ln()
                    + (ratios[right_mode].ln() - ratios[left_mode].ln()) * mode_mix)
                    .exp();
                let frequency = fundamental * ratio;
                filters[band].set_bandpass(rate, frequency.min(cutoff), 16.0);

                // Fade rather than clamp bands that cross the safe high edge,
                // avoiding a pile-up of filters at Nyquist on 4 kHz renders.
                let fade_start = cutoff * 0.78;
                gains[band] = if frequency <= fade_start {
                    1.0
                } else if frequency >= cutoff {
                    0.0
                } else {
                    1.0 - smooth((frequency - fade_start) / (cutoff - fade_start))
                };
            }
        }

        let mut sum = 0.0_f32;
        for band in 0..BANDS {
            let excitation = rngs[band].bipolar() as f32;
            let spectral_tilt = 1.0 / (1.0 + band as f32 * 0.025);
            sum += filters[band].process(excitation) * gains[band] as f32 * spectral_tilt;
        }
        *sample = sum * 0.42;
    }

    output
}
