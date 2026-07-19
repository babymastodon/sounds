use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader};
use serde::{Deserialize, Serialize};

use crate::audio::SAMPLE_RATE;
use crate::manifest::load_manifest;

const FLAC_NAME: &str = "final_mix.flac";
const AAC_NAME: &str = "final_mix.m4a";
const OPUS_NAME: &str = "final_mix.opus";
const OPUS_16K_NAME: &str = "final_mix_16k.opus";
const RF64_NAME: &str = "final_mix.rf64.wav";
const RF64_HEADER_BYTES: u64 = 80;

#[derive(Clone, Debug)]
pub struct ConcatOptions {
    pub manifest: PathBuf,
    pub metrics: PathBuf,
    pub output_dir: PathBuf,
    pub crossfade_seconds: f64,
    pub aac_bitrate_kbps: u32,
    pub opus_bitrate_kbps: u32,
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
    opus_bitrate_kbps: u32,
    rf64: EncodedFileReport,
    flac: EncodedFileReport,
    aac: EncodedFileReport,
    opus: EncodedFileReport,
    opus_16k: EncodedFileReport,
}

struct EncodingTargets<'a> {
    flac: &'a Path,
    aac: &'a Path,
    opus_64k: &'a Path,
    opus_16k: &'a Path,
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

    let rf64_path = options.output_dir.join(RF64_NAME);
    let flac_path = options.output_dir.join(FLAC_NAME);
    let aac_path = options.output_dir.join(AAC_NAME);
    let opus_path = options.output_dir.join(OPUS_NAME);
    let opus_16k_path = options.output_dir.join(OPUS_16K_NAME);

    if !rf64_path.is_file() || options.force {
        assemble_sequence(
            input_root,
            &rows,
            &transitions,
            requested_frames,
            output_frames,
            &rf64_path,
        )?;
    } else {
        eprintln!("reusing existing RF64 PCM master");
    }
    let rf64 = probe_encoding(&rf64_path, "pcm_s16le", output_seconds)?;

    let all_encodings_exist =
        flac_path.is_file() && aac_path.is_file() && opus_path.is_file() && opus_16k_path.is_file();
    if !all_encodings_exist || options.force {
        let targets = EncodingTargets {
            flac: &flac_path,
            aac: &aac_path,
            opus_64k: &opus_path,
            opus_16k: &opus_16k_path,
        };
        encode_outputs(
            &rf64_path,
            &targets,
            options.aac_bitrate_kbps,
            options.opus_bitrate_kbps,
            options.force,
        )?;
    } else {
        eprintln!("reusing existing FLAC, AAC, and Opus encodings");
    }

    let flac = probe_encoding(&flac_path, "flac", output_seconds)?;
    let aac = probe_encoding(&aac_path, "aac", output_seconds)?;
    let opus = probe_encoding(&opus_path, "opus", output_seconds)?;
    let opus_16k = probe_encoding(&opus_16k_path, "opus", output_seconds)?;
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
        opus_bitrate_kbps: options.opus_bitrate_kbps,
        rf64,
        flac,
        aac,
        opus,
        opus_16k,
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

fn assemble_sequence(
    input_root: &Path,
    rows: &[MetricsRow],
    transitions: &[usize],
    requested_frames: usize,
    expected_output_frames: u64,
    rf64_path: &Path,
) -> Result<()> {
    let temporary_rf64 = rf64_path.with_file_name("final_mix.part.rf64.wav");
    if temporary_rf64.exists() {
        fs::remove_file(&temporary_rf64)?;
    }
    let mut master = Rf64Writer::create(&temporary_rf64, expected_output_frames)?;
    let mut tail = Vec::<f32>::new();

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
        master.write_samples(&combined[..flush_frames])?;
        tail.clear();
        tail.extend_from_slice(&combined[flush_frames..]);
        combined.clear();

        let completed = index + 1;
        if completed.is_multiple_of(50) || completed == rows.len() {
            eprintln!("concatenated {completed}/{} WAVs", rows.len());
        }
    }
    master.write_samples(&tail)?;
    master.finalize()?;
    fs::rename(&temporary_rf64, rf64_path)?;
    Ok(())
}

