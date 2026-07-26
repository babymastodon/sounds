use std::collections::{HashMap, VecDeque};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use realfft::RealFftPlanner;
use serde::Serialize;

use crate::audio::{
    AudioClip, AudioMetrics, INPUT_SECONDS, SAMPLE_RATE, condition_output, encode_pcm16, measure,
    read_prepared_clip, validate_metrics,
};
use crate::dsp::{
    Algorithm, AlgorithmParameters, DEFAULT_A_WINDOW_SECONDS, DEFAULT_B_WINDOW_SECONDS,
    MAX_WINDOW_SECONDS, MIN_WINDOW_SECONDS, WindowConfig, render_algorithm_cancellable,
};
use crate::manifest::{SourceEntry, load_manifest};

#[derive(Debug, Serialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub mode: &'static str,
    pub sample_rate: u32,
    pub channels: u16,
    pub input_seconds: usize,
    pub sources: Vec<SourceEntry>,
    pub algorithms: Vec<AlgorithmCatalogEntry>,
}

#[derive(Debug, Serialize)]
pub struct AlgorithmCatalogEntry {
    pub id: Algorithm,
    pub title: &'static str,
    pub description: &'static str,
    pub rank: u8,
    pub windows: Vec<WindowCatalogEntry>,
    pub parameters: Vec<ParameterCatalogEntry>,
}

#[derive(Debug, Serialize)]
pub struct ParameterCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub minimum: f32,
    pub maximum: f32,
    pub step: f32,
    pub default: f32,
    pub unit: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Serialize)]
pub struct WindowCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub minimum: f32,
    pub maximum: f32,
    pub step: f32,
    pub default: f32,
    pub scale: &'static str,
    pub description: &'static str,
}

#[derive(Debug)]
pub struct RenderedAudio {
    pub wav: Vec<u8>,
    pub metrics: AudioMetrics,
    pub config: Option<WindowConfig>,
    pub timings: RenderTimings,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreview {
    pub id: String,
    pub peaks: Vec<[f32; 2]>,
    pub spectrum_map: Vec<f32>,
    pub spectrum_columns: usize,
    pub spectrum_rows: usize,
    pub peak: f32,
    pub rms_dbfs: f32,
    pub zero_crossing_rate: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderTimings {
    pub source_milliseconds: f64,
    pub dsp_milliseconds: f64,
    pub condition_milliseconds: f64,
    pub encode_milliseconds: f64,
    pub total_milliseconds: f64,
}

#[derive(Clone, Debug)]
pub struct RenderSelection {
    pub left_id: String,
    pub right_id: String,
    pub algorithm: String,
    pub windows: HashMap<String, f32>,
    pub parameters: AlgorithmParameters,
}

pub struct OnDemandRenderer {
    sources: Vec<SourceEntry>,
    input_dir: PathBuf,
    indices: HashMap<String, usize>,
    clip_cache: Mutex<VecDeque<Arc<AudioClip>>>,
}

impl OnDemandRenderer {
    pub fn load(manifest: &Path, input_dir: &Path) -> Result<Self> {
        let sources = load_manifest(manifest)?;
        for source in &sources {
            let path = input_dir.join(format!("{}.wav", source.id));
            if !path.is_file() {
                bail!("missing prepared input {}", path.display());
            }
        }
        let indices = sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source.id.clone(), index))
            .collect();
        Ok(Self {
            sources,
            input_dir: input_dir.to_owned(),
            indices,
            clip_cache: Mutex::new(VecDeque::new()),
        })
    }

    pub fn catalog(&self) -> Catalog {
        Catalog {
            schema_version: 9,
            mode: "on_demand",
            sample_rate: SAMPLE_RATE,
            channels: 1,
            input_seconds: INPUT_SECONDS,
            sources: self.sources.clone(),
            algorithms: Algorithm::ALL
                .into_iter()
                .map(|algorithm| AlgorithmCatalogEntry {
                    id: algorithm,
                    title: algorithm.title(),
                    description: algorithm.description(),
                    rank: algorithm.rank(),
                    windows: window_catalog(algorithm),
                    parameters: parameter_catalog(algorithm),
                })
                .collect(),
        }
    }

