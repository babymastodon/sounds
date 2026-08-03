//! Additional time-frequency mask sources.
//!
//! These generators deliberately derive their upper spectral limit from the
//! requested sample rate.  That keeps the same geometry useful in low-rate
//! tests without allowing any oscillator to approach Nyquist in production.

use std::f64::consts::PI;

const PRISM_BANDS: usize = 8;
const PRISM_PARTIALS: usize = 6;

/// Equal-magnitude multisine packets whose frequency regions receive
/// independently permuted linear-phase (time-delay) ramps.
pub(super) fn phase_prism(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let rate = f64::from(sample_rate);
    let maximum_hz = rate * 0.43;
    let minimum_hz = 55.0_f64.min(maximum_hz * 0.25);
    let clip_duration = frames as f64 / rate;
    let packet_duration = 0.32;
    let event_spacing = 2.85;
    let delay_span = 0.96;
    let shuffled = [0_usize, 5, 2, 7, 3, 1, 6, 4];

    let mut event = 0_usize;
    let mut event_time = 0.0_f64;
    while event_time < clip_duration {
        for (band, &shuffled_band) in shuffled.iter().enumerate() {
            let position = band as f64 / (PRISM_BANDS - 1) as f64;
            let delay = match event % 5 {
                // Low-to-high and high-to-low phase slopes.
                0 => delay_span * position,
                1 => delay_span * (1.0 - position),
                // Convex and concave wedges.
                2 => delay_span * (2.0 * position - 1.0).abs(),
                3 => delay_span * (1.0 - (2.0 * position - 1.0).abs()),
                // A non-monotonic phase mosaic.
                _ => delay_span * shuffled_band as f64 / (PRISM_BANDS - 1) as f64,
            };

            let band_low =
                minimum_hz * (maximum_hz / minimum_hz).powf(band as f64 / PRISM_BANDS as f64);
            let band_high =
                minimum_hz * (maximum_hz / minimum_hz).powf((band + 1) as f64 / PRISM_BANDS as f64);
            add_prism_packet(
                &mut output,
                sample_rate,
                event_time + delay,
                packet_duration,
                band_low,
                band_high,
                band,
            );
        }
        event += 1;
        event_time += event_spacing;
    }

    output
}

/// Windowed log-chirp families folded into X, V, diamond, and coherent
/// focal-point shapes in the time-frequency plane.
pub(super) fn chirp_origami(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; frames];
    if sample_rate == 0 || frames == 0 {
        return output;
    }

    let rate = f64::from(sample_rate);
    let maximum_hz = rate * 0.43;
    let minimum_hz = 48.0_f64.min(maximum_hz * 0.25);
    let clip_duration = frames as f64 / rate;
    let event_duration = 2.65;
    let event_spacing = 3.65;

    let frequency_at =
        |position: f64| minimum_hz * (maximum_hz / minimum_hz).powf(position.clamp(0.0, 1.0));

    let mut event = 0_usize;
    let mut event_time = 0.0_f64;
    while event_time < clip_duration {
        match event % 4 {
            // Six nested crossing pairs form an X rather than a single sweep.
            0 => {
                for layer in 0..6 {
                    let inset = layer as f64 / 5.0;
                    let low = frequency_at(0.03 + 0.28 * inset);
                    let high = frequency_at(0.97 - 0.28 * inset);
                    let phase = hashed_phase(event, layer);
                    add_log_chirp_path(
                        &mut output,
                        sample_rate,
                        event_time,
                        event_duration,
                        &[(0.0, low), (1.0, high)],
                        phase,
                        0.060,
                    );
                    add_log_chirp_path(
                        &mut output,
                        sample_rate,
                        event_time,
                        event_duration,
                        &[(0.0, high), (1.0, low)],
                        phase + PI * 0.5,
                        0.060,
                    );
                }
            }
            // Nested V folds have slightly staggered vertices.
            1 => {
                for layer in 0..9 {
                    let position = layer as f64 / 8.0;
                    let shoulder_left = frequency_at(0.57 + 0.40 * position);
                    let shoulder_right = frequency_at(0.62 + 0.34 * (1.0 - position));
                    let vertex = frequency_at(0.04 + 0.31 * position);
                    let vertex_time = 0.43 + 0.14 * position;
                    add_log_chirp_path(
                        &mut output,
                        sample_rate,
                        event_time,
                        event_duration,
                        &[
                            (0.0, shoulder_left),
                            (vertex_time, vertex),
                            (1.0, shoulder_right),
                        ],
                        hashed_phase(event, layer),
                        0.056,
                    );
                }
            }
            // Upper and lower folds share endpoints, drawing nested diamonds.
            2 => {
                for layer in 0..6 {
                    let spread = 0.14 + layer as f64 * 0.065;
                    let center = 0.50;
                    let phase = hashed_phase(event, layer);
                    for (direction, phase_shift) in [(1.0, 0.0), (-1.0, PI * 0.5)] {
                        add_log_chirp_path(
                            &mut output,
                            sample_rate,
                            event_time,
                            event_duration,
                            &[
                                (0.0, frequency_at(center)),
                                (0.5, frequency_at(center + direction * spread)),
                                (1.0, frequency_at(center)),
                            ],
                            phase + phase_shift,
                            0.058,
                        );
                    }
                }
            }
            // A many-to-one-to-many fold: every voice is phase-aligned at the
            // common vertex so the spectral focus also becomes a sharp crest.
            _ => {
                const VOICES: usize = 12;
                let focus_position = 0.58;
                let focus_hz = frequency_at(0.52);
                for voice in 0..VOICES {
                    let start = frequency_at(0.03 + 0.94 * voice as f64 / (VOICES - 1) as f64);
                    let destination = (voice * 7) % VOICES;
                    let end = frequency_at(0.03 + 0.94 * destination as f64 / (VOICES - 1) as f64);
                    let path = [(0.0, start), (focus_position, focus_hz), (1.0, end)];
                    let cycles_to_focus =
                        integrated_path_cycles(&path, focus_position, event_duration);
                    let aligned_phase = PI * 0.5 - 2.0 * PI * cycles_to_focus;
                    add_log_chirp_path(
                        &mut output,
                        sample_rate,
                        event_time,
                        event_duration,
                        &path,
                        aligned_phase,
                        0.056,
                    );
                }
            }
        }

        event += 1;
        event_time += event_spacing;
    }

    output
}

