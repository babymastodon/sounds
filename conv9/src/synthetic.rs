//! Deterministic, self-authored source material for the synthetic catalog.
//!
//! These generators intentionally favor signals that expose magnitude, phase,
//! delay, coherence, and resonance behavior under convolution.  They are not
//! used by the real-time renderer: the preparation script renders them once to
//! ordinary WAV files, after which they follow the same validation and input
//! conditioning path as downloaded recordings.

use std::f64::consts::PI;
use std::path::Path;

use anyhow::{Result, bail};
use hound::{SampleFormat, WavSpec, WavWriter};

mod extras_chirp;
mod extras_impulse;
mod extras_mask;
mod extras_modulation;
mod extras_noise;
mod extras_oscillator;
mod extras_phase;
mod extras_resonator;

pub const SAMPLE_RATE: u32 = 48_000;
pub const SECONDS: usize = 61;
pub const FRAMES: usize = SAMPLE_RATE as usize * SECONDS;

pub const SOURCE_IDS: [&str; 64] = [
    "prime_beating_lattice",
    "duty_cycle_barcode",
    "shepard_helix",
    "cepstral_weather",
    "sideband_hive",
    "dyadic_avalanche",
    "golomb_nebula",
    "hawkes_meteor_storm",
    "dipole_separation_atlas",
    "fresnel_focus",
    "doppler_slingshots",
    "zipper_ladder",
    "coupled_bottle_organ",
    "dispersive_coil_spring",
    "friction_glass_shell",
    "counterweave",
    "cantor_cathedral",
    "moire_shutters",
    "frequency_prism_clicks",
    "causal_anticausal_twins",
    "coherence_melt_cloud",
    "bessel_staircase",
    "one_way_galaxy",
    "folded_constellation",
    "hard_sync_staircase",
    "through_zero_chirp_pendulum",
    "kicked_duffing_bell",
    "phase_doppelganger",
    "prime_band_loom",
    "roving_zero_rake",
    "cantor_derivative_dust",
    "chirped_comb_lens",
    "palindromic_sinc_constellation",
    "coherent_braid",
    "pinball_parabolas",
    "dispersion_fan",
    "chladni_plate_scanner",
    "buzz_bridge_string",
    "branching_horn_tree",
    "notch_braid",
    "formant_eclipse",
    "golden_fireflies",
    "phase_code_pulse_bursts",
    "harmonic_focus_lattice",
    "modal_group_delay_staircases",
    "chebyshev_elevator",
    "clock_shattered_bell",
    "rectifier_tide",
    "phase_reset_constellation",
    "cross_fm_knot",
    "chirplet_dust",
    "noisy_harmonic_sieve",
    "golden_lattice_moire",
    "quadratic_residue_starfield",
    "shepard_corkscrew",
    "rosette_fm",
    "bessel_drum_skin",
    "cantilever_forest",
    "phase_prism",
    "chirp_origami",
    "dispersive_waveguide_plucks",
    "swept_fractional_delay_comb_loom",
    "sync_guillotine",
    "pwm_cathedral",
];

pub fn write_wav(id: &str, path: &Path) -> Result<()> {
    let mut samples = render(id, SAMPLE_RATE, FRAMES)?;
    finish(&mut samples, SAMPLE_RATE);
    if id == "phase_code_pulse_bursts" {
        // The standard 15 Hz preparation high-pass responds to each bipolar
        // chip transition, so reserve another 6 dB for its doubled step size.
        for sample in &mut samples {
            *sample *= 0.5;
        }
    }
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    for sample in samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

pub fn render(id: &str, sample_rate: u32, frames: usize) -> Result<Vec<f32>> {
    let samples = match id {
        "prime_beating_lattice" => prime_beating_lattice(sample_rate, frames),
        "duty_cycle_barcode" => duty_cycle_barcode(sample_rate, frames),
        "shepard_helix" => shepard_helix(sample_rate, frames),
        "cepstral_weather" => cepstral_weather(sample_rate, frames),
        "sideband_hive" => sideband_hive(sample_rate, frames),
        "dyadic_avalanche" => dyadic_avalanche(sample_rate, frames),
        "golomb_nebula" => golomb_nebula(sample_rate, frames),
        "hawkes_meteor_storm" => hawkes_meteor_storm(sample_rate, frames),
        "dipole_separation_atlas" => dipole_separation_atlas(sample_rate, frames),
        "fresnel_focus" => fresnel_focus(sample_rate, frames),
        "doppler_slingshots" => doppler_slingshots(sample_rate, frames),
        "zipper_ladder" => zipper_ladder(sample_rate, frames),
        "coupled_bottle_organ" => coupled_bottle_organ(sample_rate, frames),
        "dispersive_coil_spring" => dispersive_coil_spring(sample_rate, frames),
        "friction_glass_shell" => friction_glass_shell(sample_rate, frames),
        "counterweave" => counterweave(sample_rate, frames),
        "cantor_cathedral" => cantor_cathedral(sample_rate, frames),
        "moire_shutters" => moire_shutters(sample_rate, frames),
        "frequency_prism_clicks" => frequency_prism_clicks(sample_rate, frames),
        "causal_anticausal_twins" => causal_anticausal_twins(sample_rate, frames),
        "coherence_melt_cloud" => coherence_melt_cloud(sample_rate, frames),
        "bessel_staircase" => bessel_staircase(sample_rate, frames),
        "one_way_galaxy" => one_way_galaxy(sample_rate, frames),
        "folded_constellation" => folded_constellation(sample_rate, frames),
        "hard_sync_staircase" => hard_sync_staircase(sample_rate, frames),
        "through_zero_chirp_pendulum" => through_zero_chirp_pendulum(sample_rate, frames),
        "kicked_duffing_bell" => kicked_duffing_bell(sample_rate, frames),
        "phase_doppelganger" => phase_doppelganger(sample_rate, frames),
        "prime_band_loom" => prime_band_loom(sample_rate, frames),
        "roving_zero_rake" => roving_zero_rake(sample_rate, frames),
        "cantor_derivative_dust" => cantor_derivative_dust(sample_rate, frames),
        "chirped_comb_lens" => chirped_comb_lens(sample_rate, frames),
        "palindromic_sinc_constellation" => palindromic_sinc_constellation(sample_rate, frames),
        "coherent_braid" => coherent_braid(sample_rate, frames),
        "pinball_parabolas" => pinball_parabolas(sample_rate, frames),
        "dispersion_fan" => dispersion_fan(sample_rate, frames),
        "chladni_plate_scanner" => chladni_plate_scanner(sample_rate, frames),
        "buzz_bridge_string" => buzz_bridge_string(sample_rate, frames),
        "branching_horn_tree" => branching_horn_tree(sample_rate, frames),
        "notch_braid" => notch_braid(sample_rate, frames),
        "formant_eclipse" => formant_eclipse(sample_rate, frames),
        "golden_fireflies" => golden_fireflies(sample_rate, frames),
        "phase_code_pulse_bursts" => phase_code_pulse_bursts(sample_rate, frames),
        "harmonic_focus_lattice" => harmonic_focus_lattice(sample_rate, frames),
        "modal_group_delay_staircases" => modal_group_delay_staircases(sample_rate, frames),
        "chebyshev_elevator" => chebyshev_elevator(sample_rate, frames),
        "clock_shattered_bell" => clock_shattered_bell(sample_rate, frames),
        "rectifier_tide" => rectifier_tide(sample_rate, frames),
        "phase_reset_constellation" => {
            extras_oscillator::phase_reset_constellation(sample_rate, frames)
        }
        "cross_fm_knot" => extras_oscillator::cross_fm_knot(sample_rate, frames),
        "chirplet_dust" => extras_noise::chirplet_dust(sample_rate, frames),
        "noisy_harmonic_sieve" => extras_noise::noisy_harmonic_sieve(sample_rate, frames),
        "golden_lattice_moire" => extras_impulse::golden_lattice_moire(sample_rate, frames),
        "quadratic_residue_starfield" => {
            extras_impulse::quadratic_residue_starfield(sample_rate, frames)
        }
        "shepard_corkscrew" => extras_chirp::shepard_corkscrew(sample_rate, frames),
        "rosette_fm" => extras_chirp::rosette_fm(sample_rate, frames),
        "bessel_drum_skin" => extras_resonator::bessel_drum_skin(sample_rate, frames),
        "cantilever_forest" => extras_resonator::cantilever_forest(sample_rate, frames),
        "phase_prism" => extras_mask::phase_prism(sample_rate, frames),
        "chirp_origami" => extras_mask::chirp_origami(sample_rate, frames),
        "dispersive_waveguide_plucks" => {
            extras_phase::dispersive_waveguide_plucks(sample_rate, frames)
        }
        "swept_fractional_delay_comb_loom" => {
            extras_phase::swept_fractional_delay_comb_loom(sample_rate, frames)
        }
        "sync_guillotine" => extras_modulation::sync_guillotine(sample_rate, frames),
        "pwm_cathedral" => extras_modulation::pwm_cathedral(sample_rate, frames),
        _ => bail!("unknown synthetic source {id}"),
    };
    Ok(samples)
}

fn prime_beating_lattice(sample_rate: u32, frames: usize) -> Vec<f32> {
    let primes = [2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let mut sum = 0.0;
        for (voice, prime) in primes.iter().enumerate() {
            let frequency = 23.0 * prime;
            let slow_phase = 0.9 * (2.0 * PI * time / (20.0 + voice as f64 * 6.7)).sin();
            sum += (2.0 * PI * frequency * time + slow_phase + voice as f64 * 0.37).sin()
                / (1.0 + voice as f64).sqrt();
        }
        *sample = (sum * 0.16) as f32;
    }
    output
}

fn duty_cycle_barcode(sample_rate: u32, frames: usize) -> Vec<f32> {
    let duties = [
        0.02, 0.31, 0.07, 0.43, 0.17, 0.11, 0.47, 0.23, 0.05, 0.37, 0.13, 0.29, 0.41, 0.19, 0.03,
        0.33, 0.09, 0.45, 0.21, 0.15, 0.39, 0.27, 0.01, 0.35, 0.25, 0.49, 0.06, 0.18, 0.42, 0.12,
        0.30,
    ];
    let increment = 181.0 / sample_rate as f64;
    let mut phase = 0.0_f64;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let state = ((index as f64 / sample_rate as f64 / 0.83) as usize) % duties.len();
        let duty = duties[state];
        let mut value = if phase < duty { 1.0 } else { -1.0 };
        value += poly_blep(phase, increment);
        value -= poly_blep((phase + 1.0 - duty).fract(), increment);
        *sample = value as f32 * 0.32;
        phase = (phase + increment).fract();
    }
    output
}

