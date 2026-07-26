use std::f32::consts::PI;
use std::str::FromStr;

use anyhow::{Result, bail};
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::audio::{AudioClip, INPUT_FRAMES, INPUT_SECONDS, SAMPLE_RATE};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    WindowedConvolution,
    EvolvingIr,
    ChunkCrossfade,
    FullConvolution,
    DryA,
    DryB,
}

impl Algorithm {
    pub const ALL: [Self; 6] = [
        Self::WindowedConvolution,
        Self::EvolvingIr,
        Self::ChunkCrossfade,
        Self::FullConvolution,
        Self::DryA,
        Self::DryB,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::WindowedConvolution => "windowed_convolution",
            Self::EvolvingIr => "evolving_ir",
            Self::ChunkCrossfade => "chunk_crossfade",
            Self::FullConvolution => "full_convolution",
            Self::DryA => "dry_a",
            Self::DryB => "dry_b",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::WindowedConvolution => "Windowed convolution",
            Self::EvolvingIr => "Dual evolving impulse response",
            Self::ChunkCrossfade => "Independent chunks + crossfade",
            Self::FullConvolution => "Full linear convolution",
            Self::DryA => "Dry source A",
            Self::DryB => "Dry source B",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::WindowedConvolution => {
                "Extracts synchronized A/B windows in lockstep and performs one ordinary linear \
                 FFT convolution for each pair. Results are placed at tA+tB, then root-Hann \
                 crossfades use positive-coherence-aware power normalization to prevent correlated \
                 grain swelling and hop-rate buzz, with one gradual fade at each complete edge."
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
            Self::DryA => {
                "Plays the complete conditioned clip A exactly as it enters the convolution \
                 methods. No convolution, output saturation, or second level-normalization pass \
                 is applied, making this a direct 61-second source-listening reference."
            }
            Self::DryB => {
                "Plays the complete conditioned clip B exactly as it enters the convolution \
                 methods. No convolution, output saturation, or second level-normalization pass \
                 is applied, making this a direct 61-second source-listening reference."
            }
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::WindowedConvolution => 1,
            Self::EvolvingIr => 2,
            Self::ChunkCrossfade => 3,
            Self::FullConvolution => 4,
            Self::DryA => 5,
            Self::DryB => 6,
        }
    }

    pub fn uses_windows(self) -> bool {
        matches!(
            self,
            Self::WindowedConvolution | Self::EvolvingIr | Self::ChunkCrossfade
        )
    }

    pub fn is_dry(self) -> bool {
        matches!(self, Self::DryA | Self::DryB)
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
const DEFAULT_INPUT_TAPER: f32 = 0.50;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AlgorithmParameters {
    pub input_taper: f32,
    pub evolving_a_mix: f32,
    pub evolving_mix_motion: f32,
    pub evolving_crop_position: f32,
    pub window_overlap_percent: f32,
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
            input_taper: DEFAULT_INPUT_TAPER,
            evolving_a_mix: 0.50,
            evolving_mix_motion: 0.0,
            evolving_crop_position: 0.50,
            window_overlap_percent: 75.0,
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
            Algorithm::WindowedConvolution => {
                validate_range("input taper", self.input_taper, 0.05, 1.0)?;
                validate_range("window overlap", self.window_overlap_percent, 5.0, 80.0)?;
            }
            Algorithm::EvolvingIr => {
                validate_range("input taper", self.input_taper, 0.05, 1.0)?;
                validate_range("window overlap", self.window_overlap_percent, 5.0, 80.0)?;
                validate_range("A carrier mix", self.evolving_a_mix, 0.0, 1.0)?;
                validate_range("carrier mix motion", self.evolving_mix_motion, -1.0, 1.0)?;
                validate_range(
                    "carrier crop position",
                    self.evolving_crop_position,
                    0.0,
                    1.0,
                )?;
            }
            Algorithm::ChunkCrossfade => {
                validate_range("input taper", self.input_taper, 0.05, 1.0)?;
                validate_range("chunk crossfade", self.chunk_crossfade_percent, 5.0, 75.0)?;
                validate_range("chunk crop position", self.chunk_crop_position, 0.0, 1.0)?;
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
            Algorithm::DryA | Algorithm::DryB => {}
        }
        Ok(self)
    }
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
            // The shorter analysis window controls scan density. The 75%
            // default gives four source positions per short window; physical
            // convolution placement doubles that hop and leaves four-way
            // coverage for equal A/B window lengths.
            hop_seconds: clip_a_seconds.min(clip_b_seconds) * (1.0 - overlap_percent / 100.0),
        })
    }

    pub fn for_chunks(clip_a_seconds: f32, clip_b_seconds: f32) -> Result<Self> {
        let mut config = Self::new(clip_a_seconds, clip_b_seconds, 5.0)?;
        config.hop_seconds = clip_a_seconds.max(clip_b_seconds);
        Ok(config)
    }
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
        Algorithm::WindowedConvolution => render_windowed(
            clip_a,
            clip_b,
            require_windows(config, algorithm)?,
            parameters.input_taper,
            cancelled,
        )?,
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
        Algorithm::DryA => clip_a.samples.clone(),
        Algorithm::DryB => clip_b.samples.clone(),
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
    let full = convolver.convolve(&a, &b)?;
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

