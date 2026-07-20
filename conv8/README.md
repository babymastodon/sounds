# conv8

conv8 is an additive-pitch playground built from conv7. It reuses the same 48 open-licensed inputs, 24×24 short-to-long matrix, complementary stereo trims, convolution-only outputs, ten-second final crossfades, verification gates, and FLAC/AAC/Opus encoders. The experiment compares adding the same sparse, synthesized gestures to either the long or short input before convolution.

## Controlled two-way comparison

Every short/long pair is rendered twice:

1. **Long additive synth** adds notes to the 25–35 second input while leaving the short input unchanged.
2. **Short additive synth** adds notes to the 5–15 second input while leaving the long input unchanged.

The ordered pair of track names deterministically selects one of three aggressively ruined instrument families. The selection is approach-independent, so a pair uses the same instrument whether its long or short input is augmented. A second set of hashes varies the intensity inside the listed ranges for each pair:

1. **Modal-noise resonator** runs three six-mode banks spread `±38–55` cents around the pitch, disorders upper modes by `±7–12%`, ring-modulates the banks, and mixes `30–44%` white/sample-held scrape noise behind an enlarged strike. Asymmetric clipping at `4.5–6×` drive, two or three wavefolds, and 9-bit output quantization supply the broken edge.
2. **Inharmonic FM/PM** runs three carriers spread `±28–48` cents, each driven by non-integer modulators at `sqrt(2)` and `2.731` times its frequency. A modulation index falling from `10–14` to `4–6`, `0.72–0.92` self/cross feedback, sample-held phase jitter, `2.3–3.2×` drive, two or three folds, and 10-bit quantization push it into unstable metallic noise.
3. **Destroyed saw cluster** runs seven voices across `±55–85` cents with `±12–24` cents of independent drift. It blends `60–80%` deliberately aliased signal with `45–70%` hard-sync discontinuities, holds oscillator samples for two to four frames, adds digital dust, clips at `7–10×` drive, wavefolds three or four times, and finally crushes to 5–7 bits. This is intentionally the grittiest family.

Only two or three notes occur in a short input and three to six in a long input. A pair-and-role hash chooses the count, rotates chord-tone pattern `0,1,2,1,0,2,0,1`, and jitters each onset within an evenly distributed slot, so gestures span the clip without becoming a beat grid.

The ordered pair of input names hashes to a three-entry gesture profile, one entry per chord tone. Levels range from 1.5 dB above to 4.25 dB below the surrounding 1.5-second local RMS window. Duration varies inversely from 0.4 to about 1.504 seconds:

~~~text
duration = 0.4 × 10^((dB_below_local + 1.5) / 10)
~~~

Consequently a loud note is short and a quiet note rings longer, while `relative_power × duration` stays nearly constant. The same pair hash independently assigns each pitch one of four controlled envelopes: a 4 ms pluck whose early drop is spread over 8% of the note, a quadratic swell with a slower collapse, a power-2.5 reverse rise with a broader decay, or a softly gated three-cycle tremolo arc with a 20% floor. The instrument, pitch profile, and envelopes are approach-independent, while the sparse count and placements include the processed role so the two input lengths are covered appropriately.

Compared with v11, envelope-only normalized peaks fall from `3.83×` to `3.32×` RMS for pluck, `3.18×` to `2.51×` for swell, `3.51×` to `2.70×` for reverse pluck, and `2.11×` to `1.94×` for tremolo. The portion above 12% amplitude expands from about 15% to 29% for pluck, 36% to 54% for swell, 31% to 49% for reverse pluck, and 44% to 81% for tremolo. The result retains articulation without concentrating the fixed power into such extreme peaks.

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

Each directory contains its own 24×24 `matrix.csv`, detailed metrics, `sparse-hashed-13edo-broader-envelopes-v12` algorithm marker, and verification report.

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

## Previous full-run baseline

The superseded `sparse-hashed-13edo-aggressive-grit-v11` run finished on 2026-07-19 with eight logical CPU cores. Its measurements remain below as a baseline while the broader-envelope v12 output is regenerated. Each approach produced exactly 576 stereo WAVs totaling 3,889,175,040 bytes; together they contain 1,152 WAVs and 7,778,350,080 bytes. Forced rendering and built-in verification took 1:12.51 with 1,306,648 KiB peak resident memory. A second independent full-file decode and deterministic-metadata verification took 14.44 seconds with 514,464 KiB peak resident memory.

The chord, gesture, instrument, and exact instrument-parameter columns have the same SHA-256 signature, `eacaadb3b8107caaa2b5cf9cd87f23d3c16ac15284c4f271f58e168c194d2740`, in both metrics tables. All 13 chords occur 35–58 times per approach. The filename hash assigns 190 pairs to modal noise, 175 to inharmonic FM, and 211 to destroyed saw. Across the 576 pair profiles, the 1,728 pitch gestures comprise 420 plucks, 432 reverse plucks, 432 swells, and 444 tremolo arcs. Realized base pitch levels span 1.499 dB above to 4.248 dB below local RMS, and durations span 0.400–1.503 seconds. Long inputs contain 3–6 notes; short inputs contain 2–3. Tests confirm identical unit RMS and integrated energy across all twelve instrument/envelope combinations before the unchanged hashed target gain.

The convolution-domain audit confirmed that fixed input gain was inadequate: partner spectra made nominal tone stems vary over roughly 49–57 dB. v10 boosts only the masked cases and guarantees the −1.5 dB floor:

| Approach | Unscaled convolved tone | Pairs needing no boost | Boost median | Boost range | Final convolved tone |
|---|---:|---:|---:|---:|---:|
| Long additive | −37.01 to +20.72 dB | 21 | +13.02 dB | 0 to +35.51 dB | −1.50 to +20.72 dB |
| Short additive | −30.74 to +21.38 dB | 46 | +10.88 dB | 0 to +29.24 dB | −1.50 to +21.38 dB |

| Approach | Base correlation, minimum | Base tone-minus-input range | Output RMS range dBFS | Maximum peak | Maximum L/R RMS delta | Stereo-difference range dBFS |
|---|---:|---:|---:|---:|---:|---:|
| Long additive | 0.9126 | −18.13 to −6.95 dB | −20.71 to −20.09 | 0.881 | 0.321 dB | −24.27 to −16.61 |
| Short additive | 0.8542 | −17.30 to −4.46 dB | −20.50 to −20.09 | 0.879 | 0.265 dB | −20.25 to −16.10 |

Every matrix passed finite-sample, clipping, peak, RMS, DC-offset, exact-length, matrix-membership, chord, gesture, instrument, sparse-count, convolved-tone-floor, and distinct-stereo checks.

Both final programs contain 696,287,424 frames (4:01:45.988). Every one of their 575 transitions receives the full ten-second crossfade, so the timelines remain sample-aligned. Forced assembly, eight parallel-within-approach encodes, probes, and full parallel decode checks took 7:04.70 with 116,140 KiB peak resident memory.

| Approach | RF64 | FLAC | AAC/M4A | Opus 128k | Opus 32k |
|---|---:|---:|---:|---:|---:|
| Long additive | 2,785,149,776 | 1,234,500,321 | 350,869,677 | 221,811,944 | 55,499,513 |
| Short additive | 2,785,149,776 | 1,324,163,982 | 350,869,929 | 216,938,041 | 52,757,263 |

Sizes are bytes. Every compressed master decoded without errors and independently probed as stereo 48 kHz with the expected codec and duration.
