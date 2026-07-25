use std::f32::consts::PI;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde::{Deserialize, Serialize};

pub const SAMPLE_RATE: u32 = 48_000;
pub const INPUT_SECONDS: usize = 60;
pub const INPUT_FRAMES: usize = SAMPLE_RATE as usize * INPUT_SECONDS;
const INPUT_TARGET_RMS: f32 = 0.10;
const OUTPUT_TARGET_RMS: f32 = 0.095;
const OUTPUT_CEILING: f32 = 0.92;

#[derive(Clone, Debug)]
pub struct AudioClip {
    pub id: String,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

pub fn read_prepared_clip(id: &str, path: &Path) -> Result<AudioClip> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("open input {}", path.display()))?;
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
            .map(|sample| sample.map(|value| value as f32 / 32768.0))
            .collect::<std::result::Result<Vec<_>, _>>()?,
        SampleFormat::Int => {
            let scale = (1_u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<std::result::Result<Vec<_>, _>>()?
        }
    };
    if samples.len().abs_diff(INPUT_FRAMES) > 2 {
        bail!(
            "{} has {} frames; expected {}",
            path.display(),
            samples.len(),
            INPUT_FRAMES
        );
    }
    samples.resize(INPUT_FRAMES, 0.0);
    condition_input(&mut samples)?;
    Ok(AudioClip {
        id: id.to_owned(),
        samples,
    })
}

fn condition_input(samples: &mut [f32]) -> Result<()> {
    ensure_signal(samples, "input")?;
    remove_mean(samples);
    high_pass(samples, 18.0);
    remove_mean(samples);
    let metrics = measure(samples);
    let gain = (INPUT_TARGET_RMS / metrics.rms.max(1.0e-12)).min(0.82 / metrics.peak.max(1.0e-12));
    for sample in samples.iter_mut() {
        *sample *= gain;
    }
    edge_fade(samples, 0.02);
    ensure_signal(samples, "conditioned input")
}

pub fn condition_output(samples: &mut [f32]) -> Result<AudioMetrics> {
    ensure_signal(samples, "raw convolution")?;
    remove_mean(samples);
    high_pass(samples, 18.0);
    remove_mean(samples);
    let original = samples.to_vec();
    let raw = measure(&original);
    let mut gain = OUTPUT_TARGET_RMS / raw.rms.max(1.0e-12);
    gain = gain.min(64.0);
    for _ in 0..4 {
        for (output, input) in samples.iter_mut().zip(&original) {
            *output = OUTPUT_CEILING * (*input * gain / OUTPUT_CEILING).tanh();
        }
        let rms = measure(samples).rms;
        if rms >= 0.065 {
            break;
        }
        gain *= (0.075 / rms.max(1.0e-12)).clamp(1.0, 3.0);
    }
    remove_mean(samples);
    let peak = measure(samples).peak;
    if peak > OUTPUT_CEILING {
        let gain = OUTPUT_CEILING / peak;
        for sample in samples.iter_mut() {
            *sample *= gain;
        }
    }
    edge_fade(samples, 0.02);
    let metrics = measure(samples);
    validate_metrics(&metrics, samples.len(), "conditioned output")?;
    Ok(metrics)
}

pub fn encode_pcm16(samples: &[f32]) -> Result<Vec<u8>> {
    if samples.is_empty() {
        bail!("refusing to encode an empty waveform");
    }
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    {
        let cursor = Cursor::new(&mut bytes);
        let mut writer = WavWriter::new(cursor, spec)?;
        for &sample in samples {
            writer.write_sample((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
        }
        writer.finalize()?;
    }
    Ok(bytes)
}

pub fn validate_metrics(metrics: &AudioMetrics, frames: usize, context: &str) -> Result<()> {
    if metrics.frames != frames {
        bail!(
            "{context}: expected {frames} frames, found {}",
            metrics.frames
        );
    }
    if metrics.non_finite_samples != 0 {
        bail!("{context}: contains non-finite samples");
    }
    if metrics.rms < 1.0e-4 || !metrics.rms.is_finite() {
        bail!("{context}: is silent or has invalid RMS");
    }
    if metrics.peak > 0.925 {
        bail!("{context}: peak {:.4} exceeds ceiling", metrics.peak);
    }
    if metrics.clipped_samples != 0 {
        bail!("{context}: contains clipped samples");
    }
    Ok(())
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
        sum += f64::from(sample);
        sum_squares += f64::from(sample) * f64::from(sample);
        clipped_samples += usize::from(sample.abs() >= 1.0);
    }
    let count = samples.len().max(1) as f64;
    let rms = (sum_squares / count).sqrt() as f32;
    AudioMetrics {
        frames: samples.len(),
        duration_seconds: samples.len() as f64 / f64::from(SAMPLE_RATE),
        peak,
        rms,
        rms_dbfs: 20.0 * rms.max(1.0e-12).log10(),
        dc_offset: (sum / count) as f32,
        clipped_samples,
        non_finite_samples,
    }
}

fn ensure_signal(samples: &[f32], context: &str) -> Result<()> {
    let metrics = measure(samples);
    if metrics.non_finite_samples != 0 || metrics.rms < 1.0e-10 {
        bail!("{context} is silent or non-finite");
    }
    Ok(())
}

fn remove_mean(samples: &mut [f32]) {
    let mean =
        samples.iter().map(|&value| f64::from(value)).sum::<f64>() / samples.len().max(1) as f64;
    for sample in samples {
        *sample -= mean as f32;
    }
}

fn high_pass(samples: &mut [f32], cutoff: f32) {
    let rc = 1.0 / (2.0 * PI * cutoff);
    let dt = 1.0 / SAMPLE_RATE as f32;
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

fn edge_fade(samples: &mut [f32], seconds: f32) {
    let frames = ((seconds * SAMPLE_RATE as f32).round() as usize)
        .min(samples.len() / 2)
        .max(2);
    for index in 0..frames {
        let phase = index as f32 / (frames - 1) as f32;
        let gain = 0.5 - 0.5 * (PI * phase).cos();
        samples[index] *= gain;
        let tail = samples.len() - 1 - index;
        samples[tail] *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_encoder_preserves_variable_output_length() {
        let frames = SAMPLE_RATE as usize * 2 + 137;
        let samples = (0..frames)
            .map(|frame| (2.0 * PI * 220.0 * frame as f32 / SAMPLE_RATE as f32).sin() * 0.2)
            .collect::<Vec<_>>();
        let bytes = encode_pcm16(&samples).unwrap();
        let reader = WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.duration() as usize, frames);
        assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
    }
}
