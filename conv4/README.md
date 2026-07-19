# conv4

`conv4` is an independent stereo FFT-convolution experiment built from `conv3`. It retains the complementary start/end trimming, exhaustive 48×48 matrix, parallel renderer, five-second crossfaded master, and parallel FLAC/AAC/Opus encoders, but replaces the entire input corpus.

## Corpus

The 48 distinct sources are split exactly three ways:

- 16 busy-city recordings;
- 16 storms or rain recordings;
- 16 slow music or sustained instrumental recordings.

Each group contains eight excerpts from 10 through 35 seconds and eight excerpts over 35 through 60 seconds. The full corpus therefore has exactly 24 short and 24 long inputs. None of its download URLs appear in the `conv1`–`conv3` corpus.

Sources were discovered through the [Openverse audio API](https://api.openverse.org/). Every chosen item is explicitly CC0, CC BY, or CC BY-SA; the authoritative source page, creator, license, excerpt duration, trim offset, and direct media URL are recorded in `sources.tsv`. Freesound preview files are used for Freesound items. Review the manifest and comply with attribution/share-alike terms before redistributing inputs or derivatives.

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

The final cut in track 2 receives a 20 ms fade-out, while the initial cut in track 1 receives a 20 ms fade-in. Both output channels contain convolution only—no dry source is mixed in. Each channel receives independent RMS targeting and smooth `tanh` limiting, followed by a shared peak-ceiling gain that preserves the stereo relationship. Outputs are stereo 48 kHz PCM16.

The upper triangle, including self-pairs, produces 1,176 canonical stereo WAVs. `outputs/matrix.csv` exposes the complete ordered 48×48 lookup; mirrored cells reference the same canonical file without swapping its channels.

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

`DOWNLOAD_JOBS` controls download concurrency and `CONV_JOBS` or `--jobs` controls pair rendering. Existing valid stages are reused unless `--force` is supplied. Downloaded media under `samples/` and rendered media under `outputs/` are ignored by Git.

## Verification and final master

Verification checks all 1,176 files for stereo 48 kHz PCM16 encoding, exact length, finite unclipped samples, overall and per-channel RMS/peak/DC bounds, and non-identical stereo channels. It writes `outputs/verification.json`, `outputs/metrics.csv`, and the matrix lookup.

`concat` streams matrix-order WAVs into a seekable stereo RF64 master with five-second crossfades whenever possible, writes the exact placement to `outputs/final/timeline.csv`, then runs independent encoders for:

- lossless FLAC at compression level 3;
- 192 kbit/s AAC/M4A;
- stereo VBR Opus at 128 kbit/s;
- stereo VBR Opus at 32 kbit/s.

`outputs/final/concat.json` verifies the master duration, codec, channel count, and byte size. Every compressed result is decoded end to end as a final integrity check.

## Full-run audit

The checked-in implementation and manifest tests pass. Render, matrix, and final-master measurements will be recorded here after the complete corpus run.
