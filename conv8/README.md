# conv8

conv8 is an additive-pitch playground built from conv7. It reuses the same 48 open-licensed inputs, 24×24 short-to-long matrix, complementary stereo trims, convolution-only outputs, ten-second final crossfades, verification gates, and FLAC/AAC/Opus encoders. The experiment compares adding the same sparse, synthesized gestures to either the long or short input before convolution.

## Controlled two-way comparison

Every short/long pair is rendered twice:

1. **Long additive synth** adds notes to the 25–35 second input while leaving the short input unchanged.
2. **Short additive synth** adds notes to the 5–15 second input while leaving the long input unchanged.

The ordered pair of track names deterministically selects one of three gritty instrument families. The selection is approach-independent, so a pair uses the same instrument whether its long or short input is augmented:

1. **Modal-noise resonator** combines a 22 ms noise strike with six decaying, softly saturated modes at ratios `1.00`, `1.41`, `1.93`, `2.58`, `3.77`, and `5.12`.
2. **Inharmonic FM/PM** combines a stable fundamental with two non-integer modulators at `sqrt(2)` and `2.731` times the selected pitch. Its modulation index falls from `3.4` to `1.6` through the note.
3. **Saturated saw cluster** combines antialiased saw voices at `−7`, `0`, and `+7` cents around the 13-EDO pitch, then applies normalized `tanh(2.2x)` saturation.

Only two or three notes occur in a short input and three to six in a long input. A pair-and-role hash chooses the count, rotates chord-tone pattern `0,1,2,1,0,2,0,1`, and jitters each onset within an evenly distributed slot, so gestures span the clip without becoming a beat grid.

The ordered pair of input names hashes to a three-entry gesture profile, one entry per chord tone. Levels range from 1.5 dB above to 4.25 dB below the surrounding 1.5-second local RMS window. Duration varies inversely from 0.4 to about 1.504 seconds:

~~~text
duration = 0.4 × 10^((dB_below_local + 1.5) / 10)
~~~

Consequently a loud note is short and a quiet note rings longer, while `relative_power × duration` stays nearly constant. The same pair hash independently assigns each pitch one of four envelopes: a fast pluck, smooth swell, reverse-pluck bloom, or tremolo arc. The instrument, pitch profile, and envelopes are approach-independent, while the sparse count and placements include the processed role so the two input lengths are covered appropriately.

The augmented clip retains the input's exact frame count, receives 20 ms boundary fades, and is RMS-matched to the original. Verification requires at least 0.85 dry/processed correlation and caps the processed-minus-dry RMS at −4 dB relative to the source. Every gesture hash, instrument, pitch level, duration, envelope, and scheduled count is recorded in `metrics.csv`.

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

Each directory contains its own 24×24 `matrix.csv`, detailed metrics, `sparse-hashed-13edo-instruments-v7` algorithm marker, and verification report.

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

## Last completed run (superseded tuning)

The focused `sparse-hashed-additive-v5` run finished on 2026-07-19 before the scale changed from Bohlen–Pierce to 13-EDO. Its measurements below describe the last completed outputs, not the pending 13-EDO generation. Each approach produced exactly 576 stereo WAVs totaling 3,889,175,040 bytes; together they contain 1,152 WAVs and 7,778,350,080 bytes. Rendering, built-in verification, and release compilation took 1:07.63 with 1,216,656 KiB peak resident memory. A second independent decode and metadata verification took 13.14 seconds with 504,032 KiB peak resident memory.

The chord and gesture columns have the same SHA-256 signature in both metrics tables. All 13 chords occur 35–58 times per approach. Across the 576 pair profiles, the 1,728 pitch gestures comprise 420 plucks, 432 reverse plucks, 432 swells, and 444 tremolo arcs. Realized pitch levels span 1.499 dB above to 4.248 dB below local RMS, and durations span 0.400–1.503 seconds. Long inputs contain 3–6 notes; short inputs contain 2–3.

| Approach | Preprocess correlation, minimum | Processed-minus-dry range | Output RMS range dBFS | Maximum peak | Maximum L/R RMS delta | Stereo-difference range dBFS |
|---|---:|---:|---:|---:|---:|---:|
| Long additive | 0.9118 | −18.18 to −7.54 dB | −21.18 to −20.07 | 0.886 | 0.537 dB | −26.57 to −16.28 |
| Short additive | 0.8568 | −17.36 to −5.43 dB | −20.59 to −20.07 | 0.883 | 0.343 dB | −26.50 to −16.47 |

Every matrix passed finite-sample, clipping, peak, RMS, DC-offset, exact-length, matrix-membership, chord, gesture, sparse-count, and distinct-stereo checks.

Both final programs contain 696,287,424 frames (4:01:45.988). Every one of their 575 transitions receives the full ten-second crossfade, so the timelines remain sample-aligned. Forced assembly, eight parallel-within-approach encodes, and end-to-end decode checks took 5:42.82 with 116,612 KiB peak resident memory.

| Approach | RF64 | FLAC | AAC/M4A | Opus 128k | Opus 32k |
|---|---:|---:|---:|---:|---:|
| Long additive | 2,785,149,776 | 886,377,682 | 350,869,773 | 222,670,001 | 55,806,178 |
| Short additive | 2,785,149,776 | 877,768,136 | 350,869,653 | 222,978,117 | 55,153,548 |

Sizes are bytes. Every compressed master decoded without errors and independently probed as stereo 48 kHz with the expected codec and duration.
