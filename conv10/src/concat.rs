use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader};
use serde::{Deserialize, Serialize};

use crate::audio::{CHANNELS, SAMPLE_RATE, StereoAudio};
use crate::manifest::{is_long_duration, is_short_duration, load_manifest};
use crate::pitch::{ALGORITHM_VERSION, fingerprint_bytes, fingerprint_hex};

const RF64_HEADER_BYTES: u64 = 80;
const DELIVERY_VERSION: &str = "conv10-delivery-v18-embedded-cover-all-formats";

#[derive(Clone, Debug)]
pub struct ConcatOptions {
    pub manifest: PathBuf,
    pub metrics: PathBuf,
    pub output_dir: PathBuf,
    pub scratch_dir: PathBuf,
    pub cover_art: PathBuf,
    pub output_name: String,
    pub crossfade_seconds: f64,
    pub aac_bitrate_kbps: u32,
    pub opus_bitrate_kbps: u32,
    pub metadata: AudioMetadata,
    pub stage: ConcatStage,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConcatStage {
    All,
    AssembleOnly,
    FinalizeOnly,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AudioMetadata {
    pub title: String,
    pub album: String,
    pub artist: String,
    pub album_artist: String,
    pub composer: String,
    pub genre: String,
    pub date: String,
    pub track: String,
    pub disc: String,
    pub comment: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize)]
struct MetricsRow {
    pair: String,
    sequence_index: usize,
    left: String,
    right: String,
    path: String,
    frames: usize,
}

#[derive(Debug, Serialize)]
struct TimelineRow<'a> {
    index: usize,
    pair: &'a str,
    path: &'a str,
    input_frames: usize,
    start_frame: u64,
    start_seconds: f64,
    incoming_crossfade_frames: usize,
    incoming_crossfade_seconds: f64,
}

#[derive(Debug, Serialize)]
struct EncodedFileReport {
    path: String,
    codec: String,
    bytes: u64,
    duration_seconds: f64,
}

#[derive(Debug, Serialize)]
struct ConcatReport {
    status: &'static str,
    input_files: usize,
    sample_rate: u32,
    channels: u16,
    requested_crossfade_seconds: f64,
    full_crossfades: usize,
    shortened_crossfades: usize,
    output_frames: u64,
    output_duration_seconds: f64,
    aac_bitrate_kbps: u32,
    opus_bitrate_kbps: u32,
    metadata: AudioMetadata,
    flac: EncodedFileReport,
    aac: EncodedFileReport,
    opus: EncodedFileReport,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct ConcatRecipe {
    algorithm_version: String,
    delivery_version: String,
    manifest_fingerprint: String,
    metrics_fingerprint: String,
    cover_art_fingerprint: String,
    output_frames: u64,
    crossfade_seconds: f64,
    aac_bitrate_kbps: u32,
    opus_bitrate_kbps: u32,
    metadata: AudioMetadata,
}

struct EncodingTargets<'a> {
    flac: &'a Path,
    aac: &'a Path,
    opus: &'a Path,
}

