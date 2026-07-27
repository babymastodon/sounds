use std::f32::consts::PI;

use anyhow::{Result, bail};
use rayon::prelude::*;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner};

use crate::audio::SAMPLE_RATE;

const FRAME_FRAMES: usize = 1_024;
const HOP_FRAMES: usize = FRAME_FRAMES / 2;
const ANALYSIS_BANDS: usize = 48;
const COMPONENTS: usize = 8;
const FACTORIZATION_ITERATIONS: usize = 6;
const ACTIVATION_SPARSITY: f32 = 2.0e-4;
const FACTORIZATION_FLOOR: f32 = 1.0e-7;
const MAXIMUM_SPECTRAL_GAIN: f32 = 32.0;
const PARALLEL_RENDER_BATCH: usize = 64;

/// Learns an overcomplete bank of reusable spectro-temporal responses from A,
/// learns independent activation streams from B, softly routes B's streams to
/// A's responses, and resynthesizes the resulting scene with B's events and
/// exact timeline. Coherent reference modes retain A's measured inter-frame
/// phase advance; stochastic reference energy receives decorrelated phase.
///
/// Each source magnitude spectrogram is modeled without labels as
///
/// `V(f,t) ≈ Σ_k Σ_τ W(k,τ,f) H(k,t-τ) + R(f,t)`.
///
/// `W` is a bank of short causal response patterns, `H` contains sparse
/// activations, and `R` is a capacity-limited residual. Unlike a single LPC
/// model, the dictionary can simultaneously represent impacts, sustained
/// spectra, textures, and changing mixtures. Transfer zero is an exact B
/// identity; transfer one uses A's learned responses driven by B's activations.
pub(crate) fn render_latent_convolution_bank(
    reference: &[f32],
    driver: &[f32],
    transfer: f32,
    memory_ms: f32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    if reference.is_empty() || reference.len() != driver.len() {
        bail!("latent convolution bank requires equal, non-empty source timelines");
    }
    if !transfer.is_finite() || !(0.0..=1.5).contains(&transfer) {
        bail!("latent-bank transfer must be a finite number from 0 to 1.5");
    }
    if !memory_ms.is_finite() || !(40.0..=250.0).contains(&memory_ms) {
        bail!("latent-bank memory must be a finite number from 40 to 250 ms");
    }
    if cancelled() {
        bail!("render cancelled");
    }
    if transfer == 0.0 || reference == driver {
        return Ok(driver.to_vec());
    }
    let reference_power = reference
        .iter()
        .map(|&sample| f64::from(sample).powi(2))
        .sum::<f64>();
    if reference_power <= 1.0e-16 {
        let residual = (1.0 - transfer).max(0.0);
        return Ok(driver.iter().map(|sample| sample * residual).collect());
    }

    let lags = ((memory_ms * 0.001 * SAMPLE_RATE as f32 / HOP_FRAMES as f32).round() as usize)
        .clamp(2, 32);
    let window = root_hann_window();
    let band_map = logarithmic_band_map();
    let reference_analysis = analyze_spectrogram(reference, &window, &band_map, false, cancelled)?;
    if cancelled() {
        bail!("render cancelled");
    }
    let driver_analysis = analyze_spectrogram(driver, &window, &band_map, true, cancelled)?;
    if cancelled() {
        bail!("render cancelled");
    }

    let reference_model = ConvolutiveModel::fit(
        &reference_analysis.magnitudes,
        reference_analysis.frames,
        lags,
        cancelled,
    )?;
    let driver_model = ConvolutiveModel::fit(
        &driver_analysis.magnitudes,
        driver_analysis.frames,
        lags,
        cancelled,
    )?;
    let routing = soft_activity_routing(&reference_model, &driver_model);
    let routed_activations = route_activations(&driver_model, &routing);
    let mut target = reconstruct(
        &reference_model.responses,
        &routed_activations,
        reference_model.frames,
        reference_model.lags,
    );
    add_capacity_limited_residual(
        &reference_analysis,
        &reference_model,
        &driver_analysis,
        &driver_model,
        &mut target,
    );
    if cancelled() {
        bail!("render cancelled");
    }
    synthesize_with_driver(
        driver,
        transfer,
        &window,
        &band_map,
        &reference_analysis,
        &driver_analysis,
        &target,
        cancelled,
    )
}

fn root_hann_window() -> Vec<f32> {
    (0..FRAME_FRAMES)
        .map(|index| {
            let phase = (index as f32 + 0.5) / FRAME_FRAMES as f32;
            (PI * phase).sin()
        })
        .collect()
}

fn logarithmic_band_map() -> Vec<usize> {
    let nyquist = SAMPLE_RATE as f32 * 0.5;
    let log_top = (1.0 + nyquist / 80.0).ln();
    (0..=FRAME_FRAMES / 2)
        .map(|bin| {
            let frequency = bin as f32 * SAMPLE_RATE as f32 / FRAME_FRAMES as f32;
            (((1.0 + frequency / 80.0).ln() / log_top) * ANALYSIS_BANDS as f32)
                .floor()
                .min((ANALYSIS_BANDS - 1) as f32) as usize
        })
        .collect()
}

struct SpectrogramAnalysis {
    frames: usize,
    starts: Vec<isize>,
    magnitudes: Vec<f32>,
    raw_band_power: Vec<f32>,
    frame_power: Vec<f32>,
    mean_bin_power: Vec<f32>,
    phase_origin: Vec<f32>,
    mean_phase_step: Vec<f32>,
    phase_coherence: Vec<f32>,
    spectra: Option<Vec<Complex32>>,
}

