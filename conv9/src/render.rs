use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::audio::{
    AudioClip, AudioMetrics, OUTPUT_FRAMES, SAMPLE_RATE, condition_output, measure_wav,
    read_prepared_clip, validate_metrics, write_pcm16,
};
use crate::dsp::{Algorithm, WindowConfig, WindowPreset, render_algorithm};
use crate::manifest::{SourceEntry, load_manifest};

pub const PAIR_COUNT: usize = 66;
pub const OUTPUT_COUNT: usize = PAIR_COUNT * 4 * 3;

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub manifest: PathBuf,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub force: bool,
    pub algorithm: Option<Algorithm>,
    pub preset: Option<WindowPreset>,
    pub pair: Option<String>,
}

#[derive(Clone, Debug)]
pub struct VerifyOptions {
    pub manifest: PathBuf,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
}

#[derive(Clone)]
struct Pair {
    left: usize,
    right: usize,
    slug: String,
}

#[derive(Clone)]
struct RenderTask {
    pair: Pair,
    algorithm: Algorithm,
    preset: WindowPreset,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MetricsRow {
    pub pair: String,
    pub left: String,
    pub right: String,
    pub algorithm: Algorithm,
    pub preset: WindowPreset,
    pub path: String,
    pub metrics: AudioMetrics,
}

#[derive(Debug, Serialize)]
struct Catalog {
    schema_version: u32,
    generated_by: &'static str,
    sample_rate: u32,
    channels: u16,
    output_seconds: usize,
    expected_pairs: usize,
    expected_outputs: usize,
    sources: Vec<SourceEntry>,
    algorithms: Vec<AlgorithmCatalogEntry>,
    presets: Vec<PresetCatalogEntry>,
    clips: Vec<MetricsRow>,
}

#[derive(Debug, Serialize)]
struct AlgorithmCatalogEntry {
    id: Algorithm,
    title: &'static str,
    rank: u8,
}

#[derive(Debug, Serialize)]
struct PresetCatalogEntry {
    id: WindowPreset,
    title: &'static str,
    #[serde(flatten)]
    config: WindowConfig,
}

pub fn render_matrix(options: RenderOptions) -> Result<()> {
    let sources = load_manifest(&options.manifest)?;
    let clips = load_clips(&sources, &options.input_dir)?;
    let pairs = make_pairs(&sources);
    let tasks = make_tasks(&pairs, &options)?;
    if tasks.is_empty() {
        bail!("render filters selected no tasks");
    }
    fs::create_dir_all(&options.output_dir)?;
    eprintln!(
        "rendering {} task(s) from {} pairs; each output is 60 s mono PCM16",
        tasks.len(),
        pairs.len()
    );
    let rows = tasks
        .par_iter()
        .enumerate()
        .map(|(index, task)| {
            let path = output_path(&options.output_dir, task);
            let metrics = if path.is_file() && !options.force {
                let metrics = measure_wav(&path)
                    .with_context(|| format!("validate reusable {}", path.display()))?;
                validate_metrics(&metrics, OUTPUT_FRAMES, &path.display().to_string())?;
                metrics
            } else {
                let mut output = render_algorithm(
                    task.algorithm,
                    task.preset,
                    &clips[task.pair.left],
                    &clips[task.pair.right],
                )
                .with_context(|| {
                    format!(
                        "{} / {} / {}",
                        task.algorithm.slug(),
                        task.preset.slug(),
                        task.pair.slug
                    )
                })?;
                let metrics = condition_output(&mut output)?;
                write_pcm16(&path, &output)?;
                metrics
            };
            eprintln!(
                "[{}/{}] {} / {} / {}",
                index + 1,
                tasks.len(),
                task.algorithm.slug(),
                task.preset.slug(),
                task.pair.slug
            );
            Ok(MetricsRow {
                pair: task.pair.slug.clone(),
                left: clips[task.pair.left].id.clone(),
                right: clips[task.pair.right].id.clone(),
                algorithm: task.algorithm,
                preset: task.preset,
                path: relative_output_path(task),
                metrics,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    write_metrics(&options.output_dir.join("metrics.csv"), &rows)?;
    write_catalog(&options.output_dir, sources, rows)?;
    Ok(())
}

pub fn verify_matrix(options: VerifyOptions) -> Result<()> {
    let sources = load_manifest(&options.manifest)?;
    let _clips = load_clips(&sources, &options.input_dir)?;
    let pairs = make_pairs(&sources);
    let tasks = make_tasks(
        &pairs,
        &RenderOptions {
            manifest: options.manifest.clone(),
            input_dir: options.input_dir.clone(),
            output_dir: options.output_dir.clone(),
            force: false,
            algorithm: None,
            preset: None,
            pair: None,
        },
    )?;
    if tasks.len() != OUTPUT_COUNT {
        bail!(
            "internal task count is {}, expected {OUTPUT_COUNT}",
            tasks.len()
        );
    }
    let rows = tasks
        .par_iter()
        .map(|task| {
            let path = output_path(&options.output_dir, task);
            if !path.is_file() {
                bail!("missing {}", path.display());
            }
            let metrics = measure_wav(&path)?;
            validate_metrics(&metrics, OUTPUT_FRAMES, &path.display().to_string())?;
            Ok(MetricsRow {
                pair: task.pair.slug.clone(),
                left: sources[task.pair.left].id.clone(),
                right: sources[task.pair.right].id.clone(),
                algorithm: task.algorithm,
                preset: task.preset,
                path: relative_output_path(task),
                metrics,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let unique_paths = rows
        .iter()
        .map(|row| row.path.as_str())
        .collect::<HashSet<_>>();
    if unique_paths.len() != OUTPUT_COUNT {
        bail!(
            "expected {OUTPUT_COUNT} unique paths, found {}",
            unique_paths.len()
        );
    }
    let wav_count = count_wavs(&options.output_dir)?;
    if wav_count != OUTPUT_COUNT {
        bail!("output tree contains {wav_count} WAVs; expected exactly {OUTPUT_COUNT}");
    }
    write_metrics(&options.output_dir.join("metrics.csv"), &rows)?;
    write_catalog(&options.output_dir, sources, rows)?;
    eprintln!("verified {OUTPUT_COUNT} outputs: {PAIR_COUNT} pairs × 4 algorithms × 3 presets");
    Ok(())
}

fn load_clips(sources: &[SourceEntry], input_dir: &Path) -> Result<Vec<AudioClip>> {
    sources
        .iter()
        .map(|source| read_prepared_clip(&source.id, &input_dir.join(format!("{}.wav", source.id))))
        .collect()
}

fn make_pairs(sources: &[SourceEntry]) -> Vec<Pair> {
    let mut pairs = Vec::with_capacity(PAIR_COUNT);
    for left in 0..sources.len() {
        for right in left + 1..sources.len() {
            pairs.push(Pair {
                left,
                right,
                slug: format!("{}__{}", sources[left].id, sources[right].id),
            });
        }
    }
    debug_assert_eq!(pairs.len(), PAIR_COUNT);
    pairs
}

fn make_tasks(pairs: &[Pair], options: &RenderOptions) -> Result<Vec<RenderTask>> {
    if let Some(pair) = &options.pair
        && !pairs.iter().any(|candidate| candidate.slug == *pair)
    {
        bail!("unknown pair {pair}");
    }
    let algorithms = options
        .algorithm
        .map(|value| vec![value])
        .unwrap_or_else(|| Algorithm::ALL.to_vec());
    let presets = options
        .preset
        .map(|value| vec![value])
        .unwrap_or_else(|| WindowPreset::ALL.to_vec());
    let mut tasks = Vec::new();
    for pair in pairs {
        if options
            .pair
            .as_ref()
            .is_some_and(|selected| selected != &pair.slug)
        {
            continue;
        }
        for &algorithm in &algorithms {
            for &preset in &presets {
                tasks.push(RenderTask {
                    pair: pair.clone(),
                    algorithm,
                    preset,
                });
            }
        }
    }
    Ok(tasks)
}

fn output_path(root: &Path, task: &RenderTask) -> PathBuf {
    root.join(relative_output_path(task))
}

fn relative_output_path(task: &RenderTask) -> String {
    format!(
        "{}/{}/{}.wav",
        task.algorithm.slug(),
        task.preset.slug(),
        task.pair.slug
    )
}

fn write_metrics(path: &Path, rows: &[MetricsRow]) -> Result<()> {
    let temporary = path.with_extension("csv.part");
    let mut writer = csv::Writer::from_path(&temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    writer.write_record([
        "pair",
        "left",
        "right",
        "algorithm",
        "preset",
        "path",
        "frames",
        "duration_seconds",
        "peak",
        "rms",
        "rms_dbfs",
        "dc_offset",
        "clipped_samples",
        "non_finite_samples",
    ])?;
    let mut sorted = rows.to_vec();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    for row in sorted {
        writer.write_record([
            row.pair,
            row.left,
            row.right,
            row.algorithm.slug().to_owned(),
            row.preset.slug().to_owned(),
            row.path,
            row.metrics.frames.to_string(),
            row.metrics.duration_seconds.to_string(),
            row.metrics.peak.to_string(),
            row.metrics.rms.to_string(),
            row.metrics.rms_dbfs.to_string(),
            row.metrics.dc_offset.to_string(),
            row.metrics.clipped_samples.to_string(),
            row.metrics.non_finite_samples.to_string(),
        ])?;
    }
    writer.flush()?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn write_catalog(root: &Path, sources: Vec<SourceEntry>, mut clips: Vec<MetricsRow>) -> Result<()> {
    clips.sort_by(|a, b| a.path.cmp(&b.path));
    let catalog = Catalog {
        schema_version: 1,
        generated_by: "conv9",
        sample_rate: SAMPLE_RATE,
        channels: 1,
        output_seconds: 60,
        expected_pairs: PAIR_COUNT,
        expected_outputs: OUTPUT_COUNT,
        sources,
        algorithms: Algorithm::ALL
            .into_iter()
            .map(|algorithm| AlgorithmCatalogEntry {
                id: algorithm,
                title: algorithm.title(),
                rank: algorithm.rank(),
            })
            .collect(),
        presets: WindowPreset::ALL
            .into_iter()
            .map(|preset| PresetCatalogEntry {
                id: preset,
                title: preset.title(),
                config: preset.config(),
            })
            .collect(),
        clips,
    };
    let path = root.join("catalog.json");
    let temporary = root.join("catalog.json.part");
    fs::write(&temporary, serde_json::to_vec_pretty(&catalog)?)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn count_wavs(root: &Path) -> Result<usize> {
    fn recurse(path: &Path, count: &mut usize) -> Result<()> {
        for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                recurse(&entry.path(), count)?;
            } else if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "wav")
            {
                *count += 1;
            }
        }
        Ok(())
    }
    let mut count = 0;
    recurse(root, &mut count)?;
    Ok(count)
}
