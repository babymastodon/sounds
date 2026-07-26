use std::f32::consts::PI;
use std::str::FromStr;

use anyhow::{Result, bail};
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::audio::{AudioClip, INPUT_FRAMES, INPUT_SECONDS, SAMPLE_RATE};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    Multiresolution,
    SlidingWola,
    EvolvingIr,
    ChunkCrossfade,
    FullConvolution,
}

impl Algorithm {
    pub const ALL: [Self; 5] = [
        Self::Multiresolution,
        Self::SlidingWola,
        Self::EvolvingIr,
        Self::ChunkCrossfade,
        Self::FullConvolution,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Multiresolution => "multiresolution",
            Self::SlidingWola => "sliding_wola",
            Self::EvolvingIr => "evolving_ir",
            Self::ChunkCrossfade => "chunk_crossfade",
            Self::FullConvolution => "full_convolution",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Multiresolution => "Multiresolution convolution",
            Self::SlidingWola => "Sliding WOLA convolution",
            Self::EvolvingIr => "Dual evolving impulse response",
            Self::ChunkCrossfade => "Independent chunks + crossfade",
            Self::FullConvolution => "Full linear convolution",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Multiresolution => {
                "Splits every local convolution into complementary low, mid, and high bands. \
                 Low frequencies use longer windows for stability; highs use shorter windows \
                 for sharper timing. Root-Hann synthesis and a convolution-derived power envelope \
                 keep overlapping grains even before the three aligned bands are recombined."
            }
            Self::SlidingWola => {
                "Extracts synchronized A/B windows along the full timeline, linearly convolves \
                 each pair, then merges them with power-normalized root-Hann overlap-add. This \
                 suppresses window-rate pulsing and is the neutral local-convolution baseline."
            }
            Self::EvolvingIr => {
                "Convolves synchronized A/B windows, crops the result into separate A-sized and \
                 B-sized carriers, then blends them through power-normalized root-Hann synthesis. \
                 Carrier balance shifts which source's local timing dominates."
            }
            Self::ChunkCrossfade => {
                "Convolves independent synchronized chunks, crops each result to its timeline slot, \
                 and joins neighboring chunks with equal-power crossfades. Crossfade percentage \
                 controls the transition duration."
            }
            Self::FullConvolution => {
                "Selects one segment from each clip and linearly convolves them in one FFT \
                 operation. Each selected cut receives a 20 ms edge fade, both segments default \
                 to the complete 61-second sources, and the complete A + B - 1 result is retained."
            }
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Multiresolution => 1,
            Self::SlidingWola => 2,
            Self::EvolvingIr => 3,
            Self::ChunkCrossfade => 4,
            Self::FullConvolution => 5,
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

pub const MIN_WINDOW_SECONDS: f32 = 0.10;
pub const MAX_WINDOW_SECONDS: f32 = 30.00;
pub const DEFAULT_A_WINDOW_SECONDS: f32 = 5.00;
pub const DEFAULT_B_WINDOW_SECONDS: f32 = 5.00;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AlgorithmParameters {
    pub taper: f32,
    pub multires_low_scale: f32,
    pub multires_high_scale: f32,
    pub multires_low_mix: f32,
    pub multires_high_mix: f32,
    pub multires_low_split_hz: f32,
    pub multires_high_split_hz: f32,
    pub multires_transition_width: f32,
    pub evolving_a_mix: f32,
    pub evolving_mix_motion: f32,
    pub evolving_crop_position: f32,
    pub window_overlap_percent: f32,
    pub window_b_offset_seconds: f32,
    pub chunk_crossfade_percent: f32,
    pub chunk_crop_position: f32,
    pub full_a_offset_seconds: f32,
    pub full_a_duration_seconds: f32,
    pub full_b_offset_seconds: f32,
    pub full_b_duration_seconds: f32,
}

impl Default for AlgorithmParameters {
    fn default() -> Self {
        Self {
            taper: 0.50,
            multires_low_scale: 1.60,
            multires_high_scale: 0.60,
            multires_low_mix: 0.90,
            multires_high_mix: 0.62,
            multires_low_split_hz: 230.0,
            multires_high_split_hz: 2_100.0,
            multires_transition_width: 0.30,
            evolving_a_mix: 0.50,
            evolving_mix_motion: 0.0,
            evolving_crop_position: 0.50,
            window_overlap_percent: 75.0,
            window_b_offset_seconds: 0.0,
            chunk_crossfade_percent: 50.0,
            chunk_crop_position: 0.50,
            full_a_offset_seconds: 0.0,
            full_a_duration_seconds: INPUT_SECONDS as f32,
            full_b_offset_seconds: 0.0,
            full_b_duration_seconds: INPUT_SECONDS as f32,
        }
    }
}

impl AlgorithmParameters {
    pub fn validate(self, algorithm: Algorithm) -> Result<Self> {
        match algorithm {
            Algorithm::Multiresolution => {
                validate_range("window taper", self.taper, 0.05, 1.0)?;
                validate_range("window overlap", self.window_overlap_percent, 5.0, 80.0)?;
                validate_range("low window scale", self.multires_low_scale, 1.0, 3.0)?;
                validate_range("high window scale", self.multires_high_scale, 0.15, 1.0)?;
                validate_range("low band mix", self.multires_low_mix, 0.0, 2.0)?;
                validate_range("high band mix", self.multires_high_mix, 0.0, 2.0)?;
                validate_range("low split", self.multires_low_split_hz, 80.0, 800.0)?;
                validate_range("high split", self.multires_high_split_hz, 800.0, 8_000.0)?;
                validate_range(
                    "band transition width",
                    self.multires_transition_width,
                    0.05,
                    0.75,
                )?;
                validate_window_scan(self)?;
                if self.multires_low_split_hz >= self.multires_high_split_hz {
                    bail!("low split must be below high split");
                }
            }
            Algorithm::EvolvingIr => {
                validate_range("window taper", self.taper, 0.05, 1.0)?;
                validate_range("window overlap", self.window_overlap_percent, 5.0, 80.0)?;
                validate_range("A carrier mix", self.evolving_a_mix, 0.0, 1.0)?;
                validate_range("carrier mix motion", self.evolving_mix_motion, -1.0, 1.0)?;
                validate_range(
                    "carrier crop position",
                    self.evolving_crop_position,
                    0.0,
                    1.0,
                )?;
                validate_window_scan(self)?;
            }
            Algorithm::ChunkCrossfade => {
                validate_range("window taper", self.taper, 0.05, 1.0)?;
                validate_range("chunk crossfade", self.chunk_crossfade_percent, 5.0, 75.0)?;
                validate_range("chunk crop position", self.chunk_crop_position, 0.0, 1.0)?;
                validate_range(
                    "clip B timeline offset",
                    self.window_b_offset_seconds,
                    -30.0,
                    30.0,
                )?;
            }
            Algorithm::SlidingWola => {
                validate_range("window taper", self.taper, 0.05, 1.0)?;
                validate_range("window overlap", self.window_overlap_percent, 5.0, 80.0)?;
                validate_window_scan(self)?;
            }
            Algorithm::FullConvolution => {
                validate_segment(
                    "A",
                    self.full_a_offset_seconds,
                    self.full_a_duration_seconds,
                )?;
                validate_segment(
                    "B",
                    self.full_b_offset_seconds,
                    self.full_b_duration_seconds,
                )?;
            }
        }
        Ok(self)
    }
}

fn validate_window_scan(parameters: AlgorithmParameters) -> Result<()> {
    validate_range(
        "clip B timeline offset",
        parameters.window_b_offset_seconds,
        -30.0,
        30.0,
    )
}

fn validate_range(label: &str, value: f32, minimum: f32, maximum: f32) -> Result<()> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        bail!("{label} must be a finite number from {minimum} to {maximum}");
    }
    Ok(())
}