fn analyze_spectrogram(
    source: &[f32],
    window: &[f32],
    band_map: &[usize],
    retain_spectra: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<SpectrogramAnalysis> {
    let first_start = -(FRAME_FRAMES as isize - HOP_FRAMES as isize);
    let starts = (first_start..source.len() as isize)
        .step_by(HOP_FRAMES)
        .collect::<Vec<_>>();
    let frames = starts.len();
    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(FRAME_FRAMES);
    let mut time = forward.make_input_vec();
    let mut spectrum = forward.make_output_vec();
    let mut raw_band_power = vec![0.0_f32; ANALYSIS_BANDS * frames];
    let mut frame_power = vec![0.0_f32; frames];
    let mut mean_bin_power = vec![0.0_f32; spectrum.len()];
    let mut previous_phase = vec![0.0_f32; spectrum.len()];
    let mut previous_power = vec![0.0_f32; spectrum.len()];
    let mut phase_step_real = vec![0.0_f64; spectrum.len()];
    let mut phase_step_imaginary = vec![0.0_f64; spectrum.len()];
    let mut phase_step_weight = vec![0.0_f64; spectrum.len()];
    let mut peak_bin_power = vec![0.0_f32; spectrum.len()];
    let mut peak_bin_phase = vec![0.0_f32; spectrum.len()];
    let mut peak_bin_frame = vec![0_usize; spectrum.len()];
    let mut retained = retain_spectra.then(|| Vec::with_capacity(frames * spectrum.len()));

    for (frame, &start) in starts.iter().enumerate() {
        if frame.is_multiple_of(64) && cancelled() {
            bail!("render cancelled");
        }
        for (index, &weight) in window.iter().enumerate() {
            time[index] = source[reflect_index(start + index as isize, source.len())] * weight;
        }
        forward.process(&mut time, &mut spectrum)?;
        for (bin, value) in spectrum.iter().enumerate() {
            let power = value.norm_sqr();
            let phase = value.arg();
            raw_band_power[band_map[bin] * frames + frame] += power;
            frame_power[frame] += power;
            mean_bin_power[bin] += power;
            if power > peak_bin_power[bin] {
                peak_bin_power[bin] = power;
                peak_bin_phase[bin] = phase;
                peak_bin_frame[bin] = frame;
            }
            if frame > 0 {
                let weight = f64::from((previous_power[bin] * power).sqrt());
                if weight > 1.0e-20 {
                    let step = wrap_phase(phase - previous_phase[bin]);
                    phase_step_real[bin] += weight * f64::from(step.cos());
                    phase_step_imaginary[bin] += weight * f64::from(step.sin());
                    phase_step_weight[bin] += weight;
                }
            }
            previous_phase[bin] = phase;
            previous_power[bin] = power;
        }
        if let Some(retained) = &mut retained {
            retained.extend_from_slice(&spectrum);
        }
    }

    let mean_frame_power = (frame_power
        .iter()
        .map(|&value| f64::from(value))
        .sum::<f64>()
        / frames.max(1) as f64)
        .max(1.0e-16) as f32;
    let magnitudes = raw_band_power
        .iter()
        .map(|&power| (power / mean_frame_power).sqrt())
        .collect();
    let mut mean_phase_step = vec![0.0_f32; spectrum.len()];
    let mut phase_coherence = vec![0.0_f32; spectrum.len()];
    let mut phase_origin = vec![0.0_f32; spectrum.len()];
    for bin in 0..spectrum.len() {
        if phase_step_weight[bin] > 1.0e-20 {
            mean_phase_step[bin] = phase_step_imaginary[bin].atan2(phase_step_real[bin]) as f32;
            phase_coherence[bin] = (phase_step_real[bin].hypot(phase_step_imaginary[bin])
                / phase_step_weight[bin])
                .clamp(0.0, 1.0) as f32;
        }
        phase_origin[bin] =
            wrap_phase(peak_bin_phase[bin] - mean_phase_step[bin] * peak_bin_frame[bin] as f32);
    }
    Ok(SpectrogramAnalysis {
        frames,
        starts,
        magnitudes,
        raw_band_power,
        frame_power,
        mean_bin_power,
        phase_origin,
        mean_phase_step,
        phase_coherence,
        spectra: retained,
    })
}

fn wrap_phase(phase: f32) -> f32 {
    (phase + PI).rem_euclid(2.0 * PI) - PI
}

struct ConvolutiveModel {
    frames: usize,
    lags: usize,
    responses: Vec<f32>,
    activations: Vec<f32>,
    reconstruction: Vec<f32>,
}

impl ConvolutiveModel {
    fn fit(
        input: &[f32],
        frames: usize,
        lags: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        let mut responses = initialize_responses(input, frames, lags);
        let mut activations = initialize_activations(input, frames);
        normalize_components(&mut responses, &mut activations, frames, lags);
        let mut reconstruction = vec![0.0_f32; input.len()];

        for iteration in 0..FACTORIZATION_ITERATIONS {
            if cancelled() {
                bail!("render cancelled");
            }
            reconstruct_into(&responses, &activations, frames, lags, &mut reconstruction);
            update_activations(
                input,
                &reconstruction,
                &responses,
                &mut activations,
                frames,
                lags,
            );
            reconstruct_into(&responses, &activations, frames, lags, &mut reconstruction);
            update_responses(
                input,
                &reconstruction,
                &mut responses,
                &activations,
                frames,
                lags,
            );
            normalize_components(&mut responses, &mut activations, frames, lags);
            if iteration + 1 == FACTORIZATION_ITERATIONS {
                reconstruct_into(&responses, &activations, frames, lags, &mut reconstruction);
            }
        }
        Ok(Self {
            frames,
            lags,
            responses,
            activations,
            reconstruction,
        })
    }
}

fn response_index(component: usize, lag: usize, band: usize, lags: usize) -> usize {
    (component * lags + lag) * ANALYSIS_BANDS + band
}

fn initialize_responses(input: &[f32], frames: usize, lags: usize) -> Vec<f32> {
    let mut responses = vec![0.0_f32; COMPONENTS * lags * ANALYSIS_BANDS];
    for component in 0..COMPONENTS {
        let anchor = (component + 1) * frames / (COMPONENTS + 1);
        for lag in 0..lags {
            let time = (anchor + lag * (component + 3)).min(frames - 1);
            let decay = (-2.0 * lag as f32 / lags as f32).exp();
            for band in 0..ANALYSIS_BANDS {
                let deterministic_floor =
                    1.0e-3 * (1.0 + ((component * 37 + lag * 13 + band * 7) % 17) as f32 / 17.0);
                responses[response_index(component, lag, band, lags)] =
                    input[band * frames + time] * decay + deterministic_floor;
            }
        }
    }
    responses
}

fn initialize_activations(input: &[f32], frames: usize) -> Vec<f32> {
    let mut frame_activity = vec![0.0_f32; frames];
    for band in 0..ANALYSIS_BANDS {
        for time in 0..frames {
            frame_activity[time] += input[band * frames + time];
        }
    }
    let mut activations = vec![0.0_f32; COMPONENTS * frames];
    for component in 0..COMPONENTS {
        for time in 0..frames {
            let modulation =
                0.75 + 0.25 * (((time * (component + 3) + component * 11) % 29) as f32 / 28.0);
            activations[component * frames + time] =
                (frame_activity[time] / COMPONENTS as f32 * modulation).max(FACTORIZATION_FLOOR);
        }
    }
    activations
}

fn reconstruct(responses: &[f32], activations: &[f32], frames: usize, lags: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; ANALYSIS_BANDS * frames];
    reconstruct_into(responses, activations, frames, lags, &mut output);
    output
}

