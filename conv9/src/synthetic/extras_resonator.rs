// Additional synthetic resonant bodies.
//
// Both instruments use analytic modal frequencies and mode shapes rather
// than retuned copies of a generic harmonic bank.  `Resonator` is normalized
// at its input so an impulse has comparable modal amplitude at low and high
// sample rates.

use std::f64::consts::PI;

use super::Resonator;

struct DrumMode {
    order: u32,
    root: f64,
    sine_orientation: bool,
    input_gain: f32,
    pickup: f32,
    resonator: Resonator,
}

/// A struck circular membrane whose modal frequencies follow the zeros of
/// Bessel functions.  Radial strike motion and a roaming virtual pickup expose
/// the nodal structure instead of merely changing the spectrum of each hit.
pub(super) fn bessel_drum_skin(sample_rate: u32, frames: usize) -> Vec<f32> {
    if sample_rate < 100 || frames == 0 {
        return vec![0.0; frames];
    }

    // (azimuthal order, Bessel-function zero).  Non-axisymmetric modes occur
    // as orthogonal sine/cosine pairs on an ideal circular membrane.
    const ROOTS: &[(u32, f64)] = &[
        (0, 2.404_825_558),
        (1, 3.831_705_970),
        (2, 5.135_622_302),
        (0, 5.520_078_110),
        (3, 6.380_161_896),
        (1, 7.015_586_670),
        (4, 7.588_342_435),
        (2, 8.417_244_140),
        (0, 8.653_727_913),
        (5, 8.771_483_816),
        (3, 9.761_023_130),
        (1, 10.173_468_14),
        (4, 11.064_709_49),
        (0, 11.791_534_44),
        (2, 11.619_841_17),
        (5, 12.338_604_20),
        (3, 13.015_200_72),
        (6, 13.589_290_17),
        (4, 14.372_536_67),
        (5, 15.700_174_08),
        (6, 17.003_819_67),
    ];

    let rate = sample_rate as f64;
    let frequency_limit = rate * 0.43;
    let fundamental = 225.0;
    let first_root = ROOTS[0].1;
    let mut modes = Vec::new();
    for &(order, root) in ROOTS {
        let frequency = fundamental * root / first_root;
        if frequency >= frequency_limit {
            continue;
        }
        let decay = (3.8 / (1.0 + 0.115 * (root - first_root))).max(0.48);
        let input_gain = (2.0 * PI * frequency / rate).sin() as f32;
        let orientations = if order == 0 { 1 } else { 2 };
        for orientation in 0..orientations {
            modes.push(DrumMode {
                order,
                root,
                sine_orientation: orientation == 1,
                input_gain,
                pickup: 0.0,
                resonator: Resonator::new(sample_rate, frequency, decay),
            });
        }
    }

    let strike_radii = [0.08, 0.24, 0.43, 0.61, 0.78, 0.91, 0.54];
    let intervals = [1.31, 1.87, 2.43, 1.09, 3.17, 1.53, 2.11];
    let mut strike_weights = vec![0.0_f32; modes.len()];
    let mut output = vec![0.0; frames];
    let mut next_hit = 0_usize;
    let mut hit_number = 0_usize;

    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / rate;

        // Move the pickup slowly enough that a decaying hit crosses nodal
        // lines audibly.  Updating at control rate avoids unnecessary Bessel
        // evaluations while remaining smooth on this time scale.
        if index % 128 == 0 {
            let pickup_radius = 0.52 + 0.29 * (2.0 * PI * time / 17.3).sin();
            let pickup_angle = 2.0 * PI * time / 23.7;
            for mode in &mut modes {
                let angular = angular_component(mode.order, pickup_angle, mode.sine_orientation);
                mode.pickup = (bessel_j(mode.order, mode.root * pickup_radius) * angular) as f32;
            }
        }

        let hit = index == next_hit;
        let mut hit_amplitude = 0.0_f32;
        if hit {
            let strike_radius = strike_radii[hit_number % strike_radii.len()];
            let strike_angle = hit_number as f64 * 2.399_963_229_728_653;
            // Alternating mallet hardness changes the high-mode participation,
            // while the modal frequencies remain those of the same membrane.
            let hardness = [0.030, 0.012, 0.021, 0.007][hit_number % 4];
            hit_amplitude = if hit_number % 9 == 8 { 0.48 } else { 0.82 };
            for (weight, mode) in strike_weights.iter_mut().zip(&modes) {
                let angular = angular_component(mode.order, strike_angle, mode.sine_orientation);
                let radial = bessel_j(mode.order, mode.root * strike_radius);
                let mallet = (-hardness * (mode.root - first_root).powi(2)).exp();
                *weight = (radial * angular * mallet) as f32;
            }
            let interval = intervals[hit_number % intervals.len()];
            next_hit = index
                .saturating_add((interval * rate).round() as usize)
                .max(index + 1);
            hit_number += 1;
        }

        let mut sum = 0.0_f32;
        for (mode_index, mode) in modes.iter_mut().enumerate() {
            let excitation = if hit {
                hit_amplitude * strike_weights[mode_index] * mode.input_gain
            } else {
                0.0
            };
            let order_attenuation = 1.0 / (1.0 + mode.order as f32 * 0.16);
            sum += mode.resonator.process(excitation) * mode.pickup * order_attenuation;
        }
        *sample = sum * 0.115;
    }

    output
}

