use std::f64::consts::PI;

/// A family of plucked digital waveguides whose high partials take a longer
/// trip around the loop than their fundamentals.
///
/// The fractional delay keeps the pitches independent of the sample rate. The
/// cascaded first-order all-passes have coefficients strictly inside the unit
/// circle, and every trip around the waveguide loses energy, so the recirculating
/// state remains bounded.
pub(super) fn dispersive_waveguide_plucks(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let duration = frames as f64 / sample_rate as f64;
    let pitches: [f64; 7] = [55.0, 73.42, 98.0, 130.81, 174.61, 220.0, 146.83];
    let spacings = [3.31, 4.07, 3.67, 4.43];
    let mut event = 0_usize;
    let mut event_time = 0.08_f64;

    while event_time < duration {
        let nyquist_safe = sample_rate as f64 * 0.18;
        let pitch = pitches[event % pitches.len()].min(nyquist_safe.max(24.0));
        let dispersion = 0.20 + 0.10 * (event % 6) as f64;
        let decay_seconds = 3.8 + 0.55 * (event % 5) as f64;
        let render_seconds = (decay_seconds * 1.45).min(duration - event_time);
        render_waveguide_pluck(
            &mut output,
            sample_rate,
            event_time,
            render_seconds,
            pitch,
            dispersion,
            decay_seconds,
            0x8a5c_2d71_4f03_b69du64 ^ (event as u64).wrapping_mul(0x9e37_79b9),
        );
        event_time += spacings[event % spacings.len()];
        event += 1;
    }

    output
}

#[allow(clippy::too_many_arguments)]
fn render_waveguide_pluck(
    output: &mut [f32],
    sample_rate: u32,
    start_seconds: f64,
    render_seconds: f64,
    pitch: f64,
    dispersion: f64,
    decay_seconds: f64,
    seed: u64,
) {
    if render_seconds <= 0.0 {
        return;
    }

    let rate = sample_rate as f64;
    let start = (start_seconds * rate).round() as usize;
    let render_frames = (render_seconds * rate).round() as usize;
    let allpass_count = 2 + ((dispersion * 4.0).round() as usize).min(2);
    // H(z) = (a + z^-1) / (1 + a z^-1). Positive `a` gives the
    // stiffness-like increase in delay toward Nyquist that we want here.
    let allpass_coefficient = (0.12 + dispersion * 0.62).clamp(0.12, 0.72) as f32;
    let allpass_dc_delay = allpass_count as f64 * (1.0 - allpass_coefficient as f64)
        / (1.0 + allpass_coefficient as f64);
    let damping_feed = 0.58_f64;
    let damping_dc_delay = (1.0 - damping_feed) / damping_feed;
    let desired_period = rate / pitch.max(1.0);
    let delay_samples = (desired_period - allpass_dc_delay - damping_dc_delay).max(4.0);
    let delay_integer = delay_samples.floor() as usize;
    let delay_fraction = (delay_samples - delay_integer as f64) as f32;
    let ring_length = delay_integer + 3;
    let mut ring = vec![0.0_f32; ring_length];
    let mut rng = PhaseRng::new(seed);

    // A gently differentiated noise burst makes the initial pick broadband
    // without filling the loop with full-scale white-noise discontinuities.
    let mut previous_noise = 0.0_f32;
    let ring_len = ring.len() as f64;
    for (index, slot) in ring.iter_mut().enumerate() {
        let noise = rng.bipolar();
        let pick_position = index as f64 / (ring_len - 1.0).max(1.0);
        let pick_window = (PI * pick_position).sin().powi(2) as f32;
        *slot = (noise * 0.72 + previous_noise * 0.28) * pick_window * 0.34;
        previous_noise = noise;
    }

    let mut allpasses = vec![FirstOrderAllpass::new(allpass_coefficient); allpass_count];
    let trips_in_decay = (pitch * decay_seconds).max(1.0);
    let loop_gain = 0.001_f64.powf(1.0 / trips_in_decay).clamp(0.90, 0.999_95) as f32;
    let mut damping_state = 0.0_f32;
    let mut write = 0_usize;
    let end = (start + render_frames).min(output.len());

    for sample in output.iter_mut().take(end).skip(start) {
        // Both taps are already-written samples. Linear interpolation is a
        // convex combination, so the fractional-delay read cannot overshoot.
        let recent = (write + ring_length - delay_integer % ring_length) % ring_length;
        let older = (recent + ring_length - 1) % ring_length;
        let delayed = ring[recent] * (1.0 - delay_fraction) + ring[older] * delay_fraction;

        let mut dispersed = delayed;
        for allpass in &mut allpasses {
            dispersed = allpass.process(dispersed);
        }
        damping_state += damping_feed as f32 * (dispersed - damping_state);
        let recirculated = damping_state * loop_gain;
        ring[write] = if recirculated.is_finite() {
            recirculated
        } else {
            0.0
        };
        write = (write + 1) % ring_length;

        // Emphasize the dispersive pick transient while retaining the body.
        let pick_edge = dispersed - damping_state;
        *sample += (damping_state * 0.52 + pick_edge * 0.20) * 0.58;
    }
}