fn validate_segment(label: &str, offset: f32, duration: f32) -> Result<()> {
    validate_range(
        &format!("clip {label} offset"),
        offset,
        0.0,
        INPUT_SECONDS as f32 - 0.1,
    )?;
    validate_range(
        &format!("clip {label} duration"),
        duration,
        0.1,
        INPUT_SECONDS as f32,
    )?;
    if offset + duration > INPUT_SECONDS as f32 + 1.0e-4 {
        bail!("clip {label} offset + duration must not exceed {INPUT_SECONDS} seconds");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct WindowConfig {
    pub clip_a_seconds: f32,
    pub clip_b_seconds: f32,
    pub hop_seconds: f32,
}

impl WindowConfig {
    pub fn new(clip_a_seconds: f32, clip_b_seconds: f32, overlap_percent: f32) -> Result<Self> {
        for (label, value) in [("clip A", clip_a_seconds), ("clip B", clip_b_seconds)] {
            if !value.is_finite() || !(MIN_WINDOW_SECONDS..=MAX_WINDOW_SECONDS).contains(&value) {
                bail!(
                    "{label} window must be a finite number from \
                     {MIN_WINDOW_SECONDS:.2} to {MAX_WINDOW_SECONDS:.2} seconds"
                );
            }
        }
        validate_range("window overlap", overlap_percent, 5.0, 80.0)?;
        Ok(Self {
            clip_a_seconds,
            clip_b_seconds,
            // The shorter analysis window controls scan density. The default
            // 75% overlap provides four analysis positions per short window;
            // convolution results span roughly A + B and therefore overlap
            // even more deeply during power-normalized synthesis.
            hop_seconds: clip_a_seconds.min(clip_b_seconds) * (1.0 - overlap_percent / 100.0),
        })
    }

    pub fn for_chunks(clip_a_seconds: f32, clip_b_seconds: f32) -> Result<Self> {
        let mut config = Self::new(clip_a_seconds, clip_b_seconds, 5.0)?;
        config.hop_seconds = clip_a_seconds.max(clip_b_seconds);
        Ok(config)
    }
}

#[derive(Clone, Copy)]
enum SpectrumBand {
    Full,
    Low,
    Mid,
    High,
}

pub fn render_algorithm_cancellable(
    algorithm: Algorithm,
    config: Option<WindowConfig>,
    parameters: AlgorithmParameters,
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    if clip_a.samples.len() != INPUT_FRAMES || clip_b.samples.len() != INPUT_FRAMES {
        bail!("windowed renderer requires two standard-length source clips");
    }
    if cancelled() {
        bail!("render cancelled");
    }
    let parameters = parameters.validate(algorithm)?;
    let output = match algorithm {
        Algorithm::Multiresolution => render_multiresolution(
            clip_a,
            clip_b,
            require_windows(config, algorithm)?,
            parameters,
            cancelled,
        )?,
        Algorithm::SlidingWola => {
            render_sliding(
                clip_a,
                clip_b,
                require_windows(config, algorithm)?,
                SpectrumBand::Full,
                parameters,
                cancelled,
            )?
            .samples
        }
        Algorithm::EvolvingIr => render_evolving_ir(
            clip_a,
            clip_b,
            require_windows(config, algorithm)?,
            parameters,
            cancelled,
        )?,
        Algorithm::ChunkCrossfade => render_chunk_crossfade(
            clip_a,
            clip_b,
            require_windows(config, algorithm)?,
            parameters,
            cancelled,
        )?,
        Algorithm::FullConvolution => render_full(clip_a, clip_b, parameters, cancelled)?,
    };
    if output.is_empty() {
        bail!("{} returned an empty waveform", algorithm.slug());
    }
    Ok(output)
}

fn require_windows(config: Option<WindowConfig>, algorithm: Algorithm) -> Result<WindowConfig> {
    config.ok_or_else(|| anyhow::anyhow!("{} requires A and B windows", algorithm.slug()))
}

fn render_full(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    parameters: AlgorithmParameters,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    if cancelled() {
        bail!("render cancelled");
    }
    let a = prepared_segment(
        &clip_a.samples,
        parameters.full_a_offset_seconds,
        parameters.full_a_duration_seconds,
    );
    let b = prepared_segment(
        &clip_b.samples,
        parameters.full_b_offset_seconds,
        parameters.full_b_duration_seconds,
    );
    let mut convolver = LocalConvolver::new(a.len() + b.len() - 1);
    let full = convolver.convolve(&a, &b, SpectrumBand::Full, 230.0, 2_100.0, 0.30)?;
    if cancelled() {
        bail!("render cancelled");
    }
    Ok(full)
}

fn prepared_segment(samples: &[f32], offset_seconds: f32, duration_seconds: f32) -> Vec<f32> {
    let start = seconds_to_frames_allow_zero(offset_seconds).min(samples.len() - 1);
    let frames = seconds_to_frames(duration_seconds);
    let mut segment = samples[start..(start + frames).min(samples.len())].to_vec();
    let fade_frames = (SAMPLE_RATE as usize / 50).min(segment.len() / 2);
    for index in 0..fade_frames {
        let phase = index as f32 / (fade_frames - 1).max(1) as f32;
        let gain = 0.5 - 0.5 * (PI * phase).cos();
        segment[index] *= gain;
        let tail = segment.len() - 1 - index;
        segment[tail] *= gain;
    }
    segment
}

struct TimelineRender {
    samples: Vec<f32>,
    start_frame: isize,
}

impl TimelineRender {
    fn end_frame(&self) -> isize {
        self.start_frame + self.samples.len() as isize
    }
}

fn combine_timelines(rendered: Vec<(TimelineRender, f32)>) -> Vec<f32> {
    let start_frame = rendered
        .iter()
        .map(|(band, _)| band.start_frame)
        .max()
        .expect("multiresolution has bands");
    let end_frame = rendered
        .iter()
        .map(|(band, _)| band.end_frame())
        .min()
        .expect("multiresolution has bands");
    assert!(
        end_frame > start_frame,
        "multiresolution bands must share a common timeline"
    );
    let mut output = vec![0.0; (end_frame - start_frame) as usize];
    for (band, mix) in rendered {
        let source_offset = (start_frame - band.start_frame) as usize;
        for (output, sample) in output
            .iter_mut()
            .zip(band.samples.into_iter().skip(source_offset))
        {
            *output += sample * mix;
        }
    }
    output
}

fn render_multiresolution(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    base: WindowConfig,
    parameters: AlgorithmParameters,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let bands = [
        (
            SpectrumBand::Low,
            parameters.multires_low_scale,
            parameters.multires_low_mix,
        ),
        (SpectrumBand::Mid, 1.00_f32, 1.00_f32),
        (
            SpectrumBand::High,
            parameters.multires_high_scale,
            parameters.multires_high_mix,
        ),
    ];
    let mut rendered_bands = Vec::with_capacity(bands.len());
    for (band, scale, mix) in bands {
        if cancelled() {
            bail!("render cancelled");
        }
        if mix <= 0.0 {
            continue;
        }
        let clip_a_seconds = (base.clip_a_seconds * scale).min(INPUT_SECONDS as f32);
        let clip_b_seconds = (base.clip_b_seconds * scale).min(INPUT_SECONDS as f32);
        let config = WindowConfig {
            clip_a_seconds,
            clip_b_seconds,
            hop_seconds: clip_a_seconds.min(clip_b_seconds)
                * (1.0 - parameters.window_overlap_percent / 100.0),
        };
        let rendered = render_sliding(clip_a, clip_b, config, band, parameters, cancelled)?;
        rendered_bands.push((rendered, mix));
    }
    Ok(combine_timelines(rendered_bands))
}

fn render_sliding(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    config: WindowConfig,
    band: SpectrumBand,
    parameters: AlgorithmParameters,
    cancelled: &dyn Fn() -> bool,
) -> Result<TimelineRender> {
    let a_frames = seconds_to_frames(config.clip_a_seconds);
    let b_frames = seconds_to_frames(config.clip_b_seconds);
    let hop_frames = seconds_to_frames(config.hop_seconds);
    let local_frames = a_frames + b_frames - 1;
    let render_centers = centers(hop_frames);
    let placements = render_centers
        .iter()
        .map(|&center| (center as isize, local_frames))
        .collect::<Vec<_>>();
    let mut convolver = LocalConvolver::new(local_frames);
    let power_profile = convolution_power_profile(&mut convolver, a_frames, b_frames, parameters)?;
    let mut overlap = OverlapBuffer::for_placements(&placements);
    let mut previous_gain = None;
    for center in render_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let (a, b) = extract_pair(clip_a, clip_b, center, a_frames, b_frames, parameters);
        let mut local = convolver.convolve(
            &a,
            &b,
            band,
            parameters.multires_low_split_hz,
            parameters.multires_high_split_hz,
            parameters.multires_transition_width,
        )?;
        previous_gain = Some(level_local(&mut local, previous_gain, 0.085));
        overlap.add_crossfade(center as isize, &local, &power_profile, 1.0);
    }
    Ok(overlap.finish())
}

fn render_evolving_ir(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    config: WindowConfig,
    parameters: AlgorithmParameters,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let a_frames = seconds_to_frames(config.clip_a_seconds);
    let b_frames = seconds_to_frames(config.clip_b_seconds);
    let hop_frames = seconds_to_frames(config.hop_seconds);
    let mut convolver = LocalConvolver::new(a_frames + b_frames - 1);
    let full_power = convolution_power_profile(&mut convolver, a_frames, b_frames, parameters)?;
    let a_power = normalized_power_profile(positioned_crop(
        &full_power,
        a_frames,
        parameters.evolving_crop_position,
    ));
    let b_power = normalized_power_profile(positioned_crop(
        &full_power,
        b_frames,
        parameters.evolving_crop_position,
    ));
    let render_centers = centers(hop_frames);
    let mut placements = Vec::new();
    for &center in &render_centers {
        let mix = evolving_mix(parameters, center);
        if mix > 0.0 {
            placements.push((center as isize, a_frames));
        }
        if mix < 1.0 {
            placements.push((center as isize, b_frames));
        }
    }
    let mut overlap = OverlapBuffer::for_placements(&placements);
    let mut gain_a = None;
    let mut gain_b = None;
    for center in render_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let (a, b) = extract_pair(clip_a, clip_b, center, a_frames, b_frames, parameters);
        let local = convolver.convolve(
            &a,
            &b,
            SpectrumBand::Full,
            parameters.multires_low_split_hz,
            parameters.multires_high_split_hz,
            parameters.multires_transition_width,
        )?;
        let mix = evolving_mix(parameters, center);
        if mix > 0.0 {
            let mut a_carrier =
                positioned_crop(&local, a_frames, parameters.evolving_crop_position);
            gain_a = Some(level_local(&mut a_carrier, gain_a, 0.078));
            overlap.add_crossfade(center as isize, &a_carrier, &a_power, mix);
        }
        if mix < 1.0 {
            let mut b_carrier =
                positioned_crop(&local, b_frames, parameters.evolving_crop_position);
            gain_b = Some(level_local(&mut b_carrier, gain_b, 0.078));
            overlap.add_crossfade(center as isize, &b_carrier, &b_power, 1.0 - mix);
        }
    }
    Ok(overlap.finish().samples)
}