pub fn concatenate_master(options: ConcatOptions) -> Result<()> {
    if !options.crossfade_seconds.is_finite() || options.crossfade_seconds <= 0.0 {
        bail!("--crossfade-seconds must be a positive finite number");
    }
    if options.aac_bitrate_kbps == 0 {
        bail!("--aac-bitrate-kbps must be at least 1");
    }
    if options.opus_bitrate_kbps == 0 {
        bail!("--opus-bitrate-kbps must be at least 1");
    }
    validate_output_name(&options.output_name)?;
    validate_metadata(&options.metadata)?;
    if !options.cover_art.is_file() {
        bail!("cover art not found: {}", options.cover_art.display());
    }

    let sources = load_manifest(&options.manifest)?;
    let short_ids = sources
        .iter()
        .filter(|source| is_short_duration(source.seconds))
        .map(|source| source.id.as_str())
        .collect::<HashSet<_>>();
    let long_ids = sources
        .iter()
        .filter(|source| is_long_duration(source.seconds))
        .map(|source| source.id.as_str())
        .collect::<HashSet<_>>();
    let mut rows = read_metrics(&options.metrics)?;
    rows.sort_by_key(|row| row.sequence_index);
    validate_bipartite_rows(&rows, &short_ids, &long_ids)?;
    let unique_paths = rows
        .iter()
        .map(|row| row.path.as_str())
        .collect::<HashSet<_>>();
    if unique_paths.len() != rows.len() {
        bail!("metrics contain duplicate WAV paths");
    }

    let requested_frames = (options.crossfade_seconds * f64::from(SAMPLE_RATE)).round() as usize;
    if requested_frames == 0 {
        bail!("crossfade rounds to zero frames at {SAMPLE_RATE} Hz");
    }
    let (transitions, starts, output_frames) = sequence_layout(&rows, requested_frames)?;
    let full_crossfades = transitions
        .iter()
        .filter(|&&frames| frames == requested_frames)
        .count();
    let shortened_crossfades = transitions.len() - full_crossfades;
    let output_seconds = output_frames as f64 / f64::from(SAMPLE_RATE);
    let input_root = options
        .metrics
        .parent()
        .context("metrics path has no parent directory")?;

    fs::create_dir_all(&options.output_dir)?;
    fs::create_dir_all(&options.scratch_dir)?;
    for extension in ["flac", "m4a", "opus"] {
        fs::create_dir_all(options.output_dir.join(extension))?;
    }
    write_timeline(
        &options
            .output_dir
            .join(format!("{}.timeline.csv", options.output_name)),
        &rows,
        &starts,
        &transitions,
    )?;

    let rf64_path = options
        .scratch_dir
        .join(format!("{}.part.rf64.wav", options.output_name));
    let flac_path = options
        .output_dir
        .join("flac")
        .join(format!("{}.flac", options.output_name));
    let aac_path = options
        .output_dir
        .join("m4a")
        .join(format!("{}.m4a", options.output_name));
    let opus_path = options
        .output_dir
        .join("opus")
        .join(format!("{}.opus", options.output_name));
    let recipe_path = options
        .output_dir
        .join(format!("{}.recipe.json", options.output_name));
    let expected_recipe = ConcatRecipe {
        algorithm_version: ALGORITHM_VERSION.to_owned(),
        delivery_version: DELIVERY_VERSION.to_owned(),
        manifest_fingerprint: fingerprint_file(&options.manifest)?,
        metrics_fingerprint: fingerprint_file(&options.metrics)?,
        cover_art_fingerprint: fingerprint_file(&options.cover_art)?,
        output_frames,
        crossfade_seconds: options.crossfade_seconds,
        aac_bitrate_kbps: options.aac_bitrate_kbps,
        opus_bitrate_kbps: options.opus_bitrate_kbps,
        metadata: options.metadata.clone(),
    };
    let recipe_matches = fs::read(&recipe_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ConcatRecipe>(&bytes).ok())
        .is_some_and(|recipe| recipe == expected_recipe);
    if options.stage == ConcatStage::AssembleOnly {
        if options.force || !rf64_path.is_file() {
            remove_if_present(&rf64_path)?;
            assemble_sequence(
                input_root,
                &rows,
                &transitions,
                requested_frames,
                output_frames,
                &rf64_path,
            )?;
        } else {
            eprintln!("reusing assembled RF64 master {}", rf64_path.display());
        }
        eprintln!(
            "assembly passed: {} inputs, {:.2} hours, {}",
            rows.len(),
            output_seconds / 3600.0,
            rf64_path.display()
        );
        return Ok(());
    }

    let rebuild_all = options.force || !recipe_matches;
    let needs_encoding =
        rebuild_all || !flac_path.is_file() || !aac_path.is_file() || !opus_path.is_file();

    if options.stage == ConcatStage::All && needs_encoding {
        remove_if_present(&rf64_path)?;
        assemble_sequence(
            input_root,
            &rows,
            &transitions,
            requested_frames,
            output_frames,
            &rf64_path,
        )?;
        let targets = EncodingTargets {
            flac: &flac_path,
            aac: &aac_path,
            opus: &opus_path,
        };
        encode_outputs(
            &rf64_path,
            &targets,
            options.aac_bitrate_kbps,
            options.opus_bitrate_kbps,
            &options.metadata,
            &options.cover_art,
            rebuild_all,
        )?;
        remove_if_present(&rf64_path)?;
    } else if options.stage == ConcatStage::All {
        eprintln!("reusing recipe-matched FLAC, AAC/M4A, and Opus encodings");
    }

    if options.stage == ConcatStage::FinalizeOnly {
        for path in [&flac_path, &aac_path, &opus_path] {
            if !path.is_file() {
                bail!("encoded output not found: {}", path.display());
            }
        }
    }
    let flac = probe_encoding(&flac_path, "flac", output_seconds)?;
    validate_attached_cover(&flac_path, &options.cover_art)?;
    let aac = probe_encoding(&aac_path, "aac", output_seconds)?;
    validate_attached_cover(&aac_path, &options.cover_art)?;
    let opus = probe_encoding(&opus_path, "opus", output_seconds)?;
    validate_attached_cover(&opus_path, &options.cover_art)?;
    validate_decodable_in_parallel(&[
        ("FLAC", &flac_path),
        ("AAC", &aac_path),
        ("Opus", &opus_path),
    ])?;
    let report = ConcatReport {
        status: "pass",
        input_files: rows.len(),
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        requested_crossfade_seconds: options.crossfade_seconds,
        full_crossfades,
        shortened_crossfades,
        output_frames,
        output_duration_seconds: output_seconds,
        aac_bitrate_kbps: options.aac_bitrate_kbps,
        opus_bitrate_kbps: options.opus_bitrate_kbps,
        metadata: options.metadata.clone(),
        flac,
        aac,
        opus,
    };
    fs::write(
        options
            .output_dir
            .join(format!("{}.json", options.output_name)),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(&recipe_path, serde_json::to_vec_pretty(&expected_recipe)?)?;
    if options.stage == ConcatStage::FinalizeOnly {
        remove_if_present(&rf64_path)?;
    }
    eprintln!(
        "concatenation passed: {} inputs, {} full {:.3}s fades, {} shortened fades, {:.2} hours",
        report.input_files,
        report.full_crossfades,
        report.requested_crossfade_seconds,
        report.shortened_crossfades,
        report.output_duration_seconds / 3600.0
    );
    Ok(())
}

fn fingerprint_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("fingerprint {}", path.display()))?;
    Ok(fingerprint_hex(fingerprint_bytes(&bytes)))
}