    pub fn render(
        &self,
        selection: &RenderSelection,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<RenderedAudio> {
        let total_started = Instant::now();
        let algorithm = Algorithm::from_str(&selection.algorithm)?;
        let parameters = selection.parameters.validate(algorithm)?;
        let config = if algorithm.uses_windows() {
            let clip_a_seconds = selection
                .windows
                .get("clip_a_seconds")
                .copied()
                .ok_or_else(|| anyhow::anyhow!("missing clip A window"))?;
            let clip_b_seconds = selection
                .windows
                .get("clip_b_seconds")
                .copied()
                .ok_or_else(|| anyhow::anyhow!("missing clip B window"))?;
            Some(if algorithm == Algorithm::ChunkCrossfade {
                WindowConfig::for_chunks(clip_a_seconds, clip_b_seconds)?
            } else {
                WindowConfig::new(
                    clip_a_seconds,
                    clip_b_seconds,
                    parameters.window_overlap_percent,
                )?
            })
        } else {
            None
        };
        let source_started = Instant::now();
        let left = self.clip(&selection.left_id)?;
        let right = self.clip(&selection.right_id)?;
        let source_milliseconds = elapsed_milliseconds(source_started);
        let dsp_started = Instant::now();
        let mut output =
            render_algorithm_cancellable(algorithm, config, parameters, &left, &right, cancelled)
                .with_context(|| {
                format!(
                    "{} / {} ({}) × {} ({})",
                    algorithm.slug(),
                    left.id,
                    selection
                        .windows
                        .get("clip_a_seconds")
                        .copied()
                        .map(|seconds| format!("{seconds:.2} s"))
                        .unwrap_or_else(|| "full".to_owned()),
                    right.id,
                    selection
                        .windows
                        .get("clip_b_seconds")
                        .copied()
                        .map(|seconds| format!("{seconds:.2} s"))
                        .unwrap_or_else(|| "full".to_owned()),
                )
            })?;
        let dsp_milliseconds = elapsed_milliseconds(dsp_started);
        if cancelled() {
            bail!("render cancelled");
        }
        let condition_started = Instant::now();
        let metrics = if algorithm.is_dry() {
            let metrics = measure(&output);
            validate_metrics(&metrics, output.len(), "dry source output")?;
            metrics
        } else {
            condition_output(&mut output)?
        };
        let condition_milliseconds = elapsed_milliseconds(condition_started);
        let encode_started = Instant::now();
        let wav = encode_pcm16(&output)?;
        let encode_milliseconds = elapsed_milliseconds(encode_started);
        Ok(RenderedAudio {
            wav,
            metrics,
            config,
            timings: RenderTimings {
                source_milliseconds,
                dsp_milliseconds,
                condition_milliseconds,
                encode_milliseconds,
                total_milliseconds: elapsed_milliseconds(total_started),
            },
        })
    }

    pub fn source_preview(&self, id: &str, bins: usize) -> Result<SourcePreview> {
        if !(64..=512).contains(&bins) {
            bail!("source preview bins must be between 64 and 512");
        }
        let clip = self.clip(id)?;
        let metrics = measure(&clip.samples);
        let mut peaks = Vec::with_capacity(bins);
        for index in 0..bins {
            let start = index * clip.samples.len() / bins;
            let end = ((index + 1) * clip.samples.len() / bins).max(start + 1);
            let (minimum, maximum) = clip.samples[start..end].iter().fold(
                (f32::INFINITY, f32::NEG_INFINITY),
                |(minimum, maximum), &sample| (minimum.min(sample), maximum.max(sample)),
            );
            peaks.push([minimum, maximum]);
        }
        let crossings = clip
            .samples
            .windows(2)
            .filter(|pair| pair[0].is_sign_negative() != pair[1].is_sign_negative())
            .count();
        let spectrum_rows = 96;
        let spectrum_map = source_preview_spectrum_map(&clip.samples, bins, spectrum_rows)?;
        Ok(SourcePreview {
            id: clip.id.clone(),
            peaks,
            spectrum_map,
            spectrum_columns: bins,
            spectrum_rows,
            peak: metrics.peak,
            rms_dbfs: metrics.rms_dbfs,
            zero_crossing_rate: crossings as f32
                / clip.samples.len().saturating_sub(1).max(1) as f32,
        })
    }

    fn clip(&self, id: &str) -> Result<Arc<AudioClip>> {
        if !self.indices.contains_key(id) {
            bail!("unknown source {id}");
        }
        {
            let mut cache = self
                .clip_cache
                .lock()
                .map_err(|_| anyhow::anyhow!("prepared clip cache was poisoned"))?;
            if let Some(position) = cache.iter().position(|clip| clip.id == id) {
                let clip = cache
                    .remove(position)
                    .expect("located clip must remain in cache");
                cache.push_front(Arc::clone(&clip));
                return Ok(clip);
            }
        }

        let loaded = Arc::new(read_prepared_clip(
            id,
            &self.input_dir.join(format!("{id}.wav")),
        )?);
        let mut cache = self
            .clip_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("prepared clip cache was poisoned"))?;
        cache.push_front(Arc::clone(&loaded));
        cache.truncate(4);
        Ok(loaded)
    }
}

