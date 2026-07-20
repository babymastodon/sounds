use std::f32::consts::PI;

use crate::audio::SAMPLE_RATE;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const CHORD_COUNT: usize = 13;
const BASE_FREQUENCY_HZ: f32 = 110.0;
const CHORD_INTERVALS: [usize; 3] = [0, 6, 10];
const SEQUENCE_PATTERN: [usize; 8] = [0, 1, 2, 1, 0, 2, 0, 1];
const SEQUENCE_ACCENTS: [f32; 8] = [1.0, 0.72, 0.86, 0.68, 1.0, 0.76, 0.9, 0.7];
const SEQUENCE_BPM: f32 = 96.0;
const PURE_WET_MIX: f32 = 0.03;
const SEQUENCED_WET_MIX: f32 = 0.04;
const HYBRID_WET_MIX: f32 = 0.025;
const ADDITIVE_NOTE_DB_BELOW_LOCAL: f32 = 6.0;
const ADDITIVE_NOTE_SECONDS: f32 = 0.25;
const ADDITIVE_INTERVAL_SECONDS: f32 = 1.25;
pub const ALGORITHM_VERSION: &str = "subtle-parallel-v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitchApproach {
    PureConvolution,
    SequencedConvolution,
    HybridSpectral,
    LongAdditiveSynth,
}

impl PitchApproach {
    pub const ALL: [Self; 4] = [
        Self::PureConvolution,
        Self::SequencedConvolution,
        Self::HybridSpectral,
        Self::LongAdditiveSynth,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::PureConvolution => "pure_convolution",
            Self::SequencedConvolution => "sequenced_convolution",
            Self::HybridSpectral => "hybrid_spectral",
            Self::LongAdditiveSynth => "long_additive_synth",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::PureConvolution => "subtle parallel three-note IIR convolutional resonator",
            Self::SequencedConvolution => {
                "subtle parallel 96 BPM grains through arpeggiated single-note resonators"
            }
            Self::HybridSpectral => {
                "subtle parallel ring modulation followed by a convolutional resonator bank"
            }
            Self::LongAdditiveSynth => {
                "locally leveled short additive notes mixed into the unfiltered long input"
            }
        }
    }

    pub const fn parallel_wet_mix(self) -> Option<f32> {
        match self {
            Self::PureConvolution => Some(PURE_WET_MIX),
            Self::SequencedConvolution => Some(SEQUENCED_WET_MIX),
            Self::HybridSpectral => Some(HYBRID_WET_MIX),
            Self::LongAdditiveSynth => None,
        }
    }

    pub const fn additive_note_db_below_local(self) -> Option<f32> {
        match self {
            Self::LongAdditiveSynth => Some(ADDITIVE_NOTE_DB_BELOW_LOCAL),
            _ => None,
        }
    }

    pub const fn processed_role(self) -> &'static str {
        match self {
            Self::LongAdditiveSynth => "long",
            _ => "short",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Chord {
    pub index: usize,
    pub steps: [usize; 3],
    pub frequencies_hz: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct PreprocessedClip {
    pub samples: Vec<f32>,
    pub parallel_wet_mix_percent: Option<f32>,
    pub additive_note_db_below_local: Option<f32>,
    pub dry_correlation: f32,
    pub difference_rms_db_relative: f32,
}

pub fn fingerprint_bytes(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, b"conv8-prepared-wav-bytes-v1\0");
    hash_bytes(&mut hash, &(bytes.len() as u64).to_le_bytes());
    hash_bytes(&mut hash, bytes);
    hash
}

pub fn chord_index(short_fingerprint: u64, long_fingerprint: u64) -> usize {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, b"conv8-bohlen-pierce-pair-v1\0");
    hash_bytes(&mut hash, &short_fingerprint.to_le_bytes());
    hash_bytes(&mut hash, &long_fingerprint.to_le_bytes());
    hash as usize % CHORD_COUNT
}

pub fn fingerprint_hex(fingerprint: u64) -> String {
    format!("{fingerprint:016x}")
}

