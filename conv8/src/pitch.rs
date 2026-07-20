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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitchApproach {
    PureConvolution,
    SequencedConvolution,
    HybridSpectral,
}

impl PitchApproach {
    pub const ALL: [Self; 3] = [
        Self::PureConvolution,
        Self::SequencedConvolution,
        Self::HybridSpectral,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::PureConvolution => "pure_convolution",
            Self::SequencedConvolution => "sequenced_convolution",
            Self::HybridSpectral => "hybrid_spectral",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::PureConvolution => "whole-clip three-note IIR convolutional resonator",
            Self::SequencedConvolution => {
                "96 BPM overlap-add grains through arpeggiated single-note resonators"
            }
            Self::HybridSpectral => {
                "three-note ring modulation followed by a convolutional resonator bank"
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Chord {
    pub index: usize,
    pub steps: [usize; 3],
    pub frequencies_hz: [f32; 3],
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

pub fn preprocess(input: &[f32], chord: Chord, approach: PitchApproach) -> Vec<f32> {
    let mut output = match approach {
        PitchApproach::PureConvolution => resonator_bank(input, &chord.frequencies_hz, 0.72),
        PitchApproach::SequencedConvolution => sequenced_voice(input, chord),
        PitchApproach::HybridSpectral => hybrid_voice(input, chord),
    };
    match_rms_and_fade(input, &mut output);
    output
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
        let voice = resonator(source, chord.frequencies_hz[note], 0.24);
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
    resonator_bank(&modulated, &chord.frequencies_hz, 0.32)
}

fn match_rms_and_fade(input: &[f32], output: &mut [f32]) {
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

    let fade_frames = (SAMPLE_RATE as usize / 50).min(output.len() / 2);
    for index in 0..fade_frames {
        let gain = index as f32 / fade_frames.max(1) as f32;
        output[index] *= gain;
        let tail = output.len() - 1 - index;
        output[tail] *= gain;
    }
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
            assert_eq!(output.len(), input.len());
            assert!(output.iter().all(|sample| sample.is_finite()));
            assert!(output.iter().any(|sample| sample.abs() > 1.0e-5));
        }
        assert_ne!(outputs[0], outputs[1]);
        assert_ne!(outputs[1], outputs[2]);
        assert_ne!(outputs[0], outputs[2]);
    }
}