fn render_chunk_crossfade(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    config: WindowConfig,
    parameters: AlgorithmParameters,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let a_frames = seconds_to_frames(config.clip_a_seconds);
    let b_frames = seconds_to_frames(config.clip_b_seconds);
    let slot_frames = a_frames.max(b_frames);
    let crossfade_frames =
        chunk_crossfade_frames(a_frames, b_frames, parameters.chunk_crossfade_percent);
    let local_frames = a_frames + b_frames - 1;
    let block_frames = (slot_frames + crossfade_frames).min(local_frames);
    let render_centers = chunk_centers(slot_frames);
    let placements = render_centers
        .iter()
        .map(|&center| (center as isize, block_frames))
        .collect::<Vec<_>>();
    let mut convolver = LocalConvolver::new(local_frames);
    let full_power = convolution_power_profile(&mut convolver, a_frames, b_frames, parameters)?;
    let block_power = normalized_power_profile(positioned_crop(
        &full_power,
        block_frames,
        parameters.chunk_crop_position,
    ));
    let mut overlap = OverlapBuffer::for_placements(&placements);
    let mut previous_gain = None;
    for center in render_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let (a, b) = extract_pair(clip_a, clip_b, center, a_frames, b_frames, parameters);
        let local = convolver.convolve(
            &a,
            &b,
            SpectrumBand::Full,
            parameters.multires_low_split_hz,
            parameters.multires_high_split_hz,
            parameters.multires_transition_width,
        )?;
        let mut block = positioned_crop(
            &local,
            block_frames.min(local.len()),
            parameters.chunk_crop_position,
        );
        previous_gain = Some(level_local(&mut block, previous_gain, 0.085));
        overlap.add_equal_power(center as isize, &block, &block_power, crossfade_frames);
    }
    Ok(overlap.finish().samples)
}