pub fn chord(index: usize) -> Chord {
    let index = index % CHORD_COUNT;
    let steps = CHORD_INTERVALS.map(|interval| (index + interval) % CHORD_COUNT);
    let frequencies_hz = steps.map(step_frequency_hz);
    Chord {
        index,
        steps,
        frequencies_hz,
    }
}

pub fn preprocess(input: &[f32], chord: Chord, approach: PitchApproach) -> PreprocessedClip {
    if approach == PitchApproach::LongAdditiveSynth {
        return additive_synth_voice(input, chord);
    }
    let mut effect = match approach {
        PitchApproach::PureConvolution => resonator_bank(input, &chord.frequencies_hz, 0.18),
        PitchApproach::SequencedConvolution => sequenced_voice(input, chord),
        PitchApproach::HybridSpectral => hybrid_voice(input, chord),
        PitchApproach::LongAdditiveSynth => unreachable!(),
    };
    match_rms(input, &mut effect);

    let wet = approach.parallel_wet_mix().unwrap_or(0.0);
    let dry = 1.0 - wet;
    let mut samples = input
        .iter()
        .zip(effect)
        .map(|(&input, effect)| dry.mul_add(input, wet * effect))
        .collect::<Vec<_>>();
    apply_edge_fade(&mut samples);
    match_rms(input, &mut samples);

    let dry_correlation = correlation(input, &samples);
    let difference_rms_db_relative = relative_difference_db(input, &samples);
    PreprocessedClip {
        samples,
        parallel_wet_mix_percent: Some(wet * 100.0),
        additive_note_db_below_local: None,
        dry_correlation,
        difference_rms_db_relative,
    }
}

fn additive_synth_voice(input: &[f32], chord: Chord) -> PreprocessedClip {
    let mut samples = input.to_vec();
    let interval_frames = (ADDITIVE_INTERVAL_SECONDS * SAMPLE_RATE as f32).round() as usize;
    let note_frames = (ADDITIVE_NOTE_SECONDS * SAMPLE_RATE as f32).round() as usize;
    let first_onset = interval_frames / 2;
    for (note_index, onset) in (first_onset..input.len())
        .step_by(interval_frames)
        .enumerate()
    {
        let end = (onset + note_frames).min(input.len());
        let frames = end - onset;
        let frequency = chord.frequencies_hz[SEQUENCE_PATTERN[note_index % SEQUENCE_PATTERN.len()]];
        let mut note = (0..frames)
            .map(|frame| {
                let time = frame as f32 / SAMPLE_RATE as f32;
                let attack = (time / 0.008).min(1.0);
                let decay = (-5.0 * time / ADDITIVE_NOTE_SECONDS).exp();
                let envelope = attack * attack * decay;
                let partials = (2.0 * PI * frequency * time).sin()
                    + 0.28 * (2.0 * PI * frequency * 2.01 * time + 0.3).sin()
                    + 0.11 * (2.0 * PI * frequency * 3.93 * time + 0.8).sin()
                    + 0.04 * (2.0 * PI * frequency * 6.79 * time + 1.1).sin();
                partials * envelope
            })
            .collect::<Vec<_>>();

        let local_start = onset.saturating_sub(interval_frames / 4);
        let local_end = (onset + interval_frames / 4).min(input.len());
        let local_rms = rms(&input[local_start..local_end]);
        let target_rms = local_rms * 10.0_f32.powf(-ADDITIVE_NOTE_DB_BELOW_LOCAL / 20.0);
        let note_gain = target_rms / rms(&note).max(1.0e-12);
        for (output, note) in samples[onset..end].iter_mut().zip(note.drain(..)) {
            *output += note * note_gain;
        }
    }
    apply_edge_fade(&mut samples);
    match_rms(input, &mut samples);
    PreprocessedClip {
        dry_correlation: correlation(input, &samples),
        difference_rms_db_relative: relative_difference_db(input, &samples),
        samples,
        parallel_wet_mix_percent: None,
        additive_note_db_below_local: Some(ADDITIVE_NOTE_DB_BELOW_LOCAL),
    }
}

