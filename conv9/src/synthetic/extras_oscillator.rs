//! Additional oscillator-only sources for the synthetic catalog.

use std::f64::consts::PI;

use super::{lerp, smooth};

/// A steady carrier whose phase origin is reset at Golomb-ruler marks.
///
/// Even-numbered constellations pull the phase to its new origin over 18 ms,
/// retaining a continuous waveform. Odd-numbered constellations reset it in a
/// single sample, deliberately adding broadband impulses to the carrier.
pub(super) fn phase_reset_constellation(sample_rate: u32, frames: usize) -> Vec<f32> {
    const GOLOMB_MARKS: [usize; 6] = [0, 1, 4, 10, 12, 17];
    const RESET_PHASES: [f64; 8] = [
        0.0,
        0.71 * PI,
        1.43 * PI,
        0.18 * PI,
        1.77 * PI,
        0.93 * PI,
        1.25 * PI,
        0.46 * PI,
    ];

    let rate = sample_rate.max(1) as f64;
    let constellation_frames = (5.7 * rate).round().max(1.0) as usize;
    let ruler_span_frames = (4.35 * rate).round().max(1.0) as usize;
    let correction_frames = (0.018 * rate).round().max(2.0) as usize;

    let constellation_count = frames.div_ceil(constellation_frames);
    let mut events = Vec::with_capacity(constellation_count * GOLOMB_MARKS.len());
    for constellation in 0..constellation_count {
        let origin = constellation * constellation_frames;
        for (mark_index, mark) in GOLOMB_MARKS.iter().copied().enumerate() {
            let offset = (mark * ruler_span_frames + 8) / 17;
            let frame = origin + offset;
            if frame < frames {
                let phase_index = constellation * GOLOMB_MARKS.len() + mark_index;
                events.push((
                    frame,
                    RESET_PHASES[phase_index % RESET_PHASES.len()],
                    constellation % 2 == 0,
                ));
            }
        }
    }

    let carrier_step = 2.0 * PI * 997.0 / rate;
    let mut phase = 0.0_f64;
    let mut event_index = 0_usize;
    let mut correction_delta = 0.0_f64;
    let mut correction_position = correction_frames;
    let mut output = vec![0.0; frames];

    for (frame, sample) in output.iter_mut().enumerate() {
        if let Some(&(_, target_phase, clickless)) =
            events.get(event_index).filter(|event| event.0 == frame)
        {
            if clickless {
                // The shortest signed phase displacement avoids an audible
                // step while still establishing the prescribed phase origin.
                correction_delta = (target_phase - phase + PI).rem_euclid(2.0 * PI) - PI;
                correction_position = 0;
            } else {
                phase = target_phase;
                correction_position = correction_frames;
                correction_delta = 0.0;
            }
            event_index += 1;
        }

        *sample = (0.55 * phase.sin()) as f32;

        let correction = if correction_position < correction_frames {
            let left = correction_position as f64 / correction_frames as f64;
            correction_position += 1;
            let right = correction_position as f64 / correction_frames as f64;
            correction_delta * (smooth(right) - smooth(left))
        } else {
            0.0
        };
        phase = (phase + carrier_step + correction).rem_euclid(2.0 * PI);
    }

    output
}

/// Two oscillators coupled through bounded, slowly changing mutual FM.
///
/// The coupling curve passes through nearly pure, sideband-rich, phase-locked,
/// and strongly nonlinear regions. A short state memory makes the high-depth
/// regions knot rather than reduce to ordinary two-operator FM.
pub(super) fn cross_fm_knot(sample_rate: u32, frames: usize) -> Vec<f32> {
    const COUPLING_KNOTS: [f64; 8] = [0.03, 0.19, 0.57, 0.91, 0.36, 1.0, 0.68, 0.08];

    let rate = sample_rate.max(1) as f64;
    // Keep every instantaneous oscillator frequency comfortably below Nyquist
    // even in the 4 kHz catalog tests. At 48 kHz the cap is the intended 3 kHz.
    let maximum_deviation = (rate * 0.41 - 223.0).clamp(18.0, 3_000.0);
    let mut phase_a = 0.17 * PI;
    let mut phase_b = 1.31 * PI;
    let mut last_a = phase_a.sin();
    let mut last_b = phase_b.sin();
    let mut memory_a = last_a;
    let mut memory_b = last_b;
    let mut output = vec![0.0; frames];

    for (frame, sample) in output.iter_mut().enumerate() {
        let time = frame as f64 / rate;
        let knot_position = time / 6.7;
        let knot = knot_position.floor() as usize % COUPLING_KNOTS.len();
        let blend = smooth(knot_position.fract());
        let coupling = lerp(
            COUPLING_KNOTS[knot],
            COUPLING_KNOTS[(knot + 1) % COUPLING_KNOTS.len()],
            blend,
        );
        let deviation = lerp(18.0, maximum_deviation, coupling);

        // Cross-products and leaky state provide history-dependent bends while
        // clamps guarantee bounded increments under every supported rate.
        let cross_product = last_a * last_b;
        let modulator_a = (0.74 * last_b + 0.24 * memory_b + 0.18 * cross_product).clamp(-1.0, 1.0);
        let modulator_b = (0.69 * last_a - 0.23 * memory_a - 0.21 * cross_product).clamp(-1.0, 1.0);
        let base_a = 137.0 + 5.0 * (2.0 * PI * time / 29.0).sin();
        let base_b = 223.0 + 8.0 * (2.0 * PI * time / 37.0).cos();
        let frequency_a = (base_a + deviation * modulator_a).clamp(-rate * 0.44, rate * 0.44);
        let frequency_b =
            (base_b + deviation * 0.83 * modulator_b).clamp(-rate * 0.44, rate * 0.44);

        phase_a = (phase_a + 2.0 * PI * frequency_a / rate).rem_euclid(2.0 * PI);
        phase_b = (phase_b + 2.0 * PI * frequency_b / rate).rem_euclid(2.0 * PI);
        last_a = phase_a.sin();
        last_b = phase_b.sin();
        memory_a += 0.0061 * (last_a + 0.37 * cross_product - memory_a);
        memory_b += 0.0047 * (last_b - 0.31 * cross_product - memory_b);

        *sample = (0.31 * last_a + 0.27 * last_b + 0.10 * cross_product) as f32;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oscillator_extras_are_finite_and_non_silent_at_catalog_rates() {
        for rate in [4_000, 48_000] {
            for generator in [phase_reset_constellation, cross_fm_knot] {
                let samples = generator(rate, rate as usize * 2);
                assert!(samples.iter().all(|sample| sample.is_finite()));
                let energy = samples.iter().map(|sample| sample * sample).sum::<f32>();
                assert!(energy > 1.0e-8);
            }
        }
    }
}