fn render_windowed(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    config: WindowConfig,
    input_taper: f32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let a_frames = seconds_to_frames(config.clip_a_seconds);
    let b_frames = seconds_to_frames(config.clip_b_seconds);
    let hop_frames = seconds_to_frames(config.hop_seconds);
    let mut rendered = render_windowed_samples(
        &clip_a.samples,
        &clip_b.samples,
        a_frames,
        b_frames,
        hop_frames,
        input_taper,
        cancelled,
    )?;
    // Normalization intentionally removes missing-neighbor attenuation. Restore
    // one natural half-Hann entrance/exit around the complete output only.
    debug_assert_eq!(
        rendered.start_frame,
        -((a_frames + b_frames - 1) as isize / 2)
    );
    let edge_frames = ((a_frames + b_frames - 1) / 2).min(rendered.samples.len() / 2);
    apply_half_hann_edge_fade(&mut rendered.samples, edge_frames);
    Ok(rendered.samples)
}

fn render_windowed_samples(
    clip_a: &[f32],
    clip_b: &[f32],
    a_frames: usize,
    b_frames: usize,
    hop_frames: usize,
    input_taper: f32,
    cancelled: &dyn Fn() -> bool,
) -> Result<TimelineRender> {
    if clip_a.len() != clip_b.len() || clip_a.is_empty() {
        bail!("windowed convolution requires equal, non-empty source timelines");
    }
    let local_frames = a_frames + b_frames - 1;
    let source_centers = centers_for_length(clip_a.len(), hop_frames);
    let placements = source_centers
        .iter()
        .map(|&center| (convolution_center(center), local_frames))
        .collect::<Vec<_>>();
    let mut convolver = LocalConvolver::new(local_frames);
    let power_profile = convolution_power_profile(&mut convolver, a_frames, b_frames, input_taper)?;
    let mut overlap = OverlapBuffer::for_placements(&placements);
    let mut previous_gain = None;
    let mut previous_local: Option<(isize, Vec<f32>)> = None;
    let level_smoothing = gain_smoothing_for_hop(hop_frames);
    for source_center in source_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let output_center = convolution_center(source_center);
        let (a, b) = extract_pair_samples(
            clip_a,
            clip_b,
            source_center,
            a_frames,
            b_frames,
            input_taper,
        );
        let mut local = convolver.convolve(&a, &b)?;
        previous_gain = Some(level_local(
            &mut local,
            previous_gain,
            0.085,
            level_smoothing,
        ));
        overlap.add_crossfade(output_center, &local, &power_profile, 1.0);
        if let Some((previous_center, previous)) = previous_local.take() {
            let coherence =
                aligned_positive_coherence(&previous, previous_center, &local, output_center);
            overlap.add_coherence_pair(previous_center, output_center, &power_profile, coherence);
        }
        previous_local = Some((output_center, local));
    }
    Ok(overlap.finish())
}

fn convolution_center(source_center: usize) -> isize {
    // A local convolution is centered at tA + tB. Both lockstep windows
    // advance together, so their result advances twice as far on the physical
    // convolution timeline. Placing it at only t phase-resets stationary tones
    // at every hop—the classic granular-buzz failure.
    source_center.saturating_mul(2) as isize
}