fn shepard_helix(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut phases = [0.0_f64; 9];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let climb = (time / 11.0).fract();
        let mut sum = 0.0;
        for voice in 0..phases.len() {
            let octave = (voice as f64 + climb) % phases.len() as f64;
            let frequency = 42.0 * 2.0_f64.powf(octave);
            let distance = (octave - 4.0) / 2.0;
            let weight = (-0.5 * distance * distance).exp();
            phases[voice] += 2.0 * PI * frequency / sample_rate as f64;
            sum += phases[voice].sin() * weight;
        }
        *sample = (sum * 0.18) as f32;
    }
    output
}

fn cepstral_weather(sample_rate: u32, frames: usize) -> Vec<f32> {
    let states = [
        [120.0, 430.0, 1_600.0, 5_800.0],
        [210.0, 900.0, 2_400.0, 8_100.0],
        [75.0, 620.0, 3_500.0, 11_000.0],
        [330.0, 1_200.0, 4_700.0, 7_200.0],
    ];
    let mut filters = [Biquad::default(); 4];
    let mut rng = Rng::new(0xce95_7a11);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        if index % 128 == 0 {
            let time = index as f64 / sample_rate as f64;
            let position = time / 6.7;
            let left = position.floor() as usize % states.len();
            let right = (left + 1) % states.len();
            let mix = smooth(position.fract());
            for band in 0..4 {
                let center = lerp(states[left][band], states[right][band], mix);
                filters[band].set_bandpass(sample_rate as f64, center, 0.7 + band as f64 * 0.65);
            }
        }
        let noise = rng.bipolar() as f32;
        let shaped = filters
            .iter_mut()
            .enumerate()
            .map(|(band, filter)| filter.process(noise) * (0.9 - band as f32 * 0.12))
            .sum::<f32>();
        *sample = noise * 0.05 + shaped * 0.34;
    }
    output
}

fn sideband_hive(sample_rate: u32, frames: usize) -> Vec<f32> {
    let centers = [95.0, 230.0, 570.0, 1_400.0, 3_600.0, 9_000.0];
    let modulators = [17.0, 29.0, 43.0, 71.0, 113.0, 181.0];
    let mut filters = [Biquad::default(); 6];
    for (band, filter) in filters.iter_mut().enumerate() {
        filter.set_bandpass(sample_rate as f64, centers[band], 3.5);
    }
    let mut rng = Rng::new(0x51de_ba7d);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let noise = rng.bipolar() as f32;
        let mut sum = 0.0;
        for (band, filter) in filters.iter_mut().enumerate() {
            let depth = 0.45 + 0.45 * (2.0 * PI * time / (9.0 + band as f64)).sin();
            let modulator = (2.0 * PI * modulators[band] * time).sin();
            sum += filter.process(noise) * (modulator * depth) as f32;
        }
        *sample = sum * 0.42;
    }
    output
}

fn dyadic_avalanche(sample_rate: u32, frames: usize) -> Vec<f32> {
    let centers = [
        80.0, 170.0, 360.0, 760.0, 1_600.0, 3_400.0, 7_200.0, 13_500.0,
    ];
    let mut filters = [Biquad::default(); 8];
    for (band, filter) in filters.iter_mut().enumerate() {
        filter.set_bandpass(sample_rate as f64, centers[band], 1.1);
    }
    let mut rng = Rng::new(0xd1ad_1c55);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let noise = rng.bipolar() as f32;
        let mut sum = 0.0;
        for (band, filter) in filters.iter_mut().enumerate() {
            let slow_period = 18.0 / 2.0_f64.powf(band as f64 * 0.42);
            let parent = 0.5 + 0.5 * (2.0 * PI * time / slow_period + band as f64).sin();
            let child = 0.5
                + 0.5
                    * (2.0 * PI * time / (slow_period * 0.37) + (band * band) as f64 * 0.31).sin();
            let envelope = (0.08 + 0.92 * parent * parent * child) as f32;
            sum += filter.process(noise) * envelope;
        }
        *sample = sum * 0.34;
    }
    output
}

fn golomb_nebula(sample_rate: u32, frames: usize) -> Vec<f32> {
    let marks = [
        0.0, 1.0, 4.0, 10.0, 18.0, 23.0, 25.0, 34.0, 41.0, 53.0, 55.0,
    ];
    let scales = [0.25, 0.5, 1.0, 2.0];
    let mut output = vec![0.0; frames];
    let mut cell = 0;
    while cell as f64 * 2.5 < frames as f64 / sample_rate as f64 {
        let thue = (cell as u32).count_ones() as usize & 1;
        let scale = scales[(cell * 3 + thue) % scales.len()];
        for (mark_index, mark) in marks.iter().enumerate() {
            let time = cell as f64 * 2.5 + mark / 55.0 * 1.1 * scale;
            let polarity = if (mark_index + cell + thue) & 1 == 0 {
                1.0
            } else {
                -1.0
            };
            add_click(
                &mut output,
                sample_rate,
                time,
                0.0006 + mark_index as f64 * 0.00004,
                polarity * 0.7,
            );
        }
        cell += 1;
    }
    output
}

fn hawkes_meteor_storm(sample_rate: u32, frames: usize) -> Vec<f32> {
    #[derive(Clone, Copy)]
    struct Event {
        time: f64,
        generation: usize,
        amplitude: f64,
    }
    let duration = frames as f64 / sample_rate as f64;
    let mut rng = Rng::new(0xa4e5_5107);
    let mut events = Vec::new();
    let mut time = 0.5;
    while time < duration {
        time += -rng.unit().max(1.0e-6).ln() / 0.35;
        if time < duration {
            events.push(Event {
                time,
                generation: 0,
                amplitude: 0.8,
            });
        }
    }
    let mut cursor = 0;
    while cursor < events.len() && events.len() < 4_000 {
        let parent = events[cursor];
        if parent.generation < 4 {
            let children =
                if rng.unit() < 0.55 { 1 } else { 0 } + if rng.unit() < 0.25 { 1 } else { 0 };
            for _ in 0..children {
                let delay = -rng.unit().max(1.0e-6).ln() * 0.09;
                let child_time = parent.time + delay;
                if child_time < duration {
                    events.push(Event {
                        time: child_time,
                        generation: parent.generation + 1,
                        amplitude: parent.amplitude * (0.55 + 0.25 * rng.unit()),
                    });
                }
            }
        }
        cursor += 1;
    }
    let mut output = vec![0.0; frames];
    for event in events {
        let polarity = if event.generation & 1 == 0 { 1.0 } else { -1.0 };
        let width = 0.0005 + 0.0008 * event.generation as f64;
        add_click(
            &mut output,
            sample_rate,
            event.time,
            width,
            polarity * event.amplitude,
        );
    }
    output
}

