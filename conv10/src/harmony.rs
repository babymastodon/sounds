use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::audio::AudioClip;
use crate::convolution::PairJob;
use crate::manifest::{SourceEntry, is_long_duration, is_short_duration};
use crate::pitch::Chord;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Debug, Deserialize)]
pub struct SongConfig {
    pub schema_version: u32,
    pub name: String,
    pub samples: Vec<ConfigSample>,
    pub harmony: HarmonyConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ConfigSample {
    pub id: String,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HarmonyConfig {
    pub register: RegisterConfig,
    pub allowed_inversions: Vec<usize>,
    pub tunings: Vec<TuningConfig>,
    pub palettes: Vec<PaletteConfig>,
    pub scenes: Vec<SceneConfig>,
    pub progression: Vec<ProgressionStep>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RegisterConfig {
    pub minimum_hz: f64,
    pub maximum_hz: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TuningConfig {
    pub name: String,
    pub kind: TuningKind,
    pub base_frequency_hz: f64,
    pub period_ratio: f64,
    pub divisions: Option<usize>,
    pub ratios: Option<Vec<RatioSpec>>,
    pub detune_limit_fraction: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TuningKind {
    EqualDivision,
    Ratios,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum RatioSpec {
    Number(f64),
    Fraction(String),
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaletteConfig {
    pub name: String,
    pub tuning: String,
    pub root_pool: Vec<i32>,
    pub chords: Vec<ChordConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChordConfig {
    pub name: String,
    pub degrees: [i32; 3],
    pub weight: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SceneConfig {
    pub name: String,
    pub palette_weights: Vec<PaletteWeight>,
    pub motif: Vec<String>,
    pub motif_every_pairs: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaletteWeight {
    pub palette: String,
    pub weight: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProgressionStep {
    pub scene: String,
    pub pair_count: usize,
}

#[derive(Clone, Debug)]
pub struct LoadedSongConfig {
    pub config: SongConfig,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct HarmonyAssignment {
    pub sequence_index: usize,
    pub scene: String,
    pub palette: String,
    pub tuning: String,
    pub chord_name: String,
    pub root_degree: i32,
    pub inversion: usize,
    pub selection_hash: String,
    pub chord: Chord,
}

#[derive(Debug)]
pub struct HarmonySchedule {
    assignments: HashMap<(usize, usize), HarmonyAssignment>,
}

impl HarmonySchedule {
    pub fn assignment(&self, job: &PairJob) -> Result<&HarmonyAssignment> {
        self.assignments
            .get(&(job.left, job.right))
            .with_context(|| format!("missing harmony assignment for {}-{}", job.left, job.right))
    }
}

#[derive(Clone, Debug)]
enum Tuning {
    EqualDivision {
        divisions: usize,
        period_ratio: f64,
        base_frequency_hz: f64,
        detune_limit_cents: f32,
    },
    Ratios {
        ratios: Vec<f64>,
        period_ratio: f64,
        base_frequency_hz: f64,
        detune_limit_cents: f32,
    },
}

impl Tuning {
    fn steps_per_period(&self) -> i32 {
        match self {
            Self::EqualDivision { divisions, .. } => *divisions as i32,
            Self::Ratios { ratios, .. } => ratios.len() as i32,
        }
    }

    fn period_ratio(&self) -> f64 {
        match self {
            Self::EqualDivision { period_ratio, .. } | Self::Ratios { period_ratio, .. } => {
                *period_ratio
            }
        }
    }

    fn base_frequency_hz(&self) -> f64 {
        match self {
            Self::EqualDivision {
                base_frequency_hz, ..
            }
            | Self::Ratios {
                base_frequency_hz, ..
            } => *base_frequency_hz,
        }
    }

    fn detune_limit_cents(&self) -> f32 {
        match self {
            Self::EqualDivision {
                detune_limit_cents, ..
            }
            | Self::Ratios {
                detune_limit_cents, ..
            } => *detune_limit_cents,
        }
    }

    fn frequency(&self, degree: i32) -> f64 {
        let steps = self.steps_per_period();
        let periods = degree.div_euclid(steps);
        let within = degree.rem_euclid(steps);
        let ratio = match self {
            Self::EqualDivision {
                divisions,
                period_ratio,
                ..
            } => period_ratio.powf(within as f64 / *divisions as f64),
            Self::Ratios { ratios, .. } => ratios[within as usize],
        };
        self.base_frequency_hz() * self.period_ratio().powi(periods) * ratio
    }
}

pub fn load_song_config(path: &Path) -> Result<LoadedSongConfig> {
    let bytes = fs::read(path).with_context(|| format!("read song config {}", path.display()))?;
    let config: SongConfig =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    validate_config(&config)?;
    Ok(LoadedSongConfig {
        config,
        fingerprint: format!("{:016x}", fingerprint(&bytes)),
    })
}

pub fn validate_manifest_matches_config(
    config: &SongConfig,
    sources: &[SourceEntry],
) -> Result<()> {
    if config.samples.len() != sources.len() {
        bail!(
            "{} config contains {} samples, manifest contains {}",
            config.name,
            config.samples.len(),
            sources.len()
        );
    }
    for (sample, source) in config.samples.iter().zip(sources) {
        let expected_role = if is_short_duration(source.seconds) {
            "short"
        } else if is_long_duration(source.seconds) {
            "long"
        } else {
            bail!("{} has unsupported duration {}", source.id, source.seconds);
        };
        if sample.id != source.id || sample.role != expected_role {
            bail!(
                "{} config sample {} ({}) does not match manifest {} ({})",
                config.name,
                sample.id,
                sample.role,
                source.id,
                expected_role
            );
        }
    }
    Ok(())
}

pub fn build_schedule(
    loaded: &LoadedSongConfig,
    clips: &[AudioClip],
    jobs: &[PairJob],
) -> Result<HarmonySchedule> {
    let config = &loaded.config;
    let tunings = build_tunings(&config.harmony)?;
    let palettes = config
        .harmony
        .palettes
        .iter()
        .map(|palette| (palette.name.as_str(), palette))
        .collect::<HashMap<_, _>>();
    let scenes = config
        .harmony
        .scenes
        .iter()
        .map(|scene| (scene.name.as_str(), scene))
        .collect::<HashMap<_, _>>();
    let chord_lookup = config
        .harmony
        .palettes
        .iter()
        .flat_map(|palette| {
            palette
                .chords
                .iter()
                .map(move |chord| (chord.name.as_str(), (palette, chord)))
        })
        .collect::<HashMap<_, _>>();
    let expected_pairs = config
        .harmony
        .progression
        .iter()
        .map(|step| step.pair_count)
        .sum::<usize>();
    if expected_pairs != jobs.len() {
        bail!(
            "{} progression contains {expected_pairs} pairs, matrix contains {}",
            config.name,
            jobs.len()
        );
    }

    // Motifs guarantee the core vocabulary. Reserve filename-selected free slots for
    // every remaining configured shape so a long render cannot accidentally omit a
    // low-weight transition chord.
    let motif_chords = config
        .harmony
        .scenes
        .iter()
        .flat_map(|scene| scene.motif.iter().map(String::as_str))
        .collect::<HashSet<_>>();
    let mut coverage = HashMap::new();
    for (palette, chord) in config
        .harmony
        .palettes
        .iter()
        .flat_map(|palette| palette.chords.iter().map(move |chord| (palette, chord)))
    {
        if motif_chords.contains(chord.name.as_str()) {
            continue;
        }
        let mut candidates = Vec::new();
        let mut candidate_ordinal = 0_usize;
        for step in &config.harmony.progression {
            let scene = scenes[step.scene.as_str()];
            let palette_is_available = scene
                .palette_weights
                .iter()
                .any(|weight| weight.palette == palette.name);
            for local_index in 0..step.pair_count {
                if palette_is_available
                    && local_index % scene.motif_every_pairs >= scene.motif.len()
                    && !coverage.contains_key(&candidate_ordinal)
                {
                    candidates.push(candidate_ordinal);
                }
                candidate_ordinal += 1;
            }
        }
        if candidates.is_empty() {
            bail!(
                "{} has no free progression slot for chord {} in palette {}",
                config.name,
                chord.name,
                palette.name
            );
        }
        let mut seed = FNV_OFFSET_BASIS;
        hash_bytes(&mut seed, b"conv10-config-harmony-coverage-v1\0");
        hash_bytes(&mut seed, config.name.as_bytes());
        hash_bytes(&mut seed, palette.name.as_bytes());
        hash_bytes(&mut seed, chord.name.as_bytes());
        let ordinal = candidates[seed as usize % candidates.len()];
        coverage.insert(ordinal, (palette, chord));
    }

    let mut assignments = HashMap::with_capacity(jobs.len());
    let mut ordinal = 0_usize;
    for (occurrence, step) in config.harmony.progression.iter().enumerate() {
        let scene = scenes[step.scene.as_str()];
        for local_index in 0..step.pair_count {
            let job = &jobs[ordinal];
            let short_name = &clips[job.left].id;
            let long_name = &clips[job.right].id;
            let seed = filename_seed(
                &config.name,
                &scene.name,
                occurrence,
                local_index,
                short_name,
                long_name,
            );
            let motif_index = local_index % scene.motif_every_pairs;
            let (palette, chord_config) = if let Some(&covered) = coverage.get(&ordinal) {
                covered
            } else if motif_index < scene.motif.len() {
                chord_lookup[scene.motif[motif_index].as_str()]
            } else {
                let palette_weight = choose_weighted(
                    &scene.palette_weights,
                    derived_hash(seed, b"palette"),
                    |item| item.weight,
                )?;
                let palette = palettes[palette_weight.palette.as_str()];
                let chord =
                    choose_weighted(&palette.chords, derived_hash(seed, b"chord"), |item| {
                        item.weight
                    })?;
                (palette, chord)
            };
            let tuning = &tunings[palette.tuning.as_str()];
            let root_degree =
                *choose_uniform(&palette.root_pool, derived_hash(seed, b"root"), "root pool")?;
            let inversion = *choose_uniform(
                &config.harmony.allowed_inversions,
                derived_hash(seed, b"inversion"),
                "allowed inversions",
            )?;
            let chord = realize_chord(
                tuning,
                root_degree,
                chord_config.degrees,
                inversion,
                config.harmony.register,
            )?;
            let assignment = HarmonyAssignment {
                sequence_index: ordinal,
                scene: scene.name.clone(),
                palette: palette.name.clone(),
                tuning: palette.tuning.clone(),
                chord_name: chord_config.name.clone(),
                root_degree,
                inversion,
                selection_hash: format!("{seed:016x}"),
                chord,
            };
            if assignments
                .insert((job.left, job.right), assignment)
                .is_some()
            {
                bail!(
                    "duplicate harmony assignment for {}-{}",
                    job.left,
                    job.right
                );
            }
            ordinal += 1;
        }
    }
    Ok(HarmonySchedule { assignments })
}

pub fn tuning_summary(config: &HarmonyConfig) -> String {
    config
        .tunings
        .iter()
        .map(|tuning| match tuning.kind {
            TuningKind::EqualDivision => format!(
                "{}={}ED{}",
                tuning.name,
                tuning.divisions.unwrap_or_default(),
                tuning.period_ratio
            ),
            TuningKind::Ratios => format!(
                "{}={} ratios/{}",
                tuning.name,
                tuning.ratios.as_ref().map_or(0, Vec::len),
                tuning.period_ratio
            ),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn configured_chord_count(config: &HarmonyConfig) -> usize {
    config
        .palettes
        .iter()
        .map(|palette| palette.chords.len())
        .sum()
}

fn validate_config(config: &SongConfig) -> Result<()> {
    if config.schema_version != 1 {
        bail!(
            "{} uses schema version {}, expected 1",
            config.name,
            config.schema_version
        );
    }
    validate_identifier(&config.name, "song name")?;
    if config.samples.is_empty() {
        bail!("{} has no samples", config.name);
    }
    let mut sample_ids = HashSet::new();
    let mut short_count = 0;
    let mut long_count = 0;
    for sample in &config.samples {
        validate_identifier(&sample.id, "sample id")?;
        if !sample_ids.insert(sample.id.as_str()) {
            bail!("{} has duplicate sample id {}", config.name, sample.id);
        }
        match sample.role.as_str() {
            "short" => short_count += 1,
            "long" => long_count += 1,
            other => bail!("{} has invalid sample role {other}", sample.id),
        }
    }
    if short_count == 0 || long_count == 0 {
        bail!("{} needs both short and long samples", config.name);
    }
    let harmony = &config.harmony;
    if !harmony.register.minimum_hz.is_finite()
        || !harmony.register.maximum_hz.is_finite()
        || harmony.register.minimum_hz <= 0.0
        || harmony.register.maximum_hz <= harmony.register.minimum_hz
    {
        bail!("{} has an invalid harmony register", config.name);
    }
    if harmony.allowed_inversions.is_empty()
        || harmony.allowed_inversions.iter().any(|&value| value > 2)
    {
        bail!(
            "{} inversions must be a non-empty subset of 0, 1, 2",
            config.name
        );
    }
    let tunings = build_tunings(harmony)?;
    let mut palette_names = HashSet::new();
    let mut chord_names = HashSet::new();
    for palette in &harmony.palettes {
        validate_identifier(&palette.name, "palette name")?;
        if !palette_names.insert(palette.name.as_str()) {
            bail!("{} has duplicate palette {}", config.name, palette.name);
        }
        let tuning = tunings.get(palette.tuning.as_str()).with_context(|| {
            format!(
                "{} references missing tuning {}",
                palette.name, palette.tuning
            )
        })?;
        let period_steps = tuning.steps_per_period();
        if palette.root_pool.is_empty()
            || palette
                .root_pool
                .iter()
                .any(|&root| root < 0 || root >= period_steps)
        {
            bail!(
                "{} root pool must contain degrees within one tuning period",
                palette.name
            );
        }
        if palette.chords.is_empty() {
            bail!("{} contains no chords", palette.name);
        }
        for chord in &palette.chords {
            validate_identifier(&chord.name, "chord name")?;
            if !chord_names.insert(chord.name.as_str()) {
                bail!("{} has duplicate chord name {}", config.name, chord.name);
            }
            if chord.weight == 0
                || chord.degrees[0] != 0
                || chord.degrees[0] >= chord.degrees[1]
                || chord.degrees[1] >= chord.degrees[2]
                || chord.degrees[2] > period_steps * 2
            {
                bail!("{} has invalid chord degrees or weight", chord.name);
            }
            for &root in &palette.root_pool {
                for &inversion in &harmony.allowed_inversions {
                    realize_chord(tuning, root, chord.degrees, inversion, harmony.register)
                        .with_context(|| {
                            format!(
                                "validate chord {} at root {} inversion {}",
                                chord.name, root, inversion
                            )
                        })?;
                }
            }
        }
    }
    let mut scene_names = HashSet::new();
    for scene in &harmony.scenes {
        validate_identifier(&scene.name, "scene name")?;
        if !scene_names.insert(scene.name.as_str()) {
            bail!("{} has duplicate scene {}", config.name, scene.name);
        }
        if scene.palette_weights.is_empty()
            || scene
                .palette_weights
                .iter()
                .any(|item| item.weight == 0 || !palette_names.contains(item.palette.as_str()))
        {
            bail!("{} has invalid palette weights", scene.name);
        }
        if scene.motif.is_empty()
            || scene.motif_every_pairs == 0
            || scene.motif.len() > scene.motif_every_pairs
            || scene
                .motif
                .iter()
                .any(|name| !chord_names.contains(name.as_str()))
        {
            bail!("{} has an invalid motif", scene.name);
        }
    }
    if harmony.progression.is_empty()
        || harmony
            .progression
            .iter()
            .any(|step| step.pair_count == 0 || !scene_names.contains(step.scene.as_str()))
    {
        bail!("{} has an invalid scene progression", config.name);
    }
    Ok(())
}

fn build_tunings(harmony: &HarmonyConfig) -> Result<HashMap<&str, Tuning>> {
    let mut result = HashMap::new();
    for config in &harmony.tunings {
        validate_identifier(&config.name, "tuning name")?;
        if result.contains_key(config.name.as_str())
            || !config.base_frequency_hz.is_finite()
            || config.base_frequency_hz <= 0.0
            || !config.period_ratio.is_finite()
            || config.period_ratio <= 1.0
            || !config.detune_limit_fraction.is_finite()
            || !(0.0..=1.0).contains(&config.detune_limit_fraction)
        {
            bail!("invalid tuning {}", config.name);
        }
        let tuning = match config.kind {
            TuningKind::EqualDivision => {
                let divisions = config
                    .divisions
                    .filter(|&value| value >= 2)
                    .with_context(|| format!("{} needs at least two divisions", config.name))?;
                if config.ratios.is_some() {
                    bail!(
                        "{} equal division tuning cannot contain ratios",
                        config.name
                    );
                }
                let step_cents = 1_200.0 * config.period_ratio.log2() / divisions as f64;
                Tuning::EqualDivision {
                    divisions,
                    period_ratio: config.period_ratio,
                    base_frequency_hz: config.base_frequency_hz,
                    detune_limit_cents: (step_cents * config.detune_limit_fraction) as f32,
                }
            }
            TuningKind::Ratios => {
                if config.divisions.is_some() {
                    bail!("{} ratio tuning cannot contain divisions", config.name);
                }
                let specs = config
                    .ratios
                    .as_ref()
                    .with_context(|| format!("{} needs ratios", config.name))?;
                let ratios = specs.iter().map(parse_ratio).collect::<Result<Vec<_>>>()?;
                if ratios.len() < 2
                    || (ratios[0] - 1.0).abs() > 1.0e-9
                    || ratios
                        .windows(2)
                        .any(|pair| pair[0] >= pair[1] || !pair[1].is_finite())
                    || ratios
                        .last()
                        .is_some_and(|&ratio| ratio >= config.period_ratio)
                {
                    bail!(
                        "{} ratios must increase from 1 within the period",
                        config.name
                    );
                }
                let mut adjacent_cents = ratios
                    .windows(2)
                    .map(|pair| 1_200.0 * (pair[1] / pair[0]).log2())
                    .collect::<Vec<_>>();
                adjacent_cents
                    .push(1_200.0 * (config.period_ratio / ratios[ratios.len() - 1]).log2());
                let minimum_step = adjacent_cents.into_iter().fold(f64::INFINITY, f64::min);
                Tuning::Ratios {
                    ratios,
                    period_ratio: config.period_ratio,
                    base_frequency_hz: config.base_frequency_hz,
                    detune_limit_cents: (minimum_step * config.detune_limit_fraction) as f32,
                }
            }
        };
        result.insert(config.name.as_str(), tuning);
    }
    if result.is_empty() {
        bail!("harmony needs at least one tuning");
    }
    Ok(result)
}

fn parse_ratio(spec: &RatioSpec) -> Result<f64> {
    let value = match spec {
        RatioSpec::Number(value) => *value,
        RatioSpec::Fraction(value) => {
            let (numerator, denominator) = value
                .split_once('/')
                .with_context(|| format!("invalid ratio {value:?}"))?;
            numerator.trim().parse::<f64>()? / denominator.trim().parse::<f64>()?
        }
    };
    if !value.is_finite() || value <= 0.0 {
        bail!("ratio must be positive and finite");
    }
    Ok(value)
}

fn realize_chord(
    tuning: &Tuning,
    root_degree: i32,
    intervals: [i32; 3],
    inversion: usize,
    register: RegisterConfig,
) -> Result<Chord> {
    let period_steps = tuning.steps_per_period();
    let mut degrees = intervals.map(|degree| root_degree + degree);
    for _ in 0..inversion {
        degrees[0] += period_steps;
        degrees.sort_unstable();
    }
    for _ in 0..8 {
        let frequencies = degrees.map(|degree| tuning.frequency(degree));
        let minimum = frequencies.into_iter().fold(f64::INFINITY, f64::min);
        if minimum >= register.minimum_hz {
            break;
        }
        degrees = degrees.map(|degree| degree + period_steps);
    }
    for _ in 0..8 {
        let frequencies = degrees.map(|degree| tuning.frequency(degree));
        let maximum = frequencies.into_iter().fold(f64::NEG_INFINITY, f64::max);
        if maximum <= register.maximum_hz {
            break;
        }
        degrees = degrees.map(|degree| degree - period_steps);
    }
    let frequencies = degrees.map(|degree| tuning.frequency(degree));
    if frequencies.iter().any(|frequency| {
        !frequency.is_finite()
            || *frequency < register.minimum_hz
            || *frequency > register.maximum_hz
    }) {
        bail!(
            "chord cannot fit register {:.3}..{:.3} Hz",
            register.minimum_hz,
            register.maximum_hz
        );
    }
    Ok(Chord {
        steps: degrees,
        frequencies_hz: frequencies.map(|frequency| frequency as f32),
        detune_limit_cents: tuning.detune_limit_cents(),
    })
}

fn choose_weighted<T>(items: &[T], hash: u64, weight: impl Fn(&T) -> u32) -> Result<&T> {
    let total = items
        .iter()
        .map(|item| u64::from(weight(item)))
        .sum::<u64>();
    if items.is_empty() || total == 0 {
        bail!("cannot choose from an empty or zero-weight collection");
    }
    let mut selected = hash % total;
    for item in items {
        let item_weight = u64::from(weight(item));
        if selected < item_weight {
            return Ok(item);
        }
        selected -= item_weight;
    }
    unreachable!("weighted selection is bounded by total weight")
}

fn choose_uniform<'a, T>(items: &'a [T], hash: u64, label: &str) -> Result<&'a T> {
    if items.is_empty() {
        bail!("{label} is empty");
    }
    Ok(&items[hash as usize % items.len()])
}

fn filename_seed(
    song: &str,
    scene: &str,
    occurrence: usize,
    local_index: usize,
    short_name: &str,
    long_name: &str,
) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, b"conv10-config-harmony-v1\0");
    for value in [song, scene, short_name, long_name] {
        hash_bytes(&mut hash, &(value.len() as u64).to_le_bytes());
        hash_bytes(&mut hash, value.as_bytes());
    }
    hash_bytes(&mut hash, &(occurrence as u64).to_le_bytes());
    hash_bytes(&mut hash, &(local_index as u64).to_le_bytes());
    hash
}

fn derived_hash(seed: u64, domain: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, b"conv10-config-harmony-derived-v1\0");
    hash_bytes(&mut hash, &seed.to_le_bytes());
    hash_bytes(&mut hash, domain);
    hash
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, b"conv10-song-config-v1\0");
    hash_bytes(&mut hash, &(bytes.len() as u64).to_le_bytes());
    hash_bytes(&mut hash, bytes);
    hash
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        bail!("{label} must contain only lowercase ASCII, digits, and _");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> Tuning {
        Tuning::EqualDivision {
            divisions: 19,
            period_ratio: 2.0,
            base_frequency_hz: 110.0,
            detune_limit_cents: 18.0,
        }
    }

    #[test]
    fn equal_division_frequencies_preserve_period_carries() {
        let tuning = tuning();
        let base = tuning.frequency(0);
        assert!((tuning.frequency(19) / base - 2.0).abs() < 1.0e-9);
        assert!((tuning.frequency(-19) / base - 0.5).abs() < 1.0e-9);
    }

    #[test]
    fn inversions_fit_the_configured_register() {
        let chord = realize_chord(
            &tuning(),
            17,
            [0, 6, 11],
            2,
            RegisterConfig {
                minimum_hz: 82.5,
                maximum_hz: 440.0,
            },
        )
        .unwrap();
        assert!(
            chord
                .frequencies_hz
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(
            chord
                .frequencies_hz
                .iter()
                .all(|&frequency| (82.5..=440.0).contains(&frequency))
        );
    }

    #[test]
    fn filename_selection_is_stable_and_ordered() {
        let first = filename_seed("drift", "a", 0, 0, "rain", "river");
        assert_eq!(first, filename_seed("drift", "a", 0, 0, "rain", "river"));
        assert_ne!(first, filename_seed("drift", "a", 0, 0, "river", "rain"));
    }

    #[test]
    fn fraction_ratios_are_readable_and_exact() {
        assert_eq!(
            parse_ratio(&RatioSpec::Fraction("3/2".to_owned())).unwrap(),
            1.5
        );
    }

    #[test]
    fn every_checked_in_song_config_builds_a_complete_schedule() {
        let config_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("configs");
        let mut paths = fs::read_dir(config_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        assert_eq!(paths.len(), 14);

        for path in paths {
            let loaded = load_song_config(&path)
                .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
            assert_eq!(loaded.config.samples.len(), 48);
            assert!(
                loaded
                    .config
                    .harmony
                    .tunings
                    .iter()
                    .all(|tuning| tuning.kind != TuningKind::EqualDivision
                        || tuning.divisions != Some(12)),
                "{} contains a 12-EDO tuning",
                path.display()
            );
            for tuning in loaded.config.harmony.tunings.iter().filter(|tuning| {
                tuning.kind == TuningKind::EqualDivision
                    && tuning
                        .divisions
                        .is_some_and(|divisions| divisions % 12 == 0)
            }) {
                let divisions = tuning.divisions.unwrap();
                let twelve_edo_stride = (divisions / 12) as i32;
                assert_ne!(divisions, 12, "{} contains a 12-EDO tuning", path.display());
                assert!(
                    loaded
                        .config
                        .harmony
                        .palettes
                        .iter()
                        .filter(|palette| palette.tuning == tuning.name)
                        .any(|palette| {
                            palette
                                .root_pool
                                .iter()
                                .any(|degree| degree % twelve_edo_stride != 0)
                                || palette.chords.iter().any(|chord| {
                                    chord
                                        .degrees
                                        .iter()
                                        .any(|degree| degree % twelve_edo_stride != 0)
                                })
                        }),
                    "{} uses {}-EDO only as a doubled 12-EDO grid",
                    path.display(),
                    divisions
                );
            }
            let clips = loaded
                .config
                .samples
                .iter()
                .map(|sample| AudioClip {
                    id: sample.id.clone(),
                    samples: Vec::new(),
                })
                .collect::<Vec<_>>();
            let short = loaded
                .config
                .samples
                .iter()
                .enumerate()
                .filter_map(|(index, sample)| (sample.role == "short").then_some(index))
                .collect::<Vec<_>>();
            let long = loaded
                .config
                .samples
                .iter()
                .enumerate()
                .filter_map(|(index, sample)| (sample.role == "long").then_some(index))
                .collect::<Vec<_>>();
            assert_eq!(short.len(), 24);
            assert_eq!(long.len(), 24);
            let jobs = short
                .iter()
                .flat_map(|&left| {
                    long.iter().map(move |&right| PairJob {
                        left,
                        right,
                        trim_frames: 1,
                        output_frames: 2,
                        fft_len: 2,
                    })
                })
                .collect::<Vec<_>>();
            let schedule = build_schedule(&loaded, &clips, &jobs).unwrap();
            assert_eq!(schedule.assignments.len(), 576);
            let chord_names = schedule
                .assignments
                .values()
                .map(|assignment| assignment.chord_name.as_str())
                .collect::<HashSet<_>>();
            let voicings = schedule
                .assignments
                .values()
                .map(|assignment| {
                    (
                        assignment.tuning.as_str(),
                        assignment.chord_name.as_str(),
                        assignment.root_degree,
                        assignment.inversion,
                    )
                })
                .collect::<HashSet<_>>();
            let pitch_sets = schedule
                .assignments
                .values()
                .map(|assignment| (assignment.tuning.as_str(), assignment.chord.steps))
                .collect::<HashSet<_>>();
            let roots = schedule
                .assignments
                .values()
                .map(|assignment| (assignment.tuning.as_str(), assignment.root_degree))
                .collect::<HashSet<_>>();
            let inversions = schedule
                .assignments
                .values()
                .map(|assignment| assignment.inversion)
                .collect::<HashSet<_>>();
            let mut chord_counts = HashMap::new();
            let mut voicing_counts = HashMap::new();
            let mut pitch_set_counts = HashMap::new();
            for assignment in schedule.assignments.values() {
                *chord_counts
                    .entry(assignment.chord_name.as_str())
                    .or_insert(0_usize) += 1;
                *voicing_counts
                    .entry((
                        assignment.tuning.as_str(),
                        assignment.chord_name.as_str(),
                        assignment.root_degree,
                        assignment.inversion,
                    ))
                    .or_insert(0_usize) += 1;
                *pitch_set_counts
                    .entry((assignment.tuning.as_str(), assignment.chord.steps))
                    .or_insert(0_usize) += 1;
            }
            let maximum_chord_uses = chord_counts.values().copied().max().unwrap();
            let maximum_voicing_uses = voicing_counts.values().copied().max().unwrap();
            let maximum_pitch_set_uses = pitch_set_counts.values().copied().max().unwrap();
            eprintln!(
                "{}: {} chord shapes, {} realized voicings, {} pitch sets, {} tuning/root pairs, {} inversions, max shape {}, max voicing {}, max pitch set {}",
                loaded.config.name,
                chord_names.len(),
                voicings.len(),
                pitch_sets.len(),
                roots.len(),
                inversions.len(),
                maximum_chord_uses,
                maximum_voicing_uses,
                maximum_pitch_set_uses
            );
            assert_eq!(
                chord_names.len(),
                configured_chord_count(&loaded.config.harmony),
                "{} does not realize every configured chord shape",
                loaded.config.name
            );
            assert!(
                voicings.len() >= 200,
                "{} realizes only {} named chord/root/inversion combinations",
                loaded.config.name,
                voicings.len()
            );
            assert!(
                pitch_sets.len() >= 160,
                "{} realizes only {} distinct pitch sets",
                loaded.config.name,
                pitch_sets.len()
            );
            assert!(
                roots.len() >= 6,
                "{} realizes only {} tuning/root pairs",
                loaded.config.name,
                roots.len()
            );
            assert_eq!(
                inversions,
                loaded
                    .config
                    .harmony
                    .allowed_inversions
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
                "{} does not realize every configured inversion",
                loaded.config.name
            );
            assert!(
                maximum_chord_uses <= 144,
                "{} lets one named shape occupy more than 25% of the piece",
                loaded.config.name
            );
            assert!(
                maximum_pitch_set_uses <= 18,
                "{} repeats one exact pitch set more than 18 times",
                loaded.config.name
            );
            let register = loaded.config.harmony.register;
            assert!(schedule.assignments.values().all(|assignment| {
                assignment.chord.frequencies_hz.iter().all(|frequency| {
                    (register.minimum_hz as f32..=register.maximum_hz as f32).contains(frequency)
                })
            }));
        }
    }
}
