use std::f32::consts::PI;
use std::str::FromStr;

use anyhow::{Result, bail};
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::audio::{AudioClip, OUTPUT_FRAMES, SAMPLE_RATE};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    Multiresolution,
    SlidingWola,
    EvolvingIr,
    ChunkCrossfade,
}

impl Algorithm {
    pub const ALL: [Self; 4] = [
        Self::Multiresolution,
        Self::SlidingWola,
        Self::EvolvingIr,
        Self::ChunkCrossfade,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Multiresolution => "multiresolution",
            Self::SlidingWola => "sliding_wola",
            Self::EvolvingIr => "evolving_ir",
            Self::ChunkCrossfade => "chunk_crossfade",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Multiresolution => "Multiresolution convolution",
            Self::SlidingWola => "Sliding WOLA convolution",
            Self::EvolvingIr => "Dual evolving impulse response",
            Self::ChunkCrossfade => "Independent chunks + crossfade",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Multiresolution => 1,
            Self::SlidingWola => 2,
            Self::EvolvingIr => 3,
            Self::ChunkCrossfade => 4,
        }
    }
}

impl FromStr for Algorithm {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|algorithm| algorithm.slug() == value)
            .ok_or_else(|| anyhow::anyhow!("unknown algorithm {value}"))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPreset {
    Short,
    Medium,
    Long,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct WindowConfig {
    pub clip_a_seconds: f32,
    pub clip_b_seconds: f32,
    pub hop_seconds: f32,
}

impl WindowPreset {
    pub const ALL: [Self; 3] = [Self::Short, Self::Medium, Self::Long];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Medium => "medium",
            Self::Long => "long",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Short => "Short windows",
            Self::Medium => "Medium windows",
            Self::Long => "Long windows",
        }
    }

    pub fn config(self) -> WindowConfig {
        match self {
            Self::Short => WindowConfig {
                clip_a_seconds: 0.30,
                clip_b_seconds: 0.45,
                hop_seconds: 0.55,
            },
            Self::Medium => WindowConfig {
                clip_a_seconds: 0.90,
                clip_b_seconds: 1.30,
                hop_seconds: 1.40,
            },
            Self::Long => WindowConfig {
                clip_a_seconds: 2.25,
                clip_b_seconds: 3.25,
                hop_seconds: 3.20,
            },
        }
    }
}

impl FromStr for WindowPreset {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.slug() == value)
            .ok_or_else(|| anyhow::anyhow!("unknown preset {value}"))
    }
}

#[derive(Clone, Copy)]
enum SpectrumBand {
    Full,
    Low,
    Mid,
    High,
}

pub fn render_algorithm(
    algorithm: Algorithm,
    preset: WindowPreset,
    clip_a: &AudioClip,
    clip_b: &AudioClip,
) -> Result<Vec<f32>> {
    if clip_a.samples.len() != OUTPUT_FRAMES || clip_b.samples.len() != OUTPUT_FRAMES {
        bail!("windowed renderer requires two one-minute clips");
    }
    let config = preset.config();
    let output = match algorithm {
        Algorithm::Multiresolution => render_multiresolution(clip_a, clip_b, config)?,
        Algorithm::SlidingWola => render_sliding(clip_a, clip_b, config, SpectrumBand::Full)?,
        Algorithm::EvolvingIr => render_evolving_ir(clip_a, clip_b, config)?,
        Algorithm::ChunkCrossfade => render_chunk_crossfade(clip_a, clip_b, config)?,
    };
    if output.len() != OUTPUT_FRAMES {
        bail!("{} returned the wrong output length", algorithm.slug());
    }
    Ok(output)
}

