# conv5

`conv5` is an independent stereo FFT-convolution experiment built from the verified `conv4` implementation. It retains the `conv3` complementary start/end trimming algorithm, exhaustive 48×48 matrix, parallel renderer, five-second crossfaded master, and parallel FLAC/AAC/Opus encoders, while replacing the complete source corpus again.

## Corpus

The 48 distinct sources contain exactly six recordings from each group:

- busy city;
- industrial;
- rain;
- sports;
- long instrumentals;
- speeches;
- train ambience;
- walking.

Every group has three excerpts from 10 through 35 seconds and three excerpts over 35 through 60 seconds. The complete corpus therefore has exactly 24 short and 24 long inputs. None of its download URLs appear in `conv1`–`conv4`.

Sources were discovered through the [Openverse audio API](https://api.openverse.org/). Every chosen item is explicitly CC0, CC BY, or CC BY-SA. The authoritative source page, creator, license, excerpt duration, trim offset, and direct media URL are recorded in `sources.tsv`; the manifest loader rejects unknown licenses or mismatched license URLs. Freesound preview files are used for Freesound items. Review the manifest and comply with attribution/share-alike terms before redistributing inputs or derivatives.

## Stereo algorithm inherited from conv3

For canonical pair `A, B`, track 1 is the lower manifest index and track 2 is the higher index:

~~~text
D = round(0.5 × min(length(A), length(B)))
A_short = A without its initial D frames
B_short = B without its final D frames

left  = linear_convolution(A,       B_short)
right = linear_convolution(A_short, B)
frames_per_channel = length(A) + length(B) - D - 1
~~~

The final cut in track 2 receives a 20 ms fade-out, while the initial cut in track 1 receives a 20 ms fade-in. Both output channels contain convolution only—no dry source is mixed in. Each channel receives independent RMS targeting and smooth `tanh` limiting, followed by a shared peak-ceiling gain. Outputs are stereo 48 kHz PCM16.

The upper triangle, including self-pairs, produces 1,176 canonical stereo WAVs. `outputs/matrix.csv` exposes the complete ordered 48×48 lookup; mirrored cells reference the same canonical file without swapping channels.

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

`DOWNLOAD_JOBS` controls download concurrency; completed workers are reaped immediately so slow sources do not leave the other slots idle. `CONV_JOBS` or `--jobs` controls pair rendering. Existing valid stages are reused unless `--force` is supplied. Downloaded media under `samples/` and rendered media under `outputs/` are ignored by Git.

## Verification and final master

Verification checks all 1,176 files for stereo 48 kHz PCM16 encoding, exact length, finite unclipped samples, overall and per-channel RMS/peak/DC bounds, and non-identical stereo channels. It writes `outputs/verification.json`, `outputs/metrics.csv`, and the matrix lookup.

`concat` streams the canonical WAVs into a seekable stereo RF64 master with five-second crossfades whenever possible, writes exact placement to `outputs/final/timeline.csv`, then runs independent encoders for lossless FLAC, 192 kbit/s AAC/M4A, stereo 128 kbit/s Opus, and stereo 32 kbit/s Opus. Every compressed result is decoded end to end as a final integrity check.

## Full-run audit

The complete pipeline was downloaded, rendered, and verified on 2026-07-19 with eight logical CPU cores. All 48 direct media URLs worked and produced exact-length prepared inputs. The corpus contains 23 CC0, 24 CC BY, and one CC BY-SA source across Freesound, Jamendo, and Wikimedia Commons; its download URLs have no overlap with any earlier corpus.

The run produced 1,176 canonical stereo WAVs totaling 12,762,863,040 bytes for all 2,304 matrix cells. Rendering took 53.6 seconds; release compilation, rendering, and exhaustive verification took 100.23 seconds with peak resident memory of 1,989,556 KiB. Overall RMS ranged from −20.47 to −20.08 dBFS, maximum peak was 0.863, and maximum left/right RMS imbalance was 0.493 dB. Every pair, including all 48 self-pairs, had a verified stereo difference from −26.26 to −14.91 dBFS.

The final master contains 2,908,702,824 frames (16:49:57.976), with all 1,175 transitions receiving the complete five-second crossfade. RF64 assembly, four parallel encoders, and end-to-end decode checks took 779.89 seconds with peak resident memory of 290,604 KiB. Outputs are an 11,634,811,376-byte stereo RF64, 3,272,963,968-byte FLAC, 1,465,731,113-byte AAC/M4A, 944,095,190-byte 128 kbit/s Opus, and 234,203,842-byte 32 kbit/s Opus. Every compressed master decoded without errors.
