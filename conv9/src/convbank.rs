use std::f32::consts::PI;

use anyhow::{Result, bail};
use rayon::prelude::*;

/// A compact causal scene model. Each predictor coefficient is one state in
/// the equivalent all-pole acoustic system; 64 states are enough to retain
/// broad spectral bodies and several narrow resonances without making the
/// on-demand render unreasonably expensive.
#[cfg(test)]
const MODEL_ORDER: usize = 64;
const LOCAL_MODEL_ORDER: usize = 32;
#[cfg(test)]
const AUTOCORRELATION_LAG_BATCH: usize = 8;
#[cfg(test)]
const REFLECTION_LIMIT: f64 = 0.999;
#[cfg(test)]
const DIAGONAL_LOADING: f64 = 1.0e-6;
const LOCAL_REFLECTION_LIMIT: f64 = 0.98;
const LOCAL_DIAGONAL_LOADING: f64 = 1.0e-3;
const LOCAL_FRAME_FRAMES: usize = 8_192;
const LOCAL_HOP_FRAMES: usize = LOCAL_FRAME_FRAMES / 4;
const PARALLEL_FRAME_BATCH: usize = 32;

/// Fits locally stationary causal models to overlapping frames, extracts the
/// innovation (prediction error) from `driver`, and resynthesizes it through a
/// model interpolated toward the corresponding frame of `reference`.
///
/// `transfer == 0` is an identity transform of `driver` (to numerical
/// precision), while `transfer == 1` uses the corresponding short-time learned
/// response of `reference`. `ring` applies conventional bandwidth expansion to
/// both the analysis and synthesis models, so it changes modal decay without
/// breaking the identity endpoint. Output duration is always exactly
/// `driver.len()`.
pub(crate) fn render_predictive_resonator_bank(
    reference: &[f32],
    driver: &[f32],
    transfer: f32,
    ring: f32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    if reference.is_empty() || reference.len() != driver.len() {
        bail!("predictive resonator bank requires equal, non-empty source timelines");
    }
    validate_unit_parameter("resonator transfer", transfer)?;
    validate_unit_parameter("resonator ring", ring)?;
    if cancelled() {
        bail!("render cancelled");
    }

    if transfer == 0.0 || reference == driver {
        return Ok(driver.to_vec());
    }

    let order = LOCAL_MODEL_ORDER.min(LOCAL_FRAME_FRAMES.saturating_sub(1));
    let root_hann = (0..LOCAL_FRAME_FRAMES)
        .map(|index| {
            let phase = (index as f32 + 0.5) / LOCAL_FRAME_FRAMES as f32;
            (PI * phase).sin()
        })
        .collect::<Vec<_>>();
    let analysis_window = root_hann
        .iter()
        .map(|sample| sample * sample)
        .collect::<Vec<_>>();
    let first_start = -(LOCAL_FRAME_FRAMES as isize - LOCAL_HOP_FRAMES as isize);
    let frame_starts = (first_start..driver.len() as isize)
        .step_by(LOCAL_HOP_FRAMES)
        .collect::<Vec<_>>();
    let mut output = vec![0.0_f32; driver.len()];
    let mut overlap_weight = vec![0.0_f32; driver.len()];

    for batch in frame_starts.chunks(PARALLEL_FRAME_BATCH) {
        if cancelled() {
            bail!("render cancelled");
        }
        let rendered = batch
            .par_iter()
            .map(|&frame_start| {
                render_local_frame(
                    reference,
                    driver,
                    frame_start,
                    order,
                    transfer,
                    ring,
                    &root_hann,
                    &analysis_window,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        for frame in rendered {
            for (index, (&sample, &window_sample)) in
                frame.samples.iter().zip(&root_hann).enumerate()
            {
                let output_index = frame.start + index as isize;
                if !(0..output.len() as isize).contains(&output_index) {
                    continue;
                }
                let output_index = output_index as usize;
                output[output_index] += sample * window_sample;
                overlap_weight[output_index] += window_sample * window_sample;
            }
        }
    }
    if cancelled() {
        bail!("render cancelled");
    }
    for (sample, weight) in output.iter_mut().zip(overlap_weight) {
        if weight > 1.0e-8 {
            *sample /= weight;
        } else {
            *sample = 0.0;
        }
    }
    Ok(output)
}

struct LocalFrame {
    start: isize,
    samples: Vec<f32>,
}

#[allow(clippy::too_many_arguments)]
fn render_local_frame(
    reference: &[f32],
    driver: &[f32],
    frame_start: isize,
    order: usize,
    transfer: f32,
    ring: f32,
    root_hann: &[f32],
    analysis_window: &[f32],
) -> Result<LocalFrame> {
    let reference_analysis = extract_reflected_frame(reference, frame_start, analysis_window);
    let driver_analysis = extract_reflected_frame(driver, frame_start, analysis_window);
    let reference_model = fit_local_predictor(&reference_analysis, order);
    let driver_model = fit_local_predictor(&driver_analysis, order);

    // Reflection coefficients parameterize the stable all-pole region as a
    // product of open intervals. Their convex interpolation therefore stays
    // stable, unlike direct interpolation of predictor polynomials.
    let amount = f64::from(transfer);
    let target_reflections = driver_model
        .reflections
        .iter()
        .zip(&reference_model.reflections)
        .map(|(&driver, &reference)| driver + amount * (reference - driver))
        .collect::<Vec<_>>();
    let source_coefficients =
        bandwidth_expand(&reflection_to_predictor(&driver_model.reflections), ring);
    let target_coefficients = bandwidth_expand(&reflection_to_predictor(&target_reflections), ring);
    let driver_frame = extract_reflected_frame(driver, frame_start, root_hann);
    let mut samples =
        transfilter_unchecked(&driver_frame, &source_coefficients, &target_coefficients)?;

    // Preserve B's local event energy while changing its predictable spectral
    // body. This gain is frame-local and is itself overlap-interpolated.
    let input_power = mean_square(&driver_frame);
    let output_power = mean_square(&samples);
    if input_power <= 1.0e-16 {
        samples.fill(0.0);
    } else if output_power > 1.0e-16 {
        let gain = (input_power / output_power).sqrt() as f32;
        for sample in &mut samples {
            *sample *= gain;
        }
    }
    Ok(LocalFrame {
        start: frame_start,
        samples,
    })
}

fn extract_reflected_frame(source: &[f32], start: isize, window: &[f32]) -> Vec<f32> {
    window
        .iter()
        .enumerate()
        .map(|(index, &weight)| {
            source[reflect_index(start + index as isize, source.len())] * weight
        })
        .collect()
}

fn reflect_index(index: isize, length: usize) -> usize {
    debug_assert!(length > 0);
    let period = 2 * length as isize;
    let folded = index.rem_euclid(period);
    if folded < length as isize {
        folded as usize
    } else {
        (period - folded - 1) as usize
    }
}

fn mean_square(samples: &[f32]) -> f64 {
    samples
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>()
        / samples.len().max(1) as f64
}

fn validate_unit_parameter(label: &str, value: f32) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        bail!("{label} must be a finite number from 0 to 1");
    }
    Ok(())
}

#[derive(Debug)]
struct PredictorModel {
    reflections: Vec<f64>,
}

#[cfg(test)]
fn fit_predictor(
    samples: &[f32],
    order: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<PredictorModel> {
    let autocorrelation = biased_autocorrelation(samples, order, cancelled)?;
    Ok(predictor_from_autocorrelation(&autocorrelation, order))
}

fn fit_local_predictor(samples: &[f32], order: usize) -> PredictorModel {
    let mut autocorrelation = vec![0.0_f64; order + 1];
    for (lag, value) in autocorrelation.iter_mut().enumerate() {
        *value = samples[lag..]
            .iter()
            .zip(&samples[..samples.len() - lag])
            .map(|(&current, &past)| f64::from(current) * f64::from(past))
            .sum::<f64>()
            / samples.len() as f64;
    }
    predictor_from_autocorrelation_with_regularization(
        &autocorrelation,
        order,
        LOCAL_DIAGONAL_LOADING,
        LOCAL_REFLECTION_LIMIT,
    )
}

#[cfg(test)]
fn predictor_from_autocorrelation(autocorrelation: &[f64], order: usize) -> PredictorModel {
    predictor_from_autocorrelation_with_regularization(
        autocorrelation,
        order,
        DIAGONAL_LOADING,
        REFLECTION_LIMIT,
    )
}

fn predictor_from_autocorrelation_with_regularization(
    autocorrelation: &[f64],
    order: usize,
    diagonal_loading: f64,
    reflection_limit: f64,
) -> PredictorModel {
    if autocorrelation[0] <= f64::EPSILON {
        return PredictorModel {
            reflections: vec![0.0; order],
        };
    }

    // Levinson-Durbin solves the Toeplitz Yule-Walker least-squares system in
    // O(p²). The biased autocorrelation sequence is positive semidefinite; a
    // tiny diagonal load and reflection margin make finite-precision synthesis
    // strictly stable even for near-deterministic tones.
    let mut coefficients = vec![0.0_f64; order + 1];
    coefficients[0] = 1.0;
    let mut reflections = Vec::with_capacity(order);
    let mut prediction_error = autocorrelation[0] * (1.0 + diagonal_loading);
    for model_order in 1..=order {
        let mut residual = autocorrelation[model_order];
        for lag in 1..model_order {
            residual += coefficients[lag] * autocorrelation[model_order - lag];
        }
        let reflection = (-residual / prediction_error.max(f64::MIN_POSITIVE))
            .clamp(-reflection_limit, reflection_limit);
        let previous = coefficients.clone();
        for lag in 1..model_order {
            coefficients[lag] = previous[lag] + reflection * previous[model_order - lag];
        }
        coefficients[model_order] = reflection;
        reflections.push(reflection);
        prediction_error *= 1.0 - reflection * reflection;
        prediction_error = prediction_error.max(autocorrelation[0] * diagonal_loading);
    }
    PredictorModel { reflections }
}

#[cfg(test)]
fn biased_autocorrelation(
    samples: &[f32],
    order: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f64>> {
    let mut autocorrelation = vec![0.0_f64; order + 1];
    for first_lag in (0..=order).step_by(AUTOCORRELATION_LAG_BATCH) {
        if cancelled() {
            bail!("render cancelled");
        }
        let end_lag = (first_lag + AUTOCORRELATION_LAG_BATCH).min(order + 1);
        let batch = (first_lag..end_lag)
            .into_par_iter()
            .map(|lag| {
                let sum = samples[lag..]
                    .iter()
                    .zip(&samples[..samples.len() - lag])
                    .fold(0.0_f64, |sum, (&current, &past)| {
                        sum + f64::from(current) * f64::from(past)
                    });
                (lag, sum / samples.len() as f64)
            })
            .collect::<Vec<_>>();
        for (lag, value) in batch {
            autocorrelation[lag] = value;
        }
    }
    Ok(autocorrelation)
}

fn reflection_to_predictor(reflections: &[f64]) -> Vec<f64> {
    let mut coefficients = vec![1.0_f64];
    for (index, &reflection) in reflections.iter().enumerate() {
        let model_order = index + 1;
        let previous = coefficients.clone();
        coefficients.resize(model_order + 1, 0.0);
        for lag in 1..model_order {
            coefficients[lag] = previous[lag] + reflection * previous[model_order - lag];
        }
        coefficients[model_order] = reflection;
    }
    coefficients
}

fn bandwidth_expand(coefficients: &[f64], ring: f32) -> Vec<f64> {
    // Pole radii are multiplied by gamma. The range intentionally stops short
    // of one: maximum ring remains BIBO-stable and the default avoids brittle,
    // nearly-undamped modes on deterministic or clipped references.
    let gamma = 0.98 + 0.0199 * f64::from(ring);
    coefficients
        .iter()
        .enumerate()
        .map(|(lag, &coefficient)| coefficient * gamma.powi(lag as i32))
        .collect()
}

fn transfilter_unchecked(
    driver: &[f32],
    source_coefficients: &[f64],
    target_coefficients: &[f64],
) -> Result<Vec<f32>> {
    transfilter_impl(driver, source_coefficients, target_coefficients, |_| Ok(()))
}

fn transfilter_impl(
    driver: &[f32],
    source_coefficients: &[f64],
    target_coefficients: &[f64],
    mut poll: impl FnMut(usize) -> Result<()>,
) -> Result<Vec<f32>> {
    debug_assert_eq!(source_coefficients.len(), target_coefficients.len());
    debug_assert_eq!(source_coefficients.first(), Some(&1.0));
    debug_assert_eq!(target_coefficients.first(), Some(&1.0));
    let order = source_coefficients.len() - 1;
    let mut output = Vec::with_capacity(driver.len());
    for (frame, &sample) in driver.iter().enumerate() {
        poll(frame)?;
        let available = order.min(frame);
        let mut innovation = f64::from(sample);
        let mut prediction = 0.0_f64;
        for lag in 1..=available {
            innovation += source_coefficients[lag] * f64::from(driver[frame - lag]);
            prediction += target_coefficients[lag] * f64::from(output[frame - lag]);
        }
        let rendered = innovation - prediction;
        if !rendered.is_finite() || rendered.abs() > f64::from(f32::MAX) {
            bail!("predictive resonator bank became numerically unstable");
        }
        output.push(rendered as f32);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::f32::consts::PI;
    use std::path::Path;
    use std::time::Instant;

    use super::*;
    use crate::audio::{SAMPLE_RATE, condition_output, read_prepared_clip};
    use crate::dsp::{Algorithm, AlgorithmParameters, render_algorithm_cancellable};

    const NEVER_CANCELLED: &dyn Fn() -> bool = &|| false;

    fn deterministic_noise(frames: usize) -> Vec<f32> {
        let mut state = 0x1234_5678_u32;
        (0..frames)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 0.2
            })
            .collect()
    }

    fn ar_process(innovation: &[f32], predictor: &[f64]) -> Vec<f32> {
        let mut output = Vec::with_capacity(innovation.len());
        for (frame, &sample) in innovation.iter().enumerate() {
            let mut value = f64::from(sample);
            for lag in 1..predictor.len().min(frame + 1) {
                value -= predictor[lag] * f64::from(output[frame - lag]);
            }
            output.push(value as f32);
        }
        output
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt()
    }

    #[test]
    fn zero_transfer_is_driver_identity() {
        let reference = deterministic_noise(32_768);
        let driver = (0..reference.len())
            .map(|frame| {
                0.15 * (2.0 * PI * 997.0 * frame as f32 / SAMPLE_RATE as f32).sin()
                    + reference[frame] * 0.1
            })
            .collect::<Vec<_>>();
        let output =
            render_predictive_resonator_bank(&reference, &driver, 0.0, 1.0, NEVER_CANCELLED)
                .unwrap();
        let error = output
            .iter()
            .zip(&driver)
            .map(|(&actual, &expected)| (actual - expected).abs())
            .fold(0.0_f32, f32::max);
        assert!(error < 2.0e-6, "identity error {error}");
    }

    #[test]
    fn identical_models_are_identity_at_every_setting() {
        let signal = ar_process(&deterministic_noise(24_000), &[1.0, -1.45, 0.72]);
        for transfer in [0.0, 0.37, 1.0] {
            for ring in [0.0, 0.75, 1.0] {
                let output = render_predictive_resonator_bank(
                    &signal,
                    &signal,
                    transfer,
                    ring,
                    NEVER_CANCELLED,
                )
                .unwrap();
                let relative_error = rms(&output
                    .iter()
                    .zip(&signal)
                    .map(|(&actual, &expected)| actual - expected)
                    .collect::<Vec<_>>())
                    / rms(&signal);
                assert!(
                    relative_error < 2.0e-6,
                    "transfer {transfer}, ring {ring}: error {relative_error}"
                );
            }
        }
    }

    #[test]
    fn defaults_follow_time_varying_reference_resonances_not_the_driver_body() {
        let half = 65_536;
        let low = predictor_for_resonance(620.0, 0.97);
        let high = predictor_for_resonance(4_200.0, 0.97);
        let first_innovation = deterministic_noise(half);
        let mut second_innovation = deterministic_noise(half);
        second_innovation.rotate_left(7_919);

        let mut reference = ar_process(&first_innovation, &low);
        reference.extend(ar_process(&second_innovation, &high));
        let mut driver = ar_process(&first_innovation, &high);
        driver.extend(ar_process(&second_innovation, &low));

        let output =
            render_predictive_resonator_bank(&reference, &driver, 1.0, 0.75, NEVER_CANCELLED)
                .unwrap();
        let guard = 8_192;
        for (label, range, expected_direction) in [
            ("first", guard..half - guard, 1.0_f64),
            ("second", half + guard..2 * half - guard, -1.0_f64),
        ] {
            let reference_ratio = resonance_ratio(&reference[range.clone()], 620.0, 4_200.0);
            let driver_ratio = resonance_ratio(&driver[range.clone()], 620.0, 4_200.0);
            let output_ratio = resonance_ratio(&output[range], 620.0, 4_200.0);
            let transfer_fraction = (output_ratio.ln() - driver_ratio.ln())
                / (reference_ratio.ln() - driver_ratio.ln());
            eprintln!(
                "{label} default local resonance transfer: {:.1}%",
                transfer_fraction * 100.0
            );
            assert!(
                transfer_fraction > 0.60 && transfer_fraction.is_finite(),
                "{label} local resonance transfer was only {:.1}% ({expected_direction:+} \
                 direction): A {reference_ratio:.3}, B {driver_ratio:.3}, \
                 output {output_ratio:.3}",
                transfer_fraction * 100.0
            );
        }
    }

    #[test]
    fn fitted_stationary_ar_model_recovers_the_known_optimal_predictor() {
        // Keep a direct equation-level test below the short-time renderer: this
        // conjugate-pole AR(2) process is exactly representable by LPC.
        let true_predictor = [1.0, -1.56, 0.81];
        let reference = ar_process(&deterministic_noise(262_144), &true_predictor);
        let fitted = fit_predictor(&reference, MODEL_ORDER, NEVER_CANCELLED).unwrap();
        let fitted_coefficients = reflection_to_predictor(&fitted.reflections);
        assert!((fitted_coefficients[1] - true_predictor[1]).abs() < 0.01);
        assert!((fitted_coefficients[2] - true_predictor[2]).abs() < 0.01);
    }

    #[test]
    fn sine_reference_rings_at_its_fitted_frequency() {
        let frames = 131_072;
        let frequency = 731.0;
        let reference = (0..frames)
            .map(|frame| 0.2 * (2.0 * PI * frequency * frame as f32 / SAMPLE_RATE as f32).sin())
            .collect::<Vec<_>>();
        let mut impulse = vec![0.0_f32; frames];
        impulse[0] = 1.0;
        let output =
            render_predictive_resonator_bank(&reference, &impulse, 1.0, 1.0, NEVER_CANCELLED)
                .unwrap();
        let target = sinusoidal_projection(&output[..16_384], frequency);
        let distant = sinusoidal_projection(&output[..16_384], frequency + 1_200.0);
        assert!(target > distant * 20.0, "{target} versus {distant}");
        assert!(rms(&output[..4_096]) > rms(&output[32_768..]) * 2.0);
    }

    #[test]
    fn white_reference_removes_the_drivers_predictable_resonance() {
        let frames = 131_072;
        let reference = deterministic_noise(frames);
        let driver = ar_process(
            &deterministic_noise(frames),
            &predictor_for_resonance(1_100.0, 0.97),
        );
        let output =
            render_predictive_resonator_bank(&reference, &driver, 1.0, 0.75, NEVER_CANCELLED)
                .unwrap();
        let guard = LOCAL_FRAME_FRAMES;
        let driver_ratio = resonance_ratio(&driver[guard..frames - guard], 1_100.0, 6_000.0);
        let output_ratio = resonance_ratio(&output[guard..frames - guard], 1_100.0, 6_000.0);
        assert!(
            output_ratio < driver_ratio.sqrt(),
            "flat A did not whiten B: B ratio {driver_ratio:.3}, output {output_ratio:.3}"
        );
        assert_eq!(output.len(), driver.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn driver_burst_timing_survives_default_resonance_transfer() {
        let frames = 131_072;
        let reference = ar_process(
            &deterministic_noise(frames),
            &predictor_for_resonance(780.0, 0.96),
        );
        let mut driver = vec![0.0_f32; frames];
        let burst_start = 52_000;
        let burst_end = 59_000;
        driver[burst_start..burst_end]
            .copy_from_slice(&deterministic_noise(burst_end - burst_start));
        let output =
            render_predictive_resonator_bank(&reference, &driver, 1.0, 0.75, NEVER_CANCELLED)
                .unwrap();
        let inside = output[burst_start..burst_end + 2_048]
            .iter()
            .map(|&sample| f64::from(sample) * f64::from(sample))
            .sum::<f64>();
        let outside = output[..burst_start - 2_048]
            .iter()
            .chain(&output[burst_end + 4_096..])
            .map(|&sample| f64::from(sample) * f64::from(sample))
            .sum::<f64>();
        assert!(
            inside > outside * 100.0,
            "B event energy escaped its timeline: inside {inside:.4}, outside {outside:.4}"
        );
    }

    #[test]
    fn silence_is_a_finite_zero_system() {
        let silence = vec![0.0_f32; 4_096];
        let output =
            render_predictive_resonator_bank(&silence, &silence, 1.0, 1.0, NEVER_CANCELLED)
                .unwrap();
        assert_eq!(output, silence);
    }

    #[test]
    fn fitted_predictor_satisfies_yule_walker_optimality() {
        let signal = ar_process(&deterministic_noise(131_072), &[1.0, -1.2, 0.52]);
        let order = 12;
        let autocorrelation = biased_autocorrelation(&signal, order, NEVER_CANCELLED).unwrap();
        let model = fit_predictor(&signal, order, NEVER_CANCELLED).unwrap();
        let coefficients = reflection_to_predictor(&model.reflections);
        for row in 1..=order {
            let normal_equation = autocorrelation[row]
                + (1..=order)
                    .map(|column| coefficients[column] * autocorrelation[row.abs_diff(column)])
                    .sum::<f64>();
            assert!(
                normal_equation.abs() < autocorrelation[0] * 2.0e-5,
                "row {row}: residual {normal_equation}"
            );
        }
    }

    #[test]
    fn cancellation_interrupts_analysis_and_synthesis() {
        let reference = deterministic_noise(131_072);
        let mut driver = reference.clone();
        driver.rotate_left(17_123);
        let calls = Cell::new(0_usize);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() >= 4
        };
        let error = render_predictive_resonator_bank(&reference, &driver, 1.0, 1.0, &cancelled)
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }

    fn sinusoidal_projection(samples: &[f32], frequency: f32) -> f64 {
        let angular = 2.0 * std::f64::consts::PI * f64::from(frequency) / f64::from(SAMPLE_RATE);
        let (real, imaginary) =
            samples
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(real, imaginary), (index, &sample)| {
                    let phase = angular * index as f64;
                    (
                        real + f64::from(sample) * phase.cos(),
                        imaginary - f64::from(sample) * phase.sin(),
                    )
                });
        real.hypot(imaginary)
    }

    fn predictor_for_resonance(frequency: f32, radius: f64) -> [f64; 3] {
        let angular = 2.0 * std::f64::consts::PI * f64::from(frequency) / f64::from(SAMPLE_RATE);
        [1.0, -2.0 * radius * angular.cos(), radius * radius]
    }

    fn resonance_ratio(samples: &[f32], low: f32, high: f32) -> f64 {
        let band_power = |center: f32| {
            ((center - 180.0) as usize..=(center + 180.0) as usize)
                .step_by(30)
                .map(|frequency| sinusoidal_projection(samples, frequency as f32).powi(2))
                .sum::<f64>()
        };
        band_power(low) / band_power(high).max(1.0e-20)
    }

    #[test]
    #[ignore = "release-only real-source smoke and performance measurement"]
    fn real_sources_render_at_interactive_speed() {
        let input_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/prepared");
        let reference =
            read_prepared_clip("ambient_guitar", &input_dir.join("ambient_guitar.wav")).unwrap();
        let driver = read_prepared_clip(
            "kitchen_washing_dishes",
            &input_dir.join("kitchen_washing_dishes.wav"),
        )
        .unwrap();
        let started = Instant::now();
        let mut output = render_algorithm_cancellable(
            Algorithm::PredictiveResonatorBank,
            None,
            AlgorithmParameters::default(),
            &reference,
            &driver,
            NEVER_CANCELLED,
        )
        .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(output.len(), driver.samples.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(rms(&output) > 1.0e-4);
        let metrics = condition_output(&mut output).unwrap();
        assert_eq!(metrics.frames, driver.samples.len());
        assert!(metrics.rms >= 0.065);
        assert!(metrics.peak <= 0.925);
        assert_eq!(metrics.non_finite_samples, 0);
        assert!(
            elapsed.as_secs_f32() < 3.0,
            "release render took {elapsed:?}"
        );
        eprintln!("predictive resonator bank real-source DSP: {elapsed:?}");
    }
}
