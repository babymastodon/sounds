use std::f32::consts::{PI, SQRT_2};

use crate::audio::SAMPLE_RATE;

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const CHORD_COUNT: usize = 13;
const BASE_FREQUENCY_HZ: f32 = 110.0;
const CHORD_INTERVALS: [usize; 3] = [0, 4, 8];
const NOTE_PATTERN: [usize; 8] = [0, 1, 2, 1, 0, 2, 0, 1];
pub const MINIMUM_NOTE_DB_BELOW_LOCAL: f32 = -1.5;
pub const MAXIMUM_NOTE_DB_BELOW_LOCAL: f32 = 4.25;
pub const MINIMUM_NOTE_SECONDS: f32 = 0.4;
pub const MAXIMUM_NOTE_SECONDS: f32 = 1.504;
pub const TARGET_CONVOLVED_TONE_DB_RELATIVE: f32 = -1.5;
pub const ALGORITHM_VERSION: &str = "sparse-hashed-13edo-audible-ruined-v9";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitchApproach {
    LongAdditiveSynth,
    ShortAdditiveSynth,
}

impl PitchApproach {
    pub const ALL: [Self; 2] = [Self::LongAdditiveSynth, Self::ShortAdditiveSynth];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::LongAdditiveSynth => "long_additive_synth",
            Self::ShortAdditiveSynth => "short_additive_synth",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::LongAdditiveSynth => {
                "locally leveled short additive notes mixed into the unfiltered long input"
            }
            Self::ShortAdditiveSynth => {
                "locally leveled short additive notes mixed into the unfiltered short input"
            }
        }
    }

    pub const fn processed_role(self) -> &'static str {
        match self {
            Self::LongAdditiveSynth => "long",
            Self::ShortAdditiveSynth => "short",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Chord {
    pub index: usize,
    pub steps: [usize; 3],
    pub frequencies_hz: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct NoteGesture {
    pub db_below_local: f32,
    pub duration_seconds: f32,
    pub envelope: EnvelopeKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvelopeKind {
    Pluck,
    Swell,
    ReversePluck,
    TremoloArc,
}

impl EnvelopeKind {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Pluck => "pluck",
            Self::Swell => "swell",
            Self::ReversePluck => "reverse_pluck",
            Self::TremoloArc => "tremolo_arc",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GestureProfile {
    pub fingerprint: u64,
    pub notes: [NoteGesture; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstrumentKind {
    ModalNoiseResonator,
    InharmonicFm,
    SaturatedSawCluster,
}

#[derive(Clone, Copy, Debug)]
pub struct InstrumentProfile {
    pub kind: InstrumentKind,
    pub detune_spread_cents: f32,
    pub modal_disorder: f32,
    pub sustained_noise_mix: f32,
    pub drive: f32,
    pub fm_feedback: f32,
    pub fm_index_start: f32,
    pub fm_index_end: f32,
    pub fold_count: u8,
    pub drift_cents: f32,
    pub alias_mix: f32,
    pub bit_depth: u8,
}

impl InstrumentProfile {
    pub fn parameters(self) -> String {
        match self.kind {
            InstrumentKind::ModalNoiseResonator => format!(
                "detune_cents={:.3};mode_disorder_percent={:.3};sustained_noise_percent={:.3};drive={:.3};folds={}",
                self.detune_spread_cents,
                self.modal_disorder * 100.0,
                self.sustained_noise_mix * 100.0,
                self.drive,
                self.fold_count
            ),
            InstrumentKind::InharmonicFm => format!(
                "detune_cents={:.3};feedback={:.3};index={:.3}->{:.3};drive={:.3};folds={}",
                self.detune_spread_cents,
                self.fm_feedback,
                self.fm_index_start,
                self.fm_index_end,
                self.drive,
                self.fold_count
            ),
            InstrumentKind::SaturatedSawCluster => format!(
                "detune_cents={:.3};drift_cents={:.3};drive={:.3};folds={};alias_percent={:.3};bits={}",
                self.detune_spread_cents,
                self.drift_cents,
                self.drive,
                self.fold_count,
                self.alias_mix * 100.0,
                self.bit_depth
            ),
        }
    }
}

impl InstrumentKind {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::ModalNoiseResonator => "modal_noise_resonator",
            Self::InharmonicFm => "inharmonic_fm",
            Self::SaturatedSawCluster => "saturated_saw_cluster",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreprocessedClip {
    pub tone_stem: Vec<f32>,
    pub gesture_profile: GestureProfile,
    pub instrument_profile: InstrumentProfile,
    pub scheduled_note_count: usize,
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
    // Keep the legacy domain tag so each prepared pair retains its comparable root index.
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

pub fn gesture_profile(short_name: &str, long_name: &str) -> GestureProfile {
    let mut fingerprint = FNV_OFFSET_BASIS;
    hash_bytes(&mut fingerprint, b"conv8-additive-gesture-names-v1\0");
    hash_bytes(&mut fingerprint, &(short_name.len() as u64).to_le_bytes());
    hash_bytes(&mut fingerprint, short_name.as_bytes());
    hash_bytes(&mut fingerprint, &(long_name.len() as u64).to_le_bytes());
    hash_bytes(&mut fingerprint, long_name.as_bytes());
    let notes = std::array::from_fn(|pitch_index| {
        let hash = derived_hash(fingerprint, b"pitch-gesture", pitch_index as u64);
        let unit = unit_interval(hash);
        let db_below_local = MINIMUM_NOTE_DB_BELOW_LOCAL
            + unit * (MAXIMUM_NOTE_DB_BELOW_LOCAL - MINIMUM_NOTE_DB_BELOW_LOCAL);
        let duration_seconds = MINIMUM_NOTE_SECONDS
            * 10.0_f32.powf((db_below_local - MINIMUM_NOTE_DB_BELOW_LOCAL) / 10.0);
        NoteGesture {
            db_below_local,
            duration_seconds,
            envelope: match (hash >> 32) % 4 {
                0 => EnvelopeKind::Pluck,
                1 => EnvelopeKind::Swell,
                2 => EnvelopeKind::ReversePluck,
                _ => EnvelopeKind::TremoloArc,
            },
        }
    });
    GestureProfile { fingerprint, notes }
}

pub fn scheduled_note_count(profile: GestureProfile, approach: PitchApproach) -> usize {
    let (minimum, possibilities) = match approach {
        PitchApproach::LongAdditiveSynth => (3, 4),
        PitchApproach::ShortAdditiveSynth => (2, 2),
    };
    let hash = derived_hash(profile.fingerprint, b"schedule-count", approach as u64);
    minimum + hash as usize % possibilities
}

pub fn instrument_profile(profile: GestureProfile) -> InstrumentProfile {
    let kind = match derived_hash(profile.fingerprint, b"instrument-family", 0) % 3 {
        0 => InstrumentKind::ModalNoiseResonator,
        1 => InstrumentKind::InharmonicFm,
        _ => InstrumentKind::SaturatedSawCluster,
    };
    let mut result = InstrumentProfile {
        kind,
        detune_spread_cents: 0.0,
        modal_disorder: 0.0,
        sustained_noise_mix: 0.0,
        drive: 0.0,
        fm_feedback: 0.0,
        fm_index_start: 0.0,
        fm_index_end: 0.0,
        fold_count: 0,
        drift_cents: 0.0,
        alias_mix: 0.0,
        bit_depth: 0,
    };
    match kind {
        InstrumentKind::ModalNoiseResonator => {
            result.detune_spread_cents = hashed_range(profile, b"modal-detune", 30.0, 42.0);
            result.modal_disorder = hashed_range(profile, b"modal-disorder", 0.03, 0.05);
            result.sustained_noise_mix = hashed_range(profile, b"modal-noise", 0.18, 0.28);
            result.drive = hashed_range(profile, b"modal-drive", 3.0, 4.0);
            result.fold_count = 1;
        }
        InstrumentKind::InharmonicFm => {
            result.detune_spread_cents = hashed_range(profile, b"fm-detune", 20.0, 35.0);
            result.fm_feedback = hashed_range(profile, b"fm-feedback", 0.45, 0.65);
            result.fm_index_start = hashed_range(profile, b"fm-index-start", 7.0, 9.0);
            result.fm_index_end = hashed_range(profile, b"fm-index-end", 2.5, 3.5);
            result.drive = hashed_range(profile, b"fm-drive", 1.6, 2.0);
            result.fold_count = 1 + (derived_hash(profile.fingerprint, b"fm-folds", 0) % 2) as u8;
        }
        InstrumentKind::SaturatedSawCluster => {
            result.detune_spread_cents = hashed_range(profile, b"saw-detune", 30.0, 42.0);
            result.drift_cents = hashed_range(profile, b"saw-drift", 5.0, 9.0);
            result.drive = hashed_range(profile, b"saw-drive", 3.2, 4.2);
            result.fold_count = 1 + (derived_hash(profile.fingerprint, b"saw-folds", 0) % 2) as u8;
            result.alias_mix = hashed_range(profile, b"saw-alias", 0.20, 0.30);
            result.bit_depth = 9 + (derived_hash(profile.fingerprint, b"saw-bits", 0) % 3) as u8;
        }
    }
    result
}

pub fn preprocess(
    input: &[f32],
    chord: Chord,
    profile: GestureProfile,
    approach: PitchApproach,
) -> PreprocessedClip {
    additive_synth_voice(input, chord, profile, approach)
}

fn additive_synth_voice(
    input: &[f32],
    chord: Chord,
    profile: GestureProfile,
    approach: PitchApproach,
) -> PreprocessedClip {
    let mut tone_stem = vec![0.0; input.len()];
    let scheduled_note_count = scheduled_note_count(profile, approach);
    let instrument_profile = instrument_profile(profile);
    let spacing = input.len() as f32 / (scheduled_note_count + 1) as f32;
    let pattern_offset = derived_hash(
        profile.fingerprint,
        b"pitch-pattern-offset",
        approach as u64,
    ) as usize
        % NOTE_PATTERN.len();
    for note_index in 0..scheduled_note_count {
        let pitch_index = NOTE_PATTERN[(pattern_offset + note_index) % NOTE_PATTERN.len()];
        let gesture = profile.notes[pitch_index];
        let note_frames = (gesture.duration_seconds * SAMPLE_RATE as f32).round() as usize;
        let placement_hash = derived_hash(
            profile.fingerprint,
            b"note-placement",
            ((approach as u64) << 32) | note_index as u64,
        );
        let jitter = (unit_interval(placement_hash) - 0.5) * 0.4 * spacing;
        let nominal_onset = ((note_index + 1) as f32 * spacing + jitter).round() as usize;
        let onset = nominal_onset.min(input.len().saturating_sub(note_frames));
        let end = (onset + note_frames).min(input.len());
        let frames = end - onset;
        let frequency = chord.frequencies_hz[pitch_index];
        let note_seed = derived_hash(
            profile.fingerprint,
            b"instrument-note",
            ((approach as u64) << 32) | note_index as u64,
        );
        let mut note = synthesize_note(
            instrument_profile,
            frequency,
            frames,
            gesture.duration_seconds,
            gesture.envelope,
            note_seed,
        );

        let local_radius = (SAMPLE_RATE as usize * 3) / 4;
        let local_start = onset.saturating_sub(local_radius);
        let local_end = (onset + local_radius).min(input.len());
        let local_rms = rms(&input[local_start..local_end]);
        let target_rms = local_rms * 10.0_f32.powf(-gesture.db_below_local / 20.0);
        let note_gain = target_rms / rms(&note).max(1.0e-12);
        for (output, note) in tone_stem[onset..end].iter_mut().zip(note.drain(..)) {
            *output += note * note_gain;
        }
    }
    let samples = input
        .iter()
        .zip(&tone_stem)
        .map(|(&input, &tone)| input + tone)
        .collect::<Vec<_>>();
    PreprocessedClip {
        dry_correlation: correlation(input, &samples),
        difference_rms_db_relative: relative_difference_db(input, &samples),
        tone_stem,
        gesture_profile: profile,
        instrument_profile,
        scheduled_note_count,
    }
}

fn synthesize_note(
    instrument: InstrumentProfile,
    frequency: f32,
    frames: usize,
    duration: f32,
    envelope_kind: EnvelopeKind,
    seed: u64,
) -> Vec<f32> {
    match instrument.kind {
        InstrumentKind::ModalNoiseResonator => {
            modal_noise_note(instrument, frequency, frames, duration, envelope_kind, seed)
        }
        InstrumentKind::InharmonicFm => {
            inharmonic_fm_note(instrument, frequency, frames, duration, envelope_kind, seed)
        }
        InstrumentKind::SaturatedSawCluster => {
            saturated_saw_note(instrument, frequency, frames, duration, envelope_kind, seed)
        }
    }
}

fn modal_noise_note(
    instrument: InstrumentProfile,
    frequency: f32,
    frames: usize,
    duration: f32,
    envelope_kind: EnvelopeKind,
    seed: u64,
) -> Vec<f32> {
    const RATIOS: [f32; 6] = [1.0, 1.41, 1.93, 2.58, 3.77, 5.12];
    const AMPLITUDES: [f32; 6] = [1.0, 0.62, 0.38, 0.26, 0.15, 0.09];
    let cents = [
        -instrument.detune_spread_cents,
        0.0,
        instrument.detune_spread_cents,
    ];
    let frequencies = std::array::from_fn::<_, 3, _>(|bank| {
        std::array::from_fn::<_, 6, _>(|mode| {
            let disorder = if mode == 0 {
                1.0
            } else {
                let hash = derived_hash(seed, b"modal-disorder", (bank * 6 + mode) as u64);
                1.0 + (unit_interval(hash) * 2.0 - 1.0) * instrument.modal_disorder
            };
            frequency * 2.0_f32.powf(cents[bank] / 1_200.0) * RATIOS[mode] * disorder
        })
    });
    let phases = std::array::from_fn::<_, 3, _>(|bank| {
        std::array::from_fn::<_, 6, _>(|mode| {
            2.0 * PI * unit_interval(derived_hash(seed, b"modal-phase", (bank * 6 + mode) as u64))
        })
    });
    (0..frames)
        .map(|frame| {
            let time = frame as f32 / SAMPLE_RATE as f32;
            let progress = time / duration;
            let modes = frequencies
                .iter()
                .zip(phases)
                .map(|(bank_frequencies, bank_phases)| {
                    bank_frequencies
                        .iter()
                        .zip(AMPLITUDES)
                        .zip(bank_phases)
                        .enumerate()
                        .map(|(mode, ((&frequency, amplitude), phase))| {
                            let modal_decay = (-(1.1 + mode as f32 * 0.55) * progress).exp();
                            amplitude * modal_decay * (2.0 * PI * frequency * time + phase).sin()
                        })
                        .sum::<f32>()
                })
                .sum::<f32>()
                / 3.0;
            let attack_noise = noise_sample(seed, frame) * (-time / 0.022).exp() * 0.42;
            let sustained_noise =
                noise_sample(seed ^ 0xa5a5_5a5a_1337_2468, frame) * instrument.sustained_noise_mix;
            let raw =
                modes * (1.0 - instrument.sustained_noise_mix) + attack_noise + sustained_noise;
            let clipped = asymmetric_clip(raw, instrument.drive);
            let folded = fold_signal(clipped * 1.35, instrument.fold_count);
            (0.2 * raw + 0.8 * folded) * envelope(envelope_kind, time, duration)
        })
        .collect()
}

fn inharmonic_fm_note(
    instrument: InstrumentProfile,
    frequency: f32,
    frames: usize,
    duration: f32,
    envelope_kind: EnvelopeKind,
    seed: u64,
) -> Vec<f32> {
    let cents = [
        -instrument.detune_spread_cents,
        0.0,
        instrument.detune_spread_cents,
    ];
    let frequencies = cents.map(|cents| frequency * 2.0_f32.powf(cents / 1_200.0));
    let phases = std::array::from_fn::<_, 3, _>(|carrier| {
        std::array::from_fn::<_, 2, _>(|modulator| {
            2.0 * PI
                * unit_interval(derived_hash(
                    seed,
                    b"fm-phase",
                    (carrier * 2 + modulator) as u64,
                ))
        })
    });
    let mut previous = [0.0_f32; 3];
    let mut output = Vec::with_capacity(frames);
    for frame in 0..frames {
        let time = frame as f32 / SAMPLE_RATE as f32;
        let progress = (time / duration).clamp(0.0, 1.0);
        let index = instrument
            .fm_index_start
            .mul_add(1.0 - progress, instrument.fm_index_end * progress);
        let mut carriers = 0.0;
        for carrier in 0..3 {
            let carrier_frequency = frequencies[carrier];
            let carrier_phase = 2.0 * PI * carrier_frequency * time;
            let modulator_1 =
                (2.0 * PI * carrier_frequency * SQRT_2 * time + phases[carrier][0]).sin();
            let modulator_2 =
                (2.0 * PI * carrier_frequency * 2.731 * time + phases[carrier][1]).sin();
            let value = (carrier_phase
                + index * modulator_1
                + 1.4 * modulator_2
                + PI * instrument.fm_feedback * previous[carrier])
                .sin();
            previous[carrier] = value;
            carriers += value;
        }
        let raw = 0.88 * carriers / 3.0 + 0.12 * (2.0 * PI * frequency * time).sin();
        let folded = fold_signal(raw * instrument.drive, instrument.fold_count);
        output.push(folded * envelope(envelope_kind, time, duration));
    }
    output
}

fn saturated_saw_note(
    instrument: InstrumentProfile,
    frequency: f32,
    frames: usize,
    duration: f32,
    envelope_kind: EnvelopeKind,
    seed: u64,
) -> Vec<f32> {
    let cents = std::array::from_fn::<_, 7, _>(|index| {
        instrument.detune_spread_cents * (index as f32 - 3.0) / 3.0
    });
    let frequencies = cents.map(|cents| frequency * 2.0_f32.powf(cents / 1_200.0));
    let phases = std::array::from_fn::<_, 7, _>(|index| {
        unit_interval(derived_hash(seed, b"saw-phase", index as u64))
    });
    let drift_frequencies = std::array::from_fn::<_, 7, _>(|index| {
        0.55 + 0.65 * unit_interval(derived_hash(seed, b"saw-drift-rate", index as u64))
    });
    let drift_phases = std::array::from_fn::<_, 7, _>(|index| {
        2.0 * PI * unit_interval(derived_hash(seed, b"saw-drift-phase", index as u64))
    });
    let levels = ((1_u32 << instrument.bit_depth) - 1) as f32;
    (0..frames)
        .map(|frame| {
            let time = frame as f32 / SAMPLE_RATE as f32;
            let (clean, aliased) = frequencies
                .iter()
                .zip(phases)
                .zip(drift_frequencies)
                .zip(drift_phases)
                .map(|(((&frequency, phase), drift_frequency), drift_phase)| {
                    let fractional_drift = 2.0_f32.powf(instrument.drift_cents / 1_200.0) - 1.0;
                    let drift_depth = frequency * fractional_drift / (2.0 * PI * drift_frequency);
                    let phase = (frequency * time
                        + phase
                        + drift_depth * (2.0 * PI * drift_frequency * time + drift_phase).sin())
                    .fract();
                    (
                        poly_blep_saw_phase(phase, frequency / SAMPLE_RATE as f32),
                        2.0 * phase - 1.0,
                    )
                })
                .fold((0.0_f32, 0.0_f32), |(clean, aliased), (voice, naive)| {
                    (clean + voice, aliased + naive)
                });
            let clean = clean / 7.0;
            let aliased = aliased / 7.0;
            let raw = clean.mul_add(1.0 - instrument.alias_mix, aliased * instrument.alias_mix);
            let clipped = asymmetric_clip(raw, instrument.drive);
            let folded = fold_signal(clipped * 1.5, instrument.fold_count);
            let quantized = (folded * levels).round() / levels;
            quantized * envelope(envelope_kind, time, duration)
        })
        .collect()
}

fn poly_blep_saw_phase(phase: f32, increment: f32) -> f32 {
    let naive = 2.0 * phase - 1.0;
    naive - poly_blep(phase, increment)
}

fn asymmetric_clip(sample: f32, drive: f32) -> f32 {
    let bias = 0.18;
    ((drive * sample + bias).tanh() - bias.tanh()) / (drive + bias).tanh()
}

fn fold_signal(mut sample: f32, folds: u8) -> f32 {
    for fold in 0..folds {
        if fold > 0 {
            sample *= 1.7;
        }
        let wrapped = (sample + 1.0).rem_euclid(4.0);
        sample = if wrapped <= 2.0 {
            wrapped - 1.0
        } else {
            3.0 - wrapped
        };
    }
    sample
}

fn poly_blep(phase: f32, increment: f32) -> f32 {
    if phase < increment {
        let normalized = phase / increment;
        normalized + normalized - normalized * normalized - 1.0
    } else if phase > 1.0 - increment {
        let normalized = (phase - 1.0) / increment;
        normalized * normalized + normalized + normalized + 1.0
    } else {
        0.0
    }
}

fn noise_sample(seed: u64, frame: usize) -> f32 {
    let mut value = seed.wrapping_add((frame as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    ((value >> 40) as f32 / ((1_u32 << 24) - 1) as f32) * 2.0 - 1.0
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

fn derived_hash(seed: u64, tag: &[u8], value: u64) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, b"conv8-derived-hash-v1\0");
    hash_bytes(&mut hash, &seed.to_le_bytes());
    hash_bytes(&mut hash, tag);
    hash_bytes(&mut hash, &value.to_le_bytes());
    hash
}

fn unit_interval(hash: u64) -> f32 {
    (hash >> 40) as f32 / ((1_u32 << 24) - 1) as f32
}

fn hashed_range(profile: GestureProfile, tag: &[u8], minimum: f32, maximum: f32) -> f32 {
    let unit = unit_interval(derived_hash(profile.fingerprint, tag, 0));
    minimum + unit * (maximum - minimum)
}

fn envelope(kind: EnvelopeKind, time: f32, duration: f32) -> f32 {
    let phase = (time / duration).clamp(0.0, 1.0);
    match kind {
        EnvelopeKind::Pluck => {
            let attack = (time / 0.012).min(1.0);
            let release = cosine_release(phase, 0.08);
            attack * attack * (-4.0 * phase).exp() * release
        }
        EnvelopeKind::Swell => (PI * phase).sin().max(0.0).powf(0.72),
        EnvelopeKind::ReversePluck => {
            if phase < 0.62 {
                (phase / 0.62).powi(2)
            } else {
                (-5.0 * (phase - 0.62) / 0.38).exp() * cosine_release(phase, 0.06)
            }
        }
        EnvelopeKind::TremoloArc => {
            let arc = (PI * phase).sin().max(0.0).powf(0.65);
            let pulse = 0.68 + 0.32 * (2.0 * PI * (3.0 * phase + 0.15)).sin();
            arc * pulse
        }
    }
}

fn cosine_release(phase: f32, release_fraction: f32) -> f32 {
    let position = ((1.0 - phase) / release_fraction).clamp(0.0, 1.0);
    0.5 - 0.5 * (PI * position).cos()
}

fn step_frequency_hz(step: usize) -> f32 {
    BASE_FREQUENCY_HZ * 2.0_f32.powf(step as f32 / CHORD_COUNT as f32)
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
        (0..SAMPLE_RATE as usize * 8)
            .map(|index| {
                let time = index as f32 / SAMPLE_RATE as f32;
                0.4 * (2.0 * PI * 173.0 * time).sin()
                    + 0.2 * (2.0 * PI * 997.0 * time).sin()
                    + 0.05 * ((index * 7919 % 997) as f32 / 997.0 - 0.5)
            })
            .collect()
    }

    #[test]
    fn thirteen_edo_catalog_has_thirteen_detuned_chords() {
        let chords = (0..CHORD_COUNT).map(chord).collect::<Vec<_>>();
        assert_eq!(chords.len(), 13);
        assert_eq!(chords[0].steps, [0, 4, 8]);
        assert_eq!(chords[12].steps, [12, 3, 7]);
        assert!(chords.iter().all(|chord| {
            chord
                .frequencies_hz
                .iter()
                .all(|&frequency| (110.0..220.0).contains(&frequency))
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
    fn additive_preprocessor_is_sparse_stronger_and_preserves_length() {
        let input = test_signal();
        let profile = gesture_profile("short-name", "long-name");
        let output = preprocess(&input, chord(7), profile, PitchApproach::ShortAdditiveSynth);
        assert_eq!(output.tone_stem.len(), input.len());
        assert!(output.tone_stem.iter().all(|sample| sample.is_finite()));
        assert!(output.tone_stem.iter().any(|sample| sample.abs() > 1.0e-5));
        assert_eq!(output.scheduled_note_count, 2);
        assert!(
            output.dry_correlation >= 0.80,
            "correlation={}",
            output.dry_correlation
        );
        assert!(output.difference_rms_db_relative.is_finite());
        assert!(
            output.difference_rms_db_relative <= -4.0,
            "difference={} dB",
            output.difference_rms_db_relative
        );
    }

    #[test]
    fn hashed_gestures_trade_amplitude_for_duration_at_nearly_constant_energy() {
        let profile = gesture_profile("short-name", "long-name");
        assert_eq!(
            profile.fingerprint,
            gesture_profile("short-name", "long-name").fingerprint
        );
        assert_ne!(
            profile.fingerprint,
            gesture_profile("other", "long-name").fingerprint
        );
        for note in profile.notes {
            assert!(
                (MINIMUM_NOTE_DB_BELOW_LOCAL..=MAXIMUM_NOTE_DB_BELOW_LOCAL)
                    .contains(&note.db_below_local)
            );
            assert!((MINIMUM_NOTE_SECONDS..=MAXIMUM_NOTE_SECONDS).contains(&note.duration_seconds));
            assert!(!note.envelope.slug().is_empty());
            let relative_energy =
                10.0_f32.powf(-note.db_below_local / 10.0) * note.duration_seconds;
            assert!((relative_energy - 0.565_0).abs() < 1.0e-4);
        }
    }

    #[test]
    fn note_counts_are_sparse_and_role_dependent() {
        for index in 0..100 {
            let profile = gesture_profile(&format!("short-{index}"), "long");
            assert!((2..=3).contains(&scheduled_note_count(
                profile,
                PitchApproach::ShortAdditiveSynth
            )));
            assert!((3..=6).contains(&scheduled_note_count(
                profile,
                PitchApproach::LongAdditiveSynth
            )));
        }
    }

    #[test]
    fn instrument_selection_is_deterministic_and_uses_all_three_families() {
        let mut observed = [false; 3];
        for index in 0..100 {
            let profile = gesture_profile(&format!("short-{index}"), "long");
            let instrument = instrument_profile(profile);
            assert_eq!(instrument.kind, instrument_profile(profile).kind);
            assert_eq!(
                instrument.parameters(),
                instrument_profile(profile).parameters()
            );
            observed[instrument.kind as usize] = true;
        }
        assert!(observed.into_iter().all(|seen| seen));
    }

    #[test]
    fn all_instruments_generate_distinct_finite_notes() {
        let mut profiles = [None; 3];
        for index in 0..100 {
            let profile = instrument_profile(gesture_profile(&format!("short-{index}"), "long"));
            profiles[profile.kind as usize] = Some(profile);
        }
        let outputs = profiles.map(|profile| {
            synthesize_note(
                profile.expect("all instrument families should be selected"),
                151.0,
                4_800,
                0.5,
                EnvelopeKind::Swell,
                42,
            )
        });
        for output in &outputs {
            assert!(output.iter().all(|sample| sample.is_finite()));
            assert!(rms(output) > 0.01);
        }
        assert_ne!(outputs[0], outputs[1]);
        assert_ne!(outputs[1], outputs[2]);
        assert_ne!(outputs[0], outputs[2]);
    }

    #[test]
    fn approaches_target_opposite_input_roles() {
        assert_eq!(PitchApproach::LongAdditiveSynth.processed_role(), "long");
        assert_eq!(PitchApproach::ShortAdditiveSynth.processed_role(), "short");
    }
}