fn dipole_separation_atlas(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    let count = (frames as f64 / sample_rate as f64 / 0.6).ceil() as usize;
    for event in 0..count {
        let reversed = event.reverse_bits() >> (usize::BITS - 7);
        let position = (reversed % 101) as f64 / 100.0;
        let separation = (1.0 / sample_rate as f64) * (0.12 * sample_rate as f64).powf(position);
        let time = 0.25 + event as f64 * 0.6;
        add_click(&mut output, sample_rate, time, 0.00045, 0.72);
        add_click(&mut output, sample_rate, time + separation, 0.00045, -0.72);
    }
    output
}

fn fresnel_focus(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    for (focus_index, focus) in [15.0, 30.5, 46.0].iter().enumerate() {
        for voice in 0..14 {
            let ratio = voice as f64 / 13.0;
            let start_frequency = 70.0 * (180.0_f64).powf(ratio);
            let focus_frequency = 900.0 + focus_index as f64 * 420.0;
            let duration = 2.2 + ratio * 3.3;
            let phase_advance = PI * duration * (start_frequency + focus_frequency);
            add_chirp(
                &mut output,
                sample_rate,
                focus - duration,
                duration,
                start_frequency,
                focus_frequency,
                -phase_advance,
                0.09,
            );
            add_chirp(
                &mut output,
                sample_rate,
                *focus,
                duration,
                focus_frequency,
                start_frequency * (0.7 + 0.6 * ratio),
                0.0,
                0.075,
            );
        }
    }
    output
}

fn doppler_slingshots(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut phases = [0.0_f64; 4];
    let centers = [8.0, 22.0, 38.0, 53.0];
    let bases = [95.0, 180.0, 310.0, 520.0];
    let directions = [1.0, -1.0, 1.0, -1.0];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let mut sum = 0.0;
        for voice in 0..phases.len() {
            let normalized = ((time - centers[voice]) / (0.8 + voice as f64 * 0.25)).tanh();
            let frequency = bases[voice] * 2.0_f64.powf(directions[voice] * 4.2 * normalized);
            phases[voice] += 2.0 * PI * frequency.clamp(25.0, 18_000.0) / sample_rate as f64;
            let envelope = (-0.5 * ((time - centers[voice]) / 8.0).powi(2)).exp();
            sum += phases[voice].sin() * envelope;
        }
        *sample = (sum * 0.22) as f32;
    }
    output
}

fn zipper_ladder(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    for tooth in 0..168 {
        if tooth % 11 == 5 || tooth % 17 == 9 {
            continue;
        }
        let start = 0.25 + tooth as f64 * 0.36;
        let position = tooth as f64 / 167.0;
        let duration = 0.08 + 0.22 * ((tooth * 37 % 101) as f64 / 100.0);
        let center = 90.0 * 2.0_f64.powf(position * 7.2);
        let width = 1.3 + 1.8 * position;
        let (from, to) = if tooth & 1 == 0 {
            (center / width, center * width)
        } else {
            (center * width, center / width)
        };
        add_chirp(
            &mut output,
            sample_rate,
            start,
            duration,
            from,
            to,
            0.0,
            0.42,
        );
    }
    output
}

fn coupled_bottle_organ(sample_rate: u32, frames: usize) -> Vec<f32> {
    let frequencies = [82.0, 103.0, 131.0, 166.0, 211.0, 269.0, 344.0, 441.0, 566.0];
    let mut resonators = frequencies.map(|frequency| Resonator::new(sample_rate, frequency, 2.8));
    let mut rng = Rng::new(0xb077_1e05);
    let mut output = vec![0.0; frames];
    let mut shared = 0.0_f32;
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let puff_phase = (time / 3.7).fract();
        let puff = if puff_phase < 0.12 {
            ((1.0 - puff_phase / 0.12) * 0.45) as f32 * rng.bipolar() as f32
        } else {
            0.0
        };
        let mut sum = 0.0;
        for (voice, resonator) in resonators.iter_mut().enumerate() {
            let coupling = shared * (0.007 + voice as f32 * 0.0007);
            sum += resonator.process(puff * (0.8 - voice as f32 * 0.045) + coupling);
        }
        shared = sum.tanh();
        *sample = sum * 0.12;
    }
    output
}

fn dispersive_coil_spring(sample_rate: u32, frames: usize) -> Vec<f32> {
    let frequencies =
        std::array::from_fn::<_, 16, _>(|index| 55.0 * (index as f64 + 1.0).powf(1.37));
    let mut resonators = frequencies.map(|frequency| {
        Resonator::new(
            sample_rate,
            frequency.min(18_000.0),
            1.2 + 0.11 * frequency.sqrt(),
        )
    });
    let mut rng = Rng::new(0xc011_5a71);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let event_phase = (time / 5.3).fract();
        let impulse = if event_phase < 1.0 / sample_rate as f64 * 3.0 {
            if ((time / 5.3) as usize) & 1 == 0 {
                0.8
            } else {
                -0.8
            }
        } else {
            0.0
        };
        let scrape = if (time / 13.0).fract() < 0.16 {
            rng.bipolar() as f32 * 0.012
        } else {
            0.0
        };
        let mut sum = 0.0;
        for (mode, resonator) in resonators.iter_mut().enumerate() {
            sum += resonator.process((impulse + scrape) / (mode as f32 + 1.0).sqrt());
        }
        *sample = sum * 0.11;
    }
    output
}

fn friction_glass_shell(sample_rate: u32, frames: usize) -> Vec<f32> {
    let frequencies = [
        173.0, 281.0, 419.0, 612.0, 857.0, 1_166.0, 1_551.0, 2_023.0, 2_601.0, 3_293.0, 4_117.0,
        5_089.0,
    ];
    let mut resonators = frequencies.map(|frequency| Resonator::new(sample_rate, frequency, 3.5));
    let mut rng = Rng::new(0x61a5_5e11);
    let mut output = vec![0.0; frames];
    let mut friction_state = 0.0_f32;
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let velocity = (2.0 * PI * (0.43 + 0.08 * (time / 17.0).sin()) * time).sin() as f32;
        let pressure = (0.45 + 0.4 * (2.0 * PI * time / 9.7).sin()) as f32;
        let error = velocity - friction_state;
        let stick_slip = (error * (8.0 + 16.0 * pressure)).tanh() - error * 0.18;
        friction_state += 0.002 * stick_slip;
        let excitation = stick_slip * 0.012 + rng.bipolar() as f32 * pressure.abs() * 0.002;
        let mut sum = 0.0;
        for (mode, resonator) in resonators.iter_mut().enumerate() {
            let contact = (0.3 + 0.7 * (2.0 * PI * time / (11.0 + mode as f64)).sin().abs()) as f32;
            sum += resonator.process(excitation * contact / (mode as f32 + 1.0).sqrt());
        }
        *sample = (sum * 0.17).tanh();
    }
    output
}

fn counterweave(sample_rate: u32, frames: usize) -> Vec<f32> {
    let centers = log_centers(16, 70.0, 15_000.0);
    let mut filters = centers
        .iter()
        .map(|&frequency| {
            let mut filter = Biquad::default();
            filter.set_bandpass(sample_rate as f64, frequency, 3.2);
            filter
        })
        .collect::<Vec<_>>();
    let mut rng = Rng::new(0xc017_e2e1);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let section = (time / 4.0) as usize;
        let mode = section % 3;
        let rotation = (section / 3) % 4;
        let noise = rng.bipolar() as f32;
        let mut sum = 0.0;
        for (band, filter) in filters.iter_mut().enumerate() {
            let parity = (band + rotation) & 1;
            let gain = match mode {
                0 if parity == 0 => 1.0,
                1 if parity == 1 => 1.0,
                2 => 0.72,
                _ => 0.02,
            };
            sum += filter.process(noise) * gain;
        }
        *sample = sum * 0.28;
    }
    output
}