fn render_multiresolution(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    base: WindowConfig,
) -> Result<Vec<f32>> {
    let bands = [
        (SpectrumBand::Low, 1.60_f32, 0.90_f32),
        (SpectrumBand::Mid, 1.00_f32, 1.00_f32),
        (SpectrumBand::High, 0.60_f32, 0.62_f32),
    ];
    let mut output = vec![0.0; OUTPUT_FRAMES];
    for (band, scale, mix) in bands {
        let config = WindowConfig {
            clip_a_seconds: base.clip_a_seconds * scale,
            clip_b_seconds: base.clip_b_seconds * scale,
            hop_seconds: base.hop_seconds * scale,
        };
        let rendered = render_sliding(clip_a, clip_b, config, band)?;
        for (output, band_sample) in output.iter_mut().zip(rendered) {
            *output += band_sample * mix;
        }
    }
    Ok(output)
}

fn render_sliding(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    config: WindowConfig,
    band: SpectrumBand,
) -> Result<Vec<f32>> {
    let a_frames = seconds_to_frames(config.clip_a_seconds);
    let b_frames = seconds_to_frames(config.clip_b_seconds);
    let hop_frames = seconds_to_frames(config.hop_seconds);
    let mut convolver = LocalConvolver::new(a_frames + b_frames - 1);
    let mut overlap = OverlapBuffer::new();
    let mut previous_gain = None;
    for center in centers(hop_frames) {
        let source_position = normalized_source_position(center);
        let a = extract_window(&clip_a.samples, source_position, a_frames);
        let b = extract_window(&clip_b.samples, source_position, b_frames);
        let mut local = convolver.convolve(&a, &b, band)?;
        previous_gain = Some(level_local(&mut local, previous_gain, 0.085));
        overlap.add_hann(center as isize, &local, 1.0);
    }
    Ok(overlap.finish())
}

fn render_evolving_ir(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    config: WindowConfig,
) -> Result<Vec<f32>> {
    let a_frames = seconds_to_frames(config.clip_a_seconds);
    let b_frames = seconds_to_frames(config.clip_b_seconds);
    let hop_frames = seconds_to_frames(config.hop_seconds);
    let mut convolver = LocalConvolver::new(a_frames + b_frames - 1);
    let mut overlap = OverlapBuffer::new();
    let mut gain_a = None;
    let mut gain_b = None;
    for center in centers(hop_frames) {
        let source_position = normalized_source_position(center);
        let a = extract_window(&clip_a.samples, source_position, a_frames);
        let b = extract_window(&clip_b.samples, source_position, b_frames);
        let local = convolver.convolve(&a, &b, SpectrumBand::Full)?;
        let mut a_carrier = center_crop(&local, a_frames);
        let mut b_carrier = center_crop(&local, b_frames);
        gain_a = Some(level_local(&mut a_carrier, gain_a, 0.078));
        gain_b = Some(level_local(&mut b_carrier, gain_b, 0.078));
        overlap.add_hann(center as isize, &a_carrier, 0.5);
        overlap.add_hann(center as isize, &b_carrier, 0.5);
    }
    Ok(overlap.finish())
}

fn render_chunk_crossfade(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    config: WindowConfig,
) -> Result<Vec<f32>> {
    let a_frames = seconds_to_frames(config.clip_a_seconds);
    let b_frames = seconds_to_frames(config.clip_b_seconds);
    let slot_frames = a_frames.max(b_frames);
    let crossfade_frames = (slot_frames / 4).max(2);
    let block_frames = slot_frames + crossfade_frames;
    let mut convolver = LocalConvolver::new(a_frames + b_frames - 1);
    let mut overlap = OverlapBuffer::new();
    let mut previous_gain = None;
    let mut start = 0_usize;
    while start < OUTPUT_FRAMES {
        let center = (start + slot_frames / 2).min(OUTPUT_FRAMES - 1);
        let source_position = normalized_source_position(center);
        let a = extract_window(&clip_a.samples, source_position, a_frames);
        let b = extract_window(&clip_b.samples, source_position, b_frames);
        let local = convolver.convolve(&a, &b, SpectrumBand::Full)?;
        let mut block = center_crop(&local, block_frames.min(local.len()));
        previous_gain = Some(level_local(&mut block, previous_gain, 0.085));
        overlap.add_equal_power(center as isize, &block, crossfade_frames);
        start = start.saturating_add(slot_frames);
    }
    Ok(overlap.finish())
}

