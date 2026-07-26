use std::f32::consts::PI;
use std::str::FromStr;

use anyhow::{Result, bail};
use rayon::join;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::audio::{AudioClip, INPUT_FRAMES, INPUT_SECONDS, SAMPLE_RATE};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    WindowedConvolution,
    SourceFilterVocoder,
    ChunkCrossfade,
    FullConvolution,
    DryA,
    DryB,
}

impl Algorithm {
    pub const ALL: [Self; 6] = [
        Self::WindowedConvolution,
        Self::SourceFilterVocoder,
        Self::ChunkCrossfade,
        Self::FullConvolution,
        Self::DryA,
        Self::DryB,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::WindowedConvolution => "windowed_convolution",
            Self::SourceFilterVocoder => "source_filter_vocoder",
            Self::ChunkCrossfade => "chunk_crossfade",
            Self::FullConvolution => "full_convolution",
            Self::DryA => "dry_a",
            Self::DryB => "dry_b",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::WindowedConvolution => "Windowed convolution",
            Self::SourceFilterVocoder => "Source-filter vocoder",
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
            Self::SourceFilterVocoder => {
                "Uses clip A as the excitation and temporal source while clip B supplies a smooth \
                 short-time spectral envelope. A's phase, amplitude motion, and protected transients \
                 remain in place as B's time-varying broad-band color is transferred by a dense \
                 overlap-add source-filter vocoder."
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
            Self::SourceFilterVocoder => 2,
            Self::ChunkCrossfade => 3,
            Self::FullConvolution => 4,
            Self::DryA => 5,
            Self::DryB => 6,
        }
    }