fn cantor_cathedral(sample_rate: u32, frames: usize) -> Vec<f32> {
    let positions = [
        0.015, 0.045, 0.105, 0.135, 0.225, 0.255, 0.315, 0.345, 0.655, 0.685, 0.745, 0.775, 0.865,
        0.895, 0.955, 0.985,
    ];
    let centers = positions.map(|position| 55.0 * (18_000.0_f64 / 55.0).powf(position));
    let mut filters = centers.map(|frequency| {
        let mut filter = Biquad::default();
        filter.set_bandpass(sample_rate as f64, frequency, 7.0);
        filter
    });
    let mut rng = Rng::new(0xca70_4a1a);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let depth = 0.5 + 0.5 * (2.0 * PI * time / 14.0).sin();
        let noise = rng.bipolar() as f32;
        let mut sum = 0.0;
        for (band, filter) in filters.iter_mut().enumerate() {
            let layer = 1.0 - depth * ((band % 4) as f64 / 4.0);
            sum += filter.process(noise) * layer as f32;
        }
        *sample = sum * 0.31;
    }
    output
}

fn moire_shutters(sample_rate: u32, frames: usize) -> Vec<f32> {
    let centers = log_centers(18, 60.0, 17_000.0);
    let mut filters = centers
        .iter()
        .map(|&frequency| {
            let mut filter = Biquad::default();
            filter.set_bandpass(sample_rate as f64, frequency, 4.0);
            filter
        })
        .collect::<Vec<_>>();
    let mut rng = Rng::new(0xa011_e501);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let noise = rng.bipolar() as f32;
        let mut sum = 0.0;
        let last_band = filters.len() - 1;
        for (band, filter) in filters.iter_mut().enumerate() {
            let position = band as f64 / last_band as f64;
            let a = 0.5 + 0.5 * (2.0 * PI * (3.0 * position + time / 13.0)).cos();
            let b = 0.5 + 0.5 * (2.0 * PI * (3.37 * position - time / (13.0 * 1.618))).cos();
            let gain = 0.015 + (a * b).powf(1.7);
            sum += filter.process(noise) * gain as f32;
        }
        *sample = sum * 0.34;
    }
    output
}

fn frequency_prism_clicks(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    let frequencies = log_centers(24, 90.0, 16_000.0);
    let duration = frames as f64 / sample_rate as f64;
    let mut event = 0;
    let mut base_time = 1.0;
    while base_time < duration - 0.6 {
        let curve = match event % 3 {
            0 => 1.7,
            1 => 0.58,
            _ => -1.35,
        };
        for (band, &frequency) in frequencies.iter().enumerate() {
            let position = band as f64 / (frequencies.len() - 1) as f64;
            let delay = if curve > 0.0 {
                0.35 * position.powf(curve)
            } else {
                0.35 * (1.0 - position).powf(-curve)
            };
            add_tone_burst(
                &mut output,
                sample_rate,
                base_time + delay,
                0.035 + 0.045 * position,
                frequency,
                0.11,
            );
        }
        event += 1;
        base_time += 4.1;
    }
    output
}

fn causal_anticausal_twins(sample_rate: u32, frames: usize) -> Vec<f32> {
    let kernel_frames = (sample_rate as f64 * 0.72) as usize;
    let mut causal = vec![0.0_f32; kernel_frames];
    let formants = [(180.0, 1.0), (510.0, 0.7), (1_430.0, 0.48), (3_900.0, 0.25)];
    for (index, sample) in causal.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let envelope = (-time * 10.0).exp();
        *sample = formants
            .iter()
            .map(|(frequency, gain)| (2.0 * PI * frequency * time).sin() * gain)
            .sum::<f64>() as f32
            * envelope as f32;
    }
    let mut maximum = causal.clone();
    maximum.reverse();
    let split = kernel_frames / 2;
    let mut centered = Vec::with_capacity(kernel_frames);
    centered.extend_from_slice(&causal[split..]);
    centered.extend_from_slice(&causal[..split]);
    let mut output = vec![0.0; frames];
    let duration = frames as f64 / sample_rate as f64;
    let mut time = 0.8;
    let mut mode = 0;
    while time < duration - 0.8 {
        let kernel = match mode % 3 {
            0 => &causal,
            1 => &centered,
            _ => &maximum,
        };
        add_buffer(&mut output, sample_rate, time, kernel, 0.62);
        mode += 1;
        time += 2.4;
    }
    output
}

fn coherence_melt_cloud(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut rng = Rng::new(0xc04e_3e11);
    let offsets = std::array::from_fn::<_, 20, _>(|_| rng.unit() * 2.0 * PI - PI);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let cycle = (time / 10.0).fract();
        let melt = if cycle < 0.45 {
            smooth(cycle / 0.45)
        } else if cycle < 0.85 {
            1.0
        } else {
            1.0 - smooth((cycle - 0.85) / 0.15)
        };
        let fundamental = 73.0 * (1.0 + 0.025 * (2.0 * PI * time / 17.0).sin());
        let mut sum = 0.0;
        for harmonic in 1..=20 {
            sum += (2.0 * PI * fundamental * harmonic as f64 * time + offsets[harmonic - 1] * melt)
                .sin()
                / harmonic as f64;
        }
        *sample = (sum * 0.34) as f32;
    }
    output
}

fn bessel_staircase(sample_rate: u32, frames: usize) -> Vec<f32> {
    let indices = [0.0, 0.7, 1.8, 3.2, 5.6, 2.4];
    let mut carrier_phase = 0.0_f64;
    let mut modulator_phase = 0.0_f64;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let section = (time / 4.2) as usize;
        let local = (time / 4.2).fract();
        let left = indices[section % indices.len()];
        let right = indices[(section + 1) % indices.len()];
        let modulation_index = lerp(left, right, smooth((local - 0.82).max(0.0) / 0.18));
        let ratio = 0.61 + 0.44 * (2.0 * PI * time / 27.0).sin();
        carrier_phase += 2.0 * PI * 173.0 / sample_rate as f64;
        modulator_phase += 2.0 * PI * (109.0 * ratio) / sample_rate as f64;
        *sample = (carrier_phase + modulation_index * modulator_phase.sin()).sin() as f32 * 0.48;
    }
    output
}

fn one_way_galaxy(sample_rate: u32, frames: usize) -> Vec<f32> {
    let bases = [83.0, 137.0, 229.0, 367.0, 593.0];
    let mut phases = [[0.0_f64; 4]; 5];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let sweep = -900.0 + 2_200.0 * smooth((time / 61.0).clamp(0.0, 1.0));
        let wobble = 75.0 * (2.0 * PI * time / 7.9).sin();
        let mut sum = 0.0;
        for (voice, (base, voice_phases)) in bases.iter().zip(phases.iter_mut()).enumerate() {
            for (order, phase) in voice_phases.iter_mut().enumerate() {
                let frequency = base + order as f64 * (sweep + wobble);
                if frequency.abs() < 19_000.0 {
                    *phase += 2.0 * PI * frequency / sample_rate as f64;
                    sum += phase.sin() * 0.18_f64.powi(order as i32) / (voice as f64 + 1.0).sqrt();
                }
            }
        }
        *sample = (sum * 0.31) as f32;
    }
    output
}

fn folded_constellation(sample_rate: u32, frames: usize) -> Vec<f32> {
    let frequencies = [97.0, 97.0 * 2.0_f64.sqrt(), 97.0 * (5.0_f64).sqrt()];
    let mut phases = [0.0_f64; 3];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let drive_stage = ((time / 6.1) as usize % 5) as f64;
        let local = (time / 6.1).fract();
        let drive = 0.8 + drive_stage * 0.9 + 0.7 * smooth(local);
        let bias = 0.28 * (2.0 * PI * time / 13.0).sin();
        let mut input = bias;
        for voice in 0..phases.len() {
            phases[voice] += 2.0 * PI * frequencies[voice] / sample_rate as f64;
            input += phases[voice].sin() * (0.72 - voice as f64 * 0.14);
        }
        *sample = wavefold(input * drive) as f32 * 0.52;
    }
    output
}

