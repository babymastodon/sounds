# conv8

conv8 is a pitch-and-rhythm playground built from conv7. It reuses the same 48 open-licensed inputs, 24×24 short-to-long matrix, complementary stereo trims, convolution-only outputs, ten-second final crossfades, verification gates, and FLAC/AAC/Opus encoders. Its experimental variable is subtle chord-related preprocessing applied before each convolution.

## Controlled four-way comparison

Every short/long pair is rendered four times. The first three treatments preserve the short recording as the dominant signal by RMS-matching an effect branch, mixing it quietly in parallel, fading the boundaries, and matching the combined RMS back to the input:

1. **Pure convolution** mixes 3% of a three-note, 0.18-second two-pole resonator bank into 97% of the short input. Each resonator is an IIR realization of convolution by an exponentially decaying sinusoidal impulse response:

   ~~~text
   H_i(z) = (1-r) / (1 - 2r cos(ω_i)z^-1 + r²z^-2)
   decay = 0.18 seconds
   ~~~

2. **Sequenced convolution** mixes 4% of a rhythmic effect into 96% of the short input. The effect divides the recording into 50%-overlapped Hann grains on a 96 BPM eighth-note grid. Each grain passes through a single-note 0.12-second resonator, follows chord-tone pattern `0,1,2,1,0,2,0,1`, and receives repeating accents `1.00,0.72,0.86,0.68,1.00,0.76,0.90,0.70`.

3. **Hybrid spectral** mixes 2.5% of a modulated effect into 97.5% of the short input. The effect multiplies the recording by three cosine carriers at the chord frequencies, then passes it through a three-note 0.10-second resonator bank. Time multiplication creates sum-and-difference spectral translations that an LTI filter alone cannot.

4. **Long additive synth** leaves the short input unchanged and adds 250 ms muted-kalimba-like notes to the long input every 1.25 seconds. The deterministic note sequence draws from the same selected chord. Each four-partial note is independently scaled to 6 dB below the surrounding long-input RMS, using a local half-interval window, before the augmented long input is RMS-matched to the original.

Every treatment preserves the processed input's exact frame count and applies 20 ms boundary fades. There is no post-convolution dry-source mix: both output channels remain convolution products. For the first three approaches, `P(A)` is the subtly processed short clip and `B` is unchanged:

~~~text
D = round(0.5 × min(length(A), length(B)))
B_short = B without its final D frames
P_short = P(A) without its initial D frames

left  = linear_convolution(P(A),    B_short)
right = linear_convolution(P_short, B)
frames_per_channel = length(A) + length(B) - D - 1
~~~

For the fourth, `A` is unchanged and `Q(B)` is the augmented long clip:

~~~text
Q_short = Q(B) without its final D frames
A_short = A without its initial D frames

left  = linear_convolution(A,       Q_short)
right = linear_convolution(A_short, Q(B))
~~~

All approaches use identical source pairs, trim lengths, output lengths, channel roles, conditioning, ordering, and final encoding settings. Verification requires the first three processed clips to correlate at least 0.98 with the dry short input and the fourth to correlate at least 0.95 with the dry long input.

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

The domain tags and byte order are fixed in `src/pitch.rs`. The ordered short→long role, chord index, steps, frequencies, and both file fingerprints are recorded in each approach's `metrics.csv`. This makes the mapping deterministic across reruns and reusable by later experiments.

## Corpus and matrix

The [sources.tsv](sources.tsv) manifest is inherited unchanged from conv7: one 5–15 second and one 25–35 second recording in each of 24 themes. Only short→long pairs exist, producing 576 WAVs per approach and 2,304 total. Each output directory contains its own 24×24 `matrix.csv`, detailed metrics, algorithm-version marker, and verification report:

~~~text
outputs/pure_convolution/
outputs/sequenced_convolution/
outputs/hybrid_spectral/
outputs/long_additive_synth/
~~~

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

render, verify, and concat process the approaches in the fixed order pure, sequenced, hybrid, additive. `DOWNLOAD_JOBS` controls download concurrency, while `CONV_JOBS` or `--jobs` controls FFT rendering and verification. Render reuse requires a matching pitch-algorithm marker; `--force` rebuilds every file.

## Final masters

Each approach is independently concatenated in pair order with crossfades of up to ten seconds and independently encoded as RF64 PCM, lossless FLAC, 192 kbit/s AAC/M4A, stereo 128 kbit/s Opus, and stereo 32 kbit/s Opus:

~~~text
outputs/final/pure_convolution/
outputs/final/sequenced_convolution/
outputs/final/hybrid_spectral/
outputs/final/long_additive_synth/
~~~

Every compressed master is decoded end to end after encoding. Downloaded inputs, matrix WAVs, and final media are ignored by Git.

## Full-run audit

The earlier fully wet three-way run was superseded because its pitched treatments masked the source material. The `subtle-parallel-v2` four-way matrices and masters are being rebuilt from scratch; this section will record their measured verification and encoding results after completion.
