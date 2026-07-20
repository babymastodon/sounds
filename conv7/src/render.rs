use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Serialize;

use crate::audio::{
    AudioClip, StereoMetrics, condition_stereo_output, measure_wav, read_prepared_clip,
    validate_stereo_metrics, write_pcm16_stereo,
};
use crate::convolution::{CUT_FADE_MILLISECONDS, TRIM_FRACTION_OF_SHORTER};
use crate::convolution::{PairJob, convolve_stereo_spectra, group_jobs, make_jobs, prepare_group};
use crate::manifest::{SourceEntry, is_long_duration, is_short_duration, load_manifest};

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub manifest: PathBuf,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub jobs: usize,
    pub force: bool,
}

#[derive(Clone, Debug)]
pub struct VerifyOptions {
    pub manifest: PathBuf,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub jobs: usize,
}

#[derive(Clone, Debug, Serialize)]
struct PairMetrics {
    pair: String,
    left: String,
    right: String,
    path: String,
    channels: u16,
    trim_frames: usize,
    trim_seconds: f64,
    frames: usize,
    duration_seconds: f64,
    peak: f32,
    rms: f32,
    rms_dbfs: f32,
    dc_offset: f32,
    clipped_samples: usize,
    non_finite_samples: usize,
    left_peak: f32,
    left_rms_dbfs: f32,
    left_dc_offset: f32,
    right_peak: f32,
    right_rms_dbfs: f32,
    right_dc_offset: f32,
    stereo_difference_rms: f32,
    stereo_difference_rms_dbfs: f32,
}

#[derive(Debug, Serialize)]
struct VerificationReport {
    status: &'static str,
    source_count: usize,
    category_count: usize,
    sources_per_category: usize,
    short_input_count: usize,
    long_input_count: usize,
    matrix_rows: usize,
    matrix_columns: usize,
    ordered_matrix_cells: usize,
    unique_pair_files: usize,
    sample_rate: u32,
    channels: u16,
    trim_fraction_of_shorter: f32,
    cut_fade_milliseconds: u32,
    minimum_input_seconds: f64,
    maximum_input_seconds: f64,
    minimum_output_rms_dbfs: f32,
    maximum_output_rms_dbfs: f32,
    maximum_output_peak: f32,
    maximum_left_right_rms_delta_db: f32,
    minimum_stereo_difference_rms_dbfs: f32,
    maximum_stereo_difference_rms_dbfs: f32,
    verified_stereo_pairs: usize,
}

pub fn render_matrix(options: RenderOptions) -> Result<()> {
    if options.jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let sources = load_manifest(&options.manifest)?;
    let clips = load_clips(&sources, &options.input_dir)?;
    fs::create_dir_all(options.output_dir.join("wav"))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .thread_name(|index| format!("conv7-{index}"))
        .build()?;
    let started = Instant::now();
    let (short_indices, long_indices) = duration_indices(&sources);
    let all_jobs = make_jobs(&clips, &short_indices, &long_indices);
    let expected_pairs = all_jobs.len();
    let grouped = group_jobs(all_jobs);
    let completed = AtomicUsize::new(0);
    let mut all_metrics = Vec::with_capacity(expected_pairs);

    eprintln!(
        "rendering {} cross-duration pairs ({}x{} bipartite matrix) on {} threads",
        expected_pairs,
        short_indices.len(),
        long_indices.len(),
        options.jobs
    );

    for (fft_len, jobs) in grouped {
        eprintln!(
            "FFT group {fft_len}: {} pairs; caching source spectra",
            jobs.len()
        );
        let group = pool.install(|| prepare_group(fft_len, jobs, &clips))?;
        let group_metrics = pool.install(|| {
            group
                .jobs
                .par_iter()
                .map(|job| {
                    let path = pair_path(&options.output_dir, &clips, job);
                    let metrics = if path.exists() && !options.force {
                        let metrics = measure_wav(&path)?;
                        validate_stereo_metrics(
                            &metrics,
                            job.output_frames,
                            &path.display().to_string(),
                        )?;
                        metrics
                    } else {
                        let mut output = convolve_stereo_spectra(&group, job, &clips)?;
                        let metrics = condition_stereo_output(&mut output)?;
                        write_pcm16_stereo(&path, &output)?;
                        metrics
                    };
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if done.is_multiple_of(10) || done == expected_pairs {
                        eprintln!("completed {done}/{expected_pairs} pairs");
                    }
                    Ok(pair_metrics(&options.output_dir, &clips, job, metrics))
                })
                .collect::<Result<Vec<_>>>()
        })?;
        all_metrics.extend(group_metrics);
    }

    all_metrics.sort_by(|a, b| a.pair.cmp(&b.pair));
    write_metrics(&options.output_dir, &all_metrics)?;
    write_matrix(&options.output_dir, &clips, &short_indices, &long_indices)?;
    eprintln!("render completed in {:.1?}", started.elapsed());

    verify_loaded(&sources, &clips, &options.output_dir, &pool)?;
    Ok(())
}

