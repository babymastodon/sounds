# conv8

conv8 is an additive-pitch playground built from conv7. It reuses the same 48 open-licensed inputs, 24×24 short-to-long matrix, complementary stereo trims, convolution-only outputs, ten-second final crossfades, verification gates, and FLAC/AAC/Opus encoders. The experiment compares adding the same sparse, synthesized gestures to either the long or short input before convolution.

## Controlled two-way comparison

Every short/long pair is rendered twice:

1. **Long additive synth** adds notes to the 25–35 second input while leaving the short input unchanged.
2. **Short additive synth** adds notes to the 5–15 second input while leaving the long input unchanged.

The ordered pair of track names deterministically selects one of three aggressively ruined instrument families. The selection is approach-independent, so a pair uses the same instrument whether its long or short input is augmented. A second set of hashes varies the intensity inside the listed ranges for each pair:

1. **Modal-noise resonator** runs three six-mode banks spread `±38–55` cents around the pitch and limits upper-mode disorder to `±3–6%`. Its output is always 60% clean modal core, 10–18% noise passed through the same 18 resonant bands, and 22–30% parallel ruined signal. The noise has no direct broadband path and its 15 ms excitation is only a small increase above the sustained excitation. Asymmetric clipping at `3.8–5.2×` drive, one or two wavefolds, and 9-bit quantization affect only the parallel ruin path.
2. **Inharmonic FM/PM** runs three carriers spread `±28–48` cents, each driven by non-integer modulators at `sqrt(2)` and `2.731` times its frequency. Its index now falls from `6–9` to `2.5–4`, self feedback is `0.35–0.55`, cross-feedback is only 8% of the feedback term, and the former sample-held phase jumps are replaced by smooth 0.08-radian drift. The final voice is 45% clean detuned carrier, 40% raw FM, and 15% parallel 10-bit folded ruin, keeping metallic grit around an audible pitch center.
3. **Destroyed saw cluster** runs seven voices across `±55–85` cents with `±12–24` cents of independent drift. It blends `60–80%` deliberately aliased signal with `45–70%` hard-sync discontinuities, holds oscillator samples for two to four frames, adds digital dust, clips at `7–10×` drive, wavefolds three or four times, and finally crushes to 5–7 bits. This is intentionally the grittiest family.

Only two or three notes occur in a short input and three to six in a long input. A pair-and-role hash chooses the count, rotates chord-tone pattern `0,1,2,1,0,2,0,1`, and jitters each onset within an evenly distributed slot, so gestures span the clip without becoming a beat grid.

The ordered pair of input names hashes to a three-entry gesture profile, one entry per chord tone. Levels range from 1.5 dB above to 4.25 dB below the surrounding 1.5-second local RMS window. Duration varies inversely from 0.4 to about 1.504 seconds:

~~~text
duration = 0.4 × 10^((dB_below_local + 1.5) / 10)
~~~

Consequently a loud note is short and a quiet note rings longer, while `relative_power × duration` stays nearly constant. The same pair hash independently assigns each pitch one of four controlled envelopes: a bitten sustain with a 12 ms attack, a gradual decay to 60%, a long hold, and a 25% cosine release; a quadratic swell with a slower collapse; a power-2.5 reverse rise with a broader decay; or a softly gated three-cycle tremolo arc with a 20% floor. The instrument, pitch profile, and envelopes are approach-independent, while the sparse count and placements include the processed role so the two input lengths are covered appropriately.

The bitten sustain replaces the percussive v12 pluck, which put roughly 91% of a one-second note's energy into its first 150 ms after oscillator interaction. The replacement envelope has a `1.555×` normalized peak, remains above 12% amplitude for 92.6% of its duration, and places only 33.4% of its own one-second energy in the first 150 ms. All four shapes still start and finish at zero.

Regression tests render every instrument/envelope combination and require a detectable period around the intended pitch. At 151 Hz the v13 autocorrelation scores are `0.721–0.807` for modal, `0.190–0.367` for FM, and `0.401–0.439` for destroyed saw; the diagnosed modal and FM designs were previously about `0.01–0.11`. These tests specifically prevent a future grit change from turning either generator back into an unpitched impact.

Each complete oscillator-plus-envelope note is normalized to unit RMS before its hashed local-RMS target is applied. Thus added grit and envelope changes redistribute samples in time and spectrum without changing the note's average power. Combined with the inverse level/duration equation above, this also preserves the intended nearly constant total energy across differently shaped gestures.