fn seconds_to_frames(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32).round().max(16.0) as usize
}

fn chunk_crossfade_frames(a_frames: usize, b_frames: usize, percent: f32) -> usize {
    ((a_frames.min(b_frames) as f32 * percent / 100.0).round() as usize).max(2)
}

fn seconds_to_frames_allow_zero(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32).round().max(0.0) as usize
}

fn centers(hop_frames: usize) -> Vec<usize> {
    let mut result = Vec::with_capacity(INPUT_FRAMES / hop_frames + 2);
    let mut center = 0;
    while center < INPUT_FRAMES {
        result.push(center);
        center += hop_frames;
    }
    if result.last().copied() != Some(INPUT_FRAMES - 1) {
        result.push(INPUT_FRAMES - 1);
    }
    result
}

fn chunk_centers(slot_frames: usize) -> Vec<usize> {
    let mut result = Vec::with_capacity(INPUT_FRAMES / slot_frames + 1);
    let mut start = 0_usize;
    while start < INPUT_FRAMES {
        result.push((start + slot_frames / 2).min(INPUT_FRAMES - 1));
        start = start.saturating_add(slot_frames);
    }
    result
}

fn normalized_source_position(output_center: usize) -> usize {
    let phase = output_center as f64 / (INPUT_FRAMES - 1) as f64;
    (phase * (INPUT_FRAMES - 1) as f64).round() as usize
}