fn hard_sync_staircase(sample_rate: u32, frames: usize) -> Vec<f32> {
    let ratios = [1.1, 1.5, 2.0, 2.7, 3.9, 5.4, 7.3, 10.2, 14.8, 21.0, 31.0];
    let mut master_phase = 0.0_f64;
    let mut slave_phase = 0.0_f64;
    let master_increment = 73.0 / sample_rate as f64;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let position = time / 3.1;
        let step = position.floor() as usize;
        let transition = smooth((position.fract() - 0.78).max(0.0) / 0.22);
        let ratio = lerp(
            ratios[step % ratios.len()],
            ratios[(step + 1) % ratios.len()],
            transition,
        );
        let previous_master = master_phase;
        master_phase = (master_phase + master_increment).fract();
        if master_phase < previous_master {
            slave_phase = 0.0;
        } else {
            slave_phase = (slave_phase + master_increment * ratio).fract();
        }
        let saw = 2.0 * slave_phase - 1.0 - poly_blep(slave_phase, master_increment * ratio);
        *sample = (0.72 * (2.0 * PI * slave_phase).sin() + 0.28 * saw) as f32 * 0.42;
    }
    output
}

fn through_zero_chirp_pendulum(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut phase = 0.0_f64;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let pendulum = (2.0 * PI * time / 9.7).sin();
        let frequency = 7_800.0 * pendulum.powi(3) + 260.0 * (2.0 * PI * time / 17.0).sin();
        phase += 2.0 * PI * frequency / sample_rate as f64;
        let near_zero = 0.72 + 0.28 * (frequency.abs() / 7_800.0).sqrt();
        *sample = (phase.sin() * near_zero) as f32 * 0.48;
    }
    output
}

fn kicked_duffing_bell(sample_rate: u32, frames: usize) -> Vec<f32> {
    let rate = sample_rate as f64;
    let omega = 2.0 * PI * 118.0;
    let damping = 2.0 * 0.025 * omega;
    let stiffness = omega * omega;
    let cubic = stiffness * 18.0;
    let mut position = 0.0_f64;
    let mut velocity = 0.0_f64;
    let mut next_kick = (0.7 * rate) as usize;
    let mut kick_index = 0_usize;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        if index == next_kick {
            let polarity = if kick_index & 1 == 0 { 1.0 } else { -1.0 };
            velocity += polarity * (58.0 + 13.0 * (kick_index % 5) as f64);
            let interval = 1.1 + ((kick_index as f64 * 1.618_033_988_75).fract() * 3.4);
            next_kick = next_kick.saturating_add((interval * rate) as usize);
            kick_index += 1;
        }
        let acceleration = -damping * velocity - stiffness * position - cubic * position.powi(3);
        velocity += acceleration / rate;
        position += velocity / rate;
        *sample = (position * 5.0).tanh() as f32;
    }
    output
}

fn phase_doppelganger(sample_rate: u32, frames: usize) -> Vec<f32> {
    let grain_frames = (sample_rate as f64 * 0.82) as usize;
    let mut output = vec![0.0; frames];
    let mut rng = Rng::new(0xd099_e16a);
    let duration = frames as f64 / sample_rate as f64;
    let mut pair_time = 0.5;
    while pair_time < duration - 2.0 {
        let mut causal = (0..grain_frames)
            .map(|index| {
                let position = index as f64 / grain_frames as f64;
                let envelope = (-position * 7.5).exp() * (PI * position).sin().powi(2);
                (rng.bipolar() * envelope) as f32
            })
            .collect::<Vec<_>>();
        let mut previous = 0.0_f32;
        for sample in &mut causal {
            previous += 0.18 * (*sample - previous);
            *sample = previous;
        }
        let mut anticausal = causal.clone();
        anticausal.reverse();
        add_buffer(&mut output, sample_rate, pair_time, &causal, 0.58);
        add_buffer(
            &mut output,
            sample_rate,
            pair_time + 1.05,
            &anticausal,
            0.58,
        );
        pair_time += 3.05;
    }
    output
}

fn prime_band_loom(sample_rate: u32, frames: usize) -> Vec<f32> {
    let centers = log_centers(13, 70.0, 15_500.0);
    let primes = [2_usize, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41];
    let mut filters = centers
        .iter()
        .map(|&frequency| {
            let mut filter = Biquad::default();
            filter.set_bandpass(sample_rate as f64, frequency, 4.5);
            filter
        })
        .collect::<Vec<_>>();
    let mut gains = vec![0.0_f32; filters.len()];
    let mut rng = Rng::new(0x91e5_100d);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let noise = rng.bipolar() as f32;
        let mut sum = 0.0;
        for (band, filter) in filters.iter_mut().enumerate() {
            let step_seconds = 0.11 + primes[band] as f64 * 0.014;
            let step = (time / step_seconds) as u64;
            let target = if mix_hash(step ^ ((band as u64 + 1) * 0x9e37)) & 1 == 0 {
                0.03
            } else {
                1.0
            };
            gains[band] += 0.0025 * (target - gains[band]);
            sum += filter.process(noise) * gains[band];
        }
        *sample = sum * 0.31;
    }
    output
}

fn roving_zero_rake(sample_rate: u32, frames: usize) -> Vec<f32> {
    let maximum_delay = (sample_rate as f64 * 0.013).ceil() as usize + 2;
    let mut delay = vec![0.0_f32; maximum_delay];
    let mut position = 0_usize;
    let mut rng = Rng::new(0x20a0_2a4e);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let sweep = 0.5 + 0.5 * (2.0 * PI * time / 8.9).sin();
        let delay_seconds = 0.0004 * (30.0_f64).powf(sweep);
        let delay_samples = delay_seconds * sample_rate as f64;
        let integer = delay_samples.floor() as usize;
        let fraction = (delay_samples - integer as f64) as f32;
        let first = (position + maximum_delay - integer % maximum_delay) % maximum_delay;
        let second = (first + maximum_delay - 1) % maximum_delay;
        let delayed = delay[first] * (1.0 - fraction) + delay[second] * fraction;
        let input = rng.bipolar() as f32;
        delay[position] = input;
        position = (position + 1) % maximum_delay;
        let polarity = if (time / 6.4) as usize & 1 == 0 {
            1.0
        } else {
            -1.0
        };
        *sample = (input + delayed * polarity) * 0.42;
    }
    output
}

fn cantor_derivative_dust(sample_rate: u32, frames: usize) -> Vec<f32> {
    fn visit(events: &mut Vec<(f64, usize)>, start: f64, end: f64, depth: usize) {
        events.push(((start + end) * 0.5, depth));
        if depth == 5 {
            return;
        }
        let third = (end - start) / 3.0;
        visit(events, start, start + third, depth + 1);
        visit(events, end - third, end, depth + 1);
    }
    let duration = frames as f64 / sample_rate as f64;
    let mut events = Vec::new();
    visit(&mut events, 0.4, duration - 0.4, 0);
    let mut output = vec![0.0; frames];
    for (time, depth) in events {
        let polarity = if depth & 1 == 0 { 1.0 } else { -1.0 };
        let width = 0.0007 + (5 - depth) as f64 * 0.00035;
        add_derivative_gaussian(&mut output, sample_rate, time, width, polarity * 0.72);
    }
    output
}

fn chirped_comb_lens(sample_rate: u32, frames: usize) -> Vec<f32> {
    let duration = frames as f64 / sample_rate as f64;
    let mut output = vec![0.0; frames];
    let mut countdown = 0.0_f64;
    let mut polarity = 1.0_f32;
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let half_position = if time <= duration * 0.5 {
            time / (duration * 0.5)
        } else {
            (duration - time) / (duration * 0.5)
        };
        let interval = 0.4 * (0.002_f64 / 0.4).powf(half_position.clamp(0.0, 1.0));
        countdown -= 1.0 / sample_rate as f64;
        if countdown <= 0.0 {
            *sample = polarity * 0.82;
            polarity = -polarity;
            countdown += interval;
        }
    }
    output
}

fn palindromic_sinc_constellation(sample_rate: u32, frames: usize) -> Vec<f32> {
    let duration = frames as f64 / sample_rate as f64;
    let cutoffs = [380.0, 920.0, 2_100.0, 5_200.0, 11_500.0];
    let mut rng = Rng::new(0x5a1c_47aa);
    let mut output = vec![0.0; frames];
    for event in 0..47 {
        let time = 0.6 + rng.unit() * (duration * 0.5 - 1.2);
        let cutoff = cutoffs[(event * 7) % cutoffs.len()];
        let amplitude = 0.35 + 0.45 * rng.unit();
        add_windowed_sinc(&mut output, sample_rate, time, 0.006, cutoff, amplitude);
        add_windowed_sinc(
            &mut output,
            sample_rate,
            duration - time,
            0.006,
            cutoff,
            -amplitude,
        );
    }
    output
}

