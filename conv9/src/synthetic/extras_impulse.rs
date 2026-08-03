//! Additional deterministic impulse-constellation sources.

use std::f64::consts::PI;

/// Two incommensurate, sinusoidally jittered click lattices.
///
/// The second period is the first multiplied by the golden ratio, so their
/// coincidences continually drift.  A golden-angle sign code prevents the
/// result from reducing to a pair of unipolar combs.
pub(super) fn golden_lattice_moire(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    const PHI: f64 = 1.618_033_988_749_895;
    let duration = frames as f64 / sample_rate as f64;
    let periods = [0.037, 0.037 * PHI];
    let starts = [0.137, 0.151];
    let jitter_depths = [0.0022, 0.0031];
    let jitter_periods = [11.3, 17.9];
    let jitter_phases = [0.31, 2.17];
    let gains = [0.72, 0.58];

    for lattice in 0..periods.len() {
        let mut event = 0_usize;
        loop {
            let nominal_time = starts[lattice] + event as f64 * periods[lattice];
            if nominal_time >= duration {
                break;
            }
            let jitter_phase =
                2.0 * PI * nominal_time / jitter_periods[lattice] + jitter_phases[lattice];
            let time = nominal_time + jitter_depths[lattice] * jitter_phase.sin();

            // Treat events as one-indexed so the first sign is well-defined.
            let golden_phase = 2.0 * PI * (event + 1) as f64 / PHI;
            let polarity = if golden_phase.sin() >= 0.0 { 1.0 } else { -1.0 };
            add_exponential_click(
                &mut output,
                sample_rate,
                time,
                0.0008,
                polarity * gains[lattice],
            );
            event += 1;
        }
    }

    output
}

/// A shifted quadratic-residue support carrying a second Legendre sign code.
///
/// Every 997-sample block rotates the residue support by the next prime.  The
/// sign at the rotated location is its Legendre symbol, and each event excites
/// a nine-tap, front-loaded minimum-phase kernel.
pub(super) fn quadratic_residue_starfield(_sample_rate: u32, frames: usize) -> Vec<f32> {
    const MODULUS: usize = 997;
    const KERNEL_RATIO: f32 = 0.55;

    let mut output = vec![0.0; frames];
    if frames == 0 {
        return output;
    }

    let legendre: [i8; MODULUS] = std::array::from_fn(legendre_symbol);
    let kernel: [f32; 9] = std::array::from_fn(|tap| KERNEL_RATIO.powi(tap as i32));
    let mut prime_candidate = 1_usize;

    for block_start in (0..frames).step_by(MODULUS) {
        let shift = next_prime(&mut prime_candidate) % MODULUS;
        let block_length = (frames - block_start).min(MODULUS);

        for local in 0..block_length {
            // Undo the block rotation to test membership in the base residue
            // support.  Zero is excluded, leaving exactly 498 marks per full
            // block before the end-of-file truncation.
            let canonical = (local + MODULUS - shift) % MODULUS;
            if legendre[canonical] != 1 {
                continue;
            }

            // The additive rotation makes this independent of the support's
            // all-positive Legendre value.  A rare zero takes block parity.
            let polarity = match legendre[local] {
                -1 => -1.0,
                1 => 1.0,
                _ if (block_start / MODULUS) & 1 == 0 => 1.0,
                _ => -1.0,
            };
            let event_start = block_start + local;
            for (tap, &coefficient) in kernel.iter().enumerate() {
                if let Some(sample) = output.get_mut(event_start + tap) {
                    *sample += polarity * coefficient * 0.16;
                }
            }
        }
    }

    output
}

fn add_exponential_click(
    output: &mut [f32],
    sample_rate: u32,
    time: f64,
    width: f64,
    amplitude: f64,
) {
    let start = (time * sample_rate as f64).round() as isize;
    let length = (width * sample_rate as f64).round().max(4.0) as usize;
    for index in 0..length {
        let target = start + index as isize;
        if target < 0 || target as usize >= output.len() {
            continue;
        }
        let position = index as f64 / (length - 1) as f64;
        // Roughly 40 dB of decay across the compact causal kernel.
        let envelope = (-position * 100.0_f64.ln()).exp();
        output[target as usize] += (amplitude * envelope) as f32;
    }
}

fn legendre_symbol(value: usize) -> i8 {
    const MODULUS: usize = 997;
    let value = value % MODULUS;
    if value == 0 {
        return 0;
    }

    // Euler's criterion; MODULUS is prime.
    let mut exponent = (MODULUS - 1) / 2;
    let mut base = value;
    let mut result = 1_usize;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result * base % MODULUS;
        }
        base = base * base % MODULUS;
        exponent >>= 1;
    }
    if result == 1 { 1 } else { -1 }
}

fn next_prime(candidate: &mut usize) -> usize {
    let mut value = (*candidate + 1).max(2);
    while !is_prime(value) {
        value += 1;
    }
    *candidate = value;
    value
}

fn is_prime(value: usize) -> bool {
    if value < 2 {
        return false;
    }
    if value & 1 == 0 {
        return value == 2;
    }
    let mut divisor = 3_usize;
    while divisor <= value / divisor {
        if value.is_multiple_of(divisor) {
            return false;
        }
        divisor += 2;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_extras_are_finite_and_non_silent_at_low_sample_rate() {
        let frames = 4_000 * 61;
        for samples in [
            golden_lattice_moire(4_000, frames),
            quadratic_residue_starfield(4_000, frames),
        ] {
            assert_eq!(samples.len(), frames);
            assert!(samples.iter().all(|sample| sample.is_finite()));
            assert!(samples.iter().map(|sample| sample * sample).sum::<f32>() > 1.0e-8);
        }
    }
}