fn seconds_to_frames(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32).round().max(16.0) as usize
}

fn centers(hop_frames: usize) -> Vec<usize> {
    let mut result = Vec::with_capacity(OUTPUT_FRAMES / hop_frames + 2);
    let mut center = 0;
    while center < OUTPUT_FRAMES {
        result.push(center);
        center += hop_frames;
    }
    if result.last().copied() != Some(OUTPUT_FRAMES - 1) {
        result.push(OUTPUT_FRAMES - 1);
    }
    result
}

fn normalized_source_position(output_center: usize) -> usize {
    let phase = output_center as f64 / (OUTPUT_FRAMES - 1) as f64;
    (phase * (OUTPUT_FRAMES - 1) as f64).round() as usize
}

fn extract_window(source: &[f32], center: usize, frames: usize) -> Vec<f32> {
    let half = frames as isize / 2;
    let mut output = Vec::with_capacity(frames);
    for index in 0..frames {
        let source_index = reflect_index(center as isize + index as isize - half, source.len());
        output.push(source[source_index] * tukey(index, frames, 0.5));
    }
    output
}

fn reflect_index(mut index: isize, length: usize) -> usize {
    let maximum = length as isize - 1;
    while index < 0 || index > maximum {
        if index < 0 {
            index = -index;
        }
        if index > maximum {
            index = 2 * maximum - index;
        }
    }
    index as usize
}

fn tukey(index: usize, length: usize, alpha: f32) -> f32 {
    if length < 2 {
        return 1.0;
    }
    let x = index as f32 / (length - 1) as f32;
    if x < alpha / 2.0 {
        0.5 * (1.0 + (PI * (2.0 * x / alpha - 1.0)).cos())
    } else if x <= 1.0 - alpha / 2.0 {
        1.0
    } else {
        0.5 * (1.0 + (PI * (2.0 * x / alpha - 2.0 / alpha + 1.0)).cos())
    }
}

fn center_crop(input: &[f32], length: usize) -> Vec<f32> {
    if length >= input.len() {
        return input.to_vec();
    }
    let start = (input.len() - length) / 2;
    input[start..start + length].to_vec()
}