struct BeamMode {
    beam: usize,
    mode_number: usize,
    input_gain: f32,
    pickup: f32,
    base_participation: f32,
    resonator: Resonator,
}

/// A set of clamped-free Euler--Bernoulli beams struck by a travelling mallet.
/// Their overtones use the squared cantilever eigenvalues, and a separately
/// resonating compliant base supplies weak, stable feed-forward coupling.
pub(super) fn cantilever_forest(sample_rate: u32, frames: usize) -> Vec<f32> {
    if sample_rate < 100 || frames == 0 {
        return vec![0.0; frames];
    }

    // Roots of cosh(beta) * cos(beta) = -1 for a clamped-free beam.
    const BETA: [f64; 5] = [
        1.875_104_069,
        4.694_091_133,
        7.854_757_438,
        10.995_540_73,
        14.137_168_39,
    ];
    const FUNDAMENTALS: [f64; 7] = [54.0, 72.0, 99.0, 137.0, 191.0, 267.0, 376.0];

    let rate = sample_rate as f64;
    let frequency_limit = rate * 0.43;
    let mut modes = Vec::new();
    for (beam, fundamental) in FUNDAMENTALS.iter().copied().enumerate() {
        let pickup_position = 0.73 + 0.035 * (beam as f64 * 1.7).sin();
        for (mode_number, beta) in BETA.iter().copied().enumerate() {
            let frequency = fundamental * (beta / BETA[0]).powi(2);
            if frequency >= frequency_limit {
                continue;
            }
            let shape_at_pickup = cantilever_shape(beta, pickup_position);
            let decay =
                (3.2 / (1.0 + mode_number as f64 * 0.62) * (1.0 + beam as f64 * 0.025)).max(0.34);
            modes.push(BeamMode {
                beam,
                mode_number,
                input_gain: (2.0 * PI * frequency / rate).sin() as f32,
                pickup: shape_at_pickup as f32,
                base_participation: ((if mode_number & 1 == 0 { 1.0 } else { -1.0 }) * 0.055
                    / (mode_number as f64 + 1.0)) as f32,
                resonator: Resonator::new(sample_rate, frequency, decay),
            });
        }
    }

    let base_frequency = 38.0_f64.min(frequency_limit * 0.5);
    let base_input_gain = (2.0 * PI * base_frequency / rate).sin() as f32;
    let mut common_base = Resonator::new(sample_rate, base_frequency, 0.72);
    let intervals = [0.79, 0.93, 0.71, 1.07, 0.83, 1.31, 0.67, 1.73];
    let strike_positions = [0.96, 0.82, 0.68, 0.90, 0.75];
    let mut output = vec![0.0; frames];
    let mut next_strike = 0_usize;
    let mut strike_number = 0_usize;

    for (index, sample) in output.iter_mut().enumerate() {
        let strike = index == next_strike;
        let target_beam = (strike_number * 3 + strike_number / 7) % FUNDAMENTALS.len();
        let strike_position = strike_positions[strike_number % strike_positions.len()];
        let strike_amplitude = if strike_number % 11 == 10 { 0.42 } else { 0.78 };
        let base_impulse = if strike {
            strike_amplitude * base_input_gain * 0.34
        } else {
            0.0
        };
        let base_motion = common_base.process(base_impulse) * 0.16;

        let mut sum = base_motion * 0.08;
        for mode in &mut modes {
            let direct = if strike && mode.beam == target_beam {
                let beta = BETA[mode.mode_number];
                strike_amplitude * cantilever_shape(beta, strike_position) as f32
            } else {
                0.0
            };
            let shared = base_motion * mode.base_participation;
            let response = mode.resonator.process((direct + shared) * mode.input_gain);
            sum += response * mode.pickup / (mode.mode_number as f32 + 1.0).sqrt();
        }
        *sample = sum * 0.105;

        if strike {
            let interval = intervals[strike_number % intervals.len()];
            next_strike = index
                .saturating_add((interval * rate).round() as usize)
                .max(index + 1);
            strike_number += 1;
        }
    }

    output
}

fn angular_component(order: u32, angle: f64, sine_orientation: bool) -> f64 {
    let phase = order as f64 * angle;
    if sine_orientation {
        phase.sin()
    } else {
        phase.cos()
    }
}

/// Integer-order Bessel J evaluated by its convergent power series.  Arguments
/// in this module are bounded by the tabulated membrane roots, where f64 has
/// ample cancellation headroom.
fn bessel_j(order: u32, argument: f64) -> f64 {
    let half = argument * 0.5;
    let mut term = 1.0_f64;
    for divisor in 1..=order {
        term *= half / divisor as f64;
    }
    let mut sum = term;
    for k in 1..=48_u32 {
        term *= -(half * half) / (k as f64 * (k + order) as f64);
        sum += term;
        if k > 8 && term.abs() < sum.abs().max(1.0) * 1.0e-15 {
            break;
        }
    }
    sum
}

/// Normalized displacement eigenfunction for a unit-length clamped-free beam.
fn cantilever_shape(beta: f64, position: f64) -> f64 {
    let sigma = (beta.cosh() + beta.cos()) / (beta.sinh() + beta.sin());
    let raw = (beta * position).cosh()
        - (beta * position).cos()
        - sigma * ((beta * position).sinh() - (beta * position).sin());
    let tip = beta.cosh() - beta.cos() - sigma * (beta.sinh() - beta.sin());
    raw / tip.abs().max(1.0e-9)
}