The synthesized stem retains the input's exact frame count. Its initial local levels preserve the amplitude/duration variation above, but those levels alone proved too easy to mask: in the superseded v8 render the base processed-minus-dry contribution had medians of −11.16 dB for long-input augmentation and −8.94 dB for short-input augmentation, with worst cases near −18 dB.

v10 therefore calibrates audibility after convolution. For each pair and stereo trim layout it separately computes the unaugmented convolution `C_dry` and the tone-only convolution `C_tone`, then applies only the positive gain needed to put the stereo RMS of the tone contribution at least 1.5 dB below the unaugmented convolution. An already-strong stem is never attenuated:

~~~text
unscaled_dB = 20 log10(rms(C_tone) / rms(C_dry))
gain_dB     = max(0, −1.5 − unscaled_dB)
C_output    = C_dry + 10^(gain_dB/20) C_tone
~~~

By linearity this is exactly the convolution of the partner with `input + gain × tone_stem`; it is not a post-convolution dry-source mix. The tone component is therefore close enough to the underlying convolution to remain foreground-audible even when a particular partner spectrum strongly masks its base level. Verification checks the base stem, exact gain identity, and −1.5 dB minimum for every pair. Every gesture hash, instrument, exact intensity parameters, base pitch level, duration, envelope, scheduled count, unscaled level, and applied gain is recorded in `metrics.csv`.

There is no post-convolution dry-source mix: both output channels are convolution products. Let `A` be the short input, `B` the long input, `D` half the shorter duration, `P(A)` the augmented short input, and `Q(B)` the augmented long input.

Long-additive version:

~~~text
Q_short = Q(B) without its final D frames
A_short = A without its initial D frames

left  = linear_convolution(A,       Q_short)
right = linear_convolution(A_short, Q(B))
~~~

Short-additive version:

~~~text
B_short = B without its final D frames
P_short = P(A) without its initial D frames

left  = linear_convolution(P(A),    B_short)
right = linear_convolution(P_short, B)
~~~

Both have `length(A) + length(B) - D - 1` frames per channel and use identical pairs, trims, conditioning, ordering, and encoding settings.

## Naturally detuned 13-EDO scale

The catalog uses 13 equal divisions of the ordinary 2:1 octave. Unlike Bohlen–Pierce, which is designed around consonant 3:5:7 relationships, this tuning places familiar harmonic landmarks deliberately off center. With base frequency 110 Hz:

~~~text
frequency(step) = 110 × 2^(step/13)
chord(root) = [root, root+4, root+8] modulo 13
~~~

This gives 13 transpositions of a recognizably major-like but intrinsically mistuned chord: its third is about 31 cents flat and its fifth about 37 cents sharp. [chords.tsv](chords.tsv) is the rounded reference catalog; the formula above is authoritative. This is a mathematical tuning and does not claim to represent a living musical tradition.

Chord selection is content-based and approach-independent:

1. Hash every complete prepared WAV file with domain-separated FNV-1a-64.
2. Hash the legacy fixed tag `conv8-bohlen-pierce-pair-v1`, the short-file hash, then the long-file hash. The tag is intentionally retained so existing pairs keep the same comparable root index after the tuning change.
3. Select `pair_hash mod 13`.

The ordered short→long role, chord index, steps, frequencies, both file fingerprints, and name-derived gesture profile are recorded in each `metrics.csv`, making assignments deterministic across reruns. The chord and per-pitch gestures are identical between approaches.

## Corpus and matrix

The [sources.tsv](sources.tsv) manifest is inherited unchanged from conv7: one 5–15 second and one 25–35 second recording in each of 24 themes. Only short→long pairs exist, producing 576 WAVs per approach and 1,152 total:

~~~text
outputs/long_additive_synth/
outputs/short_additive_synth/
~~~

Each directory contains its own 24×24 `matrix.csv`, detailed metrics, `sparse-hashed-13edo-pitched-grit-v13` algorithm marker, and verification report.

## Run the complete pipeline

Requirements: a current Rust toolchain, curl, FFmpeg/FFprobe, awk, and sha256sum.

~~~bash
./scripts/render_all.sh
~~~

Stages can also be run separately:

~~~bash
./scripts/download_samples.sh
cargo run --release -- render
cargo run --release -- verify
cargo run --release -- concat
~~~

