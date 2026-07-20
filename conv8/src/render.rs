use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::audio::{
    AudioClip, StereoMetrics, condition_stereo_output, measure_wav, read_prepared_clip,
    validate_stereo_metrics, write_pcm16_stereo,
};
use crate::convolution::{CUT_FADE_MILLISECONDS, TRIM_FRACTION_OF_SHORTER};
use crate::convolution::{PairJob, convolve_stereo_spectra, group_jobs, make_jobs, prepare_group};
use crate::manifest::{SourceEntry, is_long_duration, is_short_duration, load_manifest};
use crate::pitch::{
    ALGORITHM_VERSION, PitchApproach, PreprocessedClip, chord, chord_index, fingerprint_bytes,
    fingerprint_hex, preprocess,
};

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub manifest: PathBuf,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub jobs: usize,
    pub force: bool,
    pub approach: PitchApproach,
}

#[derive(Clone, Debug)]
pub struct VerifyOptions {
    pub manifest: PathBuf,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub jobs: usize,
    pub approach: PitchApproach,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PairMetrics {
    pair: String,
    approach: String,
    left: String,
    right: String,
    short_fingerprint: String,
    long_fingerprint: String,
    chord_index: usize,
    chord_steps: String,
    chord_frequencies_hz: String,
    pitch_algorithm_version: String,
    processed_role: String,
    parallel_wet_mix_percent: Option<f32>,
    additive_note_db_below_local: Option<f32>,
    preprocess_dry_correlation: f32,
    preprocess_difference_rms_db_relative: f32,
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
    pitch_approach: &'static str,
    pitch_description: &'static str,
    pitch_scale: &'static str,
    chord_count: usize,
    chord_hash: &'static str,
    pitch_algorithm_version: &'static str,
    processed_role: &'static str,
    parallel_wet_mix_percent: Option<f32>,
    additive_note_db_below_local: Option<f32>,
    minimum_preprocess_dry_correlation: f32,
    minimum_preprocess_difference_rms_db_relative: f32,
    maximum_preprocess_difference_rms_db_relative: f32,
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
    let (clips, fingerprints) = load_clips(&sources, &options.input_dir)?;
    fs::create_dir_all(options.output_dir.join("wav"))?;
    let version_path = options.output_dir.join("pitch_algorithm.txt");
    let cache_matches = !options.force
        && fs::read_to_string(&version_path)
            .is_ok_and(|version| version.trim() == ALGORITHM_VERSION);
    if !cache_matches && version_path.exists() {
        fs::remove_file(&version_path)?;
    }
    let thread_approach = options.approach;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .thread_name(move |index| format!("conv8-{}-{index}", thread_approach.slug()))
        .build()?;
    let started = Instant::now();
    let (short_indices, long_indices) = duration_indices(&sources);
    let all_jobs = make_jobs(&clips, &short_indices, &long_indices);
    let expected_pairs = all_jobs.len();
    let grouped = group_jobs(all_jobs);
    let completed = AtomicUsize::new(0);
    let mut all_metrics = Vec::with_capacity(expected_pairs);

    eprintln!(
        "rendering {} {} pairs ({}x{} bipartite matrix) on {} threads",
        expected_pairs,
        options.approach.slug(),
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
                    let selected_chord_index =
                        chord_index(fingerprints[job.left], fingerprints[job.right]);
                    let selected_chord = chord(selected_chord_index);
                    let preprocessing_input =
                        if options.approach == PitchApproach::LongAdditiveSynth {
                            &clips[job.right].samples
                        } else {
                            &clips[job.left].samples
                        };
                    let preprocessing =
                        preprocess(preprocessing_input, selected_chord, options.approach);
                    let path = pair_path(&options.output_dir, &clips, job);
                    let metrics = if path.exists() && cache_matches {
                        let metrics = measure_wav(&path)?;
                        validate_stereo_metrics(
                            &metrics,
                            job.output_frames,
                            &path.display().to_string(),
                        )?;
                        metrics
                    } else {
                        let (track_1, track_2) =
                            if options.approach == PitchApproach::LongAdditiveSynth {
                                (None, Some(preprocessing.samples.as_slice()))
                            } else {
                                (Some(preprocessing.samples.as_slice()), None)
                            };
                        let mut output =
                            convolve_stereo_spectra(&group, job, &clips, track_1, track_2)?;
                        let metrics = condition_stereo_output(&mut output)?;
                        write_pcm16_stereo(&path, &output)?;
                        metrics
                    };
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if done.is_multiple_of(10) || done == expected_pairs {
                        eprintln!("completed {done}/{expected_pairs} pairs");
                    }
                    Ok(pair_metrics(
                        &options.output_dir,
                        &clips,
                        &fingerprints,
                        job,
                        options.approach,
                        &preprocessing,
                        metrics,
                    ))
                })
                .collect::<Result<Vec<_>>>()
        })?;
        all_metrics.extend(group_metrics);
    }

    all_metrics.sort_by(|a, b| a.pair.cmp(&b.pair));
    write_metrics(&options.output_dir, &all_metrics)?;
    write_matrix(&options.output_dir, &clips, &short_indices, &long_indices)?;
    fs::write(&version_path, format!("{ALGORITHM_VERSION}\n"))?;
    eprintln!("render completed in {:.1?}", started.elapsed());

    verify_loaded(
        &sources,
        &clips,
        &options.output_dir,
        &pool,
        options.approach,
        &fingerprints,
    )?;
    Ok(())
}