pub fn verify_matrix(options: VerifyOptions) -> Result<()> {
    if options.jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let sources = load_manifest(&options.manifest)?;
    let clips = load_clips(&sources, &options.input_dir)?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .thread_name(|index| format!("verify-{index}"))
        .build()?;
    verify_loaded(&sources, &clips, &options.output_dir, &pool)
}

fn load_clips(sources: &[SourceEntry], input_dir: &Path) -> Result<Vec<AudioClip>> {
    let clips = sources
        .par_iter()
        .map(|source| {
            let path = input_dir.join(format!("{}.wav", source.id));
            read_prepared_clip(&source.id, &path, source.seconds)
        })
        .collect::<Result<Vec<_>>>()?;
    eprintln!("loaded and conditioned {} prepared inputs", clips.len());
    Ok(clips)
}

fn verify_loaded(
    sources: &[SourceEntry],
    clips: &[AudioClip],
    output_dir: &Path,
    pool: &rayon::ThreadPool,
) -> Result<()> {
    let (short_indices, long_indices) = duration_indices(sources);
    let jobs = make_jobs(clips, &short_indices, &long_indices);
    let metrics = pool.install(|| {
        jobs.par_iter()
            .map(|job| {
                let path = pair_path(output_dir, clips, job);
                let metrics = measure_wav(&path)?;
                validate_stereo_metrics(&metrics, job.output_frames, &path.display().to_string())?;
                Ok(metrics)
            })
            .collect::<Result<Vec<_>>>()
    })?;

    let actual_wavs = fs::read_dir(output_dir.join("wav"))?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "wav"))
        .count();
    if actual_wavs != jobs.len() {
        bail!("found {actual_wavs} WAVs, expected exactly {}", jobs.len());
    }

    let stereo_difference = metrics
        .iter()
        .map(|metric| metric.stereo_difference_rms_dbfs)
        .collect::<Vec<_>>();
    let verified_stereo_pairs = stereo_difference
        .iter()
        .filter(|&&difference| difference > -80.0)
        .count();
    if verified_stereo_pairs != jobs.len() {
        bail!(
            "only {verified_stereo_pairs}/{} pairs have distinct stereo channels",
            jobs.len()
        );
    }
    let minimum_stereo_difference = stereo_difference
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);

    let report = VerificationReport {
        status: "pass",
        source_count: sources.len(),
        category_count: crate::manifest::REQUIRED_DOMAINS.len(),
        sources_per_category: 2,
        short_input_count: short_indices.len(),
        long_input_count: long_indices.len(),
        matrix_rows: short_indices.len(),
        matrix_columns: long_indices.len(),
        ordered_matrix_cells: short_indices.len() * long_indices.len(),
        unique_pair_files: jobs.len(),
        sample_rate: crate::audio::SAMPLE_RATE,
        channels: crate::audio::CHANNELS,
        trim_fraction_of_shorter: TRIM_FRACTION_OF_SHORTER,
        cut_fade_milliseconds: CUT_FADE_MILLISECONDS,
        minimum_input_seconds: sources
            .iter()
            .map(|source| source.seconds)
            .fold(f64::INFINITY, f64::min),
        maximum_input_seconds: sources
            .iter()
            .map(|source| source.seconds)
            .fold(f64::NEG_INFINITY, f64::max),
        minimum_output_rms_dbfs: metrics
            .iter()
            .map(|metric| metric.rms_dbfs)
            .fold(f32::INFINITY, f32::min),
        maximum_output_rms_dbfs: metrics
            .iter()
            .map(|metric| metric.rms_dbfs)
            .fold(f32::NEG_INFINITY, f32::max),
        maximum_output_peak: metrics.iter().map(|metric| metric.peak).fold(0.0, f32::max),
        maximum_left_right_rms_delta_db: metrics
            .iter()
            .map(|metric| (metric.left_rms_dbfs - metric.right_rms_dbfs).abs())
            .fold(0.0, f32::max),
        minimum_stereo_difference_rms_dbfs: minimum_stereo_difference,
        maximum_stereo_difference_rms_dbfs: stereo_difference
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max),
        verified_stereo_pairs,
    };
    let report_path = output_dir.join("verification.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    eprintln!(
        "verification passed: {} inputs, {} unique pairs, RMS {:.1}..{:.1} dBFS, peak {:.3}",
        report.source_count,
        report.unique_pair_files,
        report.minimum_output_rms_dbfs,
        report.maximum_output_rms_dbfs,
        report.maximum_output_peak
    );
    Ok(())
}