fn coherent_braid(sample_rate: u32, frames: usize) -> Vec<f32> {
    let periods = [17.0, 23.0, 29.0];
    let phase_offsets = [0.0, PI * 0.5, PI];
    let mut phases = [0.0_f64; 3];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let mut sum = 0.0;
        for voice in 0..3 {
            let cycle = (time / periods[voice] + voice as f64 / 3.0).fract();
            let triangle = 1.0 - (2.0 * cycle - 1.0).abs();
            let frequency = 90.0 * (14_000.0_f64 / 90.0).powf(triangle);
            phases[voice] += 2.0 * PI * frequency / sample_rate as f64;
            sum += (phases[voice] + phase_offsets[voice]).sin();
        }
        *sample = (sum / 3.0) as f32 * 0.58;
    }
    output
}

fn pinball_parabolas(sample_rate: u32, frames: usize) -> Vec<f32> {
    let walls: [f64; 7] = [180.0, 1_100.0, 4_700.0, 13_000.0, 1_100.0, 180.0, 4_700.0];
    let durations = [3.2, 4.7, 2.4, 5.1, 3.8, 2.9];
    let cycle_duration = durations.iter().sum::<f64>();
    let mut phase = 0.0_f64;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let mut cycle_time = (index as f64 / sample_rate as f64) % cycle_duration;
        let mut segment = 0;
        while cycle_time > durations[segment] {
            cycle_time -= durations[segment];
            segment += 1;
        }
        let position = cycle_time / durations[segment];
        let curved = if segment & 1 == 0 {
            position * position
        } else {
            1.0 - (1.0 - position).powi(2)
        };
        let log_frequency = lerp(walls[segment].ln(), walls[segment + 1].ln(), curved);
        phase += 2.0 * PI * log_frequency.exp() / sample_rate as f64;
        *sample = phase.sin() as f32 * 0.5;
    }
    output
}

fn dispersion_fan(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    let frequencies = log_centers(48, 80.0, 17_000.0);
    let duration = frames as f64 / sample_rate as f64;
    let mut event_time = 2.0;
    let mut event = 0;
    while event_time < duration - 1.0 {
        for (band, &frequency) in frequencies.iter().enumerate() {
            let position = band as f64 / (frequencies.len() - 1) as f64;
            let delay = if event & 1 == 0 {
                0.62 * (1.0 - position).powf(1.7)
            } else {
                0.62 * position.powf(1.7)
            };
            add_tone_burst(
                &mut output,
                sample_rate,
                event_time + delay,
                0.028 + 0.032 * position,
                frequency,
                0.075,
            );
        }
        event += 1;
        event_time += 7.1;
    }
    output
}

fn chladni_plate_scanner(sample_rate: u32, frames: usize) -> Vec<f32> {
    let modes = std::array::from_fn::<_, 16, _>(|index| {
        let x = index % 4 + 1;
        let y = index / 4 + 1;
        let frequency = 58.0 * ((x.pow(4) as f64 + 1.35 * y.pow(4) as f64).sqrt());
        (x, y, frequency.min(17_000.0))
    });
    let mut resonators = modes.map(|(_, _, frequency)| Resonator::new(sample_rate, frequency, 2.2));
    let mut weights = [0.0_f32; 16];
    let mut rng = Rng::new(0xc41a_d11e);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        if index % 128 == 0 {
            let excite_x = 0.1 + 0.8 * (0.5 + 0.5 * (2.0 * PI * time / 13.0).sin());
            let pickup_y = 0.1 + 0.8 * (0.5 + 0.5 * (2.0 * PI * time / 17.0).cos());
            for (mode, (x, y, _)) in modes.iter().enumerate() {
                weights[mode] =
                    ((PI * *x as f64 * excite_x).sin() * (PI * *y as f64 * pickup_y).sin()) as f32;
            }
        }
        let tap = if (time / 2.75).fract() < 2.0 / sample_rate as f64 {
            0.65
        } else if (time / 9.0).fract() < 0.16 {
            rng.bipolar() as f32 * 0.006
        } else {
            0.0
        };
        let mut sum = 0.0;
        for (mode, resonator) in resonators.iter_mut().enumerate() {
            sum +=
                resonator.process(tap * weights[mode]) * weights[mode] / (mode as f32 + 1.0).sqrt();
        }
        *sample = sum * 0.15;
    }
    output
}

fn buzz_bridge_string(sample_rate: u32, frames: usize) -> Vec<f32> {
    let frequencies = [73.0, 109.0, 163.0, 244.0];
    let mut lines = frequencies
        .iter()
        .map(|frequency| vec![0.0_f32; (sample_rate as f64 / frequency) as usize])
        .collect::<Vec<_>>();
    let mut positions = [0_usize; 4];
    let mut next_plucks = [0_usize; 4];
    let mut rng = Rng::new(0xb022_ba1d);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let mut sum = 0.0;
        for voice in 0..lines.len() {
            if index == next_plucks[voice] {
                for value in &mut lines[voice] {
                    *value = rng.bipolar() as f32 * 0.35;
                }
                let interval = 2.1 + voice as f64 * 0.73 + rng.unit() * 1.8;
                next_plucks[voice] = index + (interval * sample_rate as f64) as usize;
            }
            let position = positions[voice];
            let next = (position + 1) % lines[voice].len();
            let current = lines[voice][position];
            let mut replacement = (current + lines[voice][next]) * 0.4992;
            let threshold = 0.055 + voice as f32 * 0.008;
            if replacement.abs() > threshold {
                replacement -= replacement.signum() * (replacement.abs() - threshold) * 0.42;
            }
            lines[voice][position] = replacement;
            positions[voice] = next;
            sum += current;
        }
        *sample = sum * 0.31;
    }
    output
}

fn branching_horn_tree(sample_rate: u32, frames: usize) -> Vec<f32> {
    let scale = sample_rate as f64 / 48_000.0;
    let lengths = [211, 337, 521, 809].map(|length| (length as f64 * scale).max(8.0) as usize);
    let mut branches = lengths.map(|length| vec![0.0_f32; length]);
    let mut positions = [0_usize; 4];
    let mut rng = Rng::new(0xb2a4_c411);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let returns = std::array::from_fn::<_, 4, _>(|branch| branches[branch][positions[branch]]);
        let click = if (time / 3.8).fract() < 2.0 / sample_rate as f64 {
            0.8
        } else if (time / 11.0).fract() < 0.22 {
            rng.bipolar() as f32 * 0.018
        } else {
            0.0
        };
        let junction = click + returns.iter().sum::<f32>() * 0.17;
        for branch in 0..branches.len() {
            let neighbor = returns[(branch + 1) % returns.len()] * 0.09;
            branches[branch][positions[branch]] =
                junction * (0.68 - branch as f32 * 0.06) + neighbor;
            positions[branch] = (positions[branch] + 1) % branches[branch].len();
        }
        *sample = (returns.iter().sum::<f32>() * 0.43).tanh();
    }
    output
}

fn notch_braid(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut filters = [Biquad::default(); 6];
    let mut rng = Rng::new(0xa07c_b2a1);
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        if index % 128 == 0 {
            for (notch, filter) in filters.iter_mut().enumerate() {
                let motion = 0.5
                    + 0.5
                        * (2.0 * PI * time / (7.0 + notch as f64 * 1.73) + notch as f64 * 0.81)
                            .sin();
                let center = 90.0 * (140.0_f64).powf(motion);
                filter.set_notch(sample_rate as f64, center, 16.0 + notch as f64 * 2.0);
            }
        }
        let mut value = rng.bipolar() as f32 * 0.55;
        for filter in &mut filters {
            value = filter.process(value);
        }
        *sample = value;
    }
    output
}

fn formant_eclipse(sample_rate: u32, frames: usize) -> Vec<f32> {
    let vowels = [
        [730.0, 1_090.0, 2_440.0, 3_400.0],
        [270.0, 2_290.0, 3_010.0, 4_200.0],
        [300.0, 870.0, 2_240.0, 3_600.0],
        [530.0, 1_840.0, 2_480.0, 3_900.0],
    ];
    let mut filters = [Biquad::default(); 4];
    let mut rng = Rng::new(0xf02a_ec11);
    let mut pulse_phase = 0.0_f64;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        if index % 128 == 0 {
            let position = time / 5.4;
            let left = position.floor() as usize % vowels.len();
            let right = (left + 1) % vowels.len();
            let mix = smooth(position.fract());
            for formant in 0..filters.len() {
                filters[formant].set_bandpass(
                    sample_rate as f64,
                    lerp(vowels[left][formant], vowels[right][formant], mix),
                    8.0,
                );
            }
        }
        pulse_phase =
            (pulse_phase + (105.0 + 18.0 * (time / 13.0).sin()) / sample_rate as f64).fract();
        let pulse = if pulse_phase < 0.09 { 0.75 } else { -0.075 };
        let input = pulse as f32 * 0.46 + rng.bipolar() as f32 * 0.16;
        let removed = filters
            .iter_mut()
            .map(|filter| filter.process(input))
            .sum::<f32>();
        *sample = input * 0.68 - removed * 0.42;
    }
    output
}