fn source_preview_spectrum_map(samples: &[f32], columns: usize, rows: usize) -> Result<Vec<f32>> {
    const FFT_FRAMES: usize = 4_096;
    const MINIMUM_HZ: f32 = 50.0;
    const MAXIMUM_HZ: f32 = 20_000.0;
    const FLOOR_DB: f32 = -72.0;

    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(FFT_FRAMES);
    let mut time = forward.make_input_vec();
    let mut fft = forward.make_output_vec();
    let window = (0..FFT_FRAMES)
        .map(|index| 0.5 - 0.5 * (2.0 * PI * index as f32 / (FFT_FRAMES - 1) as f32).cos())
        .collect::<Vec<_>>();
    let maximum_hz = MAXIMUM_HZ.min(SAMPLE_RATE as f32 / 2.0);
    let frequency_ratio = maximum_hz / MINIMUM_HZ;
    let row_bin_ranges = (0..rows)
        .map(|row| {
            let phase = 1.0 - row as f32 / rows.saturating_sub(1).max(1) as f32;
            let half_step = 0.5 / rows.saturating_sub(1).max(1) as f32;
            let low_hz = MINIMUM_HZ * frequency_ratio.powf((phase - half_step).clamp(0.0, 1.0));
            let high_hz = MINIMUM_HZ * frequency_ratio.powf((phase + half_step).clamp(0.0, 1.0));
            let low_bin =
                ((low_hz * FFT_FRAMES as f32 / SAMPLE_RATE as f32).floor() as usize).max(1);
            let high_bin = ((high_hz * FFT_FRAMES as f32 / SAMPLE_RATE as f32).ceil() as usize)
                .max(low_bin + 1)
                .min(fft.len());
            low_bin..high_bin
        })
        .collect::<Vec<_>>();
    let mut spectrum_map = vec![FLOOR_DB; columns * rows];
    for column in 0..columns {
        let center = column * samples.len().saturating_sub(1) / columns.saturating_sub(1).max(1);
        let start = center as isize - FFT_FRAMES as isize / 2;
        for (index, (target, &weight)) in time.iter_mut().zip(&window).enumerate() {
            let source = start + index as isize;
            *target = if source >= 0 && (source as usize) < samples.len() {
                samples[source as usize] * weight
            } else {
                0.0
            };
        }
        forward.process(&mut time, &mut fft)?;
        for (row, bins) in row_bin_ranges.iter().enumerate() {
            let band_power = fft[bins.clone()]
                .iter()
                .map(|value| value.norm_sqr())
                .fold(0.0_f32, f32::max);
            spectrum_map[column * rows + row] = 10.0 * band_power.max(1.0e-20).log10();
        }
    }
    let peak_db = spectrum_map
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    if peak_db <= -190.0 {
        spectrum_map.fill(0.0);
        return Ok(spectrum_map);
    }
    for value in &mut spectrum_map {
        *value = ((*value - peak_db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
    }
    Ok(spectrum_map)
}

fn elapsed_milliseconds(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

fn window_catalog(algorithm: Algorithm) -> Vec<WindowCatalogEntry> {
    if !algorithm.uses_windows() {
        return Vec::new();
    }
    vec![
        WindowCatalogEntry {
            id: "clip_a_seconds",
            label: "A window",
            minimum: MIN_WINDOW_SECONDS,
            maximum: MAX_WINDOW_SECONDS,
            step: 0.01,
            default: DEFAULT_A_WINDOW_SECONDS,
            scale: "soft_log",
            description: "Sets how many seconds are extracted from clip A at each synchronized \
                          timeline position. Longer windows retain more context and pitch detail \
                          but produce more temporal smear and require larger FFTs.",
        },
        WindowCatalogEntry {
            id: "clip_b_seconds",
            label: "B window",
            minimum: MIN_WINDOW_SECONDS,
            maximum: MAX_WINDOW_SECONDS,
            step: 0.01,
            default: DEFAULT_B_WINDOW_SECONDS,
            scale: "soft_log",
            description: "Sets how many seconds are extracted from clip B at each synchronized \
                          timeline position. Longer windows retain more context and pitch detail \
                          but produce more temporal smear and require larger FFTs.",
        },
    ]
}

fn parameter_catalog(algorithm: Algorithm) -> Vec<ParameterCatalogEntry> {
    let defaults = AlgorithmParameters::default();
    let input_taper = || ParameterCatalogEntry {
        id: "input_taper",
        label: "input taper",
        minimum: 0.05,
        maximum: 1.0,
        step: 0.01,
        default: defaults.input_taper,
        unit: "",
        description: "Sets the Tukey taper applied to both extracted input windows before each \
                      convolution. 0.05 is nearly rectangular, 0.50 is the balanced default, and \
                      1.0 is a full Hann window; synthesis crossfading remains separate.",
    };
    let overlap = || ParameterCatalogEntry {
        id: "window_overlap_percent",
        label: "overlap",
        minimum: 5.0,
        maximum: 80.0,
        step: 1.0,
        default: defaults.window_overlap_percent,
        unit: "%",
        description: "Sets source-scan overlap from the shorter A/B analysis window. The 75% \
                      default provides four-way synthesis coverage after each result is placed at \
                      tA+tB. Lower values expose grain and buzz; higher values cost more FFT blocks.",
    };
    match algorithm {
        Algorithm::WindowedConvolution => vec![input_taper(), overlap()],
        Algorithm::SourceFilterVocoder => vec![
            ParameterCatalogEntry {
                id: "vocoder_transfer",
                label: "transfer",
                minimum: 0.0,
                maximum: 1.5,
                step: 0.01,
                default: defaults.vocoder_transfer,
                unit: "",
                description: "Sets how strongly clip B's smoothed spectral envelope reshapes clip \
                              A. 0 keeps A's original spectrum, 1 applies the measured B/A envelope \
                              ratio, and values above 1 exaggerate the transferred color.",
            },
            ParameterCatalogEntry {
                id: "vocoder_envelope_width_hz",
                label: "envelope width",
                minimum: 100.0,
                maximum: 3_000.0,
                step: 50.0,
                default: defaults.vocoder_envelope_width_hz,
                unit: "Hz",
                description: "Sets the frequency span used to smooth both short-time spectra before \
                              their ratio is transferred. Narrow values retain detailed resonances; \
                              broad values emphasize stable formants and overall timbral shape.",
            },
            ParameterCatalogEntry {
                id: "vocoder_transient_protection",
                label: "transients",
                minimum: 0.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.vocoder_transient_protection,
                unit: "",
                description: "Reduces envelope transfer only during rapid spectral onsets from clip \
                              A. 0 colors attacks fully, while 1 preserves the strongest A transients \
                              and restores the selected transfer as each onset settles.",
            },
        ],
        Algorithm::PredictiveResonatorBank => vec![
            ParameterCatalogEntry {
                id: "resonator_transfer",
                label: "transfer",
                minimum: 0.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.resonator_transfer,
                unit: "",
                description: "Moves the stable synthesis model from clip B's own response toward \
                              clip A's learned resonances. 0 is an exact B identity transform before \
                              shared output conditioning, while 1 gives A full spectral character \
                              and keeps B's innovation, events, and complete timeline.",
            },
            ParameterCatalogEntry {
                id: "resonator_ring",
                label: "ring",
                minimum: 0.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.resonator_ring,
                unit: "",
                description: "Controls bandwidth expansion of the learned causal models. Low values \
                              pull every stable pole inward for a dry, quickly damped body; high \
                              values retain narrower modes and longer ringing while remaining \
                              strictly stable. It is neutral when transfer is 0.",
            },
        ],
        Algorithm::ChunkCrossfade => vec![
            input_taper(),
            ParameterCatalogEntry {
                id: "chunk_crossfade_percent",
                label: "overlap",
                minimum: 5.0,
                maximum: 75.0,
                step: 1.0,
                default: defaults.chunk_crossfade_percent,
                unit: "%",
                description: "Sets the power-normalized overlap as a percentage of the shorter A/B \
                              chunk, which is the convolution support available beyond the longer \
                              timeline slot. 50% is continuous by default; lower values expose seams.",
            },
            ParameterCatalogEntry {
                id: "chunk_crop_position",
                label: "crop position",
                minimum: 0.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.chunk_crop_position,
                unit: "",
                description: "Chooses where each timeline-sized block is cropped from its complete \
                              A+B local convolution. 0 favors the onset, 0.5 keeps the center, and \
                              1 favors the convolution tail before equal-power overlap.",
            },
        ],
        Algorithm::FullConvolution => vec![
            ParameterCatalogEntry {
                id: "full_a_offset_seconds",
                label: "A offset",
                minimum: 0.0,
                maximum: INPUT_SECONDS as f32 - 0.1,
                step: 0.1,
                default: defaults.full_a_offset_seconds,
                unit: "s",
                description: "Sets where the selected segment begins in clip A. The segment is \
                              convolved from this exact source time; changing the offset never pads \
                              or wraps the source and may shorten the current duration to fit.",
            },
            ParameterCatalogEntry {
                id: "full_a_duration_seconds",
                label: "A duration",
                minimum: 0.1,
                maximum: INPUT_SECONDS as f32,
                step: 0.1,
                default: defaults.full_a_duration_seconds,
                unit: "s",
                description: "Sets the duration of clip A's selected segment. It defaults to the \
                              complete 61-second source and is automatically kept within the clip \
                              after the selected A offset.",
            },
            ParameterCatalogEntry {
                id: "full_b_offset_seconds",
                label: "B offset",
                minimum: 0.0,
                maximum: INPUT_SECONDS as f32 - 0.1,
                step: 0.1,
                default: defaults.full_b_offset_seconds,
                unit: "s",
                description: "Sets where the selected segment begins in clip B. The segment is \
                              convolved from this exact source time; changing the offset never pads \
                              or wraps the source and may shorten the current duration to fit.",
            },
            ParameterCatalogEntry {
                id: "full_b_duration_seconds",
                label: "B duration",
                minimum: 0.1,
                maximum: INPUT_SECONDS as f32,
                step: 0.1,
                default: defaults.full_b_duration_seconds,
                unit: "s",
                description: "Sets the duration of clip B's selected segment. It defaults to the \
                              complete 61-second source and is automatically kept within the clip \
                              after the selected B offset.",
            },
        ],
        Algorithm::DryA | Algorithm::DryB => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_preview_fft_map_preserves_time_and_frequency() {
        let seconds = 2;
        let frames = SAMPLE_RATE as usize * seconds;
        let mut samples = vec![0.0; frames];
        for (index, sample) in samples.iter_mut().enumerate() {
            let frequency = if index < frames / 2 { 400.0 } else { 4_000.0 };
            *sample = (2.0 * PI * frequency * index as f32 / SAMPLE_RATE as f32).sin() * 0.5;
        }
        let columns = 64;
        let rows = 96;
        let map = source_preview_spectrum_map(&samples, columns, rows).unwrap();
        let row_for = |frequency: f32| {
            ((1.0 - (frequency / 50.0).ln() / (20_000.0_f32 / 50.0).ln()) * (rows - 1) as f32)
                .round() as usize
        };
        let mean_region = |column_start: usize, column_end: usize, row: usize| {
            (column_start..column_end)
                .map(|column| map[column * rows + row])
                .sum::<f32>()
                / (column_end - column_start) as f32
        };
        let low_row = row_for(400.0);
        let high_row = row_for(4_000.0);
        let early_low = mean_region(8, 24, low_row);
        let early_high = mean_region(8, 24, high_row);
        let late_low = mean_region(40, 56, low_row);
        let late_high = mean_region(40, 56, high_row);
        assert!(
            early_low > late_low + 0.3,
            "early columns must retain the low-frequency tone"
        );
        assert!(
            late_high > early_high + 0.3,
            "late columns must retain the high-frequency tone"
        );

        let silence = source_preview_spectrum_map(&vec![0.0; frames], columns, rows).unwrap();
        assert!(silence.iter().all(|&value| value == 0.0));
    }

    #[test]
    fn each_method_exposes_its_parameters() {
        for algorithm in Algorithm::ALL {
            assert!(algorithm.description().len() > 100);
            let parameters = parameter_catalog(algorithm);
            if algorithm.uses_windows() {
                assert!(!parameters.is_empty());
                assert!(
                    parameters
                        .iter()
                        .all(|parameter| parameter.description.len() > 80)
                );
                let windows = window_catalog(algorithm);
                assert_eq!(windows.len(), 2);
                assert!(windows.iter().all(|window| {
                    window.default == 5.0
                        && window.scale == "soft_log"
                        && window.description.len() > 100
                }));
            } else if algorithm == Algorithm::FullConvolution {
                assert_eq!(parameters.len(), 4);
                assert!(
                    parameters
                        .iter()
                        .all(|parameter| parameter.description.len() > 80)
                );
                assert!(window_catalog(algorithm).is_empty());
            } else if algorithm.is_dry() {
                assert!(algorithm.is_dry());
                assert!(parameters.is_empty());
                assert!(window_catalog(algorithm).is_empty());
            } else {
                let expected_count = match algorithm {
                    Algorithm::SourceFilterVocoder => 3,
                    Algorithm::PredictiveResonatorBank => 2,
                    _ => panic!("unexpected non-windowed configurable algorithm"),
                };
                assert_eq!(parameters.len(), expected_count);
                assert!(window_catalog(algorithm).is_empty());
                assert!(
                    parameters
                        .iter()
                        .all(|parameter| parameter.description.len() > 80)
                );
            }
        }
        assert!(
            parameter_catalog(Algorithm::ChunkCrossfade)
                .iter()
                .any(|parameter| parameter.id == "chunk_crossfade_percent")
        );
        assert!(Algorithm::ALL.into_iter().all(|algorithm| {
            parameter_catalog(algorithm).iter().all(|parameter| {
                parameter.id != "taper" && parameter.id != "window_b_offset_seconds"
            })
        }));

        let required = [
            (
                Algorithm::WindowedConvolution,
                &["input_taper", "window_overlap_percent"][..],
            ),
            (
                Algorithm::SourceFilterVocoder,
                &[
                    "vocoder_transfer",
                    "vocoder_envelope_width_hz",
                    "vocoder_transient_protection",
                ][..],
            ),
            (
                Algorithm::PredictiveResonatorBank,
                &["resonator_transfer", "resonator_ring"][..],
            ),
            (
                Algorithm::ChunkCrossfade,
                &[
                    "input_taper",
                    "chunk_crossfade_percent",
                    "chunk_crop_position",
                ][..],
            ),
        ];
        for (algorithm, required_ids) in required {
            let parameters = parameter_catalog(algorithm);
            for required_id in required_ids {
                assert!(
                    parameters
                        .iter()
                        .any(|parameter| parameter.id == *required_id),
                    "{} is missing {required_id}",
                    algorithm.slug()
                );
            }
        }
    }
}
