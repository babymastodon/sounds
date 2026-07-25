use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

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
            schema_version: 4,
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
    let crossfade = || ParameterCatalogEntry {
        id: "taper",
        label: "crossfade",
        minimum: 0.05,
        maximum: 1.0,
        step: 0.01,
        default: defaults.taper,
        unit: "",
        description: "Shapes both the Tukey analysis window and the normalized raised-cosine \
                      synthesis crossfade between overlapping convolution results. Higher values \
                      make the transition longer and more gradual; 1 uses the full window.",
    };
    let analysis_taper = || ParameterCatalogEntry {
        id: "taper",
        label: "edge taper",
        minimum: 0.05,
        maximum: 1.0,
        step: 0.01,
        default: defaults.taper,
        unit: "",
        description: "Sets the Tukey taper applied to each independent input chunk before \
                      convolution. Higher values soften more of each chunk edge to reduce spectral \
                      leakage; chunk-to-chunk blending is controlled separately by overlap.",
    };
    let overlap = || ParameterCatalogEntry {
        id: "window_overlap_percent",
        label: "overlap",
        minimum: 5.0,
        maximum: 80.0,
        step: 1.0,
        default: defaults.window_overlap_percent,
        unit: "%",
        description: "Sets how much of the shorter A/B analysis window overlaps the next timeline \
                      position, guaranteeing overlap for both inputs. Higher values make local \
                      transitions denser and smoother but render more FFT blocks.",
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
        Algorithm::Multiresolution => vec![
            crossfade(),
            overlap(),
            timeline_offset(),
            ParameterCatalogEntry {
                id: "multires_low_scale",
                label: "low scale",
                minimum: 1.0,
                maximum: 3.0,
                step: 0.05,
                default: defaults.multires_low_scale,
                unit: "×",
                description: "Multiplies both selected A/B window lengths for the low-frequency \
                              band. Larger values stabilize bass and preserve slower motion, at the \
                              cost of more smear and computation.",
            },
            ParameterCatalogEntry {
                id: "multires_high_scale",
                label: "high scale",
                minimum: 0.15,
                maximum: 1.0,
                step: 0.01,
                default: defaults.multires_high_scale,
                unit: "×",
                description: "Multiplies both selected A/B window lengths for the high-frequency \
                              band. Smaller values sharpen transients and timing; larger values \
                              retain more high-frequency context.",
            },
            ParameterCatalogEntry {
                id: "multires_low_mix",
                label: "low gain",
                minimum: 0.0,
                maximum: 2.0,
                step: 0.01,
                default: defaults.multires_low_mix,
                unit: "×",
                description: "Scales the low-frequency convolution band before it is recombined \
                              with the mid and high bands. Final conditioning still enforces the \
                              shared output level and peak ceiling.",
            },
            ParameterCatalogEntry {
                id: "multires_high_mix",
                label: "high gain",
                minimum: 0.0,
                maximum: 2.0,
                step: 0.01,
                default: defaults.multires_high_mix,
                unit: "×",
                description: "Scales the high-frequency convolution band before recombination. \
                              Raise it for more texture and attack, or lower it for a darker result; \
                              final output power is still conditioned.",
            },
            ParameterCatalogEntry {
                id: "multires_low_split_hz",
                label: "low split",
                minimum: 80.0,
                maximum: 800.0,
                step: 5.0,
                default: defaults.multires_low_split_hz,
                unit: "hz",
                description: "Sets the center frequency of the raised-cosine low-to-mid band split. \
                              The split-width control sets how broadly the neighboring bands blend \
                              around this frequency.",
            },
            ParameterCatalogEntry {
                id: "multires_high_split_hz",
                label: "high split",
                minimum: 800.0,
                maximum: 8_000.0,
                step: 25.0,
                default: defaults.multires_high_split_hz,
                unit: "hz",
                description: "Sets the center frequency of the raised-cosine mid-to-high band split. \
                              The split-width control sets how broadly the neighboring bands blend \
                              around this frequency.",
            },
            ParameterCatalogEntry {
                id: "multires_transition_width",
                label: "split width",
                minimum: 0.05,
                maximum: 0.75,
                step: 0.01,
                default: defaults.multires_transition_width,
                unit: "",
                description: "Sets the fractional width of both raised-cosine crossover regions. \
                              Smaller values isolate bands more sharply; larger values blend them \
                              broadly while normalized band masks keep their sum equal to one.",
            },
        ],
        Algorithm::SlidingWola => vec![crossfade(), overlap(), timeline_offset()],
        Algorithm::EvolvingIr => vec![
            crossfade(),
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
                description: "Sets the equal-power overlap as a percentage of the shorter A/B chunk, \
                              which is the convolution support available beyond the longer timeline \
                              slot. Higher values make smoother, longer transitions.",
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
                              complete 60-second source and is automatically kept within the clip \
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
                              complete 60-second source and is automatically kept within the clip \
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
                Algorithm::Multiresolution,
                &[
                    "taper",
                    "window_overlap_percent",
                    "window_b_offset_seconds",
                    "multires_transition_width",
                ][..],
            ),
            (
                Algorithm::SlidingWola,
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
