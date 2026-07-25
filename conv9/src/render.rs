use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::audio::{
    AudioClip, AudioMetrics, OUTPUT_SECONDS, SAMPLE_RATE, condition_output, encode_pcm16,
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
    pub output_seconds: usize,
    pub sources: Vec<SourceEntry>,
    pub algorithms: Vec<AlgorithmCatalogEntry>,
}

#[derive(Debug, Serialize)]
pub struct AlgorithmCatalogEntry {
    pub id: Algorithm,
    pub title: &'static str,
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
    clips: Vec<AudioClip>,
    indices: HashMap<String, usize>,
}

impl OnDemandRenderer {
    pub fn load(manifest: &Path, input_dir: &Path) -> Result<Self> {
        let sources = load_manifest(manifest)?;
        let clips = sources
            .iter()
            .map(|source| {
                read_prepared_clip(&source.id, &input_dir.join(format!("{}.wav", source.id)))
            })
            .collect::<Result<Vec<_>>>()?;
        let indices = sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source.id.clone(), index))
            .collect();
        Ok(Self {
            sources,
            clips,
            indices,
        })
    }

    pub fn catalog(&self) -> Catalog {
        Catalog {
            schema_version: 2,
            mode: "on_demand",
            sample_rate: SAMPLE_RATE,
            channels: 1,
            output_seconds: OUTPUT_SECONDS,
            sources: self.sources.clone(),
            algorithms: Algorithm::ALL
                .into_iter()
                .map(|algorithm| AlgorithmCatalogEntry {
                    id: algorithm,
                    title: algorithm.title(),
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
        let config = if algorithm == Algorithm::FullConvolution {
            None
        } else {
            Some(WindowConfig::new(
                selection
                    .windows
                    .get("clip_a_seconds")
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("missing clip A window"))?,
                selection
                    .windows
                    .get("clip_b_seconds")
                    .copied()
                    .ok_or_else(|| anyhow::anyhow!("missing clip B window"))?,
            )?)
        };
        let parameters = selection.parameters.validate(algorithm)?;
        let left = self.clip(&selection.left_id)?;
        let right = self.clip(&selection.right_id)?;
        let mut output =
            render_algorithm_cancellable(algorithm, config, parameters, left, right, cancelled)
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

    fn clip(&self, id: &str) -> Result<&AudioClip> {
        self.indices
            .get(id)
            .and_then(|index| self.clips.get(*index))
            .ok_or_else(|| anyhow::anyhow!("unknown source {id}"))
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
            scale: "log",
        },
        WindowCatalogEntry {
            id: "clip_b_seconds",
            label: "B window",
            minimum: MIN_WINDOW_SECONDS,
            maximum: MAX_WINDOW_SECONDS,
            step: 0.01,
            default: DEFAULT_B_WINDOW_SECONDS,
            scale: "log",
        },
    ]
}

fn parameter_catalog(algorithm: Algorithm) -> Vec<ParameterCatalogEntry> {
    let defaults = AlgorithmParameters::default();
    let taper = || ParameterCatalogEntry {
        id: "taper",
        label: "taper",
        minimum: 0.05,
        maximum: 1.0,
        step: 0.01,
        default: defaults.taper,
        unit: "",
    };
    match algorithm {
        Algorithm::Multiresolution => vec![
            taper(),
            ParameterCatalogEntry {
                id: "multires_low_scale",
                label: "low scale",
                minimum: 1.0,
                maximum: 3.0,
                step: 0.05,
                default: defaults.multires_low_scale,
                unit: "×",
            },
            ParameterCatalogEntry {
                id: "multires_high_scale",
                label: "high scale",
                minimum: 0.15,
                maximum: 1.0,
                step: 0.01,
                default: defaults.multires_high_scale,
                unit: "×",
            },
            ParameterCatalogEntry {
                id: "multires_low_mix",
                label: "low gain",
                minimum: 0.0,
                maximum: 2.0,
                step: 0.01,
                default: defaults.multires_low_mix,
                unit: "×",
            },
            ParameterCatalogEntry {
                id: "multires_high_mix",
                label: "high gain",
                minimum: 0.0,
                maximum: 2.0,
                step: 0.01,
                default: defaults.multires_high_mix,
                unit: "×",
            },
            ParameterCatalogEntry {
                id: "multires_low_split_hz",
                label: "low split",
                minimum: 80.0,
                maximum: 800.0,
                step: 5.0,
                default: defaults.multires_low_split_hz,
                unit: "hz",
            },
            ParameterCatalogEntry {
                id: "multires_high_split_hz",
                label: "high split",
                minimum: 800.0,
                maximum: 8_000.0,
                step: 25.0,
                default: defaults.multires_high_split_hz,
                unit: "hz",
            },
        ],
        Algorithm::SlidingWola => vec![taper()],
        Algorithm::EvolvingIr => vec![
            taper(),
            ParameterCatalogEntry {
                id: "evolving_a_mix",
                label: "A carrier",
                minimum: 0.0,
                maximum: 1.0,
                step: 0.01,
                default: defaults.evolving_a_mix,
                unit: "",
            },
        ],
        Algorithm::ChunkCrossfade => vec![
            taper(),
            ParameterCatalogEntry {
                id: "chunk_crossfade_percent",
                label: "crossfade",
                minimum: 5.0,
                maximum: 75.0,
                step: 1.0,
                default: defaults.chunk_crossfade_percent,
                unit: "%",
            },
        ],
        Algorithm::FullConvolution => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_method_exposes_its_parameters() {
        for algorithm in Algorithm::ALL {
            let parameters = parameter_catalog(algorithm);
            if algorithm == Algorithm::FullConvolution {
                assert!(parameters.is_empty());
                assert!(window_catalog(algorithm).is_empty());
            } else {
                assert!(!parameters.is_empty());
                assert!(parameters.iter().any(|parameter| parameter.id == "taper"));
                assert_eq!(window_catalog(algorithm).len(), 2);
            }
        }
        assert!(
            parameter_catalog(Algorithm::ChunkCrossfade)
                .iter()
                .any(|parameter| parameter.id == "chunk_crossfade_percent")
        );
    }
}
