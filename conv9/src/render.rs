use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::audio::{
    AudioClip, AudioMetrics, INPUT_SECONDS, SAMPLE_RATE, condition_output, encode_pcm16,
    read_prepared_clip,
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
            schema_version: 6,
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
        let algorithm = Algorithm::from_str(&selection.algorithm)?;
        let parameters = selection.parameters.validate(algorithm)?;
        let config = if algorithm == Algorithm::FullConvolution {
            None
        } else {
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
        };
        let left = self.clip(&selection.left_id)?;
        let right = self.clip(&selection.right_id)?;
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
        if cancelled() {
            bail!("render cancelled");
        }
        let metrics = condition_output(&mut output)?;
        let wav = encode_pcm16(&output)?;
        Ok(RenderedAudio {
            wav,
            metrics,
            config,
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

fn window_catalog(algorithm: Algorithm) -> Vec<WindowCatalogEntry> {
    if algorithm == Algorithm::FullConvolution {
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
    let analysis_taper = || ParameterCatalogEntry {
        id: "taper",
        label: "edge taper",
        minimum: 0.05,
        maximum: 1.0,
        step: 0.01,
        default: defaults.taper,
        unit: "",
        description: "Sets the Tukey taper applied to each input window before convolution. Higher \
                      values soften more of each edge and reduce spectral leakage. Synthesis uses \
                      a fixed root-Hann shape with automatic power normalization.",
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
    let timeline_offset = || ParameterCatalogEntry {
        id: "window_b_offset_seconds",
        label: "B offset",
        minimum: -30.0,
        maximum: 30.0,
        step: 0.1,
        default: defaults.window_b_offset_seconds,
        unit: "s",
        description: "Offsets clip B's scan position relative to clip A at every window. Positive \
                      values read B later and negative values read it earlier; reflected boundaries \
                      preserve a continuous source without adding dry audio.",
    };
    match algorithm {
        Algorithm::WindowedConvolution => {
            vec![analysis_taper(), overlap(), timeline_offset()]
        }
        Algorithm::EvolvingIr => vec![
            analysis_taper(),
            overlap(),
            timeline_offset(),
            ParameterCatalogEntry {
                id: "evolving_a_mix",
                label: "A carrier",
                minimum: 0.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.evolving_a_mix,
                unit: "",
                description: "Blends the two cropped convolution carriers. 0 keeps only the \
                              B-sized carrier, 1 keeps only the A-sized carrier, and 0.5 gives both \
                              equal synthesis weight.",
            },
            ParameterCatalogEntry {
                id: "evolving_mix_motion",
                label: "carrier motion",
                minimum: -1.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.evolving_mix_motion,
                unit: "",
                description: "Moves the A/B carrier balance over the output timeline around the \
                              selected midpoint mix. Positive values evolve toward A; negative \
                              values evolve toward B, with clamping at either pure carrier.",
            },
            ParameterCatalogEntry {
                id: "evolving_crop_position",
                label: "crop position",
                minimum: 0.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.evolving_crop_position,
                unit: "",
                description: "Chooses which region of each complete local convolution becomes the \
                              A- and B-sized carriers. 0 keeps the onset, 0.5 keeps the center, and \
                              1 keeps the decaying tail.",
            },
        ],
        Algorithm::ChunkCrossfade => vec![
            analysis_taper(),
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
            timeline_offset(),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_method_exposes_its_parameters() {
        for algorithm in Algorithm::ALL {
            assert!(algorithm.description().len() > 100);
            let parameters = parameter_catalog(algorithm);
            if algorithm == Algorithm::FullConvolution {
                assert_eq!(parameters.len(), 4);
                assert!(
                    parameters
                        .iter()
                        .all(|parameter| parameter.description.len() > 80)
                );
                assert!(window_catalog(algorithm).is_empty());
            } else {
                assert!(!parameters.is_empty());
                assert!(
                    parameters
                        .iter()
                        .all(|parameter| parameter.description.len() > 80)
                );
                assert!(parameters.iter().any(|parameter| parameter.id == "taper"));
                let windows = window_catalog(algorithm);
                assert_eq!(windows.len(), 2);
                assert!(windows.iter().all(|window| {
                    window.default == 5.0
                        && window.scale == "soft_log"
                        && window.description.len() > 100
                }));
            }
        }
        assert!(
            parameter_catalog(Algorithm::ChunkCrossfade)
                .iter()
                .any(|parameter| parameter.id == "chunk_crossfade_percent")
        );

        let required = [
            (
                Algorithm::WindowedConvolution,
                &["taper", "window_overlap_percent", "window_b_offset_seconds"][..],
            ),
            (
                Algorithm::EvolvingIr,
                &[
                    "taper",
                    "window_overlap_percent",
                    "window_b_offset_seconds",
                    "evolving_a_mix",
                    "evolving_mix_motion",
                    "evolving_crop_position",
                ][..],
            ),
            (
                Algorithm::ChunkCrossfade,
                &[
                    "taper",
                    "chunk_crossfade_percent",
                    "window_b_offset_seconds",
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
