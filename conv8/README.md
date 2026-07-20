# conv8

conv8 is an additive-pitch playground built from conv7. It reuses the same 48 open-licensed inputs, 24×24 short-to-long matrix, complementary stereo trims, convolution-only outputs, ten-second final crossfades, verification gates, and FLAC/AAC/Opus encoders. The experiment compares adding the same sparse, synthesized gestures to either the long or short input before convolution.

## Controlled two-way comparison

Every short/long pair is rendered twice:

1. **Long additive synth** adds notes to the 25–35 second input while leaving the short input unchanged.
2. **Short additive synth** adds notes to the 5–15 second input while leaving the long input unchanged.

The ordered pair of track names deterministically selects one of three near-ruined instrument families. The selection is approach-independent, so a pair uses the same instrument whether its long or short input is augmented. A second set of hashes varies the intensity inside the listed ranges for each pair:

1. **Modal-noise resonator** runs three six-mode banks spread `±30–42` cents around the pitch, disorders upper modes by `±3–5%`, and mixes `18–28%` sustained scrape noise behind the 22 ms strike. Asymmetric clipping at `3–4×` drive followed by one wavefold supplies the broken edge.
2. **Inharmonic FM/PM** runs three carriers spread `±20–35` cents, each driven by non-integer modulators at `sqrt(2)` and `2.731` times its frequency. A modulation index falling from `7–9` to `2.5–3.5`, `0.45–0.65` sample feedback, and one or two post-FM folds push it into unstable metallic noise.
3. **Saturated saw cluster** runs seven voices across `±30–42` cents with `±5–9` cents of independent drift. Asymmetric clipping at `3.2–4.2×` drive and one or two wavefolds precede a `20–30%` aliased-saw mix and deterministic 9–11-bit quantization.

Only two or three notes occur in a short input and three to six in a long input. A pair-and-role hash chooses the count, rotates chord-tone pattern `0,1,2,1,0,2,0,1`, and jitters each onset within an evenly distributed slot, so gestures span the clip without becoming a beat grid.

The ordered pair of input names hashes to a three-entry gesture profile, one entry per chord tone. Levels range from 1.5 dB above to 4.25 dB below the surrounding 1.5-second local RMS window. Duration varies inversely from 0.4 to about 1.504 seconds:

~~~text
duration = 0.4 × 10^((dB_below_local + 1.5) / 10)
~~~

Consequently a loud note is short and a quiet note rings longer, while `relative_power × duration` stays nearly constant. The same pair hash independently assigns each pitch one of four envelopes: a fast pluck, smooth swell, reverse-pluck bloom, or tremolo arc. The instrument, pitch profile, and envelopes are approach-independent, while the sparse count and placements include the processed role so the two input lengths are covered appropriately.

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

Each directory contains its own 24×24 `matrix.csv`, detailed metrics, `sparse-hashed-13edo-audible-floor-v10` algorithm marker, and verification report.

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

The superseded `sparse-hashed-13edo-ruined-v8` run finished on 2026-07-19 with eight logical CPU cores. Its measurements remain below as a baseline while the convolution-calibrated v10 output is regenerated. Each approach produced exactly 576 stereo WAVs totaling 3,889,175,040 bytes; together they contain 1,152 WAVs and 7,778,350,080 bytes. Forced rendering, built-in verification, and release compilation took 1:13.54 with 1,238,580 KiB peak resident memory. A second independent full-file decode and deterministic-metadata verification took 15.10 seconds with 503,040 KiB peak resident memory.

The chord, gesture, instrument, and exact instrument-parameter columns have the same SHA-256 signature, `aac9608e1e1545cd20d4a0764ef1a7e6fac38b3bc99b78880d7fbdaa6897e19a`, in both metrics tables. All 13 chords occur 35–58 times per approach. The filename hash assigns 190 pairs to modal noise, 175 to inharmonic FM, and 211 to saturated saw. Across the 576 pair profiles, the 1,728 pitch gestures comprise 420 plucks, 432 reverse plucks, 432 swells, and 444 tremolo arcs. Realized pitch levels span 1.499 dB above to 4.248 dB below local RMS, and durations span 0.400–1.503 seconds. Long inputs contain 3–6 notes; short inputs contain 2–3.

| Approach | Preprocess correlation, minimum | Processed-minus-dry range | Output RMS range dBFS | Maximum peak | Maximum L/R RMS delta | Stereo-difference range dBFS |
|---|---:|---:|---:|---:|---:|---:|
| Long additive | 0.9102 | −18.18 to −7.46 dB | −20.64 to −20.07 | 0.885 | 0.297 dB | −26.59 to −16.51 |
| Short additive | 0.8555 | −17.36 to −5.39 dB | −20.82 to −20.07 | 0.885 | 0.362 dB | −26.51 to −16.46 |

Every matrix passed finite-sample, clipping, peak, RMS, DC-offset, exact-length, matrix-membership, chord, gesture, instrument, sparse-count, and distinct-stereo checks.

Both final programs contain 696,287,424 frames (4:01:45.988). Every one of their 575 transitions receives the full ten-second crossfade, so the timelines remain sample-aligned. Forced assembly, eight parallel-within-approach encodes, probes, and full parallel decode checks took 7:25.14 with 116,792 KiB peak resident memory.

| Approach | RF64 | FLAC | AAC/M4A | Opus 128k | Opus 32k |
|---|---:|---:|---:|---:|---:|
| Long additive | 2,785,149,776 | 986,263,984 | 350,869,677 | 221,850,861 | 55,616,440 |
| Short additive | 2,785,149,776 | 1,029,354,907 | 350,869,929 | 219,074,215 | 53,929,443 |

Sizes are bytes. Every compressed master decoded without errors and independently probed as stereo 48 kHz with the expected codec and duration.