#[allow(clippy::too_many_arguments)]
fn add_prism_packet(
    output: &mut [f32],
    sample_rate: u32,
    start_time: f64,
    duration: f64,
    low_hz: f64,
    high_hz: f64,
    band: usize,
) {
    let start = (start_time * f64::from(sample_rate)).round() as usize;
    let length = (duration * f64::from(sample_rate)).round().max(2.0) as usize;
    if start >= output.len() {
        return;
    }

    for partial in 0..PRISM_PARTIALS {
        let position = (partial as f64 + 0.5) / PRISM_PARTIALS as f64;
        let frequency = low_hz * (high_hz / low_hz).powf(position);
        // Fixed intercepts keep every prism's magnitude spectrum identical;
        // moving a whole packet supplies the intended linear phase ramp.
        let phase_offset = hashed_phase(band, partial);
        for index in 0..length.min(output.len() - start) {
            let position = index as f64 / (length - 1) as f64;
            let window = (PI * position).sin().powi(2);
            let time = index as f64 / f64::from(sample_rate);
            output[start + index] +=
                (0.068 * window * (2.0 * PI * frequency * time + phase_offset).sin()) as f32;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_log_chirp_path(
    output: &mut [f32],
    sample_rate: u32,
    start_time: f64,
    duration: f64,
    points: &[(f64, f64)],
    phase_offset: f64,
    amplitude: f64,
) {
    debug_assert!(points.len() >= 2);
    let start = (start_time * f64::from(sample_rate)).round() as usize;
    let length = (duration * f64::from(sample_rate)).round().max(2.0) as usize;
    if start >= output.len() {
        return;
    }

    for index in 0..length.min(output.len() - start) {
        let position = index as f64 / (length - 1) as f64;
        let window = (PI * position).sin().powi(2);
        let cycles = integrated_path_cycles(points, position, duration);
        output[start + index] +=
            (amplitude * window * (phase_offset + 2.0 * PI * cycles).sin()) as f32;
    }
}

/// Integral of a piecewise exponential-frequency path, expressed in cycles.
/// Linear point interpolation in log Hz makes every edge straight on the
/// playground's log-frequency spectrogram.
fn integrated_path_cycles(points: &[(f64, f64)], position: f64, duration: f64) -> f64 {
    let target = position.clamp(points[0].0, points[points.len() - 1].0);
    let mut cycles = 0.0;

    for pair in points.windows(2) {
        let (left_position, left_hz) = pair[0];
        let (right_position, right_hz) = pair[1];
        if target <= left_position {
            break;
        }
        let covered = (target.min(right_position) - left_position).max(0.0);
        let segment_span = right_position - left_position;
        if covered == 0.0 || segment_span <= 0.0 {
            continue;
        }

        let fraction = covered / segment_span;
        let log_ratio = (right_hz / left_hz).ln();
        let segment_cycles = if log_ratio.abs() < 1.0e-12 {
            left_hz * covered * duration
        } else {
            left_hz * segment_span * duration * ((log_ratio * fraction).exp() - 1.0) / log_ratio
        };
        cycles += segment_cycles;
        if target <= right_position {
            break;
        }
    }

    cycles
}

fn hashed_phase(first: usize, second: usize) -> f64 {
    let mut value = (first as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (second as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ 0x7068_6173_655f_7072;
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    2.0 * PI * (value >> 11) as f64 / ((1_u64 << 53) - 1) as f64
}