fn level_local(samples: &mut [f32], previous_gain: Option<f32>, target_rms: f32) -> f32 {
    let rms = (samples
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt() as f32;
    let desired = (target_rms / rms.max(1.0e-10)).min(128.0);
    let gain = if let Some(previous) = previous_gain {
        let one_db = 10.0_f32.powf(1.0 / 20.0);
        desired.clamp(previous / one_db, previous * one_db)
    } else {
        desired
    };
    for sample in samples {
        *sample *= gain;
    }
    gain
}

struct LocalConvolver {
    fft_len: usize,
    forward: std::sync::Arc<dyn RealToComplex<f32>>,
    inverse: std::sync::Arc<dyn ComplexToReal<f32>>,
}

impl LocalConvolver {
    fn new(output_frames: usize) -> Self {
        let fft_len = output_frames.next_power_of_two();
        let mut planner = RealFftPlanner::<f32>::new();
        Self {
            fft_len,
            forward: planner.plan_fft_forward(fft_len),
            inverse: planner.plan_fft_inverse(fft_len),
        }
    }

    fn convolve(&mut self, left: &[f32], right: &[f32], band: SpectrumBand) -> Result<Vec<f32>> {
        let output_frames = left.len() + right.len() - 1;
        if output_frames > self.fft_len {
            bail!("local convolution exceeds planned FFT");
        }
        let mut left_time = vec![0.0; self.fft_len];
        let mut right_time = vec![0.0; self.fft_len];
        left_time[..left.len()].copy_from_slice(left);
        right_time[..right.len()].copy_from_slice(right);
        let mut left_spectrum = self.forward.make_output_vec();
        let mut right_spectrum = self.forward.make_output_vec();
        self.forward.process(&mut left_time, &mut left_spectrum)?;
        self.forward.process(&mut right_time, &mut right_spectrum)?;
        for (bin, (left, right)) in left_spectrum.iter_mut().zip(&right_spectrum).enumerate() {
            *left *= *right * band_gain(band, bin, self.fft_len);
        }
        let mut output = self.inverse.make_output_vec();
        self.inverse.process(&mut left_spectrum, &mut output)?;
        let scale = 1.0 / self.fft_len as f32;
        output.truncate(output_frames);
        for sample in &mut output {
            *sample *= scale;
        }
        Ok(output)
    }
}

fn band_gain(band: SpectrumBand, bin: usize, fft_len: usize) -> f32 {
    if matches!(band, SpectrumBand::Full) {
        return 1.0;
    }
    let frequency = bin as f32 * SAMPLE_RATE as f32 / fft_len as f32;
    let low = descending_transition(frequency, 160.0, 300.0);
    let high = ascending_transition(frequency, 1_700.0, 2_600.0);
    match band {
        SpectrumBand::Full => 1.0,
        SpectrumBand::Low => low,
        SpectrumBand::Mid => (1.0 - low - high).max(0.0),
        SpectrumBand::High => high,
    }
}

fn descending_transition(value: f32, start: f32, end: f32) -> f32 {
    if value <= start {
        1.0
    } else if value >= end {
        0.0
    } else {
        let phase = (value - start) / (end - start);
        0.5 + 0.5 * (PI * phase).cos()
    }
}

fn ascending_transition(value: f32, start: f32, end: f32) -> f32 {
    1.0 - descending_transition(value, start, end)
}

struct OverlapBuffer {
    samples: Vec<f32>,
    weights: Vec<f32>,
}

impl OverlapBuffer {
    fn new() -> Self {
        Self {
            samples: vec![0.0; OUTPUT_FRAMES],
            weights: vec![0.0; OUTPUT_FRAMES],
        }
    }

    fn add_hann(&mut self, center: isize, local: &[f32], mix: f32) {
        let start = center - local.len() as isize / 2;
        for (index, &sample) in local.iter().enumerate() {
            let output_index = start + index as isize;
            if !(0..OUTPUT_FRAMES as isize).contains(&output_index) {
                continue;
            }
            let phase = index as f32 / (local.len() - 1).max(1) as f32;
            let weight = (0.5 - 0.5 * (2.0 * PI * phase).cos()).max(1.0e-5) * mix;
            self.samples[output_index as usize] += sample * weight;
            self.weights[output_index as usize] += weight;
        }
    }

    fn add_equal_power(&mut self, center: isize, local: &[f32], fade_frames: usize) {
        let start = center - local.len() as isize / 2;
        for (index, &sample) in local.iter().enumerate() {
            let output_index = start + index as isize;
            if !(0..OUTPUT_FRAMES as isize).contains(&output_index) {
                continue;
            }
            let edge = index.min(local.len() - 1 - index);
            let weight = if edge >= fade_frames {
                1.0
            } else {
                ((edge as f32 / fade_frames.max(1) as f32) * PI / 2.0)
                    .sin()
                    .max(1.0e-5)
            };
            self.samples[output_index as usize] += sample * weight;
            self.weights[output_index as usize] += weight;
        }
    }

    fn finish(mut self) -> Vec<f32> {
        for (sample, weight) in self.samples.iter_mut().zip(self.weights) {
            if weight > 1.0e-8 {
                *sample /= weight;
            }
        }
        self.samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_masks_are_complementary() {
        for bin in 0..=4096 {
            let low = band_gain(SpectrumBand::Low, bin, 8192);
            let mid = band_gain(SpectrumBand::Mid, bin, 8192);
            let high = band_gain(SpectrumBand::High, bin, 8192);
            assert!((low + mid + high - 1.0).abs() < 1.0e-5);
        }
    }

    #[test]
    fn reflection_stays_in_bounds() {
        for index in -100..200 {
            assert!(reflect_index(index, 64) < 64);
        }
    }
}