fn validate_output_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    if name.is_empty()
        || name == "."
        || name == ".."
        || path.components().count() != 1
        || path.file_name().is_none()
    {
        bail!("--output-name must be one non-empty file-name component");
    }
    Ok(())
}

fn validate_metadata(metadata: &AudioMetadata) -> Result<()> {
    for (field, value) in [
        ("title", metadata.title.as_str()),
        ("album", metadata.album.as_str()),
        ("artist", metadata.artist.as_str()),
        ("album_artist", metadata.album_artist.as_str()),
        ("composer", metadata.composer.as_str()),
        ("genre", metadata.genre.as_str()),
        ("date", metadata.date.as_str()),
        ("track", metadata.track.as_str()),
        ("disc", metadata.disc.as_str()),
        ("comment", metadata.comment.as_str()),
        ("description", metadata.description.as_str()),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            bail!("audio metadata field {field} must be non-empty and single-line");
        }
    }
    Ok(())
}

fn read_metrics(path: &Path) -> Result<Vec<MetricsRow>> {
    let mut reader =
        csv::Reader::from_path(path).with_context(|| format!("open metrics {}", path.display()))?;
    reader
        .deserialize()
        .map(|row| row.context("parse metrics row"))
        .collect()
}

fn validate_bipartite_rows(
    rows: &[MetricsRow],
    short_ids: &HashSet<&str>,
    long_ids: &HashSet<&str>,
) -> Result<()> {
    let expected_pairs = short_ids.len() * long_ids.len();
    if rows.len() != expected_pairs {
        bail!(
            "metrics contain {} files, expected {expected_pairs}",
            rows.len()
        );
    }
    for row in rows {
        if !short_ids.contains(row.left.as_str()) || !long_ids.contains(row.right.as_str()) {
            bail!(
                "metrics pair {} is not a short-to-long convolution: {} -> {}",
                row.pair,
                row.left,
                row.right
            );
        }
    }
    let pairs = rows
        .iter()
        .map(|row| (row.left.as_str(), row.right.as_str()))
        .collect::<HashSet<_>>();
    if pairs.len() != expected_pairs {
        bail!("metrics do not contain every short-to-long pair exactly once");
    }
    let sequence_indices = rows
        .iter()
        .map(|row| row.sequence_index)
        .collect::<HashSet<_>>();
    if sequence_indices.len() != expected_pairs
        || !(0..expected_pairs).all(|index| sequence_indices.contains(&index))
    {
        bail!("metrics sequence indices must contain every value from 0 exactly once");
    }
    Ok(())
}

fn sequence_layout(
    rows: &[MetricsRow],
    requested_frames: usize,
) -> Result<(Vec<usize>, Vec<u64>, u64)> {
    let first = rows.first().context("cannot concatenate an empty matrix")?;
    if first.frames == 0 {
        bail!("{} has no audio frames", first.path);
    }
    let mut output_frames = first.frames as u64;
    let mut transitions = Vec::with_capacity(rows.len().saturating_sub(1));
    let mut starts = Vec::with_capacity(rows.len());
    starts.push(0);

    for row in rows.iter().skip(1) {
        if row.frames == 0 {
            bail!("{} has no audio frames", row.path);
        }
        let transition = requested_frames
            .min(row.frames)
            .min(usize::try_from(output_frames).unwrap_or(usize::MAX));
        starts.push(output_frames - transition as u64);
        output_frames += row.frames as u64 - transition as u64;
        transitions.push(transition);
    }
    Ok((transitions, starts, output_frames))
}