fn golden_fireflies(sample_rate: u32, frames: usize) -> Vec<f32> {
    let duration = frames as f64 / sample_rate as f64;
    let primes: [f64; 8] = [2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0];
    let mut output = vec![0.0; frames];
    let golden = 1.618_033_988_75_f64;
    for event in 0..233 {
        let time = 0.3 + ((event as f64 * golden).fract() * (duration - 0.6));
        let octave = ((event * 37) % 91) as f64 / 90.0 * 7.2;
        let detune = primes[event % primes.len()].sqrt() / 2.0;
        let frequency = (55.0 * 2.0_f64.powf(octave) * detune).clamp(60.0, 17_000.0);
        let grain_duration = 0.035 + ((event * 29) % 100) as f64 / 100.0 * 0.21;
        add_tone_burst(
            &mut output,
            sample_rate,
            time,
            grain_duration,
            frequency,
            0.24,
        );
    }
    output
}

fn phase_code_pulse_bursts(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    let widths = [0.0005, 0.001, 0.002, 0.004];
    let duration = frames as f64 / sample_rate as f64;
    let mut burst_time = 0.8;
    let mut burst = 0;
    while burst_time < duration - 1.0 {
        let width = widths[burst % widths.len()];
        let chip_frames = (width * sample_rate as f64).round().max(2.0) as usize;
        let total_frames = 127 * chip_frames;
        let start = (burst_time * sample_rate as f64) as usize;
        let mut register = 0x7f_u8;
        let mut previous_sign = 0.0;
        for chip in 0..127 {
            let sign = if register & 1 == 0 { -1.0 } else { 1.0 };
            let feedback = ((register >> 6) ^ (register >> 5)) & 1;
            register = ((register << 1) | feedback) & 0x7f;
            let transition_frames = (chip_frames / 3).clamp(2, 16);
            for offset in 0..chip_frames {
                let target = start + chip * chip_frames + offset;
                if target >= output.len() {
                    break;
                }
                let position = (chip * chip_frames + offset) as f64 / total_frames as f64;
                let window = (PI * position).sin().powi(2);
                let transition = smooth(offset as f64 / (transition_frames - 1) as f64);
                let code = lerp(previous_sign, sign, transition);
                output[target] += (code * window * 0.56) as f32;
            }
            previous_sign = sign;
        }
        burst += 1;
        burst_time += 4.7;
    }
    output
}

fn harmonic_focus_lattice(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let focus_phase = (2.0 * PI * time / 7.3).sin();
        let defocus = focus_phase * focus_phase.signum() * 0.045;
        let fundamental = 67.0;
        let mut sum = 0.0;
        for harmonic in 1..=32 {
            let harmonic_phase = 2.0 * PI * fundamental * harmonic as f64 * time
                + defocus * (harmonic * harmonic) as f64;
            sum += harmonic_phase.sin() / harmonic as f64;
        }
        *sample = (sum * 0.29) as f32;
    }
    output
}

fn modal_group_delay_staircases(sample_rate: u32, frames: usize) -> Vec<f32> {
    let frequencies = log_centers(18, 95.0, 12_500.0);
    let mut output = vec![0.0; frames];
    let duration = frames as f64 / sample_rate as f64;
    let mut event_time = 1.0;
    let mut event = 0;
    while event_time < duration - 2.0 {
        for (mode, &frequency) in frequencies.iter().enumerate() {
            let position = mode as f64 / (frequencies.len() - 1) as f64;
            let delay = if event & 1 == 0 {
                0.85 * position
            } else {
                0.85 * (1.0 - position)
            };
            add_damped_tone(
                &mut output,
                sample_rate,
                event_time + delay,
                1.0 + 0.7 * (1.0 - position),
                frequency,
                0.095,
            );
        }
        event += 1;
        event_time += 6.2;
    }
    output
}

fn chebyshev_elevator(sample_rate: u32, frames: usize) -> Vec<f32> {
    let orders = [2_usize, 3, 5, 8, 13];
    let mut phase = 0.0_f64;
    let mut values = [0.0_f64; 14];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        phase += 2.0 * PI * (83.0 + 19.0 * (2.0 * PI * time / 21.0).sin()) / sample_rate as f64;
        let input = phase.sin() * 0.94;
        values[0] = 1.0;
        values[1] = input;
        for order in 2..values.len() {
            values[order] = 2.0 * input * values[order - 1] - values[order - 2];
        }
        let position = time / 4.6;
        let stage = position.floor() as usize;
        let mix = smooth((position.fract() - 0.68).max(0.0) / 0.32);
        let left = values[orders[stage % orders.len()]];
        let right = values[orders[(stage + 1) % orders.len()]];
        *sample = lerp(left, right, mix) as f32 * 0.52;
    }
    output
}

fn clock_shattered_bell(sample_rate: u32, frames: usize) -> Vec<f32> {
    let clock_rates = [4.0, 17.0, 61.0, 230.0, 800.0];
    let mut rng = Rng::new(0xc10c_be11);
    let mut held = 0.0_f64;
    let mut slewed = 0.0_f64;
    let mut clock_phase = 0.0_f64;
    let mut carrier_phase = 0.0_f64;
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let stage = (time / 6.1) as usize;
        let clock_rate = clock_rates[stage % clock_rates.len()];
        clock_phase += clock_rate / sample_rate as f64;
        if clock_phase >= 1.0 {
            clock_phase -= 1.0;
            held = rng.bipolar();
        }
        let slew = (0.0015 + stage as f64 % 5.0 * 0.0008).min(0.02);
        slewed += slew * (held - slewed);
        let frequency = (620.0 + slewed * 2_900.0).abs().clamp(35.0, 15_000.0);
        carrier_phase += 2.0 * PI * frequency / sample_rate as f64;
        let strike = (-((time / 4.9).fract()) * 8.0).exp();
        *sample = (carrier_phase.sin() * (0.25 + 0.75 * strike)) as f32 * 0.5;
    }
    output
}

fn rectifier_tide(sample_rate: u32, frames: usize) -> Vec<f32> {
    let mut phases = [0.0_f64; 4];
    let mut output = vec![0.0; frames];
    for (index, sample) in output.iter_mut().enumerate() {
        let time = index as f64 / sample_rate as f64;
        let frequencies = [
            91.0 + 37.0 * (time / 11.0).sin(),
            147.0 + 61.0 * (time / 17.0).cos(),
            239.0 + 83.0 * (time / 13.0).sin(),
            383.0 + 109.0 * (time / 19.0).cos(),
        ];
        let mut input = 0.0;
        for voice in 0..phases.len() {
            phases[voice] += 2.0 * PI * frequencies[voice] / sample_rate as f64;
            input += phases[voice].sin() / (voice as f64 + 1.0).sqrt();
        }
        input *= 0.48;
        let half = input.max(0.0) * 1.7 - 0.34;
        let full = input.abs() * 1.35 - 0.5;
        let biased = (input + 0.31).max(0.0) - 0.31;
        let position = time / 6.8;
        let stage = position.floor() as usize % 3;
        let mix = smooth((position.fract() - 0.7).max(0.0) / 0.3);
        let modes = [half, full, biased];
        *sample = lerp(modes[stage], modes[(stage + 1) % modes.len()], mix) as f32 * 0.58;
    }
    output
}

