# conv8

conv8 is a pitch-and-rhythm playground built from conv7. It reuses the same 48 open-licensed inputs, 24×24 short-to-long matrix, complementary stereo trims, convolution-only outputs, ten-second final crossfades, verification gates, and FLAC/AAC/Opus encoders. Its experimental variable is the preprocessing applied to the short input before each convolution.

## Controlled three-way comparison

Every short/long pair is rendered three times:

1. **Pure convolution** applies the entire chord simultaneously with a bank of three two-pole resonators. Each resonator is an IIR realization of convolution by an exponentially decaying sinusoidal impulse response:

   ~~~text
   H_i(z) = (1-r) / (1 - 2r cos(ω_i)z^-1 + r²z^-2)
   decay = 0.72 seconds
   ~~~

2. **Sequenced convolution** divides the short input into 50%-overlapped Hann grains on a 96 BPM eighth-note grid. Each grain passes through a single-note 0.24-second resonator, follows chord-tone pattern `0,1,2,1,0,2,0,1`, and receives repeating accents `1.00,0.72,0.86,0.68,1.00,0.76,0.90,0.70`. Overlap-add produces an arpeggiated, explicitly rhythmic preprocessing signal.

3. **Hybrid spectral** multiplies the short input by three cosine carriers at the chord frequencies, with phases spaced by 120 degrees, creating sum-and-difference spectral translations. A three-note 0.32-second resonator bank then reinforces the selected chord. Time multiplication is frequency-domain convolution, so this approach can create spectral energy that an LTI filter alone cannot.

Every preprocessor preserves the exact short-input frame count, RMS-matches its result to the conditioned short input, and applies 20 ms boundary fades. No unprocessed short or long source is mixed into the output.

The preprocessed short clip `P(A)` replaces `A` in the inherited stereo convolution:

~~~text
D = round(0.5 × min(length(A), length(B)))
B_short = B without its final D frames
P_short = P(A) without its initial D frames

left  = linear_convolution(P(A),    B_short)
right = linear_convolution(P_short, B)
frames_per_channel = length(A) + length(B) - D - 1
~~~

Consequently all approaches use identical source pairs, trim lengths, output lengths, channel roles, conditioning, ordering, and final encoding settings.

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

The [sources.tsv](sources.tsv) manifest is inherited unchanged from conv7: one 5–15 second and one 25–35 second recording in each of 24 themes. Only short→long pairs exist, producing 576 WAVs per approach and 1,728 total. Each output directory contains its own 24×24 `matrix.csv`, detailed metrics, and verification report:

~~~text
outputs/pure_convolution/
outputs/sequenced_convolution/
outputs/hybrid_spectral/
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

render, verify, and concat process the approaches in the fixed order pure, sequenced, hybrid. `DOWNLOAD_JOBS` controls download concurrency, while `CONV_JOBS` or `--jobs` controls FFT rendering and verification. Existing valid stages are reused unless `--force` is supplied.

## Final masters

Each approach is independently concatenated in pair order with crossfades of up to ten seconds and independently encoded as RF64 PCM, lossless FLAC, 192 kbit/s AAC/M4A, stereo 128 kbit/s Opus, and stereo 32 kbit/s Opus:

~~~text
outputs/final/pure_convolution/
outputs/final/sequenced_convolution/
outputs/final/hybrid_spectral/
~~~

Every compressed master is decoded end to end after encoding. Downloaded inputs, matrix WAVs, and final media are ignored by Git.

## Full-run audit

The first complete run finished on 2026-07-19 with eight logical CPU cores, reusing the exact 48 prepared WAVs and unchanged source manifest from `conv7`. All 576 pairs selected their chord from the prepared-file byte hashes. The chord-assignment columns in the three metrics tables have the same SHA-256 signature, and all 13 chords occur in each approach, from 35 to 58 pairs per chord.

Each approach produced exactly 576 stereo WAVs totaling 3,889,175,040 bytes; the combined playground contains 1,728 WAVs and 11,667,525,120 bytes. Pure convolution rendered in 14.4 seconds, sequenced convolution in 20.0 seconds, and hybrid spectral in 20.3 seconds. Release compilation, all renders, and their built-in exhaustive verification took 1:39.61 with 1,228,104 KiB peak resident memory. A second independent verification of all 1,728 files and deterministic metadata took 27.67 seconds with 509,252 KiB peak resident memory.

| Approach | RMS range dBFS | Maximum peak | Maximum L/R RMS delta | Stereo-difference range dBFS |
|---|---:|---:|---:|---:|
| Pure convolution | −20.86 to −20.07 | 0.836 | 0.555 dB | −26.60 to −15.87 |
| Sequenced convolution | −22.02 to −20.07 | 0.857 | 0.670 dB | −26.39 to −14.59 |
| Hybrid spectral | −21.21 to −20.09 | 0.869 | 1.057 dB | −22.74 to −15.02 |

Every approach passed all finite-sample, clipping, peak, RMS, DC-offset, exact-length, matrix-membership, chord-metadata, and distinct-stereo checks.

All three final programs contain 696,287,424 frames (4:01:45.988). Every one of their 575 transitions receives the full ten-second crossfade, so their timelines remain sample-aligned. Assembly, twelve parallel-within-approach encodes, and end-to-end decode checks took 7:36.23.

| Approach | RF64 | FLAC | AAC/M4A | Opus 128k | Opus 32k |
|---|---:|---:|---:|---:|---:|
| Pure convolution | 2,785,149,776 | 420,904,133 | 350,867,229 | 224,887,484 | 62,143,438 |
| Sequenced convolution | 2,785,149,776 | 422,378,380 | 350,867,301 | 223,160,765 | 61,312,296 |
| Hybrid spectral | 2,785,149,776 | 419,983,165 | 350,867,181 | 220,475,120 | 59,297,809 |

Sizes are bytes. Every compressed master decoded without errors and independently probed as stereo 48 kHz with the expected codec and duration.
