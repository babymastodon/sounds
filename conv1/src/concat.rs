use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader};
use serde::{Deserialize, Serialize};

use crate::audio::SAMPLE_RATE;
use crate::manifest::load_manifest;

const FLAC_NAME: &str = "final_mix.flac";
const AAC_NAME: &str = "final_mix.m4a";

#[derive(Clone, Debug)]
pub struct ConcatOptions {
    pub manifest: PathBuf,
    pub metrics: PathBuf,
    pub output_dir: PathBuf,
    pub crossfade_seconds: f64,
    pub aac_bitrate_kbps: u32,
    pub force: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct MetricsRow {
    pair: String,
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
    requested_crossfade_seconds: f64,
    full_crossfades: usize,
    shortened_crossfades: usize,
    output_frames: u64,
    output_duration_seconds: f64,
    aac_bitrate_kbps: u32,
    flac: EncodedFileReport,
    aac: EncodedFileReport,
}

pub fn concatenate_master(options: ConcatOptions) -> Result<()> {
    if !options.crossfade_seconds.is_finite() || options.crossfade_seconds <= 0.0 {
        bail!("--crossfade-seconds must be a positive finite number");
    }
    if options.aac_bitrate_kbps == 0 {
        bail!("--aac-bitrate-kbps must be at least 1");
    }

    let sources = load_manifest(&options.manifest)?;
    let expected_pairs = sources.len() * (sources.len() + 1) / 2;
    let mut rows = read_metrics(&options.metrics)?;
    rows.sort_by(|left, right| left.pair.cmp(&right.pair));
    if rows.len() != expected_pairs {
        bail!(
            "metrics contain {} files, expected {expected_pairs}",
            rows.len()
        );
    }
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
    write_timeline(
        &options.output_dir.join("timeline.csv"),
        &rows,
        &starts,
        &transitions,
    )?;

    let flac_path = options.output_dir.join(FLAC_NAME);
    let aac_path = options.output_dir.join(AAC_NAME);
    let both_exist = flac_path.is_file() && aac_path.is_file();
    if (flac_path.exists() || aac_path.exists()) && !both_exist && !options.force {
        bail!("only one final encoding exists; pass --force to rebuild both");
    }

    if !both_exist || options.force {
        encode_sequence(
            input_root,
            &rows,
            &transitions,
            output_frames,
            &flac_path,
            &aac_path,
            options.aac_bitrate_kbps,
        )?;
    } else {
        eprintln!("reusing existing FLAC and AAC encodings");
    }

    let flac = probe_encoding(&flac_path, "flac", output_seconds)?;
    let aac = probe_encoding(&aac_path, "aac", output_seconds)?;
    let report = ConcatReport {
        status: "pass",
        input_files: rows.len(),
        sample_rate: SAMPLE_RATE,
        requested_crossfade_seconds: options.crossfade_seconds,
        full_crossfades,
        shortened_crossfades,
        output_frames,
        output_duration_seconds: output_seconds,
        aac_bitrate_kbps: options.aac_bitrate_kbps,
        flac,
        aac,
    };
    fs::write(
        options.output_dir.join("concat.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    eprintln!(
        "concatenation passed: {} inputs, {} full 5s fades, {} shortened fades, {:.2} hours",
        report.input_files,
        report.full_crossfades,
        report.shortened_crossfades,
        report.output_duration_seconds / 3600.0
    );
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

#[allow(clippy::too_many_arguments)]
fn encode_sequence(
    input_root: &Path,
    rows: &[MetricsRow],
    transitions: &[usize],
    expected_output_frames: u64,
    flac_path: &Path,
    aac_path: &Path,
    aac_bitrate_kbps: u32,
) -> Result<()> {
    let temporary_flac = flac_path.with_file_name("final_mix.part.flac");
    let temporary_aac = aac_path.with_file_name("final_mix.part.m4a");
    for temporary in [&temporary_flac, &temporary_aac] {
        if temporary.exists() {
            fs::remove_file(temporary)?;
        }
    }

    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "f32le", "-ar", "48000", "-ac", "1"])
        .args(["-i", "pipe:0"])
        .args(["-map", "0:a:0", "-c:a", "flac", "-compression_level", "8"])
        .arg(&temporary_flac)
        .args(["-map", "0:a:0", "-c:a", "aac", "-b:a"])
        .arg(format!("{aac_bitrate_kbps}k"))
        .args(["-movflags", "+faststart"])
        .arg(&temporary_aac)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .context("start ffmpeg FLAC/AAC encoder")?;
    let mut encoder = child.stdin.take().context("open ffmpeg input pipe")?;
    let mut tail = Vec::<f32>::new();
    let requested_frames = transitions.iter().copied().max().unwrap_or(0);
    let mut written_frames = 0_u64;

    for (index, row) in rows.iter().enumerate() {
        let path = input_root.join(&row.path);
        let samples = read_pcm16(&path, row.frames)?;
        let mut combined = if index == 0 {
            samples
        } else {
            let transition = transitions[index - 1];
            if transition > tail.len() || transition > samples.len() {
                bail!("invalid transition length before {}", row.path);
            }
            let prefix_frames = tail.len() - transition;
            let mut combined = Vec::with_capacity(prefix_frames + samples.len());
            combined.extend_from_slice(&tail[..prefix_frames]);
            append_linear_crossfade(
                &mut combined,
                &tail[prefix_frames..],
                &samples[..transition],
            );
            combined.extend_from_slice(&samples[transition..]);
            combined
        };

        let flush_frames = combined.len().saturating_sub(requested_frames);
        write_f32le(&mut encoder, &combined[..flush_frames])?;
        written_frames += flush_frames as u64;
        tail.clear();
        tail.extend_from_slice(&combined[flush_frames..]);
        combined.clear();

        let completed = index + 1;
        if completed.is_multiple_of(50) || completed == rows.len() {
            eprintln!("concatenated {completed}/{} WAVs", rows.len());
        }
    }
    write_f32le(&mut encoder, &tail)?;
    written_frames += tail.len() as u64;
    if written_frames != expected_output_frames {
        bail!("wrote {written_frames} master frames, expected {expected_output_frames}");
    }
    drop(encoder);

    let status = child.wait().context("wait for ffmpeg encoders")?;
    if !status.success() {
        bail!("ffmpeg encoding failed with {status}");
    }
    fs::rename(&temporary_flac, flac_path)?;
    fs::rename(&temporary_aac, aac_path)?;
    Ok(())
}

fn read_pcm16(path: &Path, expected_frames: usize) -> Result<Vec<f32>> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("open input WAV {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != SAMPLE_RATE
        || spec.bits_per_sample != 16
        || spec.sample_format != SampleFormat::Int
    {
        bail!("{} is not mono 48 kHz PCM16", path.display());
    }
    let samples = reader
        .samples::<i16>()
        .map(|sample| sample.map(|value| value as f32 / i16::MAX as f32))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if samples.len() != expected_frames {
        bail!(
            "{} has {} frames, metrics expect {expected_frames}",
            path.display(),
            samples.len()
        );
    }
    Ok(samples)
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

fn write_f32le(writer: &mut impl Write, samples: &[f32]) -> Result<()> {
    const CHUNK_FRAMES: usize = 16_384;
    let mut bytes = Vec::with_capacity(CHUNK_FRAMES * size_of::<f32>());
    for chunk in samples.chunks(CHUNK_FRAMES) {
        bytes.clear();
        for &sample in chunk {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        writer.write_all(&bytes).context("stream PCM to ffmpeg")?;
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
    if codec != expected_codec || sample_rate != SAMPLE_RATE || channels != 1 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pair: &str, frames: usize) -> MetricsRow {
        MetricsRow {
            pair: pair.into(),
            path: format!("wav/{pair}.wav"),
            frames,
        }
    }

    #[test]
    fn layout_uses_full_fades_after_the_master_is_long_enough() {
        let rows = vec![row("01-01", 2), row("01-02", 3), row("01-03", 20)];
        let (transitions, starts, frames) = sequence_layout(&rows, 5).unwrap();

        assert_eq!(transitions, vec![2, 3]);
        assert_eq!(starts, vec![0, 0, 0]);
        assert_eq!(frames, 20);
    }

    #[test]
    fn linear_crossfade_preserves_endpoints_and_length() {
        let mut output = Vec::new();
        append_linear_crossfade(&mut output, &[1.0, 1.0, 1.0], &[0.0, 0.0, 0.0]);

        assert_eq!(output, vec![1.0, 0.5, 0.0]);
    }
}