/// A feed-forward fractional-delay comb whose notch spacing and polarity move
/// continuously over broadband noise and a small harmonic tracer.
///
/// There is deliberately no feedback path. The delay tap uses bounded linear
/// interpolation, and its gain never exceeds unity, making this safe even at
/// low sample rates where the shortest delay is less than one sample.
pub(super) fn swept_fractional_delay_comb_loom(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let rate = sample_rate as f64;
    let minimum_delay_seconds = 0.000_15_f64;
    let maximum_delay_seconds = 0.018_f64;
    let maximum_delay_samples = maximum_delay_seconds * rate;
    let ring_length = maximum_delay_samples.ceil() as usize + 4;
    let mut ring = vec![0.0_f32; ring_length];
    let mut write = 0_usize;
    let mut rng = PhaseRng::new(0x4c00_6d1e_5e11_a7edu64);

    // One-pole filters keep the stochastic excitation away from DC and from
    // the least reliable portion of the spectrum near Nyquist.
    let noise_cutoff = 8_500.0_f64.min(rate * 0.34).max(40.0);
    let lowpass_feed = (1.0 - (-2.0 * PI * noise_cutoff / rate).exp()).clamp(0.0, 1.0) as f32;
    let dc_feed = (1.0 - (-2.0 * PI * 28.0_f64.min(rate * 0.01) / rate).exp()) as f32;
    let mut lowpass_state = 0.0_f32;
    let mut dc_state = 0.0_f32;
    let tracer_frequencies = [83.0_f64, 191.0, 433.0, 977.0];
    let mut tracer_phases = [0.0_f64; 4];

    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / rate;
        lowpass_state += lowpass_feed * (rng.bipolar() - lowpass_state);
        dc_state += dc_feed * (lowpass_state - dc_state);
        let band_noise = lowpass_state - dc_state;

        let mut tracer = 0.0_f64;
        for (voice, phase) in tracer_phases.iter_mut().enumerate() {
            let frequency = tracer_frequencies[voice].min(rate * (0.12 + voice as f64 * 0.045));
            *phase = (*phase + 2.0 * PI * frequency / rate).rem_euclid(2.0 * PI);
            tracer += phase.sin() / (voice as f64 + 1.0).sqrt();
        }
        let noise_mix = 0.42 + 0.24 * (2.0 * PI * time / 13.7).sin();
        let input = band_noise * noise_mix as f32 + tracer as f32 * 0.075;

        // Write first so delays below one sample interpolate between x[n] and
        // x[n-1] rather than accidentally reading an unwritten future slot.
        ring[write] = input;
        let sweep_phase = (time / 10.9).fract();
        let triangle = if sweep_phase < 0.5 {
            sweep_phase * 2.0
        } else {
            (1.0 - sweep_phase) * 2.0
        };
        let eased = triangle * triangle * (3.0 - 2.0 * triangle);
        let delay_seconds =
            minimum_delay_seconds * (maximum_delay_seconds / minimum_delay_seconds).powf(eased);
        let delay_samples = (delay_seconds * rate).clamp(0.0, maximum_delay_samples);
        let integer = delay_samples.floor() as usize;
        let fraction = (delay_samples - integer as f64) as f32;
        let recent = (write + ring_length - integer % ring_length) % ring_length;
        let older = (recent + ring_length - 1) % ring_length;
        let delayed = ring[recent] * (1.0 - fraction) + ring[older] * fraction;

        // Cosine rotation moves smoothly between constructive and destructive
        // interference; avoiding polarity steps also avoids artificial clicks.
        let polarity = (2.0 * PI * time / 15.3).cos() as f32;
        *sample = (input + delayed * polarity * 0.92) * 0.38;
        write = (write + 1) % ring_length;
    }

    output
}

#[derive(Clone, Copy)]
struct FirstOrderAllpass {
    coefficient: f32,
    previous_input: f32,
    previous_output: f32,
}

impl FirstOrderAllpass {
    fn new(coefficient: f32) -> Self {
        Self {
            coefficient: coefficient.clamp(-0.95, 0.95),
            previous_input: 0.0,
            previous_output: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.coefficient * input + self.previous_input
            - self.coefficient * self.previous_output;
        self.previous_input = input;
        self.previous_output = if output.is_finite() { output } else { 0.0 };
        self.previous_output
    }
}

struct PhaseRng {
    state: u64,
}

impl PhaseRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn bipolar(&mut self) -> f32 {
        // xorshift64* is small, deterministic, and sufficient for excitation.
        let mut value = self.state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.state = value;
        let bits = value.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40;
        bits as f32 * (2.0 / 16_777_215.0) - 1.0
    }
}
