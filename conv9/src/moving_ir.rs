use std::sync::Arc;

use anyhow::{Result, bail};
use rayon::prelude::*;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use crate::audio::{AudioClip, SAMPLE_RATE};
use crate::dsp::AlgorithmParameters;

const PARTITION_FRAMES: usize = 1_024;
const FFT_FRAMES: usize = PARTITION_FRAMES * 2;
const FFT_BINS: usize = FFT_FRAMES / 2 + 1;
const PARALLEL_BATCH_BLOCKS: usize = 128;
const PARALLEL_BATCH_SNAPSHOTS: usize = 8;
const WORKSPACE_LIMIT_BYTES: usize = 384 * 1024 * 1024;

#[derive(Clone, Copy)]
struct FilterBlend {
    lower: usize,
    upper: usize,
    lower_gain: f32,
    upper_gain: f32,
}

pub(crate) fn render_moving_impulse_response(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    parameters: AlgorithmParameters,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let ir_frames =
        seconds_to_frames(parameters.moving_ir_seconds, SAMPLE_RATE).min(clip_b.samples.len());
    let update_frames = seconds_to_frames(parameters.moving_ir_update_seconds, SAMPLE_RATE);
    render_moving_ir_samples(
        &clip_a.samples,
        &clip_b.samples,
        ir_frames,
        update_frames,
        parameters.moving_ir_taper,
        cancelled,
    )
}

fn render_moving_ir_samples(
    source: &[f32],
    impulse_source: &[f32],
    ir_frames: usize,
    update_frames: usize,
    taper: f32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    if source.is_empty() || impulse_source.is_empty() || ir_frames == 0 || update_frames == 0 {
        bail!("moving IR requires non-empty sources and positive durations");
    }
    if cancelled() {
        bail!("render cancelled");
    }

    let input_blocks = source.len().div_ceil(PARTITION_FRAMES);
    let ir_partitions = ir_frames.div_ceil(PARTITION_FRAMES);
    let output_blocks = input_blocks + ir_partitions - 1;
    let last_center = (input_blocks - 1)
        .saturating_mul(PARTITION_FRAMES)
        .saturating_add(PARTITION_FRAMES / 2)
        .min(source.len() - 1);
    let snapshot_count = last_center.div_ceil(update_frames) + 2;
    validate_workspace(input_blocks, ir_partitions, output_blocks, snapshot_count)?;

    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(FFT_FRAMES);
    let inverse = planner.plan_fft_inverse(FFT_FRAMES);

    let input_spectra =
        transform_input_blocks(source, input_blocks, Arc::clone(&forward), cancelled)?;
    let filter_spectra = transform_filter_snapshots(
        impulse_source,
        source.len(),
        ir_frames,
        update_frames,
        taper,
        ir_partitions,
        snapshot_count,
        Arc::clone(&forward),
        cancelled,
    )?;
    let coherence = adjacent_filter_coherence(&filter_spectra, ir_partitions, snapshot_count);
    let blends = filter_blends(
        input_blocks,
        source.len(),
        update_frames,
        snapshot_count,
        &coherence,
    );

    let time_blocks = synthesize_output_blocks(
        &input_spectra,
        &filter_spectra,
        &blends,
        input_blocks,
        ir_partitions,
        output_blocks,
        inverse,
        cancelled,
    )?;
    if cancelled() {
        bail!("render cancelled");
    }
    Ok(overlap_add_output(
        &time_blocks,
        source.len() + ir_frames - 1,
    ))
}