fn reconstruct_into(
    responses: &[f32],
    activations: &[f32],
    frames: usize,
    lags: usize,
    output: &mut [f32],
) {
    output
        .par_chunks_mut(frames)
        .enumerate()
        .for_each(|(band, row)| {
            row.fill(FACTORIZATION_FLOOR);
            for component in 0..COMPONENTS {
                let activation = &activations[component * frames..(component + 1) * frames];
                for lag in 0..lags {
                    let weight = responses[response_index(component, lag, band, lags)];
                    for time in 0..frames - lag {
                        row[time + lag] += weight * activation[time];
                    }
                }
            }
        });
}

fn update_activations(
    input: &[f32],
    reconstruction: &[f32],
    responses: &[f32],
    activations: &mut [f32],
    frames: usize,
    lags: usize,
) {
    activations
        .par_chunks_mut(frames)
        .enumerate()
        .for_each(|(component, row)| {
            for (time, activation) in row.iter_mut().enumerate() {
                let mut numerator = 0.0_f64;
                let mut denominator = f64::from(ACTIVATION_SPARSITY);
                for lag in 0..lags.min(frames - time) {
                    for band in 0..ANALYSIS_BANDS {
                        let weight = responses[response_index(component, lag, band, lags)];
                        let index = band * frames + time + lag;
                        numerator += f64::from(weight * input[index]);
                        denominator += f64::from(weight * reconstruction[index]);
                    }
                }
                let multiplier = (numerator / denominator.max(1.0e-20)) as f32;
                *activation = (*activation * multiplier).clamp(FACTORIZATION_FLOOR, 1.0e6);
            }
        });
}

fn update_responses(
    input: &[f32],
    reconstruction: &[f32],
    responses: &mut [f32],
    activations: &[f32],
    frames: usize,
    lags: usize,
) {
    responses
        .par_chunks_mut(lags * ANALYSIS_BANDS)
        .enumerate()
        .for_each(|(component, component_responses)| {
            let activation = &activations[component * frames..(component + 1) * frames];
            for lag in 0..lags {
                for band in 0..ANALYSIS_BANDS {
                    let mut numerator = 0.0_f64;
                    let mut denominator = 0.0_f64;
                    let input_row = &input[band * frames..(band + 1) * frames];
                    let reconstruction_row = &reconstruction[band * frames..(band + 1) * frames];
                    for time in 0..frames - lag {
                        let activity = activation[time];
                        numerator += f64::from(activity * input_row[time + lag]);
                        denominator += f64::from(activity * reconstruction_row[time + lag]);
                    }
                    let index = lag * ANALYSIS_BANDS + band;
                    let multiplier = (numerator / denominator.max(1.0e-20)) as f32;
                    component_responses[index] =
                        (component_responses[index] * multiplier).clamp(FACTORIZATION_FLOOR, 1.0e6);
                }
            }
        });
}

fn normalize_components(
    responses: &mut [f32],
    activations: &mut [f32],
    frames: usize,
    lags: usize,
) {
    for component in 0..COMPONENTS {
        let response = &mut responses
            [component * lags * ANALYSIS_BANDS..(component + 1) * lags * ANALYSIS_BANDS];
        let scale = response
            .iter()
            .map(|&value| f64::from(value))
            .sum::<f64>()
            .max(1.0e-20) as f32;
        for value in response {
            *value /= scale;
        }
        for value in &mut activations[component * frames..(component + 1) * frames] {
            *value *= scale;
        }
    }
}

fn component_signature(model: &ConvolutiveModel, component: usize) -> Vec<f32> {
    let activation = &model.activations[component * model.frames..(component + 1) * model.frames];
    let mut signature = Vec::with_capacity(model.lags + 7);
    let mut response_envelope = vec![0.0_f32; model.lags];
    for (lag, envelope) in response_envelope.iter_mut().enumerate() {
        *envelope = (0..ANALYSIS_BANDS)
            .map(|band| model.responses[response_index(component, lag, band, model.lags)])
            .sum();
    }
    let envelope_sum = response_envelope
        .iter()
        .sum::<f32>()
        .max(FACTORIZATION_FLOOR);
    signature.extend(
        response_envelope
            .into_iter()
            .map(|value| value / envelope_sum),
    );

    let energy = activation
        .iter()
        .map(|&value| f64::from(value).powi(2))
        .sum::<f64>()
        .max(1.0e-20);
    for lag in [1, 2, 4, 8, 16, 32] {
        let correlation = if lag < activation.len() {
            activation[lag..]
                .iter()
                .zip(&activation[..activation.len() - lag])
                .map(|(&current, &past)| f64::from(current) * f64::from(past))
                .sum::<f64>()
                / energy
        } else {
            0.0
        };
        signature.push(correlation as f32);
    }
    let mean = activation
        .iter()
        .map(|&value| f64::from(value))
        .sum::<f64>()
        / activation.len().max(1) as f64;
    let mean_square = energy / activation.len().max(1) as f64;
    signature.push((mean * mean / mean_square.max(1.0e-20)) as f32);
    signature
}