fn finish(samples: &mut [f32], sample_rate: u32) {
    let mean = samples.iter().map(|&sample| sample as f64).sum::<f64>() / samples.len() as f64;
    for sample in samples.iter_mut() {
        *sample -= mean as f32;
        if !sample.is_finite() {
            *sample = 0.0;
        }
    }
    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, &sample| peak.max(sample.abs()));
    let rms = (samples
        .iter()
        .map(|&sample| sample as f64 * sample as f64)
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt() as f32;
    // Sparse impulses and hard phase codes can overshoot substantially in the
    // preparation high/low-pass filters, so leave them additional peak margin.
    let crest_factor = peak / rms.max(1.0e-9);
    let peak_target = if crest_factor > 5.0 { 0.68 } else { 0.84 };
    let gain = (0.18 / rms.max(1.0e-9)).min(peak_target / peak.max(1.0e-9));
    let fade_frames = (sample_rate as f32 * 0.02) as usize;
    let length = samples.len();
    for (index, sample) in samples.iter_mut().enumerate() {
        let edge = index.min(length - 1 - index);
        let fade = if edge < fade_frames {
            (0.5 - 0.5 * (PI * edge as f64 / fade_frames as f64).cos()) as f32
        } else {
            1.0
        };
        *sample *= gain * fade;
    }
}

fn poly_blep(time: f64, increment: f64) -> f64 {
    if time < increment {
        let value = time / increment;
        value + value - value * value - 1.0
    } else if time > 1.0 - increment {
        let value = (time - 1.0) / increment;
        value * value + value + value + 1.0
    } else {
        0.0
    }
}

fn smooth(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn lerp(left: f64, right: f64, phase: f64) -> f64 {
    left + (right - left) * phase
}

fn wavefold(value: f64) -> f64 {
    let wrapped = (value + 1.0).rem_euclid(4.0);
    if wrapped < 2.0 {
        wrapped - 1.0
    } else {
        3.0 - wrapped
    }
}

fn log_centers(count: usize, minimum: f64, maximum: f64) -> Vec<f64> {
    (0..count)
        .map(|index| minimum * (maximum / minimum).powf(index as f64 / (count - 1) as f64))
        .collect()
}

fn add_click(output: &mut [f32], sample_rate: u32, time: f64, width: f64, amplitude: f64) {
    let start = (time * sample_rate as f64).round() as isize;
    let length = (width * sample_rate as f64).round().max(4.0) as usize;
    for index in 0..length {
        let target = start + index as isize;
        if target < 0 || target as usize >= output.len() {
            continue;
        }
        let phase = index as f64 / (length - 1) as f64;
        let window = 0.5 - 0.5 * (2.0 * PI * phase).cos();
        let bipolar = (2.0 * PI * phase).sin();
        output[target as usize] += (amplitude * window * bipolar) as f32;
    }
}

fn add_derivative_gaussian(
    output: &mut [f32],
    sample_rate: u32,
    time: f64,
    width: f64,
    amplitude: f64,
) {
    let half_length = (width * 4.0 * sample_rate as f64).round().max(2.0) as isize;
    let center = (time * sample_rate as f64).round() as isize;
    for offset in -half_length..=half_length {
        let target = center + offset;
        if target < 0 || target as usize >= output.len() {
            continue;
        }
        let x = offset as f64 / (width * sample_rate as f64).max(1.0);
        let value = -x * (-0.5 * x * x).exp();
        output[target as usize] += (amplitude * value) as f32;
    }
}

fn add_windowed_sinc(
    output: &mut [f32],
    sample_rate: u32,
    time: f64,
    duration: f64,
    cutoff: f64,
    amplitude: f64,
) {
    let length = (duration * sample_rate as f64).round().max(5.0) as usize | 1;
    let half_length = (length / 2) as isize;
    let center = (time * sample_rate as f64).round() as isize;
    for offset in -half_length..=half_length {
        let target = center + offset;
        if target < 0 || target as usize >= output.len() {
            continue;
        }
        let relative_time = offset as f64 / sample_rate as f64;
        let angle = 2.0 * PI * cutoff * relative_time;
        let sinc = if offset == 0 {
            1.0
        } else {
            angle.sin() / angle
        };
        let position = (offset + half_length) as f64 / (2 * half_length) as f64;
        let window = 0.5 - 0.5 * (2.0 * PI * position).cos();
        output[target as usize] += (amplitude * sinc * window) as f32;
    }
}

fn add_damped_tone(
    output: &mut [f32],
    sample_rate: u32,
    start_time: f64,
    duration: f64,
    frequency: f64,
    amplitude: f64,
) {
    let start = (start_time * sample_rate as f64).round() as isize;
    let length = (duration * sample_rate as f64).round().max(2.0) as usize;
    for index in 0..length {
        let target = start + index as isize;
        if target < 0 || target as usize >= output.len() {
            continue;
        }
        let time = index as f64 / sample_rate as f64;
        let position = index as f64 / (length - 1) as f64;
        let attack = smooth((position * 80.0).min(1.0));
        let envelope = attack * (-6.0 * position).exp();
        output[target as usize] +=
            (amplitude * envelope * (2.0 * PI * frequency * time).sin()) as f32;
    }
}

fn mix_hash(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[allow(clippy::too_many_arguments)]
fn add_chirp(
    output: &mut [f32],
    sample_rate: u32,
    start_time: f64,
    duration: f64,
    start_frequency: f64,
    end_frequency: f64,
    phase_offset: f64,
    amplitude: f64,
) {
    let start = (start_time * sample_rate as f64).round() as isize;
    let length = (duration * sample_rate as f64).round() as usize;
    let slope = (end_frequency - start_frequency) / duration;
    for index in 0..length {
        let target = start + index as isize;
        if target < 0 || target as usize >= output.len() {
            continue;
        }
        let time = index as f64 / sample_rate as f64;
        let position = index as f64 / length.max(2) as f64;
        let window = 0.5 - 0.5 * (2.0 * PI * position).cos();
        let phase = phase_offset + 2.0 * PI * (start_frequency * time + 0.5 * slope * time * time);
        output[target as usize] += (amplitude * window * phase.sin()) as f32;
    }
}

fn add_tone_burst(
    output: &mut [f32],
    sample_rate: u32,
    start_time: f64,
    duration: f64,
    frequency: f64,
    amplitude: f64,
) {
    add_chirp(
        output,
        sample_rate,
        start_time,
        duration,
        frequency,
        frequency,
        0.0,
        amplitude,
    );
}

fn add_buffer(output: &mut [f32], sample_rate: u32, time: f64, buffer: &[f32], gain: f32) {
    let start = (time * sample_rate as f64).round() as usize;
    for (target, &source) in output.iter_mut().skip(start).zip(buffer) {
        *target += source * gain;
    }
}

#[derive(Clone, Copy)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn unit(&mut self) -> f64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        (value >> 11) as f64 / ((1_u64 << 53) - 1) as f64
    }

    fn bipolar(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn set_bandpass(&mut self, sample_rate: f64, frequency: f64, q: f64) {
        let frequency = frequency.clamp(20.0, sample_rate * 0.45);
        let omega = 2.0 * PI * frequency / sample_rate;
        let alpha = omega.sin() / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        self.b0 = (alpha / a0) as f32;
        self.b1 = 0.0;
        self.b2 = (-alpha / a0) as f32;
        self.a1 = (-2.0 * omega.cos() / a0) as f32;
        self.a2 = ((1.0 - alpha) / a0) as f32;
    }

    fn set_notch(&mut self, sample_rate: f64, frequency: f64, q: f64) {
        let frequency = frequency.clamp(20.0, sample_rate * 0.45);
        let omega = 2.0 * PI * frequency / sample_rate;
        let alpha = omega.sin() / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 / a0) as f32;
        self.b1 = (-2.0 * omega.cos() / a0) as f32;
        self.b2 = self.b0;
        self.a1 = self.b1;
        self.a2 = ((1.0 - alpha) / a0) as f32;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

struct Resonator {
    coefficient: f32,
    radius_squared: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    fn new(sample_rate: u32, frequency: f64, decay_seconds: f64) -> Self {
        let radius = (-1.0 / (decay_seconds * sample_rate as f64)).exp();
        Self {
            coefficient: (2.0 * radius * (2.0 * PI * frequency / sample_rate as f64).cos()) as f32,
            radius_squared: (radius * radius) as f32,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = input + self.coefficient * self.y1 - self.radius_squared * self.y2;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_generator_is_finite_and_non_silent() {
        for id in SOURCE_IDS {
            let samples = render(id, 4_000, 4_000 * SECONDS).unwrap();
            assert_eq!(samples.len(), 4_000 * SECONDS, "{id}");
            assert!(samples.iter().all(|sample| sample.is_finite()), "{id}");
            let energy = samples.iter().map(|sample| sample * sample).sum::<f32>();
            assert!(energy > 1.0e-8, "{id} is silent");
        }
    }

    #[test]
    fn unknown_generator_is_rejected() {
        assert!(render("not_a_source", 48_000, 100).is_err());
    }
}