    pub fn uses_windows(self) -> bool {
        matches!(self, Self::WindowedConvolution | Self::ChunkCrossfade)
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
const VOCODER_FRAME_FRAMES: usize = 1_024;
const VOCODER_HOP_FRAMES: usize = VOCODER_FRAME_FRAMES / 8;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AlgorithmParameters {
    pub input_taper: f32,
    pub vocoder_transfer: f32,
    pub vocoder_envelope_width_hz: f32,
    pub vocoder_transient_protection: f32,
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
            vocoder_transfer: 0.85,
            vocoder_envelope_width_hz: 900.0,
            vocoder_transient_protection: 0.65,
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
            Algorithm::SourceFilterVocoder => {
                validate_range("envelope transfer", self.vocoder_transfer, 0.0, 1.5)?;
                validate_range(
                    "envelope width",
                    self.vocoder_envelope_width_hz,
                    100.0,
                    3_000.0,
                )?;
                validate_range(
                    "transient protection",
                    self.vocoder_transient_protection,
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
        Algorithm::SourceFilterVocoder => {
            render_source_filter_vocoder(clip_a, clip_b, parameters, cancelled)?
        }
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
    let a_taper = tukey_window(a_frames, input_taper);
    let b_taper = tukey_window(b_frames, input_taper);
    let power_profile = convolution_power_profile_from_tapers(&mut convolver, &a_taper, &b_taper)?;
    let synthesis_profile = (0..local_frames)
        .map(|index| synthesis_weight(index, local_frames))
        .collect::<Vec<_>>();
    let power_amplitude = synthesis_profile
        .iter()
        .zip(&power_profile)
        .map(|(&weight, &power)| weight * power.sqrt())
        .collect::<Vec<_>>();
    let mut overlap = OverlapBuffer::for_placements(&placements);
    let mut previous_gain = None;
    let mut previous_local: Option<(isize, Vec<f32>)> = None;
    let level_smoothing = gain_smoothing_for_hop(hop_frames);
    for source_center in source_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let output_center = convolution_center(source_center);
        let (a, b) =
            extract_pair_samples_with_tapers(clip_a, clip_b, source_center, &a_taper, &b_taper);
        let mut local = convolver.convolve(&a, &b)?;
        previous_gain = Some(level_local(
            &mut local,
            previous_gain,
            0.085,
            level_smoothing,
        ));
        overlap.add_crossfade_precomputed(
            output_center,
            &local,
            &synthesis_profile,
            &power_amplitude,
            1.0,
        );
        if let Some((previous_center, previous)) = previous_local.take() {
            let coherence =
                aligned_positive_coherence(&previous, previous_center, &local, output_center);
            overlap.add_coherence_pair_precomputed(
                previous_center,
                output_center,
                &power_amplitude,
                coherence,
            );
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

fn render_source_filter_vocoder(
    clip_a: &AudioClip,
    clip_b: &AudioClip,
    parameters: AlgorithmParameters,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>> {
    let window = (0..VOCODER_FRAME_FRAMES)
        .map(|index| {
            let phase = (index as f32 + 0.5) / VOCODER_FRAME_FRAMES as f32;
            (0.5 - 0.5 * (2.0 * PI * phase).cos()).sqrt()
        })
        .collect::<Vec<_>>();
    let envelope_radius = ((parameters.vocoder_envelope_width_hz * VOCODER_FRAME_FRAMES as f32
        / SAMPLE_RATE as f32)
        .round() as usize)
        .max(1);
    let mut vocoder = SourceFilterVocoder::new();
    let mut output = vec![0.0_f32; clip_a.samples.len()];
    let mut overlap_weight = vec![0.0_f32; output.len()];
    let first_start = -(VOCODER_FRAME_FRAMES as isize - VOCODER_HOP_FRAMES as isize);
    let mut frame_start = first_start;
    let mut frame_index = 0_usize;
    while frame_start < output.len() as isize {
        if frame_index.is_multiple_of(16) && cancelled() {
            bail!("render cancelled");
        }
        vocoder.process_frame(
            &clip_a.samples,
            &clip_b.samples,
            frame_start,
            &window,
            envelope_radius,
            parameters,
            &mut output,
            &mut overlap_weight,
        )?;
        frame_start += VOCODER_HOP_FRAMES as isize;
        frame_index += 1;
    }
    for (sample, weight) in output.iter_mut().zip(overlap_weight) {
        if weight > 1.0e-8 {
            *sample /= weight;
        } else {
            *sample = 0.0;
        }
    }
    Ok(output)
}

struct SourceFilterVocoder {
    forward: std::sync::Arc<dyn RealToComplex<f32>>,
    inverse: std::sync::Arc<dyn ComplexToReal<f32>>,
    a_time: Vec<f32>,
    b_time: Vec<f32>,
    output_time: Vec<f32>,
    a_spectrum: Vec<realfft::num_complex::Complex32>,
    b_spectrum: Vec<realfft::num_complex::Complex32>,
    a_log_magnitude: Vec<f32>,
    b_log_magnitude: Vec<f32>,
    a_envelope: Vec<f32>,
    b_envelope: Vec<f32>,
    previous_a_magnitude: Vec<f32>,
    previous_log_ratio: Vec<f32>,
    prefix: Vec<f64>,
    transient_state: f32,
    transient_hold_frames: usize,
    initialized: bool,
}

impl SourceFilterVocoder {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(VOCODER_FRAME_FRAMES);
        let inverse = planner.plan_fft_inverse(VOCODER_FRAME_FRAMES);
        let bins = VOCODER_FRAME_FRAMES / 2 + 1;
        Self {
            a_time: forward.make_input_vec(),
            b_time: forward.make_input_vec(),
            output_time: inverse.make_output_vec(),
            a_spectrum: forward.make_output_vec(),
            b_spectrum: forward.make_output_vec(),
            a_log_magnitude: vec![0.0; bins],
            b_log_magnitude: vec![0.0; bins],
            a_envelope: vec![0.0; bins],
            b_envelope: vec![0.0; bins],
            previous_a_magnitude: vec![0.0; bins],
            previous_log_ratio: vec![0.0; bins],
            prefix: vec![0.0; bins + 1],
            transient_state: 0.0,
            transient_hold_frames: 0,
            forward,
            inverse,
            initialized: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_frame(
        &mut self,
        source_a: &[f32],
        source_b: &[f32],
        frame_start: isize,
        window: &[f32],
        envelope_radius: usize,
        parameters: AlgorithmParameters,
        output: &mut [f32],
        overlap_weight: &mut [f32],
    ) -> Result<()> {
        for (index, &window_sample) in window.iter().enumerate() {
            let source_index = frame_start + index as isize;
            let (a, b) = if (0..source_a.len() as isize).contains(&source_index) {
                (
                    source_a[source_index as usize],
                    source_b[source_index as usize],
                )
            } else {
                (0.0, 0.0)
            };
            self.a_time[index] = a * window_sample;
            self.b_time[index] = b * window_sample;
        }
        self.forward
            .process(&mut self.a_time, &mut self.a_spectrum)?;
        self.forward
            .process(&mut self.b_time, &mut self.b_spectrum)?;

        let mut positive_flux = 0.0_f64;
        let mut a_power = 0.0_f64;
        for bin in 0..self.a_spectrum.len() {
            let a_magnitude = self.a_spectrum[bin].norm();
            let b_magnitude = self.b_spectrum[bin].norm();
            let increase = (a_magnitude - self.previous_a_magnitude[bin]).max(0.0);
            positive_flux += f64::from(increase) * f64::from(increase);
            a_power += f64::from(a_magnitude) * f64::from(a_magnitude);
            self.previous_a_magnitude[bin] = a_magnitude;
            self.a_log_magnitude[bin] = (a_magnitude + 1.0e-5).ln();
            self.b_log_magnitude[bin] = (b_magnitude + 1.0e-5).ln();
        }
        smooth_frequency(
            &self.a_log_magnitude,
            &mut self.a_envelope,
            envelope_radius,
            &mut self.prefix,
        );
        smooth_frequency(
            &self.b_log_magnitude,
            &mut self.b_envelope,
            envelope_radius,
            &mut self.prefix,
        );

        let detected_transient = if a_power > 1.0e-16 {
            (positive_flux / a_power).sqrt().min(1.0) as f32
        } else {
            0.0
        };
        // A single onset contributes to every overlapping analysis frame. Holding
        // the peak flux for one complete frame prevents later frames in that same
        // onset from undoing transient protection during overlap-add.
        if detected_transient > self.transient_state {
            self.transient_state = detected_transient;
            self.transient_hold_frames = VOCODER_FRAME_FRAMES
                .div_ceil(VOCODER_HOP_FRAMES)
                .saturating_sub(1);
        } else if self.transient_hold_frames > 0 {
            self.transient_hold_frames -= 1;
        } else {
            let transient_release =
                (-(VOCODER_HOP_FRAMES as f32) / (0.020 * SAMPLE_RATE as f32)).exp();
            self.transient_state *= transient_release;
        }
        let transient = detected_transient.max(self.transient_state);
        let transient_transfer = parameters.vocoder_transfer
            * (1.0 - parameters.vocoder_transient_protection * transient);
        let attack = 1.0 - (-(VOCODER_HOP_FRAMES as f32) / (0.008 * SAMPLE_RATE as f32)).exp();
        let release = 1.0 - (-(VOCODER_HOP_FRAMES as f32) / (0.040 * SAMPLE_RATE as f32)).exp();
        let maximum_log_ratio = 8.0_f32.ln();
        for bin in 0..self.a_spectrum.len() {
            let target = (self.b_envelope[bin] - self.a_envelope[bin])
                .clamp(-maximum_log_ratio, maximum_log_ratio);
            if !self.initialized {
                self.previous_log_ratio[bin] = target;
            } else {
                let amount = if target > self.previous_log_ratio[bin] {
                    attack
                } else {
                    release
                };
                self.previous_log_ratio[bin] += amount * (target - self.previous_log_ratio[bin]);
            }
            let applied = (self.previous_log_ratio[bin] * transient_transfer)
                .clamp(-maximum_log_ratio, maximum_log_ratio);
            self.a_spectrum[bin] *= applied.exp();
        }
        self.initialized = true;
        self.inverse
            .process(&mut self.a_spectrum, &mut self.output_time)?;

        let inverse_scale = 1.0 / VOCODER_FRAME_FRAMES as f32;
        for (index, &window_sample) in window.iter().enumerate() {
            let output_index = frame_start + index as isize;
            if !(0..output.len() as isize).contains(&output_index) {
                continue;
            }
            let output_index = output_index as usize;
            let weight = window_sample * window_sample;
            output[output_index] += self.output_time[index] * inverse_scale * window_sample;
            overlap_weight[output_index] += weight;
        }
        Ok(())
    }
}

fn smooth_frequency(input: &[f32], output: &mut [f32], radius: usize, prefix: &mut [f64]) {
    prefix[0] = 0.0;
    for (index, &value) in input.iter().enumerate() {
        prefix[index + 1] = prefix[index] + f64::from(value);
    }
    for (index, value) in output.iter_mut().enumerate() {
        let start = index.saturating_sub(radius);
        let end = (index + radius + 1).min(input.len());
        *value = ((prefix[end] - prefix[start]) / (end - start) as f64) as f32;
    }
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
    let a_taper = tukey_window(a_frames, parameters.input_taper);
    let b_taper = tukey_window(b_frames, parameters.input_taper);
    let full_power = convolution_power_profile_from_tapers(&mut convolver, &a_taper, &b_taper)?;
    let block_power = normalized_power_profile(positioned_crop(
        &full_power,
        block_frames,
        parameters.chunk_crop_position,
    ));
    let equal_power_profile = (0..block_frames)
        .map(|index| {
            let edge = index.min(block_frames - 1 - index);
            if edge >= crossfade_frames {
                1.0
            } else {
                ((edge as f32 / crossfade_frames.max(1) as f32) * PI / 2.0)
                    .sin()
                    .max(1.0e-5)
            }
        })
        .collect::<Vec<_>>();
    let mut overlap = OverlapBuffer::for_placements(&placements);
    let mut previous_gain = None;
    let level_smoothing = gain_smoothing_for_hop(slot_frames);
    for center in render_centers {
        if cancelled() {
            bail!("render cancelled");
        }
        let (a, b) = extract_pair_samples_with_tapers(
            &clip_a.samples,
            &clip_b.samples,
            center,
            &a_taper,
            &b_taper,
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
        overlap.add_equal_power_precomputed(
            center as isize,
            &block,
            &block_power,
            &equal_power_profile,
        );
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

#[cfg(test)]
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

#[cfg(test)]
fn extract_window(source: &[f32], center: isize, frames: usize, input_taper: f32) -> Vec<f32> {
    let taper = tukey_window(frames, input_taper);
    extract_window_with_taper(source, center, &taper)
}

fn extract_window_with_taper(source: &[f32], center: isize, taper: &[f32]) -> Vec<f32> {
    let frames = taper.len();
    let half = frames as isize / 2;
    let mut output = Vec::with_capacity(frames);
    for (index, &weight) in taper.iter().enumerate() {
        let source_index = reflect_index(center + index as isize - half, source.len());
        output.push(source[source_index] * weight);
    }
    output
}

fn extract_pair_samples_with_tapers(
    clip_a: &[f32],
    clip_b: &[f32],
    output_center: usize,
    a_taper: &[f32],
    b_taper: &[f32],
) -> (Vec<f32>, Vec<f32>) {
    let output_frames = clip_a.len().max(clip_b.len());
    let a_center = normalized_source_position(output_center, output_frames, clip_a.len()) as isize;
    let b_center = normalized_source_position(output_center, output_frames, clip_b.len()) as isize;
    (
        extract_window_with_taper(clip_a, a_center, a_taper),
        extract_window_with_taper(clip_b, b_center, b_taper),
    )
}

fn tukey_window(frames: usize, alpha: f32) -> Vec<f32> {
    (0..frames)
        .map(|index| tukey(index, frames, alpha))
        .collect()
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

#[cfg(test)]
fn convolution_power_profile(
    convolver: &mut LocalConvolver,
    a_frames: usize,
    b_frames: usize,
    input_taper: f32,
) -> Result<Vec<f32>> {
    let a_taper = tukey_window(a_frames, input_taper);
    let b_taper = tukey_window(b_frames, input_taper);
    convolution_power_profile_from_tapers(convolver, &a_taper, &b_taper)
}

fn convolution_power_profile_from_tapers(
    convolver: &mut LocalConvolver,
    a_taper: &[f32],
    b_taper: &[f32],
) -> Result<Vec<f32>> {
    let analysis_power = |taper: &[f32]| {
        taper
            .iter()
            .map(|sample| sample * sample)
            .collect::<Vec<_>>()
    };
    let profile = convolver.convolve(&analysis_power(a_taper), &analysis_power(b_taper))?;
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
    left_spectrum: Vec<realfft::num_complex::Complex32>,
    right_spectrum: Vec<realfft::num_complex::Complex32>,
}

impl LocalConvolver {
    fn new(output_frames: usize) -> Self {
        let fft_len = output_frames.next_power_of_two();
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(fft_len);
        let inverse = planner.plan_fft_inverse(fft_len);
        let left_time = forward.make_input_vec();
        let right_time = forward.make_input_vec();
        let output_time = inverse.make_output_vec();
        let left_spectrum = forward.make_output_vec();
        let right_spectrum = forward.make_output_vec();
        Self {
            fft_len,
            forward,
            inverse,
            left_time,
            right_time,
            output_time,
            left_spectrum,
            right_spectrum,
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
        let forward = &self.forward;
        let (left_result, right_result) = join(
            || forward.process(&mut self.left_time, &mut self.left_spectrum),
            || forward.process(&mut self.right_time, &mut self.right_spectrum),
        );
        left_result?;
        right_result?;
        for (left, right) in self.left_spectrum.iter_mut().zip(&self.right_spectrum) {
            *left *= *right;
        }
        self.inverse
            .process(&mut self.left_spectrum, &mut self.output_time)?;
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

    #[cfg(test)]
    fn add_crossfade(&mut self, center: isize, local: &[f32], power_profile: &[f32], mix: f32) {
        assert_eq!(local.len(), power_profile.len());
        let synthesis_profile = (0..local.len())
            .map(|index| synthesis_weight(index, local.len()))
            .collect::<Vec<_>>();
        let power_amplitude = synthesis_profile
            .iter()
            .zip(power_profile)
            .map(|(&weight, &power)| weight * power.sqrt())
            .collect::<Vec<_>>();
        self.add_crossfade_precomputed(center, local, &synthesis_profile, &power_amplitude, mix);
    }

    fn add_crossfade_precomputed(
        &mut self,
        center: isize,
        local: &[f32],
        synthesis_profile: &[f32],
        power_amplitude: &[f32],
        mix: f32,
    ) {
        assert_eq!(local.len(), synthesis_profile.len());
        assert_eq!(local.len(), power_amplitude.len());
        let start = center - local.len() as isize / 2;
        for (index, &sample) in local.iter().enumerate() {
            let output_index = start + index as isize - self.start_frame;
            if !(0..self.samples.len() as isize).contains(&output_index) {
                continue;
            }
            let weight = synthesis_profile[index] * mix;
            let amplitude = power_amplitude[index] * mix;
            self.samples[output_index as usize] += sample * weight;
            self.power_weights[output_index as usize] += amplitude * amplitude;
            self.amplitude_weights[output_index as usize] += amplitude;
        }
    }

    #[cfg(test)]
    fn add_coherence_pair(
        &mut self,
        previous_center: isize,
        current_center: isize,
        power_profile: &[f32],
        coherence: f32,
    ) {
        let power_amplitude = (0..power_profile.len())
            .map(|index| synthesis_weight(index, power_profile.len()) * power_profile[index].sqrt())
            .collect::<Vec<_>>();
        self.add_coherence_pair_precomputed(
            previous_center,
            current_center,
            &power_amplitude,
            coherence,
        );
    }

    fn add_coherence_pair_precomputed(
        &mut self,
        previous_center: isize,
        current_center: isize,
        power_amplitude: &[f32],
        coherence: f32,
    ) {
        let length = power_amplitude.len();
        let previous_start = previous_center - length as isize / 2;
        let current_start = current_center - length as isize / 2;
        let overlap_start = previous_start.max(current_start);
        let overlap_end = (previous_start + length as isize).min(current_start + length as isize);
        for frame in overlap_start..overlap_end {
            let previous_index = (frame - previous_start) as usize;
            let current_index = (frame - current_start) as usize;
            let pair_weight = power_amplitude[previous_index] * power_amplitude[current_index];
            let output_index = (frame - self.start_frame) as usize;
            self.coherence_sum[output_index] += coherence * pair_weight;
            self.coherence_weights[output_index] += pair_weight;
        }
    }

    fn add_equal_power_precomputed(
        &mut self,
        center: isize,
        local: &[f32],
        power_profile: &[f32],
        weights: &[f32],
    ) {
        assert_eq!(local.len(), power_profile.len());
        assert_eq!(local.len(), weights.len());
        let start = center - local.len() as isize / 2;
        for (index, (&sample, &local_power)) in local.iter().zip(power_profile).enumerate() {
            let output_index = start + index as isize - self.start_frame;
            if !(0..self.samples.len() as isize).contains(&output_index) {
                continue;
            }
            let weight = weights[index];
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
    // Adjacent local convolutions are already band-limited and the estimate is
    // smoothed over 20 ms, so full-rate correlation adds cost without useful
    // temporal detail. A prime 31-frame stride retains a 1.55 kHz control
    // bandwidth without locking onto common power-of-two tone periods.
    for frame in (overlap_start..overlap_end).step_by(31) {
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
        parameters.vocoder_transfer = 1.6;
        assert!(parameters.validate(Algorithm::SourceFilterVocoder).is_err());
        parameters = AlgorithmParameters::default();
        parameters.vocoder_envelope_width_hz = 50.0;
        assert!(parameters.validate(Algorithm::SourceFilterVocoder).is_err());
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
    fn crop_position_covers_its_full_range() {
        let input = (0..10).map(|value| value as f32).collect::<Vec<_>>();
        assert_eq!(positioned_crop(&input, 4, 0.0), vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(positioned_crop(&input, 4, 0.5), vec![3.0, 4.0, 5.0, 6.0]);
        assert_eq!(positioned_crop(&input, 4, 1.0), vec![6.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn vocoder_zero_transfer_reconstructs_a_with_exact_duration_and_phase() {
        let frames = SAMPLE_RATE as usize;
        let a = (0..frames)
            .map(|index| {
                let time = index as f32 / SAMPLE_RATE as f32;
                0.35 * (2.0 * PI * 311.0 * time).sin() + 0.18 * (2.0 * PI * 2_017.0 * time).sin()
            })
            .collect::<Vec<_>>();
        let b = (0..frames)
            .map(|index| {
                let time = index as f32 / SAMPLE_RATE as f32;
                0.4 * (2.0 * PI * 733.0 * time).sin()
            })
            .collect::<Vec<_>>();
        let clip_a = AudioClip {
            id: "a".to_owned(),
            samples: a.clone(),
        };
        let clip_b = AudioClip {
            id: "b".to_owned(),
            samples: b,
        };
        let parameters = AlgorithmParameters {
            vocoder_transfer: 0.0,
            ..AlgorithmParameters::default()
        };
        let output = render_source_filter_vocoder(&clip_a, &clip_b, parameters, &|| false).unwrap();
        assert_eq!(output.len(), a.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
        let trim = VOCODER_FRAME_FRAMES;
        let error = output[trim..output.len() - trim]
            .iter()
            .zip(&a[trim..a.len() - trim])
            .map(|(&actual, &expected)| {
                let difference = f64::from(actual - expected);
                difference * difference
            })
            .sum::<f64>()
            / (output.len() - 2 * trim) as f64;
        assert!(error.sqrt() < 1.0e-5, "reconstruction RMS error {error}");
    }

    #[test]
    fn vocoder_transfers_b_envelope_while_retaining_a_phase() {
        let frames = SAMPLE_RATE as usize;
        let signal = |low_gain: f32, high_gain: f32| {
            (0..frames)
                .map(|index| {
                    let time = index as f32 / SAMPLE_RATE as f32;
                    low_gain * (2.0 * PI * 500.0 * time).sin()
                        + high_gain * (2.0 * PI * 6_000.0 * time).sin()
                })
                .collect::<Vec<_>>()
        };
        let a = signal(0.3, 0.3);
        let clip_a = AudioClip {
            id: "a".to_owned(),
            samples: a.clone(),
        };
        let clip_b = AudioClip {
            id: "b".to_owned(),
            samples: signal(0.55, 0.025),
        };
        let parameters = AlgorithmParameters {
            vocoder_transfer: 1.0,
            vocoder_envelope_width_hz: 100.0,
            vocoder_transient_protection: 0.0,
            ..AlgorithmParameters::default()
        };
        let output = render_source_filter_vocoder(&clip_a, &clip_b, parameters, &|| false).unwrap();
        let trim = VOCODER_FRAME_FRAMES;
        let input_ratio = tone_amplitude(&a[trim..frames - trim], 500.0)
            / tone_amplitude(&a[trim..frames - trim], 6_000.0);
        let output_ratio = tone_amplitude(&output[trim..frames - trim], 500.0)
            / tone_amplitude(&output[trim..frames - trim], 6_000.0).max(1.0e-9);
        assert!(
            output_ratio > input_ratio * 5.0,
            "B envelope was not transferred: input {input_ratio}, output {output_ratio}"
        );
        for frequency in [500.0, 6_000.0] {
            let input_phase = tone_phase(&a[trim..frames - trim], frequency);
            let output_phase = tone_phase(&output[trim..frames - trim], frequency);
            let difference = wrapped_phase_difference(input_phase, output_phase);
            assert!(
                difference < 0.01,
                "A phase changed by {difference} radians at {frequency} Hz"
            );
        }
    }

    #[test]
    fn vocoder_keeps_a_burst_on_a_timeline_and_honors_cancellation() {
        use std::cell::Cell;

        let frames = SAMPLE_RATE as usize;
        let mut a = vec![0.0; frames];
        for (index, sample) in a[12_000..15_000].iter_mut().enumerate() {
            *sample = 0.5 * (2.0 * PI * 900.0 * index as f32 / SAMPLE_RATE as f32).sin();
        }
        let b = (0..frames)
            .map(|index| 0.4 * (2.0 * PI * 1_800.0 * index as f32 / SAMPLE_RATE as f32).sin())
            .collect();
        let clip_a = AudioClip {
            id: "a".to_owned(),
            samples: a,
        };
        let clip_b = AudioClip {
            id: "b".to_owned(),
            samples: b,
        };
        let output =
            render_source_filter_vocoder(&clip_a, &clip_b, AlgorithmParameters::default(), &|| {
                false
            })
            .unwrap();
        let inside = output[11_000..16_000]
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>();
        let outside = output[..10_000]
            .iter()
            .chain(&output[17_000..])
            .map(|sample| sample * sample)
            .sum::<f32>();
        assert!(
            inside > outside * 20.0,
            "A timing smeared outside its burst"
        );

        let polls = Cell::new(0);
        let cancelled = || {
            polls.set(polls.get() + 1);
            polls.get() >= 2
        };
        assert!(
            render_source_filter_vocoder(
                &clip_a,
                &clip_b,
                AlgorithmParameters::default(),
                &cancelled,
            )
            .is_err()
        );
        assert_eq!(polls.get(), 2);
    }

    #[test]
    fn vocoder_identity_is_exact_across_control_extremes() {
        let frames = SAMPLE_RATE as usize / 2;
        let noise = deterministic_noise(frames, 0x1234_5678);
        for (transfer, width, protection) in
            [(0.0, 100.0, 0.0), (1.0, 900.0, 0.65), (1.5, 3_000.0, 1.0)]
        {
            let parameters = AlgorithmParameters {
                vocoder_transfer: transfer,
                vocoder_envelope_width_hz: width,
                vocoder_transient_protection: protection,
                ..AlgorithmParameters::default()
            };
            let identical = render_test_vocoder(&noise, &noise, parameters);
            let error = rms_difference(&identical, &noise);
            assert_eq!(identical.len(), noise.len());
            assert!(identical.iter().all(|sample| sample.is_finite()));
            assert!(
                error < 1.0e-6,
                "identical A/B changed at transfer={transfer}, width={width}, \
                 protection={protection}: RMS error {error}"
            );
        }

        // A time-domain impulse has a perfectly flat-magnitude spectrum, so
        // identical impulses are the exact constant-envelope identity case.
        let mut flat_spectrum = vec![0.0; frames];
        flat_spectrum[frames / 2] = 0.5;
        let output = render_test_vocoder(
            &flat_spectrum,
            &flat_spectrum,
            AlgorithmParameters {
                vocoder_transfer: 1.5,
                vocoder_envelope_width_hz: 100.0,
                vocoder_transient_protection: 0.0,
                ..AlgorithmParameters::default()
            },
        );
        assert!(rms_difference(&output, &flat_spectrum) < 1.0e-6);
    }

    #[test]
    fn vocoder_silence_and_extreme_level_mismatches_are_bounded() {
        let frames = SAMPLE_RATE as usize / 2;
        let noise = deterministic_noise(frames, 0x8765_4321);
        let silence = vec![0.0; frames];
        let near_silence = noise
            .iter()
            .map(|sample| sample * 1.0e-7)
            .collect::<Vec<_>>();
        let parameters = AlgorithmParameters {
            vocoder_transfer: 1.5,
            vocoder_envelope_width_hz: 100.0,
            vocoder_transient_protection: 0.0,
            ..AlgorithmParameters::default()
        };
        let silent_output = render_test_vocoder(&silence, &noise, parameters);
        assert!(silent_output.iter().all(|&sample| sample == 0.0));

        let attenuated = render_test_vocoder(&noise, &silence, parameters);
        let attenuation = rms(&attenuated) / rms(&noise);
        assert!(
            (attenuation - 0.125).abs() < 2.0e-3,
            "maximum attenuation escaped the deliberate 1/8 bound: {attenuation}"
        );

        let amplified = render_test_vocoder(&near_silence, &noise, parameters);
        let gain = rms(&amplified) / rms(&near_silence);
        assert!(
            (gain - 8.0).abs() < 0.02,
            "maximum gain escaped the deliberate 8x bound: {gain}"
        );
        assert!(amplified.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn vocoder_full_transient_protection_exactly_preserves_an_isolated_impulse() {
        let frames = SAMPLE_RATE as usize / 2;
        let impulse_index = frames / 2;
        let mut impulse = vec![0.0; frames];
        impulse[impulse_index] = 0.5;
        let spectrally_unlike_b = synthetic_tone(frames, 1_800.0, 0.4);
        let parameters = AlgorithmParameters {
            vocoder_transfer: 1.0,
            vocoder_envelope_width_hz: 100.0,
            vocoder_transient_protection: 1.0,
            ..AlgorithmParameters::default()
        };
        let output = render_test_vocoder(&impulse, &spectrally_unlike_b, parameters);
        let peak_index = output
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.abs().total_cmp(&right.1.abs()))
            .unwrap()
            .0;
        assert_eq!(peak_index, impulse_index);
        assert!(
            rms_difference(&output, &impulse) < 1.0e-6,
            "full protection recolored or smeared the impulse"
        );
    }

    #[test]
    fn vocoder_narrow_b_envelope_concentrates_broadband_a_without_inventing_timing() {
        let frames = SAMPLE_RATE as usize;
        let a = deterministic_noise(frames, 0x0ddc_0ffe);
        let mut b = vec![0.0; frames];
        let mut state = 0x5eed_u64;
        for frequency in (2_400..=3_600).step_by(100) {
            let phase = random_unit_variance(&mut state) * PI;
            for (index, sample) in b.iter_mut().enumerate() {
                *sample += 0.025
                    * (2.0 * PI * frequency as f32 * index as f32 / SAMPLE_RATE as f32 + phase)
                        .sin();
            }
        }
        let parameters = AlgorithmParameters {
            vocoder_transfer: 1.0,
            vocoder_envelope_width_hz: 100.0,
            vocoder_transient_protection: 0.0,
            ..AlgorithmParameters::default()
        };
        let output = render_test_vocoder(&a, &b, parameters);
        let input_ratio = spectral_band_power(&a, 2_200.0, 3_800.0)
            / spectral_band_power(&a, 6_000.0, 10_000.0).max(1.0e-20);
        let output_ratio = spectral_band_power(&output, 2_200.0, 3_800.0)
            / spectral_band_power(&output, 6_000.0, 10_000.0).max(1.0e-20);
        assert!(
            output_ratio > input_ratio * 20.0,
            "B's narrow envelope was not transferred: input ratio {input_ratio}, \
             output ratio {output_ratio}"
        );
        assert_eq!(output.len(), a.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn vocoder_cannot_and_does_not_invent_frequencies_absent_from_a() {
        let frames = SAMPLE_RATE as usize;
        let a_frequency = 997.0;
        let a = synthetic_tone(frames, a_frequency, 0.25);
        let b = deterministic_noise(frames, 0xc001_d00d);
        let parameters = AlgorithmParameters {
            vocoder_transfer: 1.5,
            vocoder_envelope_width_hz: 3_000.0,
            vocoder_transient_protection: 0.0,
            ..AlgorithmParameters::default()
        };
        let output = render_test_vocoder(&a, &b, parameters);
        let source_band = spectral_band_power(&output, a_frequency - 150.0, a_frequency + 150.0);
        let other_band = spectral_band_power(&output, 4_000.0, 12_000.0);
        assert!(
            source_band > other_band * 1_000.0,
            "envelope transfer invented unsupported high frequencies: source band \
             {source_band}, other band {other_band}"
        );
        let input_peak = a.iter().copied().map(f32::abs).fold(0.0, f32::max);
        let output_peak = output.iter().copied().map(f32::abs).fold(0.0, f32::max);
        assert!(
            output_peak <= input_peak * 8.01,
            "spectral gain clamp did not bound a simple sine: input peak {input_peak}, \
             output peak {output_peak}"
        );
    }

    #[test]
    fn chunk_overlap_uses_available_short_window_support() {
        assert_eq!(chunk_crossfade_frames(48_000, 96_000, 25.0), 12_000);
        assert_eq!(chunk_crossfade_frames(4_800, 1_440_000, 75.0), 3_600);
    }

    fn tone_amplitude(samples: &[f32], frequency: f32) -> f32 {
        let (real, imaginary) = tone_projection(samples, frequency);
        (2.0 * (real * real + imaginary * imaginary).sqrt() / samples.len() as f64) as f32
    }

    fn tone_phase(samples: &[f32], frequency: f32) -> f64 {
        let (real, imaginary) = tone_projection(samples, frequency);
        imaginary.atan2(real)
    }

    fn tone_projection(samples: &[f32], frequency: f32) -> (f64, f64) {
        let mut real = 0.0_f64;
        let mut imaginary = 0.0_f64;
        for (index, &sample) in samples.iter().enumerate() {
            let phase = 2.0 * std::f64::consts::PI * f64::from(frequency) * index as f64
                / f64::from(SAMPLE_RATE);
            real += f64::from(sample) * phase.cos();
            imaginary -= f64::from(sample) * phase.sin();
        }
        (real, imaginary)
    }

    fn wrapped_phase_difference(a: f64, b: f64) -> f64 {
        let difference = (a - b).abs();
        difference.min(2.0 * std::f64::consts::PI - difference)
    }

    fn synthetic_tone(frames: usize, frequency: f32, amplitude: f32) -> Vec<f32> {
        (0..frames)
            .map(|index| {
                amplitude * (2.0 * PI * frequency * index as f32 / SAMPLE_RATE as f32).sin()
            })
            .collect()
    }

    fn deterministic_noise(frames: usize, seed: u64) -> Vec<f32> {
        let mut state = seed;
        (0..frames)
            .map(|_| 0.2 * random_unit_variance(&mut state))
            .collect()
    }

    fn render_test_vocoder(a: &[f32], b: &[f32], parameters: AlgorithmParameters) -> Vec<f32> {
        render_source_filter_vocoder(
            &AudioClip {
                id: "a".to_owned(),
                samples: a.to_vec(),
            },
            &AudioClip {
                id: "b".to_owned(),
                samples: b.to_vec(),
            },
            parameters,
            &|| false,
        )
        .unwrap()
    }

    fn spectral_band_power(samples: &[f32], low_hz: f32, high_hz: f32) -> f64 {
        let fft_frames = 16_384.min(samples.len());
        let offset = (samples.len() - fft_frames) / 2;
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(fft_frames);
        let mut time = forward.make_input_vec();
        for (index, sample) in time.iter_mut().enumerate() {
            let window = (PI * (index as f32 + 0.5) / fft_frames as f32)
                .sin()
                .powi(2);
            *sample = samples[offset + index] * window;
        }
        let mut spectrum = forward.make_output_vec();
        forward.process(&mut time, &mut spectrum).unwrap();
        spectrum
            .iter()
            .enumerate()
            .filter(|(bin, _)| {
                let frequency = *bin as f32 * SAMPLE_RATE as f32 / fft_frames as f32;
                (low_hz..high_hz).contains(&frequency)
            })
            .map(|(_, value)| f64::from(value.norm_sqr()))
            .sum()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples
            .iter()
            .map(|&sample| f64::from(sample) * f64::from(sample))
            .sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt() as f32
    }

    fn rms_difference(a: &[f32], b: &[f32]) -> f32 {
        (a.iter()
            .zip(b)
            .map(|(&left, &right)| {
                let difference = f64::from(left - right);
                difference * difference
            })
            .sum::<f64>()
            / a.len().max(1) as f64)
            .sqrt() as f32
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