fn rms(samples: &[f32]) -> f32 {
    (samples
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt() as f32
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn step_frequency_hz(step: usize) -> f32 {
    BASE_FREQUENCY_HZ * 3.0_f32.powf(step as f32 / CHORD_COUNT as f32)
}

fn resonator_bank(input: &[f32], frequencies_hz: &[f32], decay_seconds: f32) -> Vec<f32> {
    let mut output = vec![0.0; input.len()];
    let mix_gain = 1.0 / (frequencies_hz.len() as f32).sqrt();
    for &frequency_hz in frequencies_hz {
        let voice = resonator(input, frequency_hz, decay_seconds);
        for (output, voice) in output.iter_mut().zip(voice) {
            *output += voice * mix_gain;
        }
    }
    output
}

fn resonator(input: &[f32], frequency_hz: f32, decay_seconds: f32) -> Vec<f32> {
    let radius = (-1.0 / (decay_seconds * SAMPLE_RATE as f32)).exp();
    let coefficient = 2.0 * radius * (2.0 * PI * frequency_hz / SAMPLE_RATE as f32).cos();
    let radius_squared = radius * radius;
    let input_gain = 1.0 - radius;
    let mut previous_1 = 0.0_f32;
    let mut previous_2 = 0.0_f32;
    let mut output = Vec::with_capacity(input.len());
    for &sample in input {
        let value = input_gain.mul_add(
            sample,
            coefficient.mul_add(previous_1, -radius_squared * previous_2),
        );
        output.push(value);
        previous_2 = previous_1;
        previous_1 = value;
    }
    output
}

fn sequenced_voice(input: &[f32], chord: Chord) -> Vec<f32> {
    let hop_frames = (60.0 * SAMPLE_RATE as f32 / (SEQUENCE_BPM * 2.0)).round() as usize;
    let grain_frames = hop_frames * 2;
    let mut output = vec![0.0_f32; input.len()];
    let mut weights = vec![0.0_f32; input.len()];

    for (grain_index, start) in (0..input.len()).step_by(hop_frames).enumerate() {
        let end = (start + grain_frames).min(input.len());
        let source = &input[start..end];
        let note = SEQUENCE_PATTERN[grain_index % SEQUENCE_PATTERN.len()];
        let accent = SEQUENCE_ACCENTS[grain_index % SEQUENCE_ACCENTS.len()];
        let voice = resonator(source, chord.frequencies_hz[note], 0.12);
        let denominator = voice.len().saturating_sub(1).max(1) as f32;
        for (offset, voice) in voice.into_iter().enumerate() {
            let window = 0.5 - 0.5 * (2.0 * PI * offset as f32 / denominator).cos();
            output[start + offset] += voice * window * accent;
            weights[start + offset] += window;
        }
    }
    for (output, weight) in output.iter_mut().zip(weights) {
        if weight > 1.0e-6 {
            *output /= weight;
        }
    }
    output
}

fn hybrid_voice(input: &[f32], chord: Chord) -> Vec<f32> {
    let phases = [0.0, 2.0 * PI / 3.0, 4.0 * PI / 3.0];
    let normalization = 1.0 / 3.0_f32.sqrt();
    let modulated = input
        .iter()
        .enumerate()
        .map(|(index, &sample)| {
            let time = index as f32 / SAMPLE_RATE as f32;
            let carrier = chord
                .frequencies_hz
                .iter()
                .zip(phases)
                .map(|(&frequency, phase)| (2.0 * PI * frequency * time + phase).cos())
                .sum::<f32>()
                * normalization;
            sample * carrier
        })
        .collect::<Vec<_>>();
    resonator_bank(&modulated, &chord.frequencies_hz, 0.10)
}

fn match_rms(input: &[f32], output: &mut [f32]) {
    let input_energy = input
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>();
    let output_energy = output
        .iter()
        .map(|&sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>();
    let gain = (input_energy / output_energy.max(1.0e-24)).sqrt() as f32;
    for sample in output.iter_mut() {
        *sample *= gain.min(100.0);
    }
}

fn apply_edge_fade(output: &mut [f32]) {
    let fade_frames = (SAMPLE_RATE as usize / 50).min(output.len() / 2);
    for index in 0..fade_frames {
        let gain = index as f32 / fade_frames.max(1) as f32;
        output[index] *= gain;
        let tail = output.len() - 1 - index;
        output[tail] *= gain;
    }
}

fn correlation(left: &[f32], right: &[f32]) -> f32 {
    let (dot, left_energy, right_energy) = left.iter().zip(right).fold(
        (0.0_f64, 0.0_f64, 0.0_f64),
        |(dot, left_energy, right_energy), (&left, &right)| {
            let left = f64::from(left);
            let right = f64::from(right);
            (
                dot + left * right,
                left_energy + left * left,
                right_energy + right * right,
            )
        },
    );
    (dot / (left_energy * right_energy).sqrt().max(1.0e-24)) as f32
}

fn relative_difference_db(input: &[f32], output: &[f32]) -> f32 {
    let (difference_energy, input_energy) = input.iter().zip(output).fold(
        (0.0_f64, 0.0_f64),
        |(difference_energy, input_energy), (&input, &output)| {
            let input = f64::from(input);
            let difference = f64::from(output) - input;
            (
                difference_energy + difference * difference,
                input_energy + input * input,
            )
        },
    );
    (10.0
        * (difference_energy / input_energy.max(1.0e-24))
            .max(1.0e-24)
            .log10()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_signal() -> Vec<f32> {
        (0..SAMPLE_RATE as usize)
            .map(|index| {
                let time = index as f32 / SAMPLE_RATE as f32;
                0.4 * (2.0 * PI * 173.0 * time).sin()
                    + 0.2 * (2.0 * PI * 997.0 * time).sin()
                    + 0.05 * ((index * 7919 % 997) as f32 / 997.0 - 0.5)
            })
            .collect()
    }

    #[test]
    fn bohlen_pierce_catalog_has_thirteen_related_chords() {
        let chords = (0..CHORD_COUNT).map(chord).collect::<Vec<_>>();
        assert_eq!(chords.len(), 13);
        assert_eq!(chords[0].steps, [0, 6, 10]);
        assert_eq!(chords[12].steps, [12, 5, 9]);
        assert!(chords.iter().all(|chord| {
            chord
                .frequencies_hz
                .iter()
                .all(|&frequency| (110.0..330.0).contains(&frequency))
        }));
    }

    #[test]
    fn pair_hash_is_stable_and_independent_of_approach() {
        let short = fingerprint_bytes(b"short input file bytes");
        let long = fingerprint_bytes(b"long input file bytes");
        let selected = chord_index(short, long);
        assert!(selected < CHORD_COUNT);
        assert_eq!(selected, chord_index(short, long));
        assert_ne!(selected, chord_index(long, short));
    }

    #[test]
    fn all_preprocessors_preserve_length_and_produce_distinct_finite_audio() {
        let input = test_signal();
        let chord = chord(7);
        let outputs = PitchApproach::ALL.map(|approach| preprocess(&input, chord, approach));
        for output in &outputs {
            assert_eq!(output.samples.len(), input.len());
            assert!(output.samples.iter().all(|sample| sample.is_finite()));
            assert!(output.samples.iter().any(|sample| sample.abs() > 1.0e-5));
            let (minimum_correlation, maximum_difference_db) =
                if output.additive_note_db_below_local.is_some() {
                    (0.95, -10.0)
                } else {
                    (0.98, -18.0)
                };
            assert!(output.dry_correlation >= minimum_correlation);
            assert!(output.difference_rms_db_relative.is_finite());
            assert!(output.difference_rms_db_relative <= maximum_difference_db);
        }
        assert_ne!(outputs[0].samples, outputs[1].samples);
        assert_ne!(outputs[1].samples, outputs[2].samples);
        assert_ne!(outputs[0].samples, outputs[2].samples);
    }
}