fn transform_input_blocks(
    source: &[f32],
    input_blocks: usize,
    forward: Arc<dyn RealToComplex<f32>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Complex32>> {
    let mut spectra = vec![Complex32::new(0.0, 0.0); input_blocks * FFT_BINS];
    for batch_start in (0..input_blocks).step_by(PARALLEL_BATCH_BLOCKS) {
        if cancelled() {
            bail!("render cancelled");
        }
        let batch_end = (batch_start + PARALLEL_BATCH_BLOCKS).min(input_blocks);
        spectra[batch_start * FFT_BINS..batch_end * FFT_BINS]
            .par_chunks_mut(FFT_BINS)
            .enumerate()
            .try_for_each_init(
                || ForwardWorkspace::new(Arc::clone(&forward)),
                |workspace, (local_index, destination)| -> Result<()> {
                    let block = batch_start + local_index;
                    workspace.time.fill(0.0);
                    let start = block * PARTITION_FRAMES;
                    let end = (start + PARTITION_FRAMES).min(source.len());
                    workspace.time[..end - start].copy_from_slice(&source[start..end]);
                    workspace
                        .forward
                        .process(&mut workspace.time, &mut workspace.spectrum)?;
                    destination.copy_from_slice(&workspace.spectrum);
                    Ok(())
                },
            )?;
    }
    Ok(spectra)
}

#[allow(clippy::too_many_arguments)]
fn transform_filter_snapshots(
    impulse_source: &[f32],
    source_frames: usize,
    ir_frames: usize,
    update_frames: usize,
    taper: f32,
    ir_partitions: usize,
    snapshot_count: usize,
    forward: Arc<dyn RealToComplex<f32>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Complex32>> {
    let snapshot_bins = ir_partitions * FFT_BINS;
    let mut spectra = vec![Complex32::new(0.0, 0.0); snapshot_count * snapshot_bins];
    for batch_start in (0..snapshot_count).step_by(PARALLEL_BATCH_SNAPSHOTS) {
        if cancelled() {
            bail!("render cancelled");
        }
        let batch_end = (batch_start + PARALLEL_BATCH_SNAPSHOTS).min(snapshot_count);
        spectra[batch_start * snapshot_bins..batch_end * snapshot_bins]
            .par_chunks_mut(snapshot_bins)
            .enumerate()
            .try_for_each_init(
                || ForwardWorkspace::new(Arc::clone(&forward)),
                |workspace, (local_index, destination)| -> Result<()> {
                    let snapshot = batch_start + local_index;
                    let anchor = snapshot
                        .saturating_mul(update_frames)
                        .min(source_frames - 1);
                    let ir = prepare_ir_snapshot(
                        impulse_source,
                        source_frames,
                        anchor,
                        ir_frames,
                        taper,
                    );
                    for partition in 0..ir_partitions {
                        workspace.time.fill(0.0);
                        let start = partition * PARTITION_FRAMES;
                        let end = (start + PARTITION_FRAMES).min(ir.len());
                        workspace.time[..end - start].copy_from_slice(&ir[start..end]);
                        workspace
                            .forward
                            .process(&mut workspace.time, &mut workspace.spectrum)?;
                        let target =
                            &mut destination[partition * FFT_BINS..(partition + 1) * FFT_BINS];
                        target.copy_from_slice(&workspace.spectrum);
                    }
                    Ok(())
                },
            )?;
    }
    Ok(spectra)
}

fn prepare_ir_snapshot(
    impulse_source: &[f32],
    source_frames: usize,
    anchor_frame: usize,
    ir_frames: usize,
    taper: f32,
) -> Vec<f32> {
    let normalized_position = if source_frames > 1 {
        anchor_frame as f64 / (source_frames - 1) as f64
    } else {
        0.0
    };
    let center = (normalized_position * (impulse_source.len() - 1) as f64).round() as usize;
    let start = center
        .saturating_sub(ir_frames / 2)
        .min(impulse_source.len().saturating_sub(ir_frames));
    let mut ir = impulse_source[start..start + ir_frames].to_vec();
    let mean = ir.iter().map(|&sample| f64::from(sample)).sum::<f64>() / ir.len() as f64;
    for (index, sample) in ir.iter_mut().enumerate() {
        *sample = (*sample - mean as f32) * tukey_weight(index, ir_frames, taper);
    }
    let energy = ir
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>();
    if energy <= 1.0e-16 {
        ir.fill(0.0);
    } else {
        let gain = energy.sqrt().recip() as f32;
        for sample in &mut ir {
            *sample *= gain;
        }
    }
    ir
}

fn adjacent_filter_coherence(
    filters: &[Complex32],
    ir_partitions: usize,
    snapshot_count: usize,
) -> Vec<f32> {
    let snapshot_bins = ir_partitions * FFT_BINS;
    (0..snapshot_count - 1)
        .into_par_iter()
        .map(|snapshot| {
            let left = &filters[snapshot * snapshot_bins..(snapshot + 1) * snapshot_bins];
            let right = &filters[(snapshot + 1) * snapshot_bins..(snapshot + 2) * snapshot_bins];
            let dot = left
                .chunks_exact(FFT_BINS)
                .zip(right.chunks_exact(FFT_BINS))
                .map(|(a, b)| {
                    let edge =
                        (a[0].conj() * b[0]).re + (a[FFT_BINS - 1].conj() * b[FFT_BINS - 1]).re;
                    let interior = a[1..FFT_BINS - 1]
                        .iter()
                        .zip(&b[1..FFT_BINS - 1])
                        .map(|(&x, &y)| (x.conj() * y).re)
                        .sum::<f32>();
                    edge + 2.0 * interior
                })
                .sum::<f32>()
                / FFT_FRAMES as f32;
            dot.clamp(0.0, 1.0)
        })
        .collect()
}

fn filter_blends(
    input_blocks: usize,
    source_frames: usize,
    update_frames: usize,
    snapshot_count: usize,
    coherence: &[f32],
) -> Vec<FilterBlend> {
    (0..input_blocks)
        .map(|block| {
            let center = block
                .saturating_mul(PARTITION_FRAMES)
                .saturating_add(PARTITION_FRAMES / 2)
                .min(source_frames - 1);
            let lower = (center / update_frames).min(snapshot_count - 2);
            let fraction = (center - lower * update_frames) as f32 / update_frames as f32;
            let upper = lower + 1;
            let lower_linear = 1.0 - fraction;
            let upper_linear = fraction;
            let denominator = (lower_linear * lower_linear
                + upper_linear * upper_linear
                + 2.0 * lower_linear * upper_linear * coherence[lower])
                .max(1.0e-12)
                .sqrt();
            FilterBlend {
                lower,
                upper,
                lower_gain: lower_linear / denominator,
                upper_gain: upper_linear / denominator,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn synthesize_output_blocks(
    input_spectra: &[Complex32],
    filter_spectra: &[Complex32],
    blends: &[FilterBlend],
    input_blocks: usize,
    ir_partitions: usize,
    output_blocks: usize,
    inverse: Arc<dyn ComplexToReal<f32>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let snapshot_bins = ir_partitions * FFT_BINS;
    let mut time_blocks = vec![0.0_f32; output_blocks * FFT_FRAMES];
    for batch_start in (0..output_blocks).step_by(PARALLEL_BATCH_BLOCKS) {
        if cancelled() {
            bail!("render cancelled");
        }
        let batch_end = (batch_start + PARALLEL_BATCH_BLOCKS).min(output_blocks);
        time_blocks[batch_start * FFT_FRAMES..batch_end * FFT_FRAMES]
            .par_chunks_mut(FFT_FRAMES)
            .enumerate()
            .try_for_each_init(
                || InverseWorkspace::new(Arc::clone(&inverse)),
                |workspace, (local_index, destination)| -> Result<()> {
                    let output_block = batch_start + local_index;
                    workspace.spectrum.fill(Complex32::new(0.0, 0.0));
                    let first_partition =
                        output_block.saturating_sub(input_blocks.saturating_sub(1));
                    let last_partition = output_block.min(ir_partitions - 1);
                    for partition in first_partition..=last_partition {
                        let input_block = output_block - partition;
                        let blend = blends[input_block];
                        let input =
                            &input_spectra[input_block * FFT_BINS..(input_block + 1) * FFT_BINS];
                        let filter_offset = partition * FFT_BINS;
                        let lower = &filter_spectra[blend.lower * snapshot_bins + filter_offset
                            ..blend.lower * snapshot_bins + filter_offset + FFT_BINS];
                        let upper = &filter_spectra[blend.upper * snapshot_bins + filter_offset
                            ..blend.upper * snapshot_bins + filter_offset + FFT_BINS];
                        accumulate_interpolated_product(
                            &mut workspace.spectrum,
                            input,
                            lower,
                            upper,
                            blend.lower_gain,
                            blend.upper_gain,
                        );
                    }
                    workspace
                        .inverse
                        .process(&mut workspace.spectrum, &mut workspace.time)?;
                    let normalization = 1.0 / FFT_FRAMES as f32;
                    for (target, &sample) in destination.iter_mut().zip(&workspace.time) {
                        *target = sample * normalization;
                    }
                    Ok(())
                },
            )?;
    }
    Ok(time_blocks)
}

#[inline]
fn accumulate_interpolated_product(
    destination: &mut [Complex32],
    input: &[Complex32],
    lower: &[Complex32],
    upper: &[Complex32],
    lower_gain: f32,
    upper_gain: f32,
) {
    for (((destination, &input), &lower), &upper) in
        destination.iter_mut().zip(input).zip(lower).zip(upper)
    {
        let filter_re = lower.re.mul_add(lower_gain, upper.re * upper_gain);
        let filter_im = lower.im.mul_add(lower_gain, upper.im * upper_gain);
        destination.re += input.re * filter_re - input.im * filter_im;
        destination.im += input.re * filter_im + input.im * filter_re;
    }
}

fn overlap_add_output(time_blocks: &[f32], output_frames: usize) -> Vec<f32> {
    let mut output = vec![0.0_f32; output_frames];
    output
        .par_chunks_mut(PARTITION_FRAMES)
        .enumerate()
        .for_each(|(block, destination)| {
            let current = (block * FFT_FRAMES < time_blocks.len())
                .then(|| &time_blocks[block * FFT_FRAMES..(block + 1) * FFT_FRAMES]);
            let previous = block
                .checked_sub(1)
                .map(|previous| &time_blocks[previous * FFT_FRAMES..(previous + 1) * FFT_FRAMES]);
            for (index, sample) in destination.iter_mut().enumerate() {
                *sample = current.map(|current| current[index]).unwrap_or(0.0)
                    + previous
                        .map(|previous| previous[PARTITION_FRAMES + index])
                        .unwrap_or(0.0);
            }
        });
    output
}

fn validate_workspace(
    input_blocks: usize,
    ir_partitions: usize,
    output_blocks: usize,
    snapshot_count: usize,
) -> Result<()> {
    let complex_bytes = std::mem::size_of::<Complex32>();
    let float_bytes = std::mem::size_of::<f32>();
    let input_bytes = input_blocks
        .checked_mul(FFT_BINS)
        .and_then(|value| value.checked_mul(complex_bytes));
    let filter_bytes = snapshot_count
        .checked_mul(ir_partitions)
        .and_then(|value| value.checked_mul(FFT_BINS))
        .and_then(|value| value.checked_mul(complex_bytes));
    let output_bytes = output_blocks
        .checked_mul(FFT_FRAMES)
        .and_then(|value| value.checked_mul(float_bytes));
    let total = input_bytes
        .zip(filter_bytes)
        .zip(output_bytes)
        .and_then(|((input, filters), output)| input.checked_add(filters)?.checked_add(output))
        .ok_or_else(|| anyhow::anyhow!("moving IR workspace size overflow"))?;
    if total > WORKSPACE_LIMIT_BYTES {
        bail!(
            "moving IR workspace requires {:.1} MiB; reduce IR length or increase update spacing",
            total as f64 / (1024.0 * 1024.0)
        );
    }
    Ok(())
}

fn seconds_to_frames(seconds: f32, sample_rate: u32) -> usize {
    ((seconds * sample_rate as f32).round() as usize).max(1)
}

fn tukey_weight(index: usize, frames: usize, ratio: f32) -> f32 {
    if frames <= 1 {
        return 1.0;
    }
    let phase = index as f32 / (frames - 1) as f32;
    let edge = ratio.clamp(0.0, 1.0) * 0.5;
    if edge <= f32::EPSILON {
        1.0
    } else if phase < edge {
        0.5 * (1.0 - (std::f32::consts::PI * phase / edge).cos())
    } else if phase > 1.0 - edge {
        0.5 * (1.0 - (std::f32::consts::PI * (1.0 - phase) / edge).cos())
    } else {
        1.0
    }
}

struct ForwardWorkspace {
    forward: Arc<dyn RealToComplex<f32>>,
    time: Vec<f32>,
    spectrum: Vec<Complex32>,
}

impl ForwardWorkspace {
    fn new(forward: Arc<dyn RealToComplex<f32>>) -> Self {
        Self {
            time: forward.make_input_vec(),
            spectrum: forward.make_output_vec(),
            forward,
        }
    }
}

struct InverseWorkspace {
    inverse: Arc<dyn ComplexToReal<f32>>,
    spectrum: Vec<Complex32>,
    time: Vec<f32>,
}

impl InverseWorkspace {
    fn new(inverse: Arc<dyn ComplexToReal<f32>>) -> Self {
        Self {
            spectrum: inverse.make_input_vec(),
            time: inverse.make_output_vec(),
            inverse,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;
    use std::hint::black_box;
    use std::time::Instant;

    use rayon::ThreadPoolBuilder;

    use super::*;

    #[test]
    fn constant_filter_matches_direct_linear_convolution_and_retains_tail() {
        let input = deterministic_signal(5_777);
        let filter = normalized_filter(2_301);
        let snapshots = vec![filter.clone(), filter.clone(), filter.clone()];
        let actual = render_with_prepared_snapshots(&input, &snapshots, 2_000).unwrap();
        let expected = direct_convolution(&input, &filter);
        assert_eq!(actual.len(), input.len() + filter.len() - 1);
        assert_close(&actual, &expected, 2.0e-5);
        assert!(
            actual[input.len()..]
                .iter()
                .any(|sample| sample.abs() > 1.0e-5),
            "the complete FIR tail must survive past A's final sample"
        );
    }

    #[test]
    fn source_impulse_produces_the_interpolated_current_ir() {
        let mut input = vec![0.0; PARTITION_FRAMES * 3];
        input[PARTITION_FRAMES] = 1.0;
        let first = normalized_filter(1_377);
        let mut second = first.clone();
        second.rotate_left(37);
        let snapshots = vec![first.clone(), second.clone(), second.clone()];
        let update_frames = PARTITION_FRAMES * 4;
        let output = render_with_prepared_snapshots(&input, &snapshots, update_frames).unwrap();
        let center = PARTITION_FRAMES + PARTITION_FRAMES / 2;
        let fraction = center as f32 / update_frames as f32;
        let coherence = first
            .iter()
            .zip(&second)
            .map(|(&a, &b)| a * b)
            .sum::<f32>()
            .clamp(0.0, 1.0);
        let denominator = ((1.0 - fraction).powi(2)
            + fraction.powi(2)
            + 2.0 * fraction * (1.0 - fraction) * coherence)
            .sqrt();
        let expected = first
            .iter()
            .zip(&second)
            .map(|(&a, &b)| (a * (1.0 - fraction) + b * fraction) / denominator)
            .collect::<Vec<_>>();
        assert_close(
            &output[PARTITION_FRAMES..PARTITION_FRAMES + expected.len()],
            &expected,
            2.0e-5,
        );
    }

    #[test]
    fn identical_ir_snapshots_do_not_swell_during_interpolation() {
        let filter = normalized_filter(1_831);
        let snapshots = vec![filter.clone(), filter.clone(), filter.clone()];
        let input = vec![1.0; PARTITION_FRAMES * 7];
        let output =
            render_with_prepared_snapshots(&input, &snapshots, PARTITION_FRAMES * 2).unwrap();
        let reference = direct_convolution(&input, &filter);
        assert_close(&output, &reference, 2.0e-5);
    }

    #[test]
    fn silence_is_finite_and_does_not_receive_normalization_gain() {
        let output = render_moving_ir_samples(
            &vec![0.0; 5_000],
            &vec![0.0; 7_000],
            2_000,
            1_000,
            0.5,
            &|| false,
        )
        .unwrap();
        assert_eq!(output.len(), 6_999);
        assert!(output.iter().all(|sample| sample.to_bits() == 0));
    }

    #[test]
    fn moving_filter_is_deterministic_across_thread_counts() {
        let source = deterministic_signal(48_000);
        let impulse_source = deterministic_signal(53_000);
        let render = || {
            render_moving_ir_samples(&source, &impulse_source, 12_000, 8_000, 0.5, &|| false)
                .unwrap()
        };
        let serial = ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(render);
        let parallel = ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(render);
        assert_eq!(
            serial
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            parallel
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn update_boundaries_do_not_add_clicks_to_smooth_inputs() {
        let frames = PARTITION_FRAMES * 24;
        let source = (0..frames)
            .map(|index| (TAU * 223.0 * index as f32 / 48_000.0).sin() * 0.2)
            .collect::<Vec<_>>();
        let impulse_source = (0..frames)
            .map(|index| {
                0.6 * (TAU * 371.0 * index as f32 / 48_000.0).sin()
                    + 0.2 * (TAU * 947.0 * index as f32 / 48_000.0).sin()
            })
            .collect::<Vec<_>>();
        let update = PARTITION_FRAMES * 5;
        let output =
            render_moving_ir_samples(&source, &impulse_source, 4_096, update, 0.5, &|| false)
                .unwrap();
        let differences = output
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .collect::<Vec<_>>();
        let ordinary = percentile(&differences, 0.999);
        for boundary in (update..source.len()).step_by(update) {
            let local = differences
                [boundary.saturating_sub(2)..(boundary + 2).min(differences.len())]
                .iter()
                .copied()
                .fold(0.0_f32, f32::max);
            assert!(
                local <= ordinary * 2.0 + 1.0e-5,
                "IR update introduced a discontinuity at {boundary}: {local} vs {ordinary}"
            );
        }
    }

    #[test]
    #[ignore = "manual release-mode moving-IR throughput characterization"]
    fn benchmark_single_and_all_core_render() {
        let seconds = 61;
        let source = deterministic_signal(48_000 * seconds);
        let impulse_source = deterministic_signal(48_000 * seconds + 997);
        let render = || {
            black_box(
                render_moving_ir_samples(
                    black_box(&source),
                    black_box(&impulse_source),
                    36_000,
                    24_000,
                    0.5,
                    &|| false,
                )
                .unwrap(),
            )
        };
        let one = ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        one.install(render);
        let mut serial_runs = (0..3)
            .map(|_| {
                let started = Instant::now();
                one.install(render);
                started.elapsed().as_secs_f64()
            })
            .collect::<Vec<_>>();
        serial_runs.sort_by(f64::total_cmp);
        let serial = serial_runs[serial_runs.len() / 2];

        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1);
        let all = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        all.install(render);
        let mut parallel_runs = (0..3)
            .map(|_| {
                let started = Instant::now();
                all.install(render);
                started.elapsed().as_secs_f64()
            })
            .collect::<Vec<_>>();
        parallel_runs.sort_by(f64::total_cmp);
        let parallel = parallel_runs[parallel_runs.len() / 2];
        eprintln!(
            "moving_ir seconds={seconds} partition={PARTITION_FRAMES} threads={threads} \
             serial_ms={:.1} parallel_ms={:.1} speedup={:.2}x realtime_serial={:.1}x \
             realtime_parallel={:.1}x",
            serial * 1_000.0,
            parallel * 1_000.0,
            serial / parallel,
            seconds as f64 / serial,
            seconds as f64 / parallel,
        );
    }

    fn render_with_prepared_snapshots(
        input: &[f32],
        snapshots: &[Vec<f32>],
        update_frames: usize,
    ) -> Result<Vec<f32>> {
        let ir_frames = snapshots[0].len();
        assert!(snapshots.iter().all(|snapshot| snapshot.len() == ir_frames));
        let input_blocks = input.len().div_ceil(PARTITION_FRAMES);
        let ir_partitions = ir_frames.div_ceil(PARTITION_FRAMES);
        let output_blocks = input_blocks + ir_partitions - 1;
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(FFT_FRAMES);
        let inverse = planner.plan_fft_inverse(FFT_FRAMES);
        let inputs = transform_input_blocks(input, input_blocks, Arc::clone(&forward), &|| false)?;
        let snapshot_bins = ir_partitions * FFT_BINS;
        let mut filters = vec![Complex32::new(0.0, 0.0); snapshots.len() * snapshot_bins];
        for (snapshot_index, snapshot) in snapshots.iter().enumerate() {
            let mut workspace = ForwardWorkspace::new(Arc::clone(&forward));
            for partition in 0..ir_partitions {
                workspace.time.fill(0.0);
                let start = partition * PARTITION_FRAMES;
                let end = (start + PARTITION_FRAMES).min(ir_frames);
                workspace.time[..end - start].copy_from_slice(&snapshot[start..end]);
                workspace
                    .forward
                    .process(&mut workspace.time, &mut workspace.spectrum)?;
                filters[snapshot_index * snapshot_bins + partition * FFT_BINS
                    ..snapshot_index * snapshot_bins + (partition + 1) * FFT_BINS]
                    .copy_from_slice(&workspace.spectrum);
            }
        }
        let coherence = adjacent_filter_coherence(&filters, ir_partitions, snapshots.len());
        let blends = filter_blends(
            input_blocks,
            input.len(),
            update_frames,
            snapshots.len(),
            &coherence,
        );
        let time = synthesize_output_blocks(
            &inputs,
            &filters,
            &blends,
            input_blocks,
            ir_partitions,
            output_blocks,
            inverse,
            &|| false,
        )?;
        Ok(overlap_add_output(&time, input.len() + ir_frames - 1))
    }

    fn deterministic_signal(frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|index| {
                let x = index as f32;
                0.17 * (x * 0.031).sin() + 0.08 * (x * 0.071).cos() + 0.03 * (x * 0.0013).sin()
            })
            .collect()
    }

    fn normalized_filter(frames: usize) -> Vec<f32> {
        let mut filter = deterministic_signal(frames);
        let energy = filter
            .iter()
            .map(|&sample| sample * sample)
            .sum::<f32>()
            .sqrt();
        for sample in &mut filter {
            *sample /= energy;
        }
        filter
    }

    fn direct_convolution(left: &[f32], right: &[f32]) -> Vec<f32> {
        let mut output = vec![0.0; left.len() + right.len() - 1];
        for (left_index, &left_sample) in left.iter().enumerate() {
            for (right_index, &right_sample) in right.iter().enumerate() {
                output[left_index + right_index] += left_sample * right_sample;
            }
        }
        output
    }

    fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
        assert_eq!(actual.len(), expected.len());
        let maximum = actual
            .iter()
            .zip(expected)
            .map(|(&a, &b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(maximum <= tolerance, "maximum error {maximum:e}");
    }

    fn percentile(values: &[f32], proportion: f32) -> f32 {
        let mut sorted = values.to_vec();
        sorted.sort_by(f32::total_cmp);
        sorted[((sorted.len() - 1) as f32 * proportion).round() as usize]
    }
}
