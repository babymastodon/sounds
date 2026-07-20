# conv7

`conv7` is an independent stereo FFT-convolution experiment built from `conv6`. It keeps the complementary start/end trimming, audio conditioning, exhaustive verification, crossfaded master, and parallel FLAC/AAC/Opus encoders, but changes both the source durations and the matrix topology.

## Corpus

The corpus keeps the same 24 themes as `conv6`: ocean surf, river rapids, ice cracking, underwater hydrophones, campfires, beehives, farm barns, airport terminals, ferry interiors, harbors, restaurant kitchens, school cafeterias, cathedrals, bowling alleys, amusement arcades, casino floors, electrical substations, laundromats, printing presses, metalworking, street festivals, protest marches, choir rehearsals, and shortwave radio.

Each theme contributes two entirely new recordings: one 5–15 second excerpt and one 25–35 second excerpt. `sources.tsv` therefore contains exactly 24 short and 24 long inputs. Its 48 direct media URLs are distinct from every URL in `conv1`, `conv4`, `conv5`, and `conv6`; 29 sources are CC0 and 19 are CC BY.

Sources were discovered through the [Openverse audio API](https://api.openverse.org/). The manifest records the source page, creator, license, excerpt duration, trim offset, and direct media URL. The loader rejects unknown licenses, mismatched license URLs, duplicate URLs, durations in the 15–25 second gap, and any theme without exactly one source in each duration class.

Rights-sensitive material follows the same policy as `conv6`: choir clips are non-compositional rehearsal notes, shortwave clips are noise and tones, and public ambience recordings should have their source pages and applicable personality or third-party rights reviewed before redistribution.

## 24×24 cross-duration algorithm

Unlike the symmetric 48×48 matrices in earlier experiments, this is a complete bipartite matrix. Every short clip is combined with every long clip, including the matching theme, but short+short, long+long, reverse-role, and self pairs do not exist. This produces exactly 24 × 24 = 576 stereo WAVs. `outputs/matrix.csv` has short clips as rows and long clips as columns; every cell names a distinct rendered file.

For every cell, `A` is always the short row clip and `B` is always the long column clip:

~~~text
D = round(0.5 × min(length(A), length(B)))
A_short = A without its initial D frames
B_short = B without its final D frames

left  = linear_convolution(A,       B_short)
right = linear_convolution(A_short, B)
frames_per_channel = length(A) + length(B) - D - 1
~~~

The final cut in `B` receives a 20 ms fade-out and the initial cut in `A` receives a 20 ms fade-in. Both channels contain convolution only, with no dry source. Each channel receives independent RMS targeting and smooth `tanh` limiting, followed by shared peak-ceiling gain. Outputs are stereo 48 kHz PCM16.

## Run the complete pipeline

Requirements: a current Rust toolchain, `curl`, FFmpeg/FFprobe, `awk`, and `sha256sum`.

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

`DOWNLOAD_JOBS` controls download concurrency. `CONV_JOBS` or `--jobs` controls FFT rendering and verification concurrency. Existing valid stages are reused unless `--force` is supplied. Downloaded media and rendered outputs are ignored by Git.

## Verification and final master

Verification requires exactly 576 WAVs and checks their bipartite membership, stereo 48 kHz PCM16 encoding, exact convolution length, finite unclipped samples, RMS/peak/DC bounds, and distinct stereo channels. It writes `outputs/verification.json`, `outputs/metrics.csv`, and the 24×24 matrix lookup.

`concat` streams all 576 WAVs into a seekable stereo RF64 master with crossfades of up to ten seconds, writes exact placement to `outputs/final/timeline.csv`, and runs independent encoders for lossless FLAC, 192 kbit/s AAC/M4A, stereo 128 kbit/s Opus, and stereo 32 kbit/s Opus. Every compressed result is decoded end to end as an integrity check. Ten seconds is the CLI default; shorter values remain available through `--crossfade-seconds`.

## Full-run audit

The complete pipeline was downloaded, rendered, verified, concatenated, and encoded on 2026-07-19 with eight logical CPU cores. All 48 Openverse media URLs worked and produced exact-length prepared inputs. The corpus has 29 CC0 and 19 CC BY sources, no duplicate URLs, and no URL overlap with `conv1`, `conv4`, `conv5`, or `conv6`. Parallel download and preparation took 10:55.51 with 71,316 KiB peak resident memory; most of that wall time was one large carnival source recording.

The run produced exactly 576 short-to-long stereo WAVs totaling 3,889,175,040 bytes. `matrix.csv` is exactly 24 rows × 24 columns, and every metric row has a short input in track-1 position and a long input in track-2 position. FFT rendering itself took 10.7 seconds; release compilation, rendering, and exhaustive built-in verification took 36.26 seconds with peak resident memory of 997,908 KiB. A second independent verification pass took 5.71 seconds. Overall RMS ranged from −20.64 to −20.07 dBFS, maximum peak was 0.887, maximum left/right RMS imbalance was 0.388 dB, and stereo difference ranged from −26.62 to −16.20 dBFS. Every pair had distinct stereo channels.

The final master contains 834,287,424 frames (4:49:40.988), with all 575 transitions receiving the complete five-second crossfade. RF64 assembly, four parallel encoders, and end-to-end decode checks took 3:27.74 with peak resident memory of 703,788 KiB. Outputs are a 3,337,149,776-byte stereo RF64, 1,069,371,470-byte FLAC, 420,409,421-byte AAC/M4A, 266,718,120-byte 128 kbit/s Opus, and 67,494,812-byte 32 kbit/s Opus. Every compressed master decoded without errors.