fn soft_activity_routing(reference: &ConvolutiveModel, driver: &ConvolutiveModel) -> Vec<f32> {
    let reference_signatures = (0..COMPONENTS)
        .map(|component| component_signature(reference, component))
        .collect::<Vec<_>>();
    let driver_signatures = (0..COMPONENTS)
        .map(|component| component_signature(driver, component))
        .collect::<Vec<_>>();
    let mut costs = vec![0.0_f32; COMPONENTS * COMPONENTS];
    for reference_component in 0..COMPONENTS {
        for driver_component in 0..COMPONENTS {
            costs[reference_component * COMPONENTS + driver_component] = reference_signatures
                [reference_component]
                .iter()
                .zip(&driver_signatures[driver_component])
                .map(|(&a, &b)| (a - b).powi(2))
                .sum::<f32>();
        }
    }
    let mean_cost = costs.iter().sum::<f32>() / costs.len().max(1) as f32 + FACTORIZATION_FLOOR;
    let mut routing = costs
        .into_iter()
        .map(|cost| (-cost / (0.15 * mean_cost)).exp() + FACTORIZATION_FLOOR)
        .collect::<Vec<_>>();
    // Sinkhorn scaling gives a soft, approximately doubly-stochastic routing:
    // one B component may excite several A responses, but no slot can absorb
    // all energy solely because of arbitrary NMF scale or ordering.
    for _ in 0..24 {
        for row in 0..COMPONENTS {
            let sum = routing[row * COMPONENTS..(row + 1) * COMPONENTS]
                .iter()
                .sum::<f32>()
                .max(FACTORIZATION_FLOOR);
            for value in &mut routing[row * COMPONENTS..(row + 1) * COMPONENTS] {
                *value /= sum;
            }
        }
        for column in 0..COMPONENTS {
            let sum = (0..COMPONENTS)
                .map(|row| routing[row * COMPONENTS + column])
                .sum::<f32>()
                .max(FACTORIZATION_FLOOR);
            for row in 0..COMPONENTS {
                routing[row * COMPONENTS + column] /= sum;
            }
        }
    }
    routing
}

fn route_activations(driver: &ConvolutiveModel, routing: &[f32]) -> Vec<f32> {
    let mut routed = vec![0.0_f32; COMPONENTS * driver.frames];
    routed
        .par_chunks_mut(driver.frames)
        .enumerate()
        .for_each(|(reference_component, output)| {
            for driver_component in 0..COMPONENTS {
                let weight = routing[reference_component * COMPONENTS + driver_component];
                let input = &driver.activations
                    [driver_component * driver.frames..(driver_component + 1) * driver.frames];
                for (output, &input) in output.iter_mut().zip(input) {
                    *output += weight * input;
                }
            }
        });
    routed
}

