# conv8

conv8 is an additive-pitch playground built from conv7. It reuses the same 48 open-licensed inputs, 24×24 short-to-long matrix, complementary stereo trims, convolution-only outputs, ten-second final crossfades, verification gates, and FLAC/AAC/Opus encoders. The experiment compares adding the same sparse, synthesized gestures to either the long or short input before convolution.

## Controlled two-way comparison

Every short/long pair is rendered twice:

1. **Long additive synth** adds notes to the 25–35 second input while leaving the short input unchanged.
2. **Short additive synth** adds notes to the 5–15 second input while leaving the long input unchanged.

Both use the same resonant, mallet-like additive instrument. Its partial frequency ratios are `1`, `2.01`, `3.93`, and `6.79`, with amplitudes `1`, `0.28`, `0.11`, and `0.04`. Only two or three notes occur in a short input and three to six in a long input. A pair-and-role hash chooses the count, rotates chord-tone pattern `0,1,2,1,0,2,0,1`, and jitters each onset within an evenly distributed slot, so gestures span the clip without becoming a beat grid.

The ordered pair of input names hashes to a three-entry gesture profile, one entry per chord tone. Levels range from 1.5 dB above to 4.25 dB below the surrounding 1.5-second local RMS window. Duration varies inversely from 0.4 to about 1.504 seconds:

~~~text
duration = 0.4 × 10^((dB_below_local + 1.5) / 10)
~~~

Consequently a loud note is short and a quiet note rings longer, while `relative_power × duration` stays nearly constant. The same pair hash independently assigns each pitch one of four envelopes: a fast pluck, smooth swell, reverse-pluck bloom, or tremolo arc. The profile is approach-independent, while the sparse count and placements include the processed role so the two input lengths are covered appropriately.

The augmented clip retains the input's exact frame count, receives 20 ms boundary fades, and is RMS-matched to the original. Verification requires at least 0.85 dry/processed correlation and caps the processed-minus-dry RMS at −4 dB relative to the source. Every gesture hash, pitch level, duration, envelope, and scheduled count is recorded in `metrics.csv`.

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

## Thirteen-chord scale and deterministic assignment

The catalog uses equal-tempered Bohlen–Pierce tuning: 13 equal steps divide a 3:1 tritave rather than the Western 2:1 octave. With base frequency 110 Hz:

~~~text
frequency(step) = 110 × 3^(step/13)
chord(root) = [root, root+6, root+10] modulo 13
~~~

This gives 13 transpositions of the same approximate 3:5:7 chord shape. [chords.tsv](chords.tsv) is the rounded reference catalog; the formula above is authoritative. This is an experimental non-octave tuning and does not claim to represent a living musical tradition.

Chord selection is content-based and approach-independent:

1. Hash every complete prepared WAV file with domain-separated FNV-1a-64.
2. Hash the fixed tag `conv8-bohlen-pierce-pair-v1`, the short-file hash, then the long-file hash.
3. Select `pair_hash mod 13`.

The ordered short→long role, chord index, steps, frequencies, both file fingerprints, and name-derived gesture profile are recorded in each `metrics.csv`, making assignments deterministic across reruns. The chord and per-pitch gestures are identical between approaches.

## Corpus and matrix

The [sources.tsv](sources.tsv) manifest is inherited unchanged from conv7: one 5–15 second and one 25–35 second recording in each of 24 themes. Only short→long pairs exist, producing 576 WAVs per approach and 1,152 total:

~~~text
outputs/long_additive_synth/
outputs/short_additive_synth/
~~~

Each directory contains its own 24×24 `matrix.csv`, detailed metrics, `sparse-hashed-additive-v5` algorithm marker, and verification report.

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

The focused `sparse-hashed-additive-v5` matrices and masters are being rebuilt from scratch. This section will record their measured verification and encoding results after completion.