fn write_timeline(
    path: &Path,
    rows: &[MetricsRow],
    starts: &[u64],
    transitions: &[usize],
) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    for (index, row) in rows.iter().enumerate() {
        let incoming = index
            .checked_sub(1)
            .and_then(|transition| transitions.get(transition))
            .copied()
            .unwrap_or(0);
        writer.serialize(TimelineRow {
            index: index + 1,
            pair: &row.pair,
            path: &row.path,
            input_frames: row.frames,
            start_frame: starts[index],
            start_seconds: starts[index] as f64 / f64::from(SAMPLE_RATE),
            incoming_crossfade_frames: incoming,
            incoming_crossfade_seconds: incoming as f64 / f64::from(SAMPLE_RATE),
        })?;
    }
    writer.flush()?;
    Ok(())
}

fn assemble_sequence(
    input_root: &Path,
    rows: &[MetricsRow],
    transitions: &[usize],
    requested_frames: usize,
    expected_output_frames: u64,
    rf64_path: &Path,
) -> Result<()> {
    let temporary_rf64 = rf64_path.with_extension("assembling.wav");
    if temporary_rf64.exists() {
        fs::remove_file(&temporary_rf64)?;
    }
    let mut master = Rf64Writer::create(&temporary_rf64, expected_output_frames)?;
    let mut tail = StereoAudio {
        left: Vec::new(),
        right: Vec::new(),
    };

    for (index, row) in rows.iter().enumerate() {
        let path = input_root.join(&row.path);
        let samples = read_pcm16_stereo(&path, row.frames)?;
        let mut combined = if index == 0 {
            samples
        } else {
            let transition = transitions[index - 1];
            if transition > tail.left.len() || transition > samples.left.len() {
                bail!("invalid transition length before {}", row.path);
            }
            let prefix_frames = tail.left.len() - transition;
            let mut combined = StereoAudio {
                left: Vec::with_capacity(prefix_frames + samples.left.len()),
                right: Vec::with_capacity(prefix_frames + samples.right.len()),
            };
            combined.left.extend_from_slice(&tail.left[..prefix_frames]);
            append_linear_crossfade(
                &mut combined.left,
                &tail.left[prefix_frames..],
                &samples.left[..transition],
            );
            combined.left.extend_from_slice(&samples.left[transition..]);
            combined
                .right
                .extend_from_slice(&tail.right[..prefix_frames]);
            append_linear_crossfade(
                &mut combined.right,
                &tail.right[prefix_frames..],
                &samples.right[..transition],
            );
            combined
                .right
                .extend_from_slice(&samples.right[transition..]);
            combined
        };

        let flush_frames = combined.left.len().saturating_sub(requested_frames);
        master.write_channels(
            &combined.left[..flush_frames],
            &combined.right[..flush_frames],
        )?;
        tail.left.clear();
        tail.right.clear();
        tail.left.extend_from_slice(&combined.left[flush_frames..]);
        tail.right
            .extend_from_slice(&combined.right[flush_frames..]);
        combined.left.clear();
        combined.right.clear();

        let completed = index + 1;
        if completed.is_multiple_of(50) || completed == rows.len() {
            eprintln!("concatenated {completed}/{} WAVs", rows.len());
        }
    }
    master.write_channels(&tail.left, &tail.right)?;
    master.finalize()?;
    fs::rename(&temporary_rf64, rf64_path)?;
    Ok(())
}