pub fn verify_matrix(options: VerifyOptions) -> Result<()> {
    if options.jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    let sources = load_manifest(&options.manifest)?;
    let (clips, fingerprints) = load_clips(&sources, &options.input_dir)?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .thread_name(|index| format!("verify-{index}"))
        .build()?;
    verify_loaded(
        &sources,
        &clips,
        &options.output_dir,
        &pool,
        options.approach,
        &fingerprints,
    )
}

fn load_clips(sources: &[SourceEntry], input_dir: &Path) -> Result<(Vec<AudioClip>, Vec<u64>)> {
    let loaded = sources
        .par_iter()
        .map(|source| {
            let path = input_dir.join(format!("{}.wav", source.id));
            let bytes = fs::read(&path)
                .with_context(|| format!("hash prepared input {}", path.display()))?;
            let fingerprint = fingerprint_bytes(&bytes);
            let clip = read_prepared_clip(&source.id, &path, source.seconds)?;
            Ok((clip, fingerprint))
        })
        .collect::<Result<Vec<_>>>()?;
    let (clips, fingerprints) = loaded.into_iter().unzip::<_, _, Vec<_>, Vec<_>>();
    eprintln!("loaded and conditioned {} prepared inputs", clips.len());
    Ok((clips, fingerprints))
}

