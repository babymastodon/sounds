use std::f64::consts::{PI, TAU};

/// An endlessly ascending, octave-coherent Shepard trajectory.
pub(super) fn shepard_corkscrew(sample_rate: u32, frames: usize) -> Vec<f32> {
    if sample_rate == 0 {
        return vec![0.0; frames];
    }

    const VOICES: usize = 9;
    const LOW_HZ: f64 = 70.0;
    const HIGH_HZ: f64 = 16_000.0;

    let rate = sample_rate as f64;
    let high = HIGH_HZ.min(rate * 0.43);
    let low = LOW_HZ.min(high * 0.25);
    let octave_span = (high / low).log2();

    // Choosing an integer number of base-frequency turns per climb makes the
    // octave re-indexing phase coherent at every nominal wrap point.
    let turns_per_climb = 1_111.0;
    let climb_seconds = turns_per_climb * 2.0_f64.ln() / low;
    let mut output = vec![0.0; frames];

    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / rate;
        let climb = (time / climb_seconds).fract();
        let base_phase = TAU * turns_per_climb * (2.0_f64.powf(climb) - 1.0);
        let phase_rotation = time / 17.0;
        let mut sum = 0.0;

        for octave in 0..VOICES {
            let log_position = octave as f64 + climb;
            if log_position >= octave_span {
                continue;
            }

            // This window vanishes at both band edges, hiding each voice's
            // reappearance while keeping adjacent octave lanes complementary.
            let window = (PI * log_position / octave_span).sin().powi(2);
            let rotating_offset =
                0.68 * (TAU * (phase_rotation + log_position / octave_span)).sin();
            let octave_phase = base_phase * 2.0_f64.powi(octave as i32);
            sum += window * (octave_phase + rotating_offset).sin();
        }

        *sample = (sum * 0.18) as f32;
    }

    output
}

/// A bounded quasi-periodic FM path with continually changing revisit phases.
pub(super) fn rosette_fm(sample_rate: u32, frames: usize) -> Vec<f32> {
    if sample_rate == 0 {
        return vec![0.0; frames];
    }

    let rate = sample_rate as f64;
    let high = 11_000.0_f64.min(rate * 0.43);
    let low = 120.0_f64.min(high * 0.25);
    let center = 1_400.0_f64.min(high * 0.78).max(low * 1.01);
    let root_two = 2.0_f64.sqrt();
    let golden_ratio = (1.0 + 5.0_f64.sqrt()) * 0.5;
    let mut carrier_phase = 0.0_f64;
    let mut output = vec![0.0; frames];

    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / rate;
        let angle_a = TAU * time / 7.3;
        let angle_b = TAU * time / (7.3 * root_two);
        let angle_c = TAU * time / (7.3 * golden_ratio);

        // Irrationally related, mutually nested modulators make a path that
        // resembles itself without closing or returning with the same slope.
        let petal_a = (angle_a + 0.82 * angle_b.sin()).sin();
        let petal_b = (angle_b + 0.61 * angle_c.sin() + 0.24 * angle_a.cos()).sin();
        let trajectory = (0.67 * petal_a + 0.33 * petal_b).clamp(-1.0, 1.0);

        let frequency = if trajectory >= 0.0 {
            center * (high / center).powf(trajectory)
        } else {
            center * (center / low).powf(trajectory)
        };
        carrier_phase = (carrier_phase + TAU * frequency / rate).rem_euclid(TAU);

        // A separate quasi-periodic phase field gives frequency revisits
        // different phase histories and produces the fine sideband lattice.
        let revisit_phase = 0.72 * (angle_a - angle_c).sin() + 0.23 * (angle_b + angle_c).sin();
        let breathing = 0.82 + 0.18 * (angle_a - angle_b).cos();
        *sample = (0.62 * breathing * (carrier_phase + revisit_phase).sin()) as f32;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::{rosette_fm, shepard_corkscrew};

    fn assert_usable(generator: fn(u32, usize) -> Vec<f32>, sample_rate: u32) {
        let frames = sample_rate as usize * 2;
        let samples = generator(sample_rate, frames);
        assert_eq!(samples.len(), frames);
        assert!(samples.iter().all(|sample| sample.is_finite()));
        assert!(samples.iter().map(|sample| sample.abs()).sum::<f32>() > 1.0);
    }

    #[test]
    fn generators_are_finite_and_non_silent_at_supported_rates() {
        for sample_rate in [4_000, 48_000] {
            assert_usable(shepard_corkscrew, sample_rate);
            assert_usable(rosette_fm, sample_rate);
        }
    }
}