fn add_capacity_limited_residual(
    reference_analysis: &SpectrogramAnalysis,
    reference_model: &ConvolutiveModel,
    driver_analysis: &SpectrogramAnalysis,
    driver_model: &ConvolutiveModel,
    target: &mut [f32],
) {
    let frames = reference_analysis.frames;
    let mut reference_profile = vec![0.0_f32; ANALYSIS_BANDS];
    let mut total_reference_residual = 0.0_f64;
    let mut total_reference = 0.0_f64;
    for (band, profile) in reference_profile.iter_mut().enumerate() {
        for time in 0..frames {
            let index = band * frames + time;
            let residual = (reference_analysis.magnitudes[index]
                - reference_model.reconstruction[index])
                .max(0.0);
            *profile += residual;
            total_reference_residual += f64::from(residual);
            total_reference += f64::from(reference_analysis.magnitudes[index]);
        }
    }
    let profile_sum = reference_profile
        .iter()
        .sum::<f32>()
        .max(FACTORIZATION_FLOOR);
    for value in &mut reference_profile {
        *value /= profile_sum;
    }
    let residual_capacity =
        (total_reference_residual / total_reference.max(1.0e-20)).clamp(0.0, 0.25) as f32;
    if residual_capacity <= FACTORIZATION_FLOOR {
        return;
    }
    let mut driver_activity = vec![0.0_f32; driver_analysis.frames];
    for band in 0..ANALYSIS_BANDS {
        for (time, activity) in driver_activity.iter_mut().enumerate() {
            let index = band * driver_analysis.frames + time;
            *activity +=
                (driver_analysis.magnitudes[index] - driver_model.reconstruction[index]).max(0.0);
        }
    }
    for (band, &profile) in reference_profile.iter().enumerate() {
        for (time, &activity) in driver_activity.iter().enumerate().take(frames) {
            target[band * frames + time] += residual_capacity * profile * activity;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn synthesize_with_driver(
    driver: &[f32],
    transfer: f32,
    window: &[f32],
    band_map: &[usize],
    reference_analysis: &SpectrogramAnalysis,
    analysis: &SpectrogramAnalysis,
    target: &[f32],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let spectra = analysis
        .spectra
        .as_ref()
        .expect("driver analysis retains complex spectra");
    let bins = FRAME_FRAMES / 2 + 1;
    let target_total = target
        .iter()
        .map(|&magnitude| f64::from(magnitude).powi(2))
        .sum::<f64>();
    let driver_total = analysis
        .frame_power
        .iter()
        .map(|&power| f64::from(power))
        .sum::<f64>();
    let target_power_scale = if target_total > 1.0e-20 {
        (driver_total / target_total) as f32
    } else {
        0.0
    };
    let mut reference_band_profile_power = [0.0_f32; ANALYSIS_BANDS];
    for (bin, &power) in reference_analysis.mean_bin_power.iter().enumerate() {
        reference_band_profile_power[band_map[bin]] += power;
    }
    let mut output = vec![0.0_f32; driver.len()];
    let mut overlap_weight = vec![0.0_f32; driver.len()];
    for first_frame in (0..analysis.frames).step_by(PARALLEL_RENDER_BATCH) {
        if cancelled() {
            bail!("render cancelled");
        }
        let end_frame = (first_frame + PARALLEL_RENDER_BATCH).min(analysis.frames);
        let rendered = (first_frame..end_frame)
            .into_par_iter()
            .map_init(InverseWorker::new, |worker, frame| {
                worker.render(
                    frame,
                    &spectra[frame * bins..(frame + 1) * bins],
                    transfer,
                    band_map,
                    reference_analysis,
                    &reference_band_profile_power,
                    analysis,
                    target,
                    target_power_scale,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        for frame in rendered {
            for (index, (&sample, &weight)) in frame.samples.iter().zip(window).enumerate() {
                let output_index = frame.start + index as isize;
                if !(0..output.len() as isize).contains(&output_index) {
                    continue;
                }
                let output_index = output_index as usize;
                output[output_index] += sample * weight;
                overlap_weight[output_index] += weight * weight;
            }
        }
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

struct InverseWorker {
    inverse: std::sync::Arc<dyn ComplexToReal<f32>>,
    spectrum: Vec<Complex32>,
    time: Vec<f32>,
}

impl InverseWorker {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let inverse = planner.plan_fft_inverse(FRAME_FRAMES);
        Self {
            spectrum: inverse.make_input_vec(),
            time: inverse.make_output_vec(),
            inverse,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        frame: usize,
        driver_spectrum: &[Complex32],
        transfer: f32,
        band_map: &[usize],
        reference_analysis: &SpectrogramAnalysis,
        reference_band_power: &[f32; ANALYSIS_BANDS],
        analysis: &SpectrogramAnalysis,
        target: &[f32],
        target_power_scale: f32,
    ) -> Result<RenderedFrame> {
        self.spectrum.copy_from_slice(driver_spectrum);
        let driver_frame_power = analysis.frame_power[frame];
        let target_frame_power = (0..ANALYSIS_BANDS)
            .map(|band| target[band * analysis.frames + frame].powi(2) * target_power_scale)
            .sum::<f32>();
        if driver_frame_power <= 1.0e-16 && target_frame_power <= 1.0e-16 {
            self.spectrum.fill(Complex32::new(0.0, 0.0));
        } else {
            let blend = transfer.min(1.0);
            let exaggeration = (transfer - 1.0).max(0.0);
            let mean_target_band_power = target_frame_power / ANALYSIS_BANDS as f32 + 1.0e-20;
            for (bin, value) in self.spectrum.iter_mut().enumerate() {
                let band = band_map[bin];
                let driver_band_power = analysis.raw_band_power[band * analysis.frames + frame];
                let within_band_share = if driver_band_power > 1.0e-16 {
                    value.norm_sqr() / driver_band_power
                } else if reference_band_power[band] > 1.0e-16 {
                    reference_analysis.mean_bin_power[bin] / reference_band_power[band]
                } else {
                    1.0 / band_map
                        .iter()
                        .filter(|&&mapped| mapped == band)
                        .count()
                        .max(1) as f32
                };
                let mut target_power = target[band * analysis.frames + frame].powi(2)
                    * target_power_scale
                    * within_band_share;
                if exaggeration > 0.0 {
                    let contrast = (target[band * analysis.frames + frame].powi(2)
                        * target_power_scale
                        / mean_target_band_power)
                        .clamp(1.0 / MAXIMUM_SPECTRAL_GAIN, MAXIMUM_SPECTRAL_GAIN);
                    target_power *= contrast.powf(exaggeration);
                }
                let output_power = (1.0 - blend) * value.norm_sqr() + blend * target_power;
                let target_unit = reference_phase_unit(reference_analysis, bin, frame);
                let driver_component = *value * (1.0 - blend).sqrt();
                let target_component = target_unit * (blend * target_power).sqrt();
                let phase_direction = driver_component + target_component;
                let output_unit = if phase_direction.norm_sqr() > 1.0e-20 {
                    phase_direction / phase_direction.norm()
                } else if blend >= 0.5 {
                    target_unit
                } else if value.norm_sqr() > 1.0e-20 {
                    *value / value.norm()
                } else {
                    Complex32::new(1.0, 0.0)
                };
                *value = output_unit * output_power.max(0.0).sqrt();
            }
            self.spectrum[0].im = 0.0;
            self.spectrum[FRAME_FRAMES / 2].im = 0.0;
        }
        self.inverse.process(&mut self.spectrum, &mut self.time)?;
        let scale = 1.0 / FRAME_FRAMES as f32;
        Ok(RenderedFrame {
            start: analysis.starts[frame],
            samples: self.time.iter().map(|sample| sample * scale).collect(),
        })
    }
}

fn reference_phase_unit(
    reference_analysis: &SpectrogramAnalysis,
    bin: usize,
    frame: usize,
) -> Complex32 {
    let tonal_phase = reference_analysis.phase_origin[bin]
        + reference_analysis.mean_phase_step[bin] * frame as f32;
    let tonal = Complex32::from_polar(1.0, tonal_phase);
    let diffuse = Complex32::from_polar(1.0, deterministic_diffuse_phase(bin, frame));

    // Circular phase-step coherence is nearly one for a stable sinusoidal mode
    // and approaches zero for stochastic energy. The smooth threshold prevents
    // weak accidental coherence in noise from becoming a pitched FFT-bin comb.
    let normalized = ((reference_analysis.phase_coherence[bin] - 0.30) / 0.50).clamp(0.0, 1.0);
    let tonal_weight = normalized * normalized * (3.0 - 2.0 * normalized);
    let mixed = tonal * tonal_weight + diffuse * (1.0 - tonal_weight).sqrt();
    if mixed.norm_sqr() > 1.0e-20 {
        mixed / mixed.norm()
    } else {
        diffuse
    }
}

fn deterministic_diffuse_phase(bin: usize, frame: usize) -> f32 {
    let mut mixed = (bin as u64)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add((frame as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9))
        .wrapping_add(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 30;
    mixed = mixed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed ^= mixed >> 27;
    mixed = mixed.wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 31;
    (mixed as f64 / u64::MAX as f64 * 2.0 * std::f64::consts::PI) as f32
}

struct RenderedFrame {
    start: isize,
    samples: Vec<f32>,
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::f32::consts::PI;
    use std::path::Path;
    use std::time::Instant;

    use super::*;
    use crate::audio::{condition_output, read_prepared_clip};

    const NEVER_CANCELLED: &dyn Fn() -> bool = &|| false;
    const DEFAULT_TRANSFER: f32 = 1.0;
    const DEFAULT_MEMORY_MS: f32 = 170.0;

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

    fn predictor_for_resonance(frequency: f32, radius: f64) -> [f64; 3] {
        let angular = 2.0 * std::f64::consts::PI * f64::from(frequency) / f64::from(SAMPLE_RATE);
        [1.0, -2.0 * radius * angular.cos(), radius * radius]
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt()
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

    fn resonance_ratio(samples: &[f32], low: f32, high: f32) -> f64 {
        let band_power = |center: f32| {
            ((center - 180.0) as usize..=(center + 180.0) as usize)
                .step_by(30)
                .map(|frequency| sinusoidal_projection(samples, frequency as f32).powi(2))
                .sum::<f64>()
        };
        band_power(low) / band_power(high).max(1.0e-20)
    }

    fn normalized_band_profile(analysis: &SpectrogramAnalysis) -> Vec<f64> {
        let mut profile = (0..ANALYSIS_BANDS)
            .map(|band| {
                analysis.raw_band_power[band * analysis.frames..(band + 1) * analysis.frames]
                    .iter()
                    .map(|&power| f64::from(power))
                    .sum::<f64>()
            })
            .collect::<Vec<_>>();
        let sum = profile.iter().sum::<f64>().max(1.0e-20);
        for value in &mut profile {
            *value /= sum;
        }
        profile
    }

    fn profile_distance(a: &[f64], b: &[f64]) -> f64 {
        a.iter()
            .zip(b)
            .map(|(&a, &b)| (a.sqrt() - b.sqrt()).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    fn correlation(a: &[f32], b: &[f32]) -> f64 {
        let mean_a = a.iter().map(|&value| f64::from(value)).sum::<f64>() / a.len() as f64;
        let mean_b = b.iter().map(|&value| f64::from(value)).sum::<f64>() / b.len() as f64;
        let mut covariance = 0.0_f64;
        let mut power_a = 0.0_f64;
        let mut power_b = 0.0_f64;
        for (&a, &b) in a.iter().zip(b) {
            let a = f64::from(a) - mean_a;
            let b = f64::from(b) - mean_b;
            covariance += a * b;
            power_a += a * a;
            power_b += b * b;
        }
        covariance / (power_a * power_b).sqrt().max(1.0e-20)
    }

    #[test]
    fn zero_transfer_is_exact_driver_identity() {
        let reference = deterministic_noise(32_768);
        let driver = (0..reference.len())
            .map(|frame| {
                0.15 * (2.0 * PI * 997.0 * frame as f32 / SAMPLE_RATE as f32).sin()
                    + reference[frame] * 0.1
            })
            .collect::<Vec<_>>();
        let output = render_latent_convolution_bank(
            &reference,
            &driver,
            0.0,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        assert_eq!(output, driver);
    }

    #[test]
    fn identical_sources_are_identity_for_all_controls() {
        let signal = deterministic_noise(24_000);
        for transfer in [0.25, 1.0, 1.5] {
            for memory in [40.0, 170.0, 250.0] {
                assert_eq!(
                    render_latent_convolution_bank(
                        &signal,
                        &signal,
                        transfer,
                        memory,
                        NEVER_CANCELLED
                    )
                    .unwrap(),
                    signal
                );
            }
        }
    }

    #[test]
    fn default_bank_replaces_driver_color_with_reference_color() {
        let frames = 65_536;
        let innovation = deterministic_noise(frames);
        let reference = ar_process(&innovation, &predictor_for_resonance(720.0, 0.97));
        let driver = ar_process(&innovation, &predictor_for_resonance(4_600.0, 0.97));
        let output = render_latent_convolution_bank(
            &reference,
            &driver,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let guard = 4_096;
        let range = guard..frames - guard;
        let reference_ratio = resonance_ratio(&reference[range.clone()], 720.0, 4_600.0);
        let driver_ratio = resonance_ratio(&driver[range.clone()], 720.0, 4_600.0);
        let output_ratio = resonance_ratio(&output[range], 720.0, 4_600.0);
        let fraction =
            (output_ratio.ln() - driver_ratio.ln()) / (reference_ratio.ln() - driver_ratio.ln());
        eprintln!(
            "latent-bank default spectral transfer: {:.1}%",
            fraction * 100.0
        );
        assert!(
            fraction > 0.70 && fraction.is_finite(),
            "A {reference_ratio:.3}, B {driver_ratio:.3}, output {output_ratio:.3}, \
             transfer {:.1}%",
            fraction * 100.0
        );
    }

    #[test]
    fn driver_burst_stays_on_the_driver_timeline() {
        let frames = 98_304;
        let reference = ar_process(
            &deterministic_noise(frames),
            &predictor_for_resonance(880.0, 0.96),
        );
        let mut driver = vec![0.0_f32; frames];
        let start = 41_000;
        let end = 49_000;
        driver[start..end].copy_from_slice(&deterministic_noise(end - start));
        let output = render_latent_convolution_bank(
            &reference,
            &driver,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let memory_frames = (DEFAULT_MEMORY_MS * 0.001 * SAMPLE_RATE as f32).round() as usize;
        let inside = output[start - FRAME_FRAMES..end + memory_frames + 2 * FRAME_FRAMES]
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>();
        let outside = output[..start - 2 * FRAME_FRAMES]
            .iter()
            .chain(&output[end + memory_frames + 3 * FRAME_FRAMES..])
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>();
        assert!(
            inside > outside * 100.0,
            "B event escaped its timeline: inside {inside:.4}, outside {outside:.4}"
        );
    }

    #[test]
    fn learned_response_can_ring_after_a_short_driver_excitation() {
        let frames = 131_072;
        let mut reference_innovation = vec![0.0_f32; frames];
        for onset in (8_000..frames).step_by(20_000) {
            reference_innovation[onset] = 0.2;
        }
        let reference = ar_process(
            &reference_innovation,
            &predictor_for_resonance(930.0, 0.9997),
        );
        let onset = 60_000;
        let mut driver = vec![0.0_f32; frames];
        driver[onset..onset + 32].copy_from_slice(&deterministic_noise(32));
        let output = render_latent_convolution_bank(
            &reference,
            &driver,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let delayed_tail = rms(&output[onset + 2_000..onset + 7_000]);
        let distant_floor = rms(&output[onset + 14_000..onset + 19_000]);
        assert!(
            delayed_tail > distant_floor * 5.0 && delayed_tail > 1.0e-5,
            "learned A response did not outlive B: tail {delayed_tail:.7}, \
             floor {distant_floor:.7}"
        );
    }

    #[test]
    fn off_bin_reference_mode_does_not_split_into_fft_bin_beating() {
        let frames = 131_072;
        let reference_frequency = 1_013.7_f32;
        let mut reference = vec![0.0_f32; frames];
        for onset in (8_000..frames - 12_000).step_by(24_000) {
            for offset in 0..10_000 {
                let envelope = (-(offset as f32) / 3_200.0).exp();
                reference[onset + offset] += envelope
                    * (2.0 * PI * reference_frequency * offset as f32 / SAMPLE_RATE as f32).sin()
                    * 0.15;
            }
        }
        let onset = 60_000;
        let mut driver = vec![0.0_f32; frames];
        driver[onset] = 0.5;
        let output = render_latent_convolution_bank(
            &reference,
            &driver,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let tail = &output[onset + 1_500..onset + 7_500];
        let bin_hz = SAMPLE_RATE as f32 / FRAME_FRAMES as f32;
        let lower_bin = (reference_frequency / bin_hz).floor() * bin_hz;
        let upper_bin = lower_bin + bin_hz;
        let desired = sinusoidal_projection(tail, reference_frequency);
        let bin_centered =
            sinusoidal_projection(tail, lower_bin).max(sinusoidal_projection(tail, upper_bin));
        eprintln!(
            "off-bin mode: desired/bin-centered projection {:.2}x",
            desired / bin_centered.max(1.0e-20)
        );
        assert!(
            desired > bin_centered * 1.5,
            "off-bin A mode became bin tones: desired {desired:.4}, \
             strongest {lower_bin:.2}/{upper_bin:.2} Hz bin {bin_centered:.4}"
        );
    }

    #[test]
    fn stochastic_learned_tail_does_not_repeat_every_fft_frame() {
        let frames = 147_456;
        let mut reference = vec![0.0_f32; frames];
        let noise = deterministic_noise(frames);
        for onset in (8_000..frames - 10_000).step_by(24_000) {
            for offset in 0..8_000 {
                reference[onset + offset] +=
                    noise[onset + offset] * (-(offset as f32) / 2_800.0).exp();
            }
        }
        let onset = 64_000;
        let mut driver = vec![0.0_f32; frames];
        driver[onset] = 0.5;
        let output = render_latent_convolution_bank(
            &reference,
            &driver,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let tail = &output[onset + 1_500..onset + 7_500];
        let normalized_autocorrelation = |lag: usize| {
            let numerator = tail[lag..]
                .iter()
                .zip(&tail[..tail.len() - lag])
                .map(|(&current, &past)| f64::from(current) * f64::from(past))
                .sum::<f64>();
            let denominator = tail
                .iter()
                .map(|&sample| f64::from(sample).powi(2))
                .sum::<f64>();
            numerator / denominator.max(1.0e-20)
        };
        let frame_repeat = normalized_autocorrelation(FRAME_FRAMES).abs();
        eprintln!("stochastic tail 1024-sample correlation: {frame_repeat:.3}");
        assert!(
            frame_repeat < 0.25,
            "stochastic A tail repeats every FFT frame with \
             correlation {frame_repeat:.3}"
        );
    }

    #[test]
    fn self_supervised_factorization_explains_a_known_convolutive_scene() {
        let frames = 384;
        let lags = 6;
        let mut responses = vec![0.0_f32; COMPONENTS * lags * ANALYSIS_BANDS];
        let mut activations = vec![0.0_f32; COMPONENTS * frames];
        for component in 0..3 {
            for lag in 0..lags {
                let band = 5 + component * 13 + lag / 2;
                responses[response_index(component, lag, band, lags)] = (-0.45 * lag as f32).exp();
            }
            for time in (20 + component * 17..frames - lags).step_by(53 + component * 11) {
                activations[component * frames + time] = 0.8 + 0.1 * component as f32;
            }
        }
        let input = reconstruct(&responses, &activations, frames, lags);
        let model = ConvolutiveModel::fit(&input, frames, lags, NEVER_CANCELLED).unwrap();
        let model_error = input
            .iter()
            .zip(&model.reconstruction)
            .map(|(&actual, &predicted)| f64::from(actual - predicted).powi(2))
            .sum::<f64>();
        let mut baseline_error = 0.0_f64;
        for band in 0..ANALYSIS_BANDS {
            let row = &input[band * frames..(band + 1) * frames];
            let mean = row.iter().sum::<f32>() / frames as f32;
            baseline_error += row
                .iter()
                .map(|&value| f64::from(value - mean).powi(2))
                .sum::<f64>();
        }
        assert!(
            model_error < baseline_error * 0.35,
            "convolutive model error {model_error:.4} was not much below \
             stationary baseline {baseline_error:.4}"
        );
    }

    #[test]
    fn soft_routing_matches_response_dynamics_without_source_labels() {
        let frames = 256;
        let lags = 8;
        let make_model = |permutation: &[usize; COMPONENTS]| {
            let mut responses = vec![0.0_f32; COMPONENTS * lags * ANALYSIS_BANDS];
            let mut activations = vec![0.0_f32; COMPONENTS * frames];
            for output_component in 0..COMPONENTS {
                let character = permutation[output_component];
                let decay = 0.15 + 0.09 * character as f32;
                for lag in 0..lags {
                    responses[response_index(output_component, lag, 4 + character * 5, lags)] =
                        (-decay * lag as f32).exp();
                }
                let spacing = 13 + character * 7;
                for time in (character + 3..frames).step_by(spacing) {
                    activations[output_component * frames + time] = 1.0;
                }
            }
            ConvolutiveModel {
                frames,
                lags,
                responses,
                activations,
                reconstruction: vec![0.0; ANALYSIS_BANDS * frames],
            }
        };
        let identity = std::array::from_fn(|index| index);
        let reverse = std::array::from_fn(|index| COMPONENTS - 1 - index);
        let reference = make_model(&identity);
        let driver = make_model(&reverse);
        let routing = soft_activity_routing(&reference, &driver);
        for reference_component in 0..COMPONENTS {
            let expected_driver = COMPONENTS - 1 - reference_component;
            let (actual_driver, &actual_weight) = routing
                [reference_component * COMPONENTS..(reference_component + 1) * COMPONENTS]
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap();
            assert_eq!(
                actual_driver, expected_driver,
                "slot {reference_component} routed to {actual_driver}, \
                 expected {expected_driver} (weight {actual_weight:.3})"
            );
        }
    }

    #[test]
    fn stationary_inputs_do_not_acquire_a_hop_rate_power_pulse() {
        let frames = 131_072;
        let reference = ar_process(
            &deterministic_noise(frames),
            &predictor_for_resonance(1_350.0, 0.96),
        );
        let mut driver_noise = deterministic_noise(frames);
        driver_noise.rotate_left(31_337);
        let output = render_latent_convolution_bank(
            &reference,
            &driver_noise,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let guard = 8_192;
        let hop_phase_ripple = |samples: &[f32]| {
            let mut phase_power = vec![0.0_f64; HOP_FRAMES];
            let mut phase_count = vec![0_usize; HOP_FRAMES];
            for (index, &sample) in samples.iter().enumerate() {
                let phase = index % HOP_FRAMES;
                phase_power[phase] += f64::from(sample).powi(2);
                phase_count[phase] += 1;
            }
            let phase_rms = phase_power
                .iter()
                .zip(phase_count)
                .map(|(&power, count)| (power / count as f64).sqrt())
                .collect::<Vec<_>>();
            let minimum = phase_rms.iter().copied().fold(f64::INFINITY, f64::min);
            let maximum = phase_rms.iter().copied().fold(0.0_f64, f64::max);
            20.0 * (maximum / minimum.max(1.0e-20)).log10()
        };
        let input_ripple = hop_phase_ripple(&driver_noise[guard..frames - guard]);
        let output_ripple = hop_phase_ripple(&output[guard..frames - guard]);
        assert!(
            output_ripple < input_ripple + 0.75 && output_ripple < 2.5,
            "stationary hop-phase ripple grew from {input_ripple:.2} dB \
             to {output_ripple:.2} dB"
        );
    }

    #[test]
    fn stochastic_reference_is_not_misclassified_as_disposable_noise() {
        let frames = 65_536;
        let reference = deterministic_noise(frames);
        let driver = ar_process(
            &deterministic_noise(frames),
            &predictor_for_resonance(1_200.0, 0.97),
        );
        let output = render_latent_convolution_bank(
            &reference,
            &driver,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let guard = 4_096;
        let driver_ratio = resonance_ratio(&driver[guard..frames - guard], 1_200.0, 6_000.0);
        let output_ratio = resonance_ratio(&output[guard..frames - guard], 1_200.0, 6_000.0);
        assert!(
            output_ratio < driver_ratio.sqrt(),
            "stochastic A did not flatten B: B {driver_ratio:.3}, output {output_ratio:.3}"
        );
        assert!(rms(&output) > 1.0e-4);
    }

    #[test]
    fn silence_is_finite_and_silent() {
        let silence = vec![0.0_f32; 8_192];
        let noise = deterministic_noise(silence.len());
        assert_eq!(
            render_latent_convolution_bank(
                &silence,
                &silence,
                DEFAULT_TRANSFER,
                DEFAULT_MEMORY_MS,
                NEVER_CANCELLED
            )
            .unwrap(),
            silence
        );
        let silent_driver = render_latent_convolution_bank(
            &noise,
            &silence,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        let silent_reference = render_latent_convolution_bank(
            &silence,
            &noise,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            NEVER_CANCELLED,
        )
        .unwrap();
        assert!(silent_driver.iter().all(|&sample| sample == 0.0));
        assert!(silent_reference.iter().all(|&sample| sample == 0.0));
    }

    #[test]
    fn cancellation_interrupts_factorization() {
        let reference = deterministic_noise(131_072);
        let mut driver = reference.clone();
        driver.rotate_left(17_123);
        let calls = Cell::new(0_usize);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() >= 8
        };
        let error = render_latent_convolution_bank(
            &reference,
            &driver,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
            &cancelled,
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
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
        let mut output = render_latent_convolution_bank(
            &reference.samples,
            &driver.samples,
            DEFAULT_TRANSFER,
            DEFAULT_MEMORY_MS,
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
            elapsed.as_secs_f32() < 8.0,
            "release render took {elapsed:?}"
        );
        let window = root_hann_window();
        let band_map = logarithmic_band_map();
        let reference_analysis = analyze_spectrogram(
            &reference.samples,
            &window,
            &band_map,
            false,
            NEVER_CANCELLED,
        )
        .unwrap();
        let driver_analysis =
            analyze_spectrogram(&driver.samples, &window, &band_map, false, NEVER_CANCELLED)
                .unwrap();
        let output_analysis =
            analyze_spectrogram(&output, &window, &band_map, false, NEVER_CANCELLED).unwrap();
        let reference_profile = normalized_band_profile(&reference_analysis);
        let driver_profile = normalized_band_profile(&driver_analysis);
        let output_profile = normalized_band_profile(&output_analysis);
        let source_distance = profile_distance(&reference_profile, &driver_profile);
        let output_to_reference = profile_distance(&output_profile, &reference_profile);
        let spectral_transfer = 1.0 - output_to_reference / source_distance.max(1.0e-20);
        let log_power = |analysis: &SpectrogramAnalysis| {
            analysis
                .frame_power
                .iter()
                .map(|power| (1.0 + power).ln())
                .collect::<Vec<_>>()
        };
        let temporal_correlation =
            correlation(&log_power(&driver_analysis), &log_power(&output_analysis));
        assert!(
            spectral_transfer > 0.55,
            "real-source A character transfer was only {:.1}%",
            spectral_transfer * 100.0
        );
        assert!(
            temporal_correlation > 0.70,
            "real-source B activity correlation was only {temporal_correlation:.3}"
        );
        eprintln!(
            "latent convolution bank real-source DSP: {elapsed:?}; \
             A spectral transfer {:.1}%; B activity correlation {temporal_correlation:.3}",
            spectral_transfer * 100.0
        );
    }
}