fn pair_path(output_dir: &Path, clips: &[AudioClip], job: &PairJob) -> PathBuf {
    output_dir.join("wav").join(format!(
        "{:02}_{}__{:02}_{}.wav",
        job.left + 1,
        clips[job.left].id,
        job.right + 1,
        clips[job.right].id
    ))
}

fn pair_metrics(
    output_dir: &Path,
    clips: &[AudioClip],
    job: &PairJob,
    audio: StereoMetrics,
) -> PairMetrics {
    let path = pair_path(output_dir, clips, job);
    PairMetrics {
        pair: format!("{:02}-{:02}", job.left + 1, job.right + 1),
        left: clips[job.left].id.clone(),
        right: clips[job.right].id.clone(),
        path: path
            .strip_prefix(output_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned(),
        channels: crate::audio::CHANNELS,
        trim_frames: job.trim_frames,
        trim_seconds: job.trim_frames as f64 / f64::from(crate::audio::SAMPLE_RATE),
        frames: audio.frames,
        duration_seconds: audio.duration_seconds,
        peak: audio.peak,
        rms: audio.rms,
        rms_dbfs: audio.rms_dbfs,
        dc_offset: audio.dc_offset,
        clipped_samples: audio.clipped_samples,
        non_finite_samples: audio.non_finite_samples,
        left_peak: audio.left_peak,
        left_rms_dbfs: audio.left_rms_dbfs,
        left_dc_offset: audio.left_dc_offset,
        right_peak: audio.right_peak,
        right_rms_dbfs: audio.right_rms_dbfs,
        right_dc_offset: audio.right_dc_offset,
        stereo_difference_rms: audio.stereo_difference_rms,
        stereo_difference_rms_dbfs: audio.stereo_difference_rms_dbfs,
    }
}

fn write_metrics(output_dir: &Path, metrics: &[PairMetrics]) -> Result<()> {
    let path = output_dir.join("metrics.csv");
    let mut writer = csv::Writer::from_path(&path)?;
    for metric in metrics {
        writer.serialize(metric)?;
    }
    writer.flush()?;
    Ok(())
}

fn duration_indices(sources: &[SourceEntry]) -> (Vec<usize>, Vec<usize>) {
    let short = sources
        .iter()
        .enumerate()
        .filter_map(|(index, source)| is_short_duration(source.seconds).then_some(index))
        .collect();
    let long = sources
        .iter()
        .enumerate()
        .filter_map(|(index, source)| is_long_duration(source.seconds).then_some(index))
        .collect();
    (short, long)
}

fn write_matrix(
    output_dir: &Path,
    clips: &[AudioClip],
    short_indices: &[usize],
    long_indices: &[usize],
) -> Result<()> {
    let path = output_dir.join("matrix.csv");
    let mut writer = csv::Writer::from_path(&path)?;
    let mut header = vec!["short\\long".to_owned()];
    header.extend(long_indices.iter().map(|&index| clips[index].id.clone()));
    writer.write_record(header)?;

    for &row_index in short_indices {
        let mut record = vec![clips[row_index].id.clone()];
        for &column_index in long_indices {
            let job = PairJob {
                left: row_index,
                right: column_index,
                trim_frames: 0,
                output_frames: 0,
                fft_len: 0,
            };
            let path = pair_path(output_dir, clips, &job);
            record.push(
                path.strip_prefix(output_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        writer.write_record(record)?;
    }
    writer.flush().context("flush matrix CSV")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_metrics_is_a_flat_csv_record() {
        let metrics = PairMetrics {
            pair: "01-02".into(),
            left: "left".into(),
            right: "right".into(),
            path: "wav/left__right.wav".into(),
            channels: 2,
            trim_frames: 21,
            trim_seconds: 0.000_437_5,
            frames: 42,
            duration_seconds: 0.000_875,
            peak: 0.5,
            rms: 0.1,
            rms_dbfs: -20.0,
            dc_offset: 0.0,
            clipped_samples: 0,
            non_finite_samples: 0,
            left_peak: 0.5,
            left_rms_dbfs: -20.0,
            left_dc_offset: 0.0,
            right_peak: 0.5,
            right_rms_dbfs: -20.0,
            right_dc_offset: 0.0,
            stereo_difference_rms: 0.1,
            stereo_difference_rms_dbfs: -20.0,
        };
        let mut writer = csv::Writer::from_writer(Vec::new());
        writer.serialize(metrics).unwrap();
        let encoded = String::from_utf8(writer.into_inner().unwrap()).unwrap();

        assert!(encoded.starts_with(
            "pair,left,right,path,channels,trim_frames,trim_seconds,frames,duration_seconds"
        ));
        assert_eq!(encoded.lines().count(), 2);
    }
}
