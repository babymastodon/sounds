//! Modulation sources whose discontinuities need explicit antialiasing.

use std::f64::consts::PI;

use super::{lerp, poly_blep, smooth};

/// A hard-synced oscillator whose slave pitch bends inside each master cycle.
///
/// The noninteger ratios make every master reset cut the slave at a different
/// phase. Four-times oversampling followed by a Blackman-windowed low-pass
/// keeps those intentional reset edges bright without folding most of their
/// upper spectrum back into the audible band.
pub(super) fn sync_guillotine(sample_rate: u32, frames: usize) -> Vec<f32> {
    const OVERSAMPLE: usize = 4;
    const RATIOS: [f64; 5] = [1.25, 1.5, 1.618_033_988_749_895, 2.2, 3.01];

    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let oversampled_rate = sample_rate as f64 * OVERSAMPLE as f64;
    let mut master_phase = 0.0_f64;
    let mut decimator = Decimator4::new();

    for (frame, sample) in output.iter_mut().enumerate() {
        for subsample in 0..OVERSAMPLE {
            let oversampled_index = frame * OVERSAMPLE + subsample;
            let time = oversampled_index as f64 / oversampled_rate;

            // Hold each sync family almost to the boundary, then make a short
            // smooth move so the reset spectrum changes decisively by section.
            let position = time / 5.3;
            let section = position.floor() as usize;
            let section_phase = position.fract();
            let transition = smooth((section_phase - 0.955) / 0.045);
            let ratio = lerp(
                RATIOS[section % RATIOS.len()],
                RATIOS[(section + 1) % RATIOS.len()],
                transition,
            );

            let master_frequency = 67.0 + 5.0 * (2.0 * PI * time / 23.0).sin();
            let master_increment = master_frequency / oversampled_rate;

            // This monotonic phase warp integrates a pitch curve that is high
            // at the reset and low around the middle of each master cycle.
            let bend = 0.52 + 0.18 * (2.0 * PI * time / 17.3).sin();
            let master_angle = 2.0 * PI * master_phase;
            let warped_master = master_phase + bend * master_angle.sin() / (2.0 * PI);
            let slave_cycles = ratio * warped_master;
            let slave_phase = slave_cycles.fract();
            let slave_increment =
                (ratio * (1.0 + bend * master_angle.cos()) * master_increment).clamp(1.0e-9, 0.49);

            let sine = (2.0 * PI * slave_phase).sin();
            let second = (4.0 * PI * slave_phase).sin();
            let saw = 2.0 * slave_phase - 1.0 - poly_blep(slave_phase, slave_increment);
            decimator.push(0.63 * sine + 0.13 * second + 0.24 * saw);

            master_phase += master_increment;
            if master_phase >= 1.0 {
                master_phase -= 1.0;
            }
        }
        *sample = (decimator.output() * 0.48) as f32;
    }

    output
}

/// A bandlimited pulse oscillator with independently moving pitch and duty.
///
/// Analytic DC removal prevents the moving duty cycle from becoming a large
/// infrasonic offset, while RMS compensation keeps narrow and wide pulses
/// comparably present. PolyBLEP correction handles both variable pulse edges.
pub(super) fn pwm_cathedral(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let rate = sample_rate as f64;
    let slow_lfo = 1.0 / 11.3;
    let fast_lfo = slow_lfo * 29.0_f64.sqrt();
    let mut phase = 0.0_f64;

    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / rate;

        // Repeated logarithmic ascents and descents preserve the low register
        // while carrying the moving harmonic nulls through two-plus octaves.
        let sweep_phase = (time / 18.7).fract();
        let triangle = if sweep_phase < 0.5 {
            sweep_phase * 2.0
        } else {
            2.0 - sweep_phase * 2.0
        };
        let sweep = smooth(triangle);
        let frequency = 72.0_f64 * (310.0_f64 / 72.0).powf(sweep);
        let increment = (frequency / rate).clamp(1.0e-9, 0.24);

        let duty = (0.5
            + 0.27 * (2.0 * PI * slow_lfo * time).sin()
            + 0.11 * (2.0 * PI * fast_lfo * time + 0.73).sin())
        .clamp(0.1, 0.9);

        let mut pulse = if phase < duty { 1.0 } else { -1.0 };
        pulse += poly_blep(phase, increment);
        pulse -= poly_blep((phase + 1.0 - duty).fract(), increment);

        let centered = pulse - (2.0 * duty - 1.0);
        let energy_compensation = 0.22 / (duty * (1.0 - duty)).sqrt();
        *sample = (centered * energy_compensation) as f32;

        phase = (phase + increment).fract();
    }

    output
}

const DECIMATOR_TAPS: usize = 33;

struct Decimator4 {
    coefficients: [f64; DECIMATOR_TAPS],
    history: [f64; DECIMATOR_TAPS],
    cursor: usize,
}

impl Decimator4 {
    fn new() -> Self {
        let mut coefficients = [0.0_f64; DECIMATOR_TAPS];
        let center = (DECIMATOR_TAPS - 1) as f64 * 0.5;
        // Cycles per sample at the 4x rate. This leaves a transition band
        // before the output-rate Nyquist frequency at 0.125.
        let cutoff = 0.095_f64;
        let mut coefficient_sum = 0.0;
        for (index, coefficient) in coefficients.iter_mut().enumerate() {
            let offset = index as f64 - center;
            let sinc = if offset.abs() < f64::EPSILON {
                2.0 * cutoff
            } else {
                (2.0 * PI * cutoff * offset).sin() / (PI * offset)
            };
            let window_phase = 2.0 * PI * index as f64 / (DECIMATOR_TAPS - 1) as f64;
            let blackman = 0.42 - 0.5 * window_phase.cos() + 0.08 * (2.0 * window_phase).cos();
            *coefficient = sinc * blackman;
            coefficient_sum += *coefficient;
        }
        for coefficient in &mut coefficients {
            *coefficient /= coefficient_sum;
        }

        Self {
            coefficients,
            history: [0.0; DECIMATOR_TAPS],
            cursor: 0,
        }
    }

    fn push(&mut self, value: f64) {
        self.history[self.cursor] = value;
        self.cursor = (self.cursor + 1) % DECIMATOR_TAPS;
    }

    fn output(&self) -> f64 {
        self.coefficients
            .iter()
            .enumerate()
            .map(|(tap, &coefficient)| {
                let history_index = (self.cursor + DECIMATOR_TAPS - 1 - tap) % DECIMATOR_TAPS;
                coefficient * self.history[history_index]
            })
            .sum()
    }
}