fn extract_pair(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    output_center: usize,
    a_frames: usize,
    b_frames: usize,
    parameters: AlgorithmParameters,
) -> (Vec<f32>, Vec<f32>) {
    let a_center = normalized_source_position(output_center) as isize;
    let b_center = a_center + seconds_to_frames_signed(parameters.window_b_offset_seconds);
    (
        extract_window(&clip_a.samples, a_center, a_frames, parameters.taper),
        extract_window(&clip_b.samples, b_center, b_frames, parameters.taper),
    )
}

fn seconds_to_frames_signed(seconds: f32) -> isize {
    (seconds * SAMPLE_RATE as f32).round() as isize
}

fn evolving_mix(parameters: AlgorithmParameters, center: usize) -> f32 {
    let phase = center as f32 / (INPUT_FRAMES - 1) as f32;
    (parameters.evolving_a_mix + parameters.evolving_mix_motion * (phase - 0.5)).clamp(0.0, 1.0)
}

fn extract_window(source: &[f32], center: isize, frames: usize, taper: f32) -> Vec<f32> {
    let half = frames as isize / 2;
    let mut output = Vec::with_capacity(frames);
    for index in 0..frames {
        let source_index = reflect_index(center + index as isize - half, source.len());
        output.push(source[source_index] * tukey(index, frames, taper));
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

fn positioned_crop(input: &[f32], length: usize, position: f32) -> Vec<f32> {
    if length >= input.len() {
        return input.to_vec();
    }
    let start = ((input.len() - length) as f32 * position).round() as usize;
    input[start..start + length].to_vec()
}

fn convolution_power_profile(
    convolver: &mut LocalConvolver,
    a_frames: usize,
    b_frames: usize,
    parameters: AlgorithmParameters,
) -> Result<Vec<f32>> {
    let analysis_power = |frames| {
        (0..frames)
            .map(|index| tukey(index, frames, parameters.taper).powi(2))
            .collect::<Vec<_>>()
    };
    let profile = convolver.convolve(
        &analysis_power(a_frames),
        &analysis_power(b_frames),
        SpectrumBand::Full,
        parameters.multires_low_split_hz,
        parameters.multires_high_split_hz,
        parameters.multires_transition_width,
    )?;
    Ok(normalized_power_profile(profile))
}

fn normalized_power_profile(mut profile: Vec<f32>) -> Vec<f32> {
    for power in &mut profile {
        *power = power.max(0.0);
    }
    let mean =
        profile.iter().map(|&power| f64::from(power)).sum::<f64>() / profile.len().max(1) as f64;
    let scale = 1.0 / (mean as f32).max(1.0e-12);
    for power in &mut profile {
        *power *= scale;
    }
    profile
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

    fn convolve(
        &mut self,
        left: &[f32],
        right: &[f32],
        band: SpectrumBand,
        low_split_hz: f32,
        high_split_hz: f32,
        transition_width: f32,
    ) -> Result<Vec<f32>> {
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
            *left *= *right
                * band_gain(
                    band,
                    bin,
                    self.fft_len,
                    low_split_hz,
                    high_split_hz,
                    transition_width,
                );
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

fn band_gain(
    band: SpectrumBand,
    bin: usize,
    fft_len: usize,
    low_split_hz: f32,
    high_split_hz: f32,
    transition_width: f32,
) -> f32 {
    if matches!(band, SpectrumBand::Full) {
        return 1.0;
    }
    let frequency = bin as f32 * SAMPLE_RATE as f32 / fft_len as f32;
    let low = descending_transition(
        frequency,
        low_split_hz * (1.0 - transition_width),
        low_split_hz * (1.0 + transition_width),
    );
    let high = ascending_transition(
        frequency,
        high_split_hz * (1.0 - transition_width),
        high_split_hz * (1.0 + transition_width),
    );
    let mid = (1.0 - low) * (1.0 - high);
    let total = (low + mid + high).max(1.0e-12);
    match band {
        SpectrumBand::Full => 1.0,
        SpectrumBand::Low => low / total,
        SpectrumBand::Mid => mid / total,
        SpectrumBand::High => high / total,
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
    power_weights: Vec<f32>,
    start_frame: isize,
}

impl OverlapBuffer {
    fn for_placements(placements: &[(isize, usize)]) -> Self {
        let start_frame = placements
            .iter()
            .map(|&(center, length)| center - length as isize / 2)
            .min()
            .expect("overlap-add requires at least one placement");
        let end_frame = placements
            .iter()
            .map(|&(center, length)| center - length as isize / 2 + length as isize)
            .max()
            .expect("overlap-add requires at least one placement");
        let frames = (end_frame - start_frame) as usize;
        Self {
            samples: vec![0.0; frames],
            power_weights: vec![0.0; frames],
            start_frame,
        }
    }

    fn add_crossfade(&mut self, center: isize, local: &[f32], power_profile: &[f32], mix: f32) {
        assert_eq!(local.len(), power_profile.len());
        let start = center - local.len() as isize / 2;
        for (index, (&sample, &local_power)) in local.iter().zip(power_profile).enumerate() {
            let output_index = start + index as isize - self.start_frame;
            if !(0..self.samples.len() as isize).contains(&output_index) {
                continue;
            }
            let weight = tukey(index, local.len(), 1.0).sqrt().max(1.0e-5) * mix;
            self.samples[output_index as usize] += sample * weight;
            self.power_weights[output_index as usize] += weight * weight * local_power;
        }
    }

    fn add_equal_power(
        &mut self,
        center: isize,
        local: &[f32],
        power_profile: &[f32],
        fade_frames: usize,
    ) {
        assert_eq!(local.len(), power_profile.len());
        let start = center - local.len() as isize / 2;
        for (index, (&sample, &local_power)) in local.iter().zip(power_profile).enumerate() {
            let output_index = start + index as isize - self.start_frame;
            if !(0..self.samples.len() as isize).contains(&output_index) {
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
            self.power_weights[output_index as usize] += weight * weight * local_power;
        }
    }

    fn finish(mut self) -> TimelineRender {
        for (sample, power_weight) in self.samples.iter_mut().zip(self.power_weights) {
            if power_weight > 1.0e-14 {
                *sample /= power_weight.sqrt();
            } else {
                *sample = 0.0;
            }
        }
        TimelineRender {
            samples: self.samples,
            start_frame: self.start_frame,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_masks_are_complementary() {
        for transition_width in [0.05, 0.30, 0.75] {
            for bin in 0..=4096 {
                let low = band_gain(SpectrumBand::Low, bin, 8192, 800.0, 801.0, transition_width);
                let mid = band_gain(SpectrumBand::Mid, bin, 8192, 800.0, 801.0, transition_width);
                let high = band_gain(
                    SpectrumBand::High,
                    bin,
                    8192,
                    800.0,
                    801.0,
                    transition_width,
                );
                assert!((low + mid + high - 1.0).abs() < 1.0e-5);
            }
        }
    }

    #[test]
    fn reflection_stays_in_bounds() {
        for index in -100..200 {
            assert!(reflect_index(index, 64) < 64);
        }
    }

    #[test]
    fn continuous_windows_validate_and_derive_hop() {
        let config = WindowConfig::new(0.37, 1.29, 35.0).unwrap();
        assert_eq!(config.clip_a_seconds, 0.37);
        assert_eq!(config.clip_b_seconds, 1.29);
        assert!((config.hop_seconds - 0.2405).abs() < 1.0e-6);
        assert!(WindowConfig::new(MIN_WINDOW_SECONDS - 0.01, 1.0, 35.0).is_err());
        assert!(WindowConfig::new(1.0, MAX_WINDOW_SECONDS + 0.01, 35.0).is_err());
        assert!(WindowConfig::new(f32::NAN, 1.0, 35.0).is_err());
        assert!(WindowConfig::new(1.0, 1.0, 90.0).is_err());
        assert!((WindowConfig::new(1.0, 2.0, 75.0).unwrap().hop_seconds - 0.25).abs() < 1.0e-6);
        assert_eq!(WindowConfig::for_chunks(1.0, 2.0).unwrap().hop_seconds, 2.0);
    }

    #[test]
    fn method_parameters_are_validated_by_method() {
        let mut parameters = AlgorithmParameters {
            chunk_crossfade_percent: 90.0,
            ..AlgorithmParameters::default()
        };
        assert!(parameters.validate(Algorithm::ChunkCrossfade).is_err());
        assert!(parameters.validate(Algorithm::SlidingWola).is_ok());

        parameters = AlgorithmParameters::default();
        parameters.multires_low_split_hz = 3_000.0;
        assert!(parameters.validate(Algorithm::Multiresolution).is_err());

        parameters = AlgorithmParameters::default();
        parameters.full_a_offset_seconds = 30.0;
        parameters.full_a_duration_seconds = 31.1;
        assert!(parameters.validate(Algorithm::FullConvolution).is_err());

        parameters = AlgorithmParameters::default();
        parameters.window_overlap_percent = 90.0;
        assert!(parameters.validate(Algorithm::SlidingWola).is_err());

        parameters = AlgorithmParameters::default();
        parameters.evolving_crop_position = 1.1;
        assert!(parameters.validate(Algorithm::EvolvingIr).is_err());
    }

    #[test]
    fn full_segments_select_exact_frame_ranges() {
        let samples = vec![1.0; INPUT_FRAMES];
        assert_eq!(
            prepared_segment(&samples, 10.0, 15.0).len(),
            15 * SAMPLE_RATE as usize
        );
        assert_eq!(
            prepared_segment(&samples, 0.0, INPUT_SECONDS as f32).len(),
            INPUT_FRAMES
        );
        let segment = prepared_segment(&samples, 10.0, 15.0);
        assert_eq!(segment[0], 0.0);
        assert_eq!(*segment.last().unwrap(), 0.0);
        assert_eq!(segment[SAMPLE_RATE as usize], 1.0);
    }

    #[test]
    fn overlap_add_keeps_leading_and_trailing_support() {
        let local_frames = 101;
        let placements = [(0, local_frames), (INPUT_FRAMES as isize - 1, local_frames)];
        let mut overlap = OverlapBuffer::for_placements(&placements);
        overlap.add_crossfade(
            placements[0].0,
            &vec![1.0; local_frames],
            &vec![1.0; local_frames],
            1.0,
        );
        overlap.add_crossfade(
            placements[1].0,
            &vec![1.0; local_frames],
            &vec![1.0; local_frames],
            1.0,
        );
        let rendered = overlap.finish();
        assert_eq!(rendered.start_frame, -50);
        assert_eq!(rendered.samples.len(), INPUT_FRAMES + local_frames - 1);
        assert!((rendered.samples[0] - 1.0).abs() < 1.0e-5);
        assert!((rendered.samples.last().unwrap() - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn overlapping_results_make_a_gradual_crossfade() {
        let placements = [(0, 101), (50, 101)];
        let mut overlap = OverlapBuffer::for_placements(&placements);
        overlap.add_crossfade(0, &vec![0.0; 101], &vec![1.0; 101], 1.0);
        overlap.add_crossfade(50, &vec![1.0; 101], &vec![1.0; 101], 1.0);
        let rendered = overlap.finish();
        let transition = &rendered.samples[50..100];
        assert!(transition.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(
            transition
                .windows(2)
                .all(|pair| (pair[1] - pair[0]).abs() < 0.05)
        );
    }

    #[test]
    fn default_overlap_keeps_the_expected_power_envelope_even() {
        let frames = 1_200;
        let hop = frames / 4;
        let local_frames = frames * 2 - 1;
        let placements = (0..=12)
            .map(|index| ((index * hop) as isize, local_frames))
            .collect::<Vec<_>>();
        let mut convolver = LocalConvolver::new(local_frames);
        let profile = convolution_power_profile(
            &mut convolver,
            frames,
            frames,
            AlgorithmParameters::default(),
        )
        .unwrap();
        let mut overlap = OverlapBuffer::for_placements(&placements);
        let silent = vec![0.0; local_frames];
        for &(center, _) in &placements {
            overlap.add_crossfade(center, &silent, &profile, 1.0);
        }
        let first = (4 * hop) as isize - overlap.start_frame;
        let last = (8 * hop) as isize - overlap.start_frame;
        let steady = &overlap.power_weights[first as usize..last as usize];
        let minimum = steady.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = steady.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let spread_db = 10.0 * (maximum / minimum).log10();
        assert!(
            spread_db < 0.25,
            "expected-power ripple was {spread_db:.3} dB"
        );
    }

    #[test]
    fn multiresolution_uses_the_common_band_timeline() {
        let output = combine_timelines(vec![
            (
                TimelineRender {
                    samples: vec![1.0; 6],
                    start_frame: -2,
                },
                1.0,
            ),
            (
                TimelineRender {
                    samples: vec![2.0; 2],
                    start_frame: 0,
                },
                0.5,
            ),
        ]);
        assert_eq!(output, vec![2.0, 2.0]);
    }

    #[test]
    fn crop_position_and_carrier_motion_cover_their_full_ranges() {
        let input = (0..10).map(|value| value as f32).collect::<Vec<_>>();
        assert_eq!(positioned_crop(&input, 4, 0.0), vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(positioned_crop(&input, 4, 0.5), vec![3.0, 4.0, 5.0, 6.0]);
        assert_eq!(positioned_crop(&input, 4, 1.0), vec![6.0, 7.0, 8.0, 9.0]);

        let parameters = AlgorithmParameters {
            evolving_mix_motion: 1.0,
            ..AlgorithmParameters::default()
        };
        assert!((evolving_mix(parameters, 0) - 0.0).abs() < 1.0e-6);
        assert!((evolving_mix(parameters, INPUT_FRAMES - 1) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn chunk_overlap_uses_available_short_window_support() {
        assert_eq!(chunk_crossfade_frames(48_000, 96_000, 25.0), 12_000);
        assert_eq!(chunk_crossfade_frames(4_800, 1_440_000, 75.0), 3_600);
    }
}
