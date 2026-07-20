use std::f32::consts::PI;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde::Serialize;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
const INPUT_TARGET_RMS: f32 = 0.1;
const OUTPUT_TARGET_RMS: f32 = 0.1;
const OUTPUT_CEILING: f32 = 0.89;

#[derive(Clone, Debug)]
pub struct AudioClip {
    pub id: String,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct StereoAudio {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AudioMetrics {
    pub frames: usize,
    pub duration_seconds: f64,
    pub peak: f32,
    pub rms: f32,
    pub rms_dbfs: f32,
    pub dc_offset: f32,
    pub clipped_samples: usize,
    pub non_finite_samples: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct StereoMetrics {
    pub frames: usize,
    pub duration_seconds: f64,
    pub peak: f32,
    pub rms: f32,
    pub rms_dbfs: f32,
    pub dc_offset: f32,
    pub clipped_samples: usize,
    pub non_finite_samples: usize,
    pub left_peak: f32,
    pub left_rms_dbfs: f32,
    pub left_dc_offset: f32,
    pub right_peak: f32,
    pub right_rms_dbfs: f32,
    pub right_dc_offset: f32,
    pub stereo_difference_rms: f32,
    pub stereo_difference_rms_dbfs: f32,
}

pub fn read_prepared_clip(id: &str, path: &Path, expected_seconds: f64) -> Result<AudioClip> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("open prepared input {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != SAMPLE_RATE {
        bail!(
            "{} must be mono {} Hz, found {} channels at {} Hz",
            path.display(),
            SAMPLE_RATE,
            spec.channels,
            spec.sample_rate
        );
    }

    let mut samples = match spec.sample_format {
        SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?,
        SampleFormat::Int if spec.bits_per_sample <= 16 => reader
            .samples::<i16>()
            .map(|sample| sample.map(|value| value as f32 / i16::MAX as f32))
            .collect::<std::result::Result<Vec<_>, _>>()?,
        SampleFormat::Int => {
            let scale = ((1_i64 << (spec.bits_per_sample - 1)) - 1) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<std::result::Result<Vec<_>, _>>()?
        }
    };

    let expected_frames = (expected_seconds * SAMPLE_RATE as f64).round() as usize;
    let tolerance = 2;
    if samples.len().abs_diff(expected_frames) > tolerance {
        bail!(
            "{} has {} frames, expected {} ({} seconds)",
            path.display(),
            samples.len(),
            expected_frames,
            expected_seconds
        );
    }
    samples.resize(expected_frames, 0.0);
    condition_input(&mut samples)?;
    Ok(AudioClip {
        id: id.to_owned(),
        samples,
    })
}

pub fn condition_input(samples: &mut [f32]) -> Result<()> {
    ensure_finite_and_non_silent(samples, "input")?;
    remove_mean(samples);
    high_pass(samples, 18.0);
    remove_mean(samples);

    let metrics = measure(samples);
    let peak_gain = 0.8 / metrics.peak.max(1.0e-12);
    let rms_gain = INPUT_TARGET_RMS / metrics.rms.max(1.0e-12);
    let gain = rms_gain.min(peak_gain);
    for sample in samples.iter_mut() {
        *sample *= gain;
    }
    apply_edge_fades(samples, 0.015);
    ensure_finite_and_non_silent(samples, "conditioned input")
}

pub fn condition_stereo_output(audio: &mut StereoAudio) -> Result<StereoMetrics> {
    validate_channel_lengths(audio)?;
    ensure_finite_and_non_silent(&audio.left, "raw left convolution")?;
    ensure_finite_and_non_silent(&audio.right, "raw right convolution")?;
    for channel in [&mut audio.left, &mut audio.right] {
        remove_mean(channel);
        high_pass(channel, 18.0);
        remove_mean(channel);
    }

    let original_left = audio.left.clone();
    let original_right = audio.right.clone();
    condition_output_channel(&mut audio.left, &original_left);
    condition_output_channel(&mut audio.right, &original_right);

    remove_mean(&mut audio.left);
    remove_mean(&mut audio.right);
    let peak = measure_stereo(audio)?.peak;
    if peak > OUTPUT_CEILING {
        let ceiling_gain = OUTPUT_CEILING / peak;
        for channel in [&mut audio.left, &mut audio.right] {
            for sample in channel {
                *sample *= ceiling_gain;
            }
        }
    }
    apply_edge_fades(&mut audio.left, 0.02);
    apply_edge_fades(&mut audio.right, 0.02);

    let metrics = measure_stereo(audio)?;
    validate_stereo_metrics(&metrics, audio.left.len(), "conditioned stereo output")?;
    Ok(metrics)
}

fn condition_output_channel(output: &mut [f32], original: &[f32]) {
    let raw_rms = measure(original).rms.max(1.0e-12);
    let mut gain = OUTPUT_TARGET_RMS / raw_rms;
    for _ in 0..4 {
        for (output, input) in output.iter_mut().zip(original) {
            *output = OUTPUT_CEILING * (*input * gain / OUTPUT_CEILING).tanh();
        }
        let rms = measure(output).rms;
        if rms >= 0.055 {
            break;
        }
        gain *= (0.07 / rms.max(1.0e-12)).clamp(1.0, 4.0);
    }
}

pub fn write_pcm16_stereo(path: &Path, audio: &StereoAudio) -> Result<()> {
    validate_channel_lengths(audio)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("wav.part");
    let spec = WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&temporary, spec)
        .with_context(|| format!("create {}", temporary.display()))?;
    for (&left, &right) in audio.left.iter().zip(&audio.right) {
        writer.write_sample(quantize(left))?;
        writer.write_sample(quantize(right))?;
    }
    writer.finalize()?;
    fs::rename(&temporary, path)
        .with_context(|| format!("move completed WAV to {}", path.display()))?;
    Ok(())
}

pub fn measure_wav(path: &Path) -> Result<StereoMetrics> {
    let reader = WavReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != CHANNELS
        || spec.sample_rate != SAMPLE_RATE
        || spec.bits_per_sample != 16
        || spec.sample_format != SampleFormat::Int
    {
        bail!("{} is not stereo 48 kHz PCM16", path.display());
    }
    let interleaved = reader
        .into_samples::<i16>()
        .map(|sample| sample.map(|value| value as f32 / i16::MAX as f32))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if interleaved.len() % usize::from(CHANNELS) != 0 {
        bail!("{} contains an incomplete stereo frame", path.display());
    }
    let mut audio = StereoAudio {
        left: Vec::with_capacity(interleaved.len() / 2),
        right: Vec::with_capacity(interleaved.len() / 2),
    };
    for frame in interleaved.chunks_exact(2) {
        audio.left.push(frame[0]);
        audio.right.push(frame[1]);
    }
    measure_stereo(&audio)
}

pub fn measure(samples: &[f32]) -> AudioMetrics {
    let mut peak = 0.0_f32;
    let mut sum = 0.0_f64;
    let mut sum_squares = 0.0_f64;
    let mut clipped_samples = 0;
    let mut non_finite_samples = 0;

    for &sample in samples {
        if !sample.is_finite() {
            non_finite_samples += 1;
            continue;
        }
        peak = peak.max(sample.abs());
        sum += sample as f64;
        sum_squares += (sample as f64) * (sample as f64);
        clipped_samples += usize::from(sample.abs() >= 0.9999);
    }

    let frames = samples.len();
    let denominator = frames.max(1) as f64;
    let rms = (sum_squares / denominator).sqrt() as f32;
    AudioMetrics {
        frames,
        duration_seconds: frames as f64 / SAMPLE_RATE as f64,
        peak,
        rms,
        rms_dbfs: 20.0 * rms.max(1.0e-12).log10(),
        dc_offset: (sum / denominator) as f32,
        clipped_samples,
        non_finite_samples,
    }
}

pub fn measure_stereo(audio: &StereoAudio) -> Result<StereoMetrics> {
    validate_channel_lengths(audio)?;
    let left = measure(&audio.left);
    let right = measure(&audio.right);
    let rms = ((left.rms * left.rms + right.rms * right.rms) * 0.5).sqrt();
    let difference_rms = (audio
        .left
        .iter()
        .zip(&audio.right)
        .map(|(&left, &right)| {
            let difference = f64::from(left - right);
            difference * difference
        })
        .sum::<f64>()
        / audio.left.len() as f64)
        .sqrt() as f32;
    Ok(StereoMetrics {
        frames: audio.left.len(),
        duration_seconds: audio.left.len() as f64 / SAMPLE_RATE as f64,
        peak: left.peak.max(right.peak),
        rms,
        rms_dbfs: 20.0 * rms.max(1.0e-12).log10(),
        dc_offset: (left.dc_offset + right.dc_offset) * 0.5,
        clipped_samples: left.clipped_samples + right.clipped_samples,
        non_finite_samples: left.non_finite_samples + right.non_finite_samples,
        left_peak: left.peak,
        left_rms_dbfs: left.rms_dbfs,
        left_dc_offset: left.dc_offset,
        right_peak: right.peak,
        right_rms_dbfs: right.rms_dbfs,
        right_dc_offset: right.dc_offset,
        stereo_difference_rms: difference_rms,
        stereo_difference_rms_dbfs: 20.0 * difference_rms.max(1.0e-12).log10(),
    })
}

pub fn validate_stereo_metrics(
    metrics: &StereoMetrics,
    expected_frames: usize,
    label: &str,
) -> Result<()> {
    if metrics.frames != expected_frames {
        bail!(
            "{label}: {} frames, expected {expected_frames}",
            metrics.frames
        );
    }
    if metrics.non_finite_samples != 0 {
        bail!(
            "{label}: contains {} non-finite samples",
            metrics.non_finite_samples
        );
    }
    if metrics.clipped_samples != 0 || metrics.peak > 0.92 {
        bail!(
            "{label}: clipping/ceiling failure (peak {}, clipped {})",
            metrics.peak,
            metrics.clipped_samples
        );
    }
    if metrics.peak < 0.12 {
        bail!("{label}: peak {} is too quiet", metrics.peak);
    }
    if !(-30.0..=-10.0).contains(&metrics.rms_dbfs) {
        bail!(
            "{label}: RMS {} dBFS is outside -30..=-10",
            metrics.rms_dbfs
        );
    }
    if metrics.dc_offset.abs() > 0.005
        || metrics.left_dc_offset.abs() > 0.005
        || metrics.right_dc_offset.abs() > 0.005
    {
        bail!(
            "{label}: excessive DC offsets overall={} left={} right={}",
            metrics.dc_offset,
            metrics.left_dc_offset,
            metrics.right_dc_offset
        );
    }
    for (channel, peak, rms_dbfs) in [
        ("left", metrics.left_peak, metrics.left_rms_dbfs),
        ("right", metrics.right_peak, metrics.right_rms_dbfs),
    ] {
        if !(0.12..=0.92).contains(&peak) || !(-30.0..=-10.0).contains(&rms_dbfs) {
            bail!("{label}: {channel} channel peak={peak}, RMS={rms_dbfs} dBFS");
        }
    }
    Ok(())
}

fn validate_channel_lengths(audio: &StereoAudio) -> Result<()> {
    if audio.left.is_empty() {
        bail!("stereo audio is empty");
    }
    if audio.left.len() != audio.right.len() {
        bail!(
            "stereo channels have different lengths: {} and {}",
            audio.left.len(),
            audio.right.len()
        );
    }
    Ok(())
}

fn quantize(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn ensure_finite_and_non_silent(samples: &[f32], label: &str) -> Result<()> {
    if samples.is_empty() {
        bail!("{label} is empty");
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        bail!("{label} contains a non-finite sample");
    }
    if measure(samples).rms < 1.0e-7 {
        bail!("{label} is silent");
    }
    Ok(())
}

fn remove_mean(samples: &mut [f32]) {
    let mean = samples.iter().map(|&x| x as f64).sum::<f64>() / samples.len().max(1) as f64;
    for sample in samples {
        *sample -= mean as f32;
    }
}

fn high_pass(samples: &mut [f32], cutoff_hz: f32) {
    let dt = 1.0 / SAMPLE_RATE as f32;
    let rc = 1.0 / (2.0 * PI * cutoff_hz);
    let alpha = rc / (rc + dt);
    let mut previous_input = samples.first().copied().unwrap_or(0.0);
    let mut previous_output = 0.0;
    for sample in samples {
        let input = *sample;
        let output = alpha * (previous_output + input - previous_input);
        *sample = output;
        previous_input = input;
        previous_output = output;
    }
}

fn apply_edge_fades(samples: &mut [f32], seconds: f32) {
    let fade_frames = ((seconds * SAMPLE_RATE as f32) as usize)
        .min(samples.len() / 2)
        .max(1);
    for index in 0..fade_frames {
        let gain = index as f32 / fade_frames as f32;
        samples[index] *= gain;
        let tail = samples.len() - 1 - index;
        samples[tail] *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_output_conditioner_hits_quality_window() {
        let samples = (0..SAMPLE_RATE as usize)
            .map(|index| 0.0001 * (2.0 * PI * 440.0 * index as f32 / SAMPLE_RATE as f32).sin())
            .collect::<Vec<_>>();
        let mut audio = StereoAudio {
            left: samples.clone(),
            right: samples,
        };
        let metrics = condition_stereo_output(&mut audio).unwrap();
        validate_stereo_metrics(&metrics, SAMPLE_RATE as usize, "test").unwrap();
    }
}
