# conv6

`conv6` is an independent stereo FFT-convolution experiment built from the verified `conv5` implementation. It retains the complementary start/end trimming algorithm, exhaustive 48×48 matrix, parallel renderer, five-second crossfaded master, and parallel FLAC/AAC/Opus encoders, while expanding the corpus from eight broad groups to 24 user-selected categories.

## Corpus

The checked choices in `category_candidates.md` are authoritative. Each selected category contributes exactly two distinct sources: one 10–35 second excerpt and one 36–60 second excerpt. The complete corpus therefore contains 48 new sources split exactly 24 short and 24 long.

The 24 categories are ocean surf, river rapids, ice cracking, underwater hydrophones, campfires, beehives, farm barns, airport terminals, ferry interiors, harbors, restaurant kitchens, school cafeterias, cathedrals, bowling alleys, amusement arcades, casino floors, electrical substations, laundromats, printing presses, metalworking, street festivals, protest marches, choir rehearsals, and shortwave radio.

Sources were discovered through the [Openverse audio API](https://api.openverse.org/). Every chosen item is explicitly CC0 or CC BY. `sources.tsv` records the authoritative source page, creator, license, excerpt duration, trim offset, and direct media URL; the manifest loader rejects unknown licenses, mismatched license URLs, duplicate URLs, missing categories, and any category without exactly one short and one long source.

Rights-sensitive categories use field ambience or mechanical texture where possible. Choir material is limited to rehearsal ambience and non-compositional single notes; shortwave sources are static, tones, and beeps rather than intelligible broadcasts. Arcade, casino, festival, cafeteria, and protest recordings remain public field recordings, so review their source pages and applicable personality or third-party rights before redistribution.

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

`DOWNLOAD_JOBS` controls download concurrency; completed workers are reaped immediately so slow sources do not leave other slots idle. `CONV_JOBS` or `--jobs` controls pair rendering. Existing valid stages are reused unless `--force` is supplied. Downloaded media under `samples/` and rendered media under `outputs/` are ignored by Git.

## Verification and final master

Verification checks all 1,176 files for stereo 48 kHz PCM16 encoding, exact length, finite unclipped samples, overall and per-channel RMS/peak/DC bounds, and non-identical stereo channels. It writes `outputs/verification.json`, `outputs/metrics.csv`, and the matrix lookup.

`concat` streams the canonical WAVs into a seekable stereo RF64 master with five-second crossfades whenever possible, writes exact placement to `outputs/final/timeline.csv`, then runs independent encoders for lossless FLAC, 192 kbit/s AAC/M4A, stereo 128 kbit/s Opus, and stereo 32 kbit/s Opus. Every compressed result is decoded end to end as a final integrity check.

## Full-run audit

The checked-in implementation and manifest tests pass. Render, matrix, and final-master measurements will be recorded here after the complete corpus run.