fn verify_loaded(
    sources: &[SourceEntry],
    clips: &[AudioClip],
    output_dir: &Path,
    pool: &rayon::ThreadPool,
    approach: PitchApproach,
    fingerprints: &[u64],
) -> Result<()> {
    let (short_indices, long_indices) = duration_indices(sources);
    let jobs = make_jobs(clips, &short_indices, &long_indices);
    let version_path = output_dir.join("pitch_algorithm.txt");
    let version = fs::read_to_string(&version_path)
        .with_context(|| format!("read pitch algorithm marker {}", version_path.display()))?;
    if version.trim() != ALGORITHM_VERSION {
        bail!(
            "{} contains pitch algorithm {:?}, expected {ALGORITHM_VERSION}",
            output_dir.display(),
            version.trim()
        );
    }
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
    let pitch_audit = verify_metric_assignments(output_dir, clips, fingerprints, &jobs, approach)?;

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
        pitch_approach: approach.slug(),
        pitch_description: approach.description(),
        pitch_scale: "13-EDT Bohlen-Pierce; 3:1 tritave; chord steps [root, root+6, root+10]",
        chord_count: 13,
        chord_hash: "FNV-1a-64 over prepared WAV bytes, then domain-separated ordered pair modulo 13",
        pitch_algorithm_version: ALGORITHM_VERSION,
        processed_role: approach.processed_role(),
        parallel_wet_mix_percent: approach.parallel_wet_mix().map(|mix| mix * 100.0),
        additive_note_db_below_local: approach.additive_note_db_below_local(),
        minimum_preprocess_dry_correlation: pitch_audit.minimum_correlation,
        minimum_preprocess_difference_rms_db_relative: pitch_audit.minimum_difference_db,
        maximum_preprocess_difference_rms_db_relative: pitch_audit.maximum_difference_db,
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
        "verification passed for {}: {} inputs, {} unique pairs, RMS {:.1}..{:.1} dBFS, peak {:.3}",
        approach.slug(),
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
    fingerprints: &[u64],
    job: &PairJob,
    approach: PitchApproach,
    preprocessing: &PreprocessedClip,
    audio: StereoMetrics,
) -> PairMetrics {
    let path = pair_path(output_dir, clips, job);
    let chord = chord(chord_index(fingerprints[job.left], fingerprints[job.right]));
    PairMetrics {
        pair: format!("{:02}-{:02}", job.left + 1, job.right + 1),
        approach: approach.slug().to_owned(),
        left: clips[job.left].id.clone(),
        right: clips[job.right].id.clone(),
        short_fingerprint: fingerprint_hex(fingerprints[job.left]),
        long_fingerprint: fingerprint_hex(fingerprints[job.right]),
        chord_index: chord.index,
        chord_steps: chord.steps.map(|step| step.to_string()).join(";"),
        chord_frequencies_hz: chord
            .frequencies_hz
            .map(|frequency| format!("{frequency:.6}"))
            .join(";"),
        pitch_algorithm_version: ALGORITHM_VERSION.to_owned(),
        processed_role: approach.processed_role().to_owned(),
        parallel_wet_mix_percent: preprocessing.parallel_wet_mix_percent,
        additive_note_db_below_local: preprocessing.additive_note_db_below_local,
        preprocess_dry_correlation: preprocessing.dry_correlation,
        preprocess_difference_rms_db_relative: preprocessing.difference_rms_db_relative,
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

struct PitchMetadataAudit {
    minimum_correlation: f32,
    minimum_difference_db: f32,
    maximum_difference_db: f32,
}

fn verify_metric_assignments(
    output_dir: &Path,
    clips: &[AudioClip],
    fingerprints: &[u64],
    jobs: &[PairJob],
    approach: PitchApproach,
) -> Result<PitchMetadataAudit> {
    let path = output_dir.join("metrics.csv");
    let mut reader = csv::Reader::from_path(&path)?;
    let rows = reader
        .deserialize::<PairMetrics>()
        .map(|row| row.context("parse pitch metrics row"))
        .collect::<Result<Vec<_>>>()?;
    if rows.len() != jobs.len() {
        bail!(
            "{} metrics contain {} rows, expected {}",
            approach.slug(),
            rows.len(),
            jobs.len()
        );
    }
    let rows = rows
        .into_iter()
        .map(|row| (row.pair.clone(), row))
        .collect::<HashMap<_, _>>();
    for job in jobs {
        let pair = format!("{:02}-{:02}", job.left + 1, job.right + 1);
        let row = rows
            .get(&pair)
            .with_context(|| format!("missing pitch metadata for pair {pair}"))?;
        let expected_chord = chord_index(fingerprints[job.left], fingerprints[job.right]);
        if row.approach != approach.slug()
            || row.left != clips[job.left].id
            || row.right != clips[job.right].id
            || row.short_fingerprint != fingerprint_hex(fingerprints[job.left])
            || row.long_fingerprint != fingerprint_hex(fingerprints[job.right])
            || row.chord_index != expected_chord
            || row.pitch_algorithm_version != ALGORITHM_VERSION
            || row.processed_role != approach.processed_role()
            || row.parallel_wet_mix_percent != approach.parallel_wet_mix().map(|mix| mix * 100.0)
            || row.additive_note_db_below_local != approach.additive_note_db_below_local()
        {
            bail!("pair {pair} has inconsistent deterministic pitch metadata");
        }
        let (minimum_correlation, difference_range) =
            if approach == PitchApproach::LongAdditiveSynth {
                (0.95, -30.0..=-10.0)
            } else {
                (0.98, -40.0..=-18.0)
            };
        if row.preprocess_dry_correlation < minimum_correlation
            || !difference_range.contains(&row.preprocess_difference_rms_db_relative)
        {
            bail!(
                "pair {pair} pitch preprocessing is not subtle: correlation={}, difference={} dB",
                row.preprocess_dry_correlation,
                row.preprocess_difference_rms_db_relative
            );
        }
    }
    Ok(PitchMetadataAudit {
        minimum_correlation: rows
            .values()
            .map(|row| row.preprocess_dry_correlation)
            .fold(f32::INFINITY, f32::min),
        minimum_difference_db: rows
            .values()
            .map(|row| row.preprocess_difference_rms_db_relative)
            .fold(f32::INFINITY, f32::min),
        maximum_difference_db: rows
            .values()
            .map(|row| row.preprocess_difference_rms_db_relative)
            .fold(f32::NEG_INFINITY, f32::max),
    })
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
            approach: "pure_convolution".into(),
            left: "left".into(),
            right: "right".into(),
            short_fingerprint: "0000000000000001".into(),
            long_fingerprint: "0000000000000002".into(),
            chord_index: 7,
            chord_steps: "7;0;4".into(),
            chord_frequencies_hz: "198.0;110.0;154.0".into(),
            pitch_algorithm_version: ALGORITHM_VERSION.into(),
            processed_role: "short".into(),
            parallel_wet_mix_percent: Some(3.0),
            additive_note_db_below_local: None,
            preprocess_dry_correlation: 0.999,
            preprocess_difference_rms_db_relative: -24.0,
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
            "pair,approach,left,right,short_fingerprint,long_fingerprint,chord_index,chord_steps,chord_frequencies_hz,pitch_algorithm_version,processed_role,parallel_wet_mix_percent,additive_note_db_below_local,preprocess_dry_correlation,preprocess_difference_rms_db_relative,path,channels,trim_frames,trim_seconds,frames,duration_seconds"
        ));
        assert_eq!(encoded.lines().count(), 2);
    }
}