fn encode_outputs(
    rf64_path: &Path,
    targets: &EncodingTargets<'_>,
    aac_bitrate_kbps: u32,
    opus_bitrate_kbps: u32,
    metadata: &AudioMetadata,
    cover_art: &Path,
    force: bool,
) -> Result<()> {
    let temporary_flac = targets.flac.with_extension("part.flac");
    let temporary_aac = targets.aac.with_extension("part.m4a");
    let temporary_opus = targets.opus.with_extension("part.opus");
    let rebuild_flac = force || !targets.flac.is_file();
    let rebuild_aac = force || !targets.aac.is_file();
    let rebuild_opus = force || !targets.opus.is_file();
    let mut jobs = Vec::new();

    if rebuild_flac {
        remove_if_present(&temporary_flac)?;
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(rf64_path)
            .args(["-i"])
            .arg(cover_art)
            .args([
                "-map",
                "0:a:0",
                "-map",
                "1:v:0",
                "-c:a",
                "flac",
                "-compression_level",
                "3",
                "-c:v",
                "copy",
                "-disposition:v:0",
                "attached_pic",
                "-metadata:s:v:0",
                "title=Album cover",
                "-metadata:s:v:0",
                "comment=Cover (front)",
            ]);
        append_metadata(&mut command, metadata, false);
        let child = command
            .arg(&temporary_flac)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start parallel FLAC encoder")?;
        jobs.push(("FLAC", child, temporary_flac, targets.flac.to_owned(), None));
    }
    if rebuild_aac {
        remove_if_present(&temporary_aac)?;
        let aac_encoder = preferred_aac_encoder()?;
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(rf64_path)
            .args(["-i"])
            .arg(cover_art)
            .args(["-map", "0:a:0", "-map", "1:v:0", "-c:a"])
            .arg(&aac_encoder)
            .arg("-b:a")
            .arg(format!("{aac_bitrate_kbps}k"))
            .args([
                "-c:v",
                "copy",
                "-disposition:v:0",
                "attached_pic",
                "-metadata:s:v:0",
                "title=Album cover",
                "-metadata:s:v:0",
                "comment=Cover (front)",
            ]);
        append_metadata(&mut command, metadata, true);
        let child = command
            .arg(&temporary_aac)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("start parallel AAC encoder {aac_encoder}"))?;
        jobs.push(("AAC", child, temporary_aac, targets.aac.to_owned(), None));
    }
    let opus_encoder = if rebuild_opus {
        Some(preferred_opus_encoder()?)
    } else {
        None
    };
    if rebuild_opus {
        remove_if_present(&temporary_opus)?;
        let metadata_sidecar = temporary_opus.with_extension("picture.ffmetadata");
        remove_if_present(&metadata_sidecar)?;
        fs::write(
            &metadata_sidecar,
            format!(
                ";FFMETADATA1\nMETADATA_BLOCK_PICTURE={}\n",
                ffmetadata_escape(&cover_picture_metadata(cover_art)?)
            ),
        )?;
        let child = spawn_opus(
            rf64_path,
            &temporary_opus,
            opus_encoder.as_deref().unwrap_or("libopus"),
            opus_bitrate_kbps,
            metadata,
            &metadata_sidecar,
        )
        .inspect_err(|_| {
            let _ = remove_if_present(&metadata_sidecar);
        })?;
        jobs.push((
            "Opus",
            child,
            temporary_opus,
            targets.opus.to_owned(),
            Some(metadata_sidecar),
        ));
    }
    let names = jobs
        .iter()
        .map(|(name, _, _, _, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("encoding in parallel: {names}");
    for (name, mut child, temporary, final_path, metadata_sidecar) in jobs {
        let status = child
            .wait()
            .with_context(|| format!("wait for {name} encoder"))?;
        if let Some(path) = metadata_sidecar {
            remove_if_present(&path)?;
        }
        if !status.success() {
            bail!("{name} encoding failed with {status}");
        }
        fs::rename(temporary, final_path)?;
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn validate_decodable_in_parallel(encodings: &[(&str, &Path)]) -> Result<()> {
    let mut jobs = encodings
        .iter()
        .map(|&(name, path)| {
            let child = Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-i"])
                .arg(path)
                .args(["-map", "0:a:0", "-f", "null", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .with_context(|| format!("start {name} end-to-end decode validation"))?;
            Ok((name, child))
        })
        .collect::<Result<Vec<_>>>()?;
    eprintln!("validating compressed masters by decoding in parallel");
    for (name, mut child) in jobs.drain(..) {
        let status = child
            .wait()
            .with_context(|| format!("wait for {name} decode validation"))?;
        if !status.success() {
            bail!("{name} end-to-end decode validation failed with {status}");
        }
    }
    Ok(())
}

fn spawn_opus(
    rf64_path: &Path,
    output: &Path,
    encoder: &str,
    bitrate_kbps: u32,
    metadata: &AudioMetadata,
    picture_metadata: &Path,
) -> Result<std::process::Child> {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(rf64_path)
        .args(["-f", "ffmetadata", "-i"])
        .arg(picture_metadata)
        .args(["-map", "0:a:0", "-map_metadata", "1", "-c:a"])
        .arg(encoder)
        .arg("-b:a")
        .arg(format!("{bitrate_kbps}k"))
        .args([
            "-ac",
            "2",
            "-vbr",
            "on",
            "-application",
            "audio",
            "-compression_level",
            "10",
        ]);
    append_metadata(&mut command, metadata, false);
    command
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("start {bitrate_kbps}k Opus encoder {encoder}"))
}

fn ffmetadata_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('=', "\\=")
        .replace(';', "\\;")
        .replace('#', "\\#")
        .replace('\n', "\\\n")
}

fn cover_picture_metadata(path: &Path) -> Result<String> {
    let image = fs::read(path).with_context(|| format!("read cover art {}", path.display()))?;
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=codec_name,width,height"])
        .args(["-of", "json"])
        .arg(path)
        .output()
        .with_context(|| format!("probe cover art {}", path.display()))?;
    if !probe.status.success() {
        bail!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&probe.stderr)
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
    let stream = value["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .context("cover art has no image stream")?;
    if stream["codec_name"].as_str() != Some("mjpeg") {
        bail!("{}: cover art must be JPEG", path.display());
    }
    let width = u32::try_from(stream["width"].as_u64().context("cover width is missing")?)?;
    let height = u32::try_from(
        stream["height"]
            .as_u64()
            .context("cover height is missing")?,
    )?;
    let mime = b"image/jpeg";
    let description = b"Album cover";
    let mut picture = Vec::with_capacity(64 + image.len());
    append_picture_field(&mut picture, 3);
    append_picture_field(&mut picture, u32::try_from(mime.len())?);
    picture.extend_from_slice(mime);
    append_picture_field(&mut picture, u32::try_from(description.len())?);
    picture.extend_from_slice(description);
    append_picture_field(&mut picture, width);
    append_picture_field(&mut picture, height);
    append_picture_field(&mut picture, 24);
    append_picture_field(&mut picture, 0);
    append_picture_field(&mut picture, u32::try_from(image.len())?);
    picture.extend_from_slice(&image);
    Ok(base64_encode(&picture))
}

fn append_picture_field(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(ALPHABET[(first >> 2) as usize] as char);
        output.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(third & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn append_metadata(command: &mut Command, metadata: &AudioMetadata, include_description: bool) {
    let fields = [
        ("title", metadata.title.as_str()),
        ("album", metadata.album.as_str()),
        ("artist", metadata.artist.as_str()),
        ("album_artist", metadata.album_artist.as_str()),
        ("composer", metadata.composer.as_str()),
        ("genre", metadata.genre.as_str()),
        ("date", metadata.date.as_str()),
        ("track", metadata.track.as_str()),
        ("disc", metadata.disc.as_str()),
        ("comment", metadata.comment.as_str()),
    ];
    for (key, value) in fields {
        command.arg("-metadata").arg(format!("{key}={value}"));
    }
    if include_description {
        command
            .arg("-metadata")
            .arg(format!("description={}", metadata.description));
    }
}

fn preferred_opus_encoder() -> Result<String> {
    Ok(if encoder_is_available("libopus")? {
        "libopus".to_owned()
    } else {
        "opus".to_owned()
    })
}

fn preferred_aac_encoder() -> Result<String> {
    Ok(if encoder_is_available("libfdk_aac")? {
        "libfdk_aac".to_owned()
    } else {
        "aac".to_owned()
    })
}

fn encoder_is_available(name: &str) -> Result<bool> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .context("inspect ffmpeg encoders")?;
    if !output.status.success() {
        bail!("ffmpeg encoder listing failed with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .any(|encoder| encoder == name))
}

fn read_pcm16_stereo(path: &Path, expected_frames: usize) -> Result<StereoAudio> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("open input WAV {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != CHANNELS
        || spec.sample_rate != SAMPLE_RATE
        || spec.bits_per_sample != 16
        || spec.sample_format != SampleFormat::Int
    {
        bail!("{} is not stereo 48 kHz PCM16", path.display());
    }
    let mut samples = reader.samples::<i16>();
    let mut audio = StereoAudio {
        left: Vec::with_capacity(expected_frames),
        right: Vec::with_capacity(expected_frames),
    };
    while let Some(left) = samples.next() {
        let right = samples
            .next()
            .context("stereo WAV ends with an incomplete frame")?;
        audio.left.push(left? as f32 / i16::MAX as f32);
        audio.right.push(right? as f32 / i16::MAX as f32);
    }
    if audio.left.len() != expected_frames {
        bail!(
            "{} has {} frames, metrics expect {expected_frames}",
            path.display(),
            audio.left.len()
        );
    }
    Ok(audio)
}

fn append_linear_crossfade(output: &mut Vec<f32>, left: &[f32], right: &[f32]) {
    debug_assert_eq!(left.len(), right.len());
    if left.len() == 1 {
        output.push((left[0] + right[0]) * 0.5);
        return;
    }
    let denominator = left.len().saturating_sub(1).max(1) as f32;
    output.extend(
        left.iter()
            .zip(right)
            .enumerate()
            .map(|(index, (&left, &right))| {
                let mix = index as f32 / denominator;
                left.mul_add(1.0 - mix, right * mix)
            }),
    );
}

struct Rf64Writer {
    writer: BufWriter<File>,
    expected_frames: u64,
    written_frames: u64,
}

impl Rf64Writer {
    fn create(path: &Path, expected_frames: u64) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("create RF64 {}", path.display()))?;
        let mut writer = BufWriter::new(file);
        write_rf64_header(&mut writer, expected_frames)?;
        Ok(Self {
            writer,
            expected_frames,
            written_frames: 0,
        })
    }

    fn write_channels(&mut self, left: &[f32], right: &[f32]) -> Result<()> {
        if left.len() != right.len() {
            bail!("cannot write RF64 channels with unequal lengths");
        }
        write_pcm16le_stereo(&mut self.writer, left, right)?;
        self.written_frames += left.len() as u64;
        Ok(())
    }

    fn finalize(mut self) -> Result<()> {
        if self.written_frames != self.expected_frames {
            bail!(
                "wrote {} master frames, expected {}",
                self.written_frames,
                self.expected_frames
            );
        }
        self.writer.flush().context("flush RF64 master")?;
        Ok(())
    }
}

fn write_rf64_header(writer: &mut impl Write, frames: u64) -> Result<()> {
    let bytes_per_frame = u64::from(CHANNELS) * 2;
    let data_bytes = frames
        .checked_mul(bytes_per_frame)
        .context("RF64 data size overflow")?;
    let riff_size = data_bytes
        .checked_add(RF64_HEADER_BYTES - 8)
        .context("RF64 RIFF size overflow")?;

    writer.write_all(b"RF64")?;
    writer.write_all(&u32::MAX.to_le_bytes())?;
    writer.write_all(b"WAVE")?;
    writer.write_all(b"ds64")?;
    writer.write_all(&28_u32.to_le_bytes())?;
    writer.write_all(&riff_size.to_le_bytes())?;
    writer.write_all(&data_bytes.to_le_bytes())?;
    writer.write_all(&frames.to_le_bytes())?;
    writer.write_all(&0_u32.to_le_bytes())?;
    writer.write_all(b"fmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&CHANNELS.to_le_bytes())?;
    writer.write_all(&SAMPLE_RATE.to_le_bytes())?;
    writer.write_all(&(SAMPLE_RATE * u32::from(CHANNELS) * 2).to_le_bytes())?;
    writer.write_all(&(CHANNELS * 2).to_le_bytes())?;
    writer.write_all(&16_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&u32::MAX.to_le_bytes())?;
    Ok(())
}

fn write_pcm16le_stereo(writer: &mut impl Write, left: &[f32], right: &[f32]) -> Result<()> {
    const CHUNK_FRAMES: usize = 16_384;
    debug_assert_eq!(left.len(), right.len());
    let mut bytes = Vec::with_capacity(CHUNK_FRAMES * usize::from(CHANNELS) * size_of::<i16>());
    for (left_chunk, right_chunk) in left.chunks(CHUNK_FRAMES).zip(right.chunks(CHUNK_FRAMES)) {
        bytes.clear();
        for (&left, &right) in left_chunk.iter().zip(right_chunk) {
            let left = (left.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            let right = (right.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            bytes.extend_from_slice(&left.to_le_bytes());
            bytes.extend_from_slice(&right.to_le_bytes());
        }
        writer.write_all(&bytes).context("write RF64 PCM")?;
    }
    Ok(())
}

fn probe_encoding(
    path: &Path,
    expected_codec: &str,
    expected_seconds: f64,
) -> Result<EncodedFileReport> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0"])
        .args([
            "-show_entries",
            "stream=codec_name,sample_rate,channels,duration:format=duration",
        ])
        .args(["-of", "json"])
        .arg(path)
        .output()
        .with_context(|| format!("probe {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let stream = value["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .context("ffprobe found no audio stream")?;
    let codec = stream["codec_name"]
        .as_str()
        .context("ffprobe omitted codec name")?;
    let sample_rate = stream["sample_rate"]
        .as_str()
        .context("ffprobe omitted sample rate")?
        .parse::<u32>()?;
    let channels = stream["channels"]
        .as_u64()
        .context("ffprobe omitted channel count")?;
    let duration = stream["duration"]
        .as_str()
        .or_else(|| value["format"]["duration"].as_str())
        .context("ffprobe omitted duration")?
        .parse::<f64>()?;
    if codec != expected_codec || sample_rate != SAMPLE_RATE || channels != u64::from(CHANNELS) {
        bail!(
            "{} has codec={codec}, rate={sample_rate}, channels={channels}",
            path.display()
        );
    }
    if (duration - expected_seconds).abs() > 0.1 {
        bail!(
            "{} duration {duration:.6}s differs from expected {expected_seconds:.6}s",
            path.display()
        );
    }
    Ok(EncodedFileReport {
        path: path.to_string_lossy().into_owned(),
        codec: codec.to_owned(),
        bytes: fs::metadata(path)?.len(),
        duration_seconds: duration,
    })
}

fn validate_attached_cover(path: &Path, expected_cover: &Path) -> Result<()> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args([
            "-show_entries",
            "stream=codec_name:stream_disposition=attached_pic",
        ])
        .args(["-of", "json"])
        .arg(path)
        .output()
        .with_context(|| format!("probe cover art in {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let stream = value["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .with_context(|| format!("{} has no embedded cover stream", path.display()))?;
    let codec = stream["codec_name"]
        .as_str()
        .context("embedded cover has no codec name")?;
    let attached = stream["disposition"]["attached_pic"].as_u64() == Some(1);
    if codec != "mjpeg" || !attached {
        bail!(
            "{} has invalid cover stream: codec={codec}, attached_pic={attached}",
            path.display()
        );
    }
    let embedded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-c:v", "copy", "-f", "image2pipe", "-"])
        .output()
        .with_context(|| format!("extract cover art from {}", path.display()))?;
    if !embedded.status.success() {
        bail!(
            "cover extraction failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&embedded.stderr)
        );
    }
    let expected = fs::read(expected_cover)
        .with_context(|| format!("read expected cover {}", expected_cover.display()))?;
    if embedded.stdout != expected {
        bail!(
            "{} embedded cover does not match {}",
            path.display(),
            expected_cover.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        sequence_index: usize,
        pair: &str,
        left: &str,
        right: &str,
        frames: usize,
    ) -> MetricsRow {
        MetricsRow {
            pair: pair.into(),
            sequence_index,
            left: left.into(),
            right: right.into(),
            path: format!("wav/{pair}.wav"),
            frames,
        }
    }

    #[test]
    fn layout_uses_full_fades_after_the_master_is_long_enough() {
        let rows = vec![
            row(0, "01-02", "short", "long_1", 2),
            row(1, "01-04", "short", "long_2", 3),
            row(2, "01-06", "short", "long_3", 20),
        ];
        let (transitions, starts, frames) = sequence_layout(&rows, 5).unwrap();

        assert_eq!(transitions, vec![2, 3]);
        assert_eq!(starts, vec![0, 0, 0]);
        assert_eq!(frames, 20);
    }

    #[test]
    fn metrics_must_be_a_complete_short_to_long_matrix() {
        let short_ids = HashSet::from(["short_1", "short_2"]);
        let long_ids = HashSet::from(["long_1", "long_2"]);
        let valid = vec![
            row(0, "01-02", "short_1", "long_1", 10),
            row(1, "01-04", "short_1", "long_2", 10),
            row(2, "03-02", "short_2", "long_1", 10),
            row(3, "03-04", "short_2", "long_2", 10),
        ];
        assert!(validate_bipartite_rows(&valid, &short_ids, &long_ids).is_ok());

        let mut wrong_role = valid.clone();
        wrong_role[3].left = "long_1".into();
        assert!(validate_bipartite_rows(&wrong_role, &short_ids, &long_ids).is_err());

        let mut duplicate = valid;
        duplicate[3].left = "short_1".into();
        duplicate[3].right = "long_1".into();
        assert!(validate_bipartite_rows(&duplicate, &short_ids, &long_ids).is_err());
    }

    #[test]
    fn linear_crossfade_preserves_endpoints_and_length() {
        let mut output = Vec::new();
        append_linear_crossfade(&mut output, &[1.0, 1.0, 1.0], &[0.0, 0.0, 0.0]);

        assert_eq!(output, vec![1.0, 0.5, 0.0]);
    }

    #[test]
    fn rf64_header_contains_64_bit_sizes() {
        let mut header = Vec::new();
        write_rf64_header(&mut header, 100).unwrap();

        assert_eq!(header.len(), RF64_HEADER_BYTES as usize);
        assert_eq!(&header[0..4], b"RF64");
        assert_eq!(&header[12..16], b"ds64");
        assert_eq!(u64::from_le_bytes(header[28..36].try_into().unwrap()), 400);
        assert_eq!(u64::from_le_bytes(header[36..44].try_into().unwrap()), 100);
        assert_eq!(u16::from_le_bytes(header[58..60].try_into().unwrap()), 2);
        assert_eq!(&header[72..76], b"data");
    }

    #[test]
    fn complete_single_line_metadata_is_required() {
        let valid = AudioMetadata {
            title: "Drift".into(),
            album: "Convolutions 10".into(),
            artist: "babymastodon".into(),
            album_artist: "babymastodon".into(),
            composer: "babymastodon".into(),
            genre: "Experimental".into(),
            date: "2026".into(),
            track: "3/14".into(),
            disc: "1/1".into(),
            comment: "Calm environmental flow.".into(),
            description:
                "Themes: calm environmental flow. Tuning: 14-tone ratio scale. Form: A-B-A.".into(),
        };
        assert!(validate_metadata(&valid).is_ok());

        let mut empty = valid.clone();
        empty.comment.clear();
        assert!(validate_metadata(&empty).is_err());

        let mut multiline = valid;
        multiline.title = "Drift\nAlternate".into();
        assert!(validate_metadata(&multiline).is_err());
    }

    #[test]
    fn picture_metadata_helpers_use_standard_encodings() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(ffmetadata_escape("a=b;c#d\\e"), "a\\=b\\;c\\#d\\\\e");
    }
}