Render, verify, and concat process long-additive first and short-additive second. `DOWNLOAD_JOBS` controls download concurrency, while `CONV_JOBS` or `--jobs` controls FFT rendering and verification. Render reuse requires a matching algorithm marker; `--force` rebuilds every file.

## Final masters

Each approach is independently concatenated in pair order with crossfades of up to ten seconds and encoded as RF64 PCM, lossless FLAC, 192 kbit/s AAC/M4A, stereo 128 kbit/s Opus, and stereo 32 kbit/s Opus:

~~~text
outputs/final/long_additive_synth/
outputs/final/short_additive_synth/
~~~

Every compressed master is decoded end to end after encoding. Downloaded inputs, matrix WAVs, and final media are ignored by Git.

## Full-run audit

The `sparse-hashed-13edo-pitched-grit-v13` run finished on 2026-07-19 with eight logical CPU cores. Each approach produced exactly 576 stereo WAVs totaling 3,889,175,040 bytes; together they contain 1,152 WAVs and 7,778,350,080 bytes. Forced rendering and built-in verification took 1:30.29 with 1,275,936 KiB peak resident memory. A second independent full-file decode and deterministic-metadata verification took 16.40 seconds with 513,872 KiB peak resident memory.

The chord, gesture, instrument, and exact instrument-parameter columns have the same SHA-256 signature, `2d1d32ef1897daa1d541e3cd31c644e7e707b300620c4ba26b638aaa295a0850`, in both metrics tables. All 13 chords occur 35–58 times per approach. The filename hash assigns 190 pairs to modal noise, 175 to inharmonic FM, and 211 to destroyed saw. Across the 576 pair profiles, the 1,728 pitch gestures comprise 420 bitten sustains, 432 reverse plucks, 432 swells, and 444 tremolo arcs. Realized base pitch levels span 1.499 dB above to 4.248 dB below local RMS, and durations span 0.400–1.503 seconds. Long inputs contain 3–6 notes; short inputs contain 2–3. All 29 tests pass, including unit-RMS and integrated-energy invariants across all twelve instrument/envelope combinations and the new pitched-period regression.

The convolution-domain audit again confirms that partner spectra strongly affect the nominal tone stems. The existing one-way calibration boosts only masked cases and guarantees the −1.5 dB floor without attenuating already-strong stems:

| Approach | Unscaled convolved tone | Pairs needing no boost | Boost median | Boost range | Final convolved tone |
|---|---:|---:|---:|---:|---:|
| Long additive | −38.29 to +20.76 dB | 41 | +9.95 dB | 0 to +36.79 dB | −1.50 to +20.76 dB |
| Short additive | −30.38 to +22.62 dB | 83 | +7.54 dB | 0 to +28.88 dB | −1.50 to +22.62 dB |

| Approach | Base correlation, minimum | Base tone-minus-input range | Output RMS range dBFS | Maximum peak | Maximum L/R RMS delta | Stereo-difference range dBFS |
|---|---:|---:|---:|---:|---:|---:|
| Long additive | 0.9137 | −18.13 to −6.95 dB | −20.84 to −20.10 | 0.876 | 0.252 dB | −25.14 to −16.62 |
| Short additive | 0.8498 | −17.30 to −4.46 dB | −20.70 to −20.08 | 0.874 | 0.485 dB | −23.63 to −15.50 |

Every matrix passed finite-sample, clipping, peak, RMS, DC-offset, exact-length, matrix-membership, chord, gesture, instrument, sparse-count, convolved-tone-floor, and distinct-stereo checks.

Both final programs contain 696,287,424 frames (4:01:45.988). Every one of their 575 transitions receives the full ten-second crossfade, so the timelines remain sample-aligned. Forced assembly, eight parallel-within-approach encodes, probes, and full parallel decode checks took 7:10.06 with 116,128 KiB peak resident memory.

| Approach | RF64 | FLAC | AAC/M4A | Opus 128k | Opus 32k |
|---|---:|---:|---:|---:|---:|
| Long additive | 2,785,149,776 | 1,007,129,876 | 350,869,773 | 220,358,022 | 54,829,455 |
| Short additive | 2,785,149,776 | 1,054,909,135 | 350,869,881 | 216,930,312 | 52,775,221 |

Sizes are bytes. Every compressed master decoded without errors and independently probed as stereo 48 kHz with the expected codec and duration.