fn encode_outputs(
    rf64_path: &Path,
    targets: &EncodingTargets<'_>,
    aac_bitrate_kbps: u32,
    opus_bitrate_kbps: u32,
    force: bool,
) -> Result<()> {
    let temporary_flac = targets.flac.with_file_name("final_mix.part.flac");
    let temporary_aac = targets.aac.with_file_name("final_mix.part.m4a");
    let temporary_opus = targets.opus_64k.with_file_name("final_mix.part.opus");
    let temporary_opus_16k = targets.opus_16k.with_file_name("final_mix_16k.part.opus");
    let rebuild_flac = force || !targets.flac.is_file();
    let rebuild_aac = force || !targets.aac.is_file();
    let rebuild_opus = force || !targets.opus_64k.is_file();
    let rebuild_opus_16k = force || !targets.opus_16k.is_file();
    let mut jobs = Vec::new();

    if rebuild_flac {
        remove_if_present(&temporary_flac)?;
        let child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(rf64_path)
            .args(["-map", "0:a:0", "-c:a", "flac", "-compression_level", "3"])
            .arg(&temporary_flac)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start parallel FLAC encoder")?;
        jobs.push(("FLAC", child, temporary_flac, targets.flac.to_owned()));
    }
    if rebuild_aac {
        remove_if_present(&temporary_aac)?;
        let aac_encoder = preferred_aac_encoder()?;
        let child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(rf64_path)
            .args(["-map", "0:a:0", "-c:a"])
            .arg(&aac_encoder)
            .arg("-b:a")
            .arg(format!("{aac_bitrate_kbps}k"))
            .arg(&temporary_aac)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("start parallel AAC encoder {aac_encoder}"))?;
        jobs.push(("AAC", child, temporary_aac, targets.aac.to_owned()));
    }
    let opus_encoder = if rebuild_opus || rebuild_opus_16k {
        Some(preferred_opus_encoder()?)
    } else {
        None
    };
    if rebuild_opus {
        remove_if_present(&temporary_opus)?;
        let child = spawn_opus(
            rf64_path,
            &temporary_opus,
            opus_encoder.as_deref().unwrap_or("libopus"),
            opus_bitrate_kbps,
        )?;
        jobs.push(("Opus", child, temporary_opus, targets.opus_64k.to_owned()));
    }
    if rebuild_opus_16k {
        remove_if_present(&temporary_opus_16k)?;
        let child = spawn_opus(
            rf64_path,
            &temporary_opus_16k,
            opus_encoder.as_deref().unwrap_or("libopus"),
            16,
        )?;
        jobs.push((
            "Opus 16k",
            child,
            temporary_opus_16k,
            targets.opus_16k.to_owned(),
        ));
    }
    let names = jobs
        .iter()
        .map(|(name, _, _, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("encoding in parallel: {names}");
    for (name, mut child, temporary, final_path) in jobs {
        let status = child
            .wait()
            .with_context(|| format!("wait for {name} encoder"))?;
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

fn spawn_opus(
    rf64_path: &Path,
    output: &Path,
    encoder: &str,
    bitrate_kbps: u32,
) -> Result<std::process::Child> {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(rf64_path)
        .args(["-map", "0:a:0", "-c:a"])
        .arg(encoder)
        .arg("-b:a")
        .arg(format!("{bitrate_kbps}k"))
        .args([
            "-ac",
            "1",
            "-vbr",
            "on",
            "-application",
            "audio",
            "-compression_level",
            "10",
        ])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("start {bitrate_kbps}k Opus encoder {encoder}"))
}

fn preferred_aac_encoder() -> Result<String> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-h", "encoder=libfdk_aac"])
        .output()
        .context("inspect ffmpeg AAC encoders")?;
    Ok(if output.status.success() {
        "libfdk_aac".to_owned()
    } else {
        "aac".to_owned()
    })
}

fn preferred_opus_encoder() -> Result<String> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-h", "encoder=libopus"])
        .output()
        .context("inspect ffmpeg Opus encoders")?;
    Ok(if output.status.success() {
        "libopus".to_owned()
    } else {
        "opus".to_owned()
    })
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

    fn write_samples(&mut self, samples: &[f32]) -> Result<()> {
        write_pcm16le(&mut self.writer, samples)?;
        self.written_frames += samples.len() as u64;
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
    let data_bytes = frames.checked_mul(2).context("RF64 data size overflow")?;
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
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&SAMPLE_RATE.to_le_bytes())?;
    writer.write_all(&(SAMPLE_RATE * 2).to_le_bytes())?;
    writer.write_all(&2_u16.to_le_bytes())?;
    writer.write_all(&16_u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&u32::MAX.to_le_bytes())?;
    Ok(())
}

fn write_pcm16le(writer: &mut impl Write, samples: &[f32]) -> Result<()> {
    const CHUNK_FRAMES: usize = 16_384;
    let mut bytes = Vec::with_capacity(CHUNK_FRAMES * size_of::<i16>());
    for chunk in samples.chunks(CHUNK_FRAMES) {
        bytes.clear();
        for &sample in chunk {
            let quantized = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            bytes.extend_from_slice(&quantized.to_le_bytes());
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

    #[test]
    fn rf64_header_contains_64_bit_sizes() {
        let mut header = Vec::new();
        write_rf64_header(&mut header, 100).unwrap();

        assert_eq!(header.len(), RF64_HEADER_BYTES as usize);
        assert_eq!(&header[0..4], b"RF64");
        assert_eq!(&header[12..16], b"ds64");
        assert_eq!(u64::from_le_bytes(header[28..36].try_into().unwrap()), 200);
        assert_eq!(u64::from_le_bytes(header[36..44].try_into().unwrap()), 100);
        assert_eq!(&header[72..76], b"data");
    }
}