fn apply_half_hann_edge_fade(samples: &mut [f32], frames: usize) {
    if frames < 2 {
        return;
    }
    for index in 0..frames {
        let phase = index as f32 / (frames - 1) as f32;
        let gain = (phase * PI / 2.0).sin().powi(2);
        samples[index] *= gain;
        let tail = samples.len() - 1 - index;
        samples[tail] *= gain;
    }
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
    let full_power =
        convolution_power_profile(&mut convolver, a_frames, b_frames, parameters.input_taper)?;
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
    let level_smoothing = gain_smoothing_for_hop(hop_frames);
    for center in render_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let (a, b) = extract_pair(
            clip_a,
            clip_b,
            center,
            a_frames,
            b_frames,
            parameters.input_taper,
        );
        let local = convolver.convolve(&a, &b)?;
        let mix = evolving_mix(parameters, center);
        if mix > 0.0 {
            let mut a_carrier =
                positioned_crop(&local, a_frames, parameters.evolving_crop_position);
            gain_a = Some(level_local(&mut a_carrier, gain_a, 0.078, level_smoothing));
            overlap.add_crossfade(center as isize, &a_carrier, &a_power, mix);
        }
        if mix < 1.0 {
            let mut b_carrier =
                positioned_crop(&local, b_frames, parameters.evolving_crop_position);
            gain_b = Some(level_local(&mut b_carrier, gain_b, 0.078, level_smoothing));
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
    let full_power =
        convolution_power_profile(&mut convolver, a_frames, b_frames, parameters.input_taper)?;
    let block_power = normalized_power_profile(positioned_crop(
        &full_power,
        block_frames,
        parameters.chunk_crop_position,
    ));
    let mut overlap = OverlapBuffer::for_placements(&placements);
    let mut previous_gain = None;
    let level_smoothing = gain_smoothing_for_hop(slot_frames);
    for center in render_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let (a, b) = extract_pair(
            clip_a,
            clip_b,
            center,
            a_frames,
            b_frames,
            parameters.input_taper,
        );
        let local = convolver.convolve(&a, &b)?;
        let mut block = positioned_crop(
            &local,
            block_frames.min(local.len()),
            parameters.chunk_crop_position,
        );
        previous_gain = Some(level_local(
            &mut block,
            previous_gain,
            0.085,
            level_smoothing,
        ));
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
    centers_for_length(INPUT_FRAMES, hop_frames)
}

fn centers_for_length(frames: usize, hop_frames: usize) -> Vec<usize> {
    if frames <= 1 {
        return vec![0];
    }
    let intervals = (frames - 1).div_ceil(hop_frames).max(1);
    let mut result = Vec::with_capacity(intervals + 1);
    for index in 0..=intervals {
        // Integer rounding distributes the sub-frame remainder over every hop
        // instead of appending one abnormally close final center.
        result.push((index * (frames - 1) + intervals / 2) / intervals);
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

fn normalized_source_position(
    output_center: usize,
    output_frames: usize,
    source_frames: usize,
) -> usize {
    if output_frames <= 1 || source_frames <= 1 {
        return 0;
    }
    let phase = output_center as f64 / (output_frames - 1) as f64;
    (phase * (source_frames - 1) as f64).round() as usize
}

fn extract_pair(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    output_center: usize,
    a_frames: usize,
    b_frames: usize,
    input_taper: f32,
) -> (Vec<f32>, Vec<f32>) {
    extract_pair_samples(
        &clip_a.samples,
        &clip_b.samples,
        output_center,
        a_frames,
        b_frames,
        input_taper,
    )
}

fn extract_pair_samples(
    clip_a: &[f32],
    clip_b: &[f32],
    output_center: usize,
    a_frames: usize,
    b_frames: usize,
    input_taper: f32,
) -> (Vec<f32>, Vec<f32>) {
    let output_frames = clip_a.len().max(clip_b.len());
    let a_center = normalized_source_position(output_center, output_frames, clip_a.len()) as isize;
    let b_center = normalized_source_position(output_center, output_frames, clip_b.len()) as isize;
    (
        extract_window(clip_a, a_center, a_frames, input_taper),
        extract_window(clip_b, b_center, b_frames, input_taper),
    )
}

fn evolving_mix(parameters: AlgorithmParameters, center: usize) -> f32 {
    let phase = center as f32 / (INPUT_FRAMES - 1) as f32;
    (parameters.evolving_a_mix + parameters.evolving_mix_motion * (phase - 0.5)).clamp(0.0, 1.0)
}

fn extract_window(source: &[f32], center: isize, frames: usize, input_taper: f32) -> Vec<f32> {
    let half = frames as isize / 2;
    let mut output = Vec::with_capacity(frames);
    for index in 0..frames {
        let source_index = reflect_index(center + index as isize - half, source.len());
        output.push(source[source_index] * tukey(index, frames, input_taper));
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
    input_taper: f32,
) -> Result<Vec<f32>> {
    let analysis_power = |frames| {
        (0..frames)
            .map(|index| tukey(index, frames, input_taper).powi(2))
            .collect::<Vec<_>>()
    };
    let profile = convolver.convolve(&analysis_power(a_frames), &analysis_power(b_frames))?;
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

fn gain_smoothing_for_hop(hop_frames: usize) -> f32 {
    1.0 - (-(hop_frames as f32) / (4.0 * SAMPLE_RATE as f32)).exp()
}

fn level_local(
    samples: &mut [f32],
    previous_gain: Option<f32>,
    target_rms: f32,
    smoothing: f32,
) -> f32 {
    let rms = (samples
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt() as f32;
    let desired = (target_rms / rms.max(1.0e-10)).min(128.0);
    let gain = if let Some(previous) = previous_gain {
        let log_gain = previous.max(1.0e-12).ln()
            + smoothing.clamp(0.0, 1.0) * (desired.max(1.0e-12).ln() - previous.max(1.0e-12).ln());
        log_gain.exp()
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
    left_time: Vec<f32>,
    right_time: Vec<f32>,
    output_time: Vec<f32>,
}

impl LocalConvolver {
    fn new(output_frames: usize) -> Self {
        let fft_len = output_frames.next_power_of_two();
        let mut planner = RealFftPlanner::<f32>::new();
        Self {
            fft_len,
            forward: planner.plan_fft_forward(fft_len),
            inverse: planner.plan_fft_inverse(fft_len),
            left_time: vec![0.0; fft_len],
            right_time: vec![0.0; fft_len],
            output_time: vec![0.0; fft_len],
        }
    }

    fn convolve(&mut self, left: &[f32], right: &[f32]) -> Result<Vec<f32>> {
        let output_frames = left.len() + right.len() - 1;
        if output_frames > self.fft_len {
            bail!("local convolution exceeds planned FFT");
        }
        self.left_time.fill(0.0);
        self.right_time.fill(0.0);
        self.left_time[..left.len()].copy_from_slice(left);
        self.right_time[..right.len()].copy_from_slice(right);
        let mut left_spectrum = self.forward.make_output_vec();
        let mut right_spectrum = self.forward.make_output_vec();
        self.forward
            .process(&mut self.left_time, &mut left_spectrum)?;
        self.forward
            .process(&mut self.right_time, &mut right_spectrum)?;
        for (left, right) in left_spectrum.iter_mut().zip(&right_spectrum) {
            *left *= *right;
        }
        self.inverse
            .process(&mut left_spectrum, &mut self.output_time)?;
        let scale = 1.0 / self.fft_len as f32;
        let output = self.output_time[..output_frames]
            .iter()
            .map(|sample| sample * scale)
            .collect();
        Ok(output)
    }
}

struct OverlapBuffer {
    samples: Vec<f32>,
    power_weights: Vec<f32>,
    amplitude_weights: Vec<f32>,
    coherence_sum: Vec<f32>,
    coherence_weights: Vec<f32>,
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
            amplitude_weights: vec![0.0; frames],
            coherence_sum: vec![0.0; frames],
            coherence_weights: vec![0.0; frames],
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
            let weight = synthesis_weight(index, local.len()) * mix;
            self.samples[output_index as usize] += sample * weight;
            self.power_weights[output_index as usize] += weight * weight * local_power;
            self.amplitude_weights[output_index as usize] += weight * local_power.sqrt();
        }
    }

    fn add_coherence_pair(
        &mut self,
        previous_center: isize,
        current_center: isize,
        power_profile: &[f32],
        coherence: f32,
    ) {
        let length = power_profile.len();
        let previous_start = previous_center - length as isize / 2;
        let current_start = current_center - length as isize / 2;
        let overlap_start = previous_start.max(current_start);
        let overlap_end = (previous_start + length as isize).min(current_start + length as isize);
        for frame in overlap_start..overlap_end {
            let previous_index = (frame - previous_start) as usize;
            let current_index = (frame - current_start) as usize;
            let previous_amplitude =
                synthesis_weight(previous_index, length) * power_profile[previous_index].sqrt();
            let current_amplitude =
                synthesis_weight(current_index, length) * power_profile[current_index].sqrt();
            let pair_weight = previous_amplitude * current_amplitude;
            let output_index = (frame - self.start_frame) as usize;
            self.coherence_sum[output_index] += coherence * pair_weight;
            self.coherence_weights[output_index] += pair_weight;
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
        let raw_coherence = self
            .coherence_sum
            .iter()
            .zip(&self.coherence_weights)
            .map(|(&sum, &weight)| {
                if weight > 1.0e-12 {
                    (sum / weight).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let coherence = smooth_coherence(raw_coherence);
        for (((sample, independent_power), amplitude), coherence) in self
            .samples
            .iter_mut()
            .zip(self.power_weights)
            .zip(self.amplitude_weights)
            .zip(coherence)
        {
            let denominator_squared =
                (1.0 - coherence) * independent_power + coherence * amplitude * amplitude;
            if denominator_squared > 1.0e-14 {
                *sample /= denominator_squared.sqrt();
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

fn synthesis_weight(index: usize, length: usize) -> f32 {
    tukey(index, length, 1.0).sqrt().max(1.0e-5)
}

fn aligned_positive_coherence(
    previous: &[f32],
    previous_center: isize,
    current: &[f32],
    current_center: isize,
) -> f32 {
    let previous_start = previous_center - previous.len() as isize / 2;
    let current_start = current_center - current.len() as isize / 2;
    let overlap_start = previous_start.max(current_start);
    let overlap_end =
        (previous_start + previous.len() as isize).min(current_start + current.len() as isize);
    if overlap_end <= overlap_start {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut previous_power = 0.0_f64;
    let mut current_power = 0.0_f64;
    for frame in overlap_start..overlap_end {
        let previous_sample = previous[(frame - previous_start) as usize] as f64;
        let current_sample = current[(frame - current_start) as usize] as f64;
        dot += previous_sample * current_sample;
        previous_power += previous_sample * previous_sample;
        current_power += current_sample * current_sample;
    }
    let denominator = (previous_power * current_power).sqrt();
    if denominator <= 1.0e-20 {
        0.0
    } else {
        (dot / denominator).clamp(0.0, 1.0) as f32
    }
}

fn smooth_coherence(mut values: Vec<f32>) -> Vec<f32> {
    if values.len() < 3 {
        return values;
    }
    let smoothing_frames = (SAMPLE_RATE as usize / 50).min(values.len() / 8).max(1);
    let coefficient = (-1.0 / smoothing_frames as f32).exp();
    let mut state = values[0];
    for value in &mut values {
        state = coefficient * state + (1.0 - coefficient) * *value;
        *value = state;
    }
    state = *values.last().unwrap();
    for value in values.iter_mut().rev() {
        state = coefficient * state + (1.0 - coefficient) * *value;
        *value = state;
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflection_stays_in_bounds() {
        for index in -100..200 {
            assert!(reflect_index(index, 64) < 64);
        }
    }

    #[test]
    fn input_taper_controls_only_the_extracted_analysis_edges() {
        let source = vec![1.0; 64];
        let nearly_rectangular = extract_window(&source, 32, 32, 0.05);
        let full_hann = extract_window(&source, 32, 32, 1.0);
        assert_eq!(nearly_rectangular[0], 0.0);
        assert_eq!(full_hann[0], 0.0);
        assert!((nearly_rectangular[8] - 1.0).abs() < 1.0e-6);
        assert!(full_hann[8] < 0.6);
        assert!((nearly_rectangular[16] - 1.0).abs() < 1.0e-6);
        assert!((full_hann[16] - 1.0).abs() < 0.01);
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
        assert!((WindowConfig::new(1.0, 2.0, 50.0).unwrap().hop_seconds - 0.5).abs() < 1.0e-6);
        assert_eq!(WindowConfig::for_chunks(1.0, 2.0).unwrap().hop_seconds, 2.0);
    }

    #[test]
    fn centers_evenly_span_non_divisible_timelines() {
        for (frames, requested_hop) in [(10_003, 512), (24_000, 2_047), (INPUT_FRAMES, 2_400)] {
            let centers = centers_for_length(frames, requested_hop);
            assert_eq!(centers[0], 0);
            assert_eq!(*centers.last().unwrap(), frames - 1);
            let gaps = centers
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            assert!(
                gaps.iter().max().unwrap() - gaps.iter().min().unwrap() <= 1,
                "uneven final hop for {frames} frames at requested hop {requested_hop}"
            );
        }
    }

    #[test]
    fn method_parameters_are_validated_by_method() {
        let mut parameters = AlgorithmParameters {
            chunk_crossfade_percent: 90.0,
            ..AlgorithmParameters::default()
        };
        assert!(parameters.validate(Algorithm::ChunkCrossfade).is_err());
        assert!(parameters.validate(Algorithm::WindowedConvolution).is_ok());

        parameters = AlgorithmParameters::default();
        parameters.full_a_offset_seconds = 30.0;
        parameters.full_a_duration_seconds = 31.1;
        assert!(parameters.validate(Algorithm::FullConvolution).is_err());

        parameters = AlgorithmParameters::default();
        parameters.window_overlap_percent = 90.0;
        assert!(parameters.validate(Algorithm::WindowedConvolution).is_err());

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
    fn crossfade_normalization_keeps_expected_power_for_varied_windows() {
        for (a_frames, b_frames, hop) in [(1_024, 1_024, 512), (512, 4_096, 256), (96, 4_800, 48)] {
            assert_expected_power(a_frames, b_frames, hop);
        }
    }

    #[test]
    fn coherence_estimate_covers_correlated_degenerate_cases() {
        let length = 4_096;
        let hop = 256_isize;
        let previous_center = 0_isize;
        let current_center = hop;
        let previous_start = previous_center - length as isize / 2;
        let current_start = current_center - length as isize / 2;
        let tone = |start: isize, phase: f32| {
            (0..length)
                .map(|index| (2.0 * PI * (start + index as isize) as f32 / 64.0 + phase).sin())
                .collect::<Vec<_>>()
        };
        let constant = vec![1.0; length];
        let opposite = vec![-1.0; length];
        let tone_0 = tone(previous_start, 0.0);
        let tone_90 = tone(current_start, PI / 2.0);
        let tone_60 = tone(current_start, PI / 3.0);
        assert!(
            aligned_positive_coherence(&constant, previous_center, &constant, current_center)
                > 0.999
        );
        assert_eq!(
            aligned_positive_coherence(&constant, previous_center, &opposite, current_center),
            0.0
        );
        assert!(
            aligned_positive_coherence(&tone_0, previous_center, &tone_90, current_center) < 0.01
        );
        let sixty = aligned_positive_coherence(&tone_0, previous_center, &tone_60, current_center);
        assert!(
            (sixty - 0.5).abs() < 0.01,
            "60-degree coherence was {sixty}"
        );
    }

    #[test]
    fn coherence_aware_crossfade_removes_identical_grain_swelling() {
        let length = 4_097;
        let hop = 2_048;
        let previous_center = 0_isize;
        let current_center = hop as isize;
        let profile = vec![1.0; length];
        let placements = [(previous_center, length), (current_center, length)];
        let constant = vec![1.0; length];
        let mut overlap = OverlapBuffer::for_placements(&placements);
        overlap.add_crossfade(previous_center, &constant, &profile, 1.0);
        overlap.add_crossfade(current_center, &constant, &profile, 1.0);
        overlap.add_coherence_pair(previous_center, current_center, &profile, 1.0);
        let rendered = overlap.finish();
        let overlap_start = (current_center - length as isize / 2 - rendered.start_frame) as usize;
        let overlap_end = (previous_center + length as isize / 2 - rendered.start_frame) as usize;
        let steady = &rendered.samples[overlap_start + 256..overlap_end - 256];
        let minimum = steady.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = steady.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let ripple_db = 20.0 * (maximum / minimum).log10();
        assert!(
            ripple_db < 0.20,
            "identical-grain swelling was {ripple_db:.3} dB"
        );
    }

    #[test]
    fn opposite_grain_cancellation_is_gradual_without_gain_explosion() {
        let length = 4_097;
        let hop = 2_048;
        let profile = vec![1.0; length];
        let placements = [(0_isize, length), (hop as isize, length)];
        let positive = vec![1.0; length];
        let negative = vec![-1.0; length];
        let mut overlap = OverlapBuffer::for_placements(&placements);
        overlap.add_crossfade(0, &positive, &profile, 1.0);
        overlap.add_crossfade(hop as isize, &negative, &profile, 1.0);
        overlap.add_coherence_pair(0, hop as isize, &profile, 0.0);
        let rendered = overlap.finish();
        let differences = rendered
            .samples
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .collect::<Vec<_>>();
        let peak = rendered
            .samples
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0, f32::max);
        assert!(rendered.samples.iter().all(|sample| sample.is_finite()));
        assert!(peak <= 1.001, "opposite grains were amplified to {peak}");
        assert!(
            differences.iter().copied().fold(0.0, f32::max) < 0.01,
            "opposite-grain cancellation contained a click"
        );
        assert!(
            rendered
                .samples
                .windows(2)
                .any(|pair| pair[0] > 0.0 && pair[1] <= 0.0),
            "opposite grains did not cross smoothly through zero"
        );
    }

    #[test]
    fn real_windowed_convolutions_have_finite_even_power_for_varied_synthetic_inputs() {
        let frames = 48_000;
        let cases = [
            (
                synthetic_noise(frames, 0x1234_5678),
                synthetic_noise(frames, 0x8765_4321),
                768,
                1_280,
                384,
                "independent noise",
            ),
            (
                synthetic_tone_noise(frames, 173.0, 0x3141_5926),
                synthetic_tone_noise(frames, 619.0, 0x2718_2818),
                1_024,
                1_024,
                512,
                "tones plus noise",
            ),
            (
                synthetic_impulses(frames, 431),
                synthetic_noise(frames, 0x0bad_cafe),
                384,
                2_048,
                192,
                "impulses and noise",
            ),
        ];
        for (a, b, a_window, b_window, hop, label) in cases {
            let rendered = render_windowed_samples(
                &a,
                &b,
                a_window,
                b_window,
                hop,
                DEFAULT_INPUT_TAPER,
                &|| false,
            )
            .unwrap();
            assert_eq!(
                rendered.samples.len(),
                2 * frames + a_window + b_window - 3,
                "{label} duration"
            );
            assert!(
                rendered.samples.iter().all(|sample| sample.is_finite()),
                "{label} produced non-finite samples"
            );
            let interior_start = a_window + b_window;
            let interior_end = rendered.samples.len() - interior_start;
            let power_blocks = block_rms(&rendered.samples[interior_start..interior_end], hop * 16);
            let mean = power_blocks.iter().sum::<f32>() / power_blocks.len() as f32;
            let spread_db = amplitude_spread_db(&power_blocks);
            let phase_ripple_db =
                hop_phase_power_ripple_db(&rendered.samples[interior_start..interior_end], hop * 2);
            let seam_ratio = seam_difference_ratio(&rendered, frames, hop);
            eprintln!(
                "{label}: mean RMS {mean:.5}, block ripple {spread_db:.3} dB, \
                 hop-phase ripple {phase_ripple_db:.3} dB, seam ratio {seam_ratio:.3}"
            );
            assert!(
                (0.045..0.20).contains(&mean),
                "{label} mean RMS was {mean:.5}"
            );
            assert!(
                spread_db < 4.0,
                "{label} block-power ripple was {spread_db:.3} dB"
            );
            assert!(
                phase_ripple_db < 1.0,
                "{label} hop-phase ripple was {phase_ripple_db:.3} dB"
            );
            assert!(
                seam_ratio < 2.0,
                "{label} hop-boundary derivative ratio was {seam_ratio:.3}"
            );
        }
    }

    #[test]
    fn silence_and_near_silence_remain_finite_without_normalization_blowup() {
        for amplitude in [0.0, 1.0e-9] {
            let a = vec![amplitude; 4_096];
            let b = synthetic_noise(4_096, 0xfeed_face)
                .into_iter()
                .map(|sample| sample * amplitude)
                .collect::<Vec<_>>();
            let rendered =
                render_windowed_samples(&a, &b, 256, 512, 128, DEFAULT_INPUT_TAPER, &|| false)
                    .unwrap();
            assert!(rendered.samples.iter().all(|sample| sample.is_finite()));
            let peak = rendered
                .samples
                .iter()
                .copied()
                .map(f32::abs)
                .fold(0.0, f32::max);
            assert!(peak < 1.0e-8, "near-silence peak grew to {peak}");
        }
    }

    #[test]
    fn sparse_convolutions_do_not_add_hop_locked_boundary_clicks() {
        let frames = 32_768;
        let cases = [
            (
                synthetic_tone_noise(frames, 233.0, 0x4455_6677),
                synthetic_impulses(frames, 503),
                "stationary tone and impulses",
            ),
            (
                synthetic_impulses(frames, 431),
                synthetic_impulses(frames, 677),
                "impulses and impulses",
            ),
        ];
        for (a, b, label) in cases {
            let hop = 256;
            let rendered =
                render_windowed_samples(&a, &b, 512, 1_024, hop, DEFAULT_INPUT_TAPER, &|| false)
                    .unwrap();
            let seam_ratio = seam_difference_ratio(&rendered, frames, hop);
            assert!(rendered.samples.iter().all(|sample| sample.is_finite()));
            assert!(
                seam_ratio < 2.0,
                "{label} hop-boundary derivative ratio was {seam_ratio:.3}"
            );
        }
    }

    #[test]
    fn slow_input_ramp_stays_smooth_instead_of_becoming_gain_steps() {
        let frames = 32_768;
        let mut a = synthetic_noise(frames, 0x1234_abcd);
        for (index, sample) in a.iter_mut().enumerate() {
            *sample *= 0.2 + 0.8 * index as f32 / (frames - 1) as f32;
        }
        let b = synthetic_noise(frames, 0xabcd_1234);
        let rendered =
            render_windowed_samples(&a, &b, 512, 1_024, 256, DEFAULT_INPUT_TAPER, &|| false)
                .unwrap();
        let trim = 2_048;
        let levels = block_rms(
            &rendered.samples[trim..rendered.samples.len() - trim],
            1_024,
        );
        let quarter = levels.len() / 4;
        let beginning = levels[..quarter].iter().sum::<f32>() / quarter as f32;
        let ending = levels[levels.len() - quarter..].iter().sum::<f32>() / quarter as f32;
        let largest_step_db = levels
            .windows(2)
            .map(|pair| 20.0 * (pair[1] / pair[0].max(1.0e-12)).log10().abs())
            .fold(0.0, f32::max);
        assert!(
            ending > beginning * 1.35,
            "ramp dynamics were flattened: {beginning:.5} to {ending:.5}"
        );
        assert!(
            largest_step_db < 2.5,
            "ramp contained a {largest_step_db:.3} dB staircase step"
        );
    }

    #[test]
    fn complete_output_fades_are_monotonic() {
        let mut samples = vec![1.0; 2_001];
        apply_half_hann_edge_fade(&mut samples, 500);
        assert_eq!(samples[0], 0.0);
        assert_eq!(*samples.last().unwrap(), 0.0);
        assert!(samples[..500].windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(
            samples[samples.len() - 500..]
                .windows(2)
                .all(|pair| pair[1] <= pair[0])
        );
    }

    #[test]
    fn minimum_window_default_is_one_single_band_scan() {
        let config = WindowConfig::new(
            0.1,
            5.0,
            AlgorithmParameters::default().window_overlap_percent,
        )
        .unwrap();
        let centers = centers(seconds_to_frames(config.hop_seconds));
        assert_eq!(centers.len(), 2_441);
        assert_eq!(seconds_to_frames(config.hop_seconds), 1_200);
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

    fn assert_expected_power(a_frames: usize, b_frames: usize, hop: usize) {
        let local_frames = a_frames + b_frames - 1;
        let placements = (0..640)
            .map(|index| ((index * hop) as isize, local_frames))
            .collect::<Vec<_>>();
        let mut convolver = LocalConvolver::new(local_frames);
        let profile =
            convolution_power_profile(&mut convolver, a_frames, b_frames, DEFAULT_INPUT_TAPER)
                .unwrap();
        let mut overlap = OverlapBuffer::for_placements(&placements);
        let mut state = (a_frames as u64) << 32 | b_frames as u64;
        for &(center, _) in &placements {
            let grain = profile
                .iter()
                .map(|power| random_unit_variance(&mut state) * power.sqrt())
                .collect::<Vec<_>>();
            overlap.add_crossfade(center, &grain, &profile, 1.0);
        }
        let rendered = overlap.finish();
        let start = (local_frames + 8 * hop) as isize - rendered.start_frame;
        let end = (placements.last().unwrap().0 - local_frames as isize - 8 * hop as isize)
            - rendered.start_frame;
        let blocks = block_rms(
            &rendered.samples[start as usize..end as usize],
            hop.max(256) * 8,
        );
        let mean = blocks.iter().sum::<f32>() / blocks.len() as f32;
        let ripple_db = amplitude_spread_db(&blocks);
        eprintln!("{a_frames}x{b_frames} expected power: RMS {mean:.4}, ripple {ripple_db:.3} dB");
        assert!(
            (0.94..1.06).contains(&mean),
            "{a_frames}x{b_frames} expected RMS was {mean:.4}"
        );
        assert!(
            ripple_db < 1.0,
            "{a_frames}x{b_frames} expected-power ripple was {ripple_db:.3} dB"
        );
    }

    fn block_rms(samples: &[f32], block_frames: usize) -> Vec<f32> {
        samples
            .chunks_exact(block_frames)
            .map(|block| {
                (block
                    .iter()
                    .map(|sample| f64::from(*sample) * f64::from(*sample))
                    .sum::<f64>()
                    / block.len() as f64)
                    .sqrt() as f32
            })
            .collect()
    }

    fn amplitude_spread_db(values: &[f32]) -> f32 {
        let minimum = values.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        20.0 * (maximum / minimum.max(1.0e-12)).log10()
    }

    fn hop_phase_power_ripple_db(samples: &[f32], hop: usize) -> f32 {
        let bins = 32_usize.min(hop);
        let mut power = vec![0.0_f64; bins];
        let mut counts = vec![0_usize; bins];
        let analysis_frames = (SAMPLE_RATE as usize / 25).min(samples.len() / 4).max(32);
        let mut prefix = Vec::with_capacity(samples.len() + 1);
        prefix.push(0.0_f64);
        for sample in samples {
            prefix.push(prefix.last().copied().unwrap() + f64::from(*sample) * f64::from(*sample));
        }
        for index in analysis_frames..samples.len() - analysis_frames {
            let start = index - analysis_frames / 2;
            let end = start + analysis_frames;
            let local_power = (prefix[end] - prefix[start]) / analysis_frames as f64;
            let bin = ((index % hop) * bins / hop).min(bins - 1);
            power[bin] += local_power;
            counts[bin] += 1;
        }
        let levels = power
            .iter()
            .zip(counts)
            .map(|(&sum, count)| 10.0 * (sum / count.max(1) as f64).max(1.0e-20).log10())
            .collect::<Vec<_>>();
        (levels.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - levels.iter().copied().fold(f64::INFINITY, f64::min)) as f32
    }

    fn seam_difference_ratio(rendered: &TimelineRender, source_frames: usize, hop: usize) -> f32 {
        let differences = rendered
            .samples
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .collect::<Vec<_>>();
        let ordinary_rms = (differences
            .iter()
            .map(|difference| f64::from(*difference) * f64::from(*difference))
            .sum::<f64>()
            / differences.len() as f64)
            .sqrt() as f32;
        let centers = centers_for_length(source_frames, hop)
            .into_iter()
            .map(convolution_center)
            .collect::<Vec<_>>();
        let seam_rms = (centers
            .iter()
            .filter_map(|center| {
                let index = (*center - rendered.start_frame)
                    .clamp(0, differences.len() as isize - 1) as usize;
                differences.get(index).copied()
            })
            .map(|difference| f64::from(difference) * f64::from(difference))
            .sum::<f64>()
            / centers.len().max(1) as f64)
            .sqrt() as f32;
        seam_rms / ordinary_rms.max(1.0e-12)
    }

    fn random_unit_variance(state: &mut u64) -> f32 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        let uniform = ((*state >> 40) as f32 / (1_u32 << 24) as f32) * 2.0 - 1.0;
        uniform * 3.0_f32.sqrt()
    }

    fn synthetic_noise(frames: usize, seed: u64) -> Vec<f32> {
        let mut state = seed;
        (0..frames)
            .map(|_| random_unit_variance(&mut state) * 0.1)
            .collect()
    }

    fn synthetic_tone_noise(frames: usize, frequency: f32, seed: u64) -> Vec<f32> {
        let mut state = seed;
        (0..frames)
            .map(|frame| {
                0.08 * (2.0 * PI * frequency * frame as f32 / SAMPLE_RATE as f32).sin()
                    + 0.035 * random_unit_variance(&mut state)
            })
            .collect()
    }

    fn synthetic_impulses(frames: usize, interval: usize) -> Vec<f32> {
        (0..frames)
            .map(|frame| {
                if frame % interval == 0 {
                    0.8
                } else {
                    0.003 * (2.0 * PI * 997.0 * frame as f32 / SAMPLE_RATE as f32).sin()
                }
            })
            .collect()
    }
}
