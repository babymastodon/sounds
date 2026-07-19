# conv2

`conv2` is an independent stereo FFT-convolution experiment built from the proven `conv1` corpus and pipeline. It renders 1,176 canonical stereo WAVs for the upper triangle, including self-pairs, and writes a complete 48×48 lookup matrix whose mirrored cells reference the same canonical file.

## Stereo experiment

For canonical pair `A, B`, track 1 is the lower manifest index and track 2 is the higher index. The renderer removes half the shorter input's duration from the end of both role-specific shortened operands:

~~~text
D = round(0.5 × min(length(A), length(B)))
A_short = A without its final D frames
B_short = B without its final D frames

left  = linear_convolution(A,       B_short)
right = linear_convolution(A_short, B)
frames_per_channel = length(A) + length(B) - D - 1
~~~

Each new cut receives a 20 ms fade before the FFT. No dry or original track is mixed into either output channel: every sample comes from a convolution. This deliberately retains the complete track 1 evolution only on the left and the complete track 2 evolution only on the right. Because convolution is commutative, self-pairs are intentionally dual-mono. Mirrored matrix cells retain the canonical channel orientation rather than swapping channels.

Inputs are conditioned once. After convolution, each channel receives independent RMS targeting and smooth `tanh` limiting; a final shared ceiling gain preserves the stereo relationship without clipping. Outputs are stereo 48 kHz PCM16.

## Run the complete pipeline

Requirements: a current Rust toolchain, `curl`, FFmpeg/FFprobe, `awk`, and `sha256sum`.

~~~bash
./scripts/render_all.sh
~~~

Downloaded and prepared inputs are stored under `samples/`; rendered audio and reports are stored under `outputs/`. Both media trees are ignored by Git. The copied `sources.tsv` remains the authoritative provenance manifest: 48 distinct sources, exactly 24 longer than 30 through 60 seconds, and 25 industrial recordings.

Stages can also be run separately:

~~~bash
./scripts/download_samples.sh
cargo run --release -- render
cargo run --release -- verify
cargo run --release -- concat
~~~

`DOWNLOAD_JOBS` controls download concurrency and `CONV_JOBS` or `--jobs` controls pair rendering. Existing valid stages are reused unless `--force` is supplied.

## FFT and matrix implementation

Jobs are grouped by FFT length. `realfft` plans and full-input spectra are cached per size group. Each pair computes the two final-trimmed spectra and performs independent left and right inverse transforms. Rayon schedules pair processing across the configured worker pool.

`outputs/matrix.csv` is the ordered 48×48 lookup table. `outputs/metrics.csv` records overall and per-channel measurements for every canonical WAV. `outputs/verification.json` contains the exhaustive audit.

Verification rejects:

- any file that is not stereo 48 kHz PCM16;
- an unexpected frame count;
- a non-finite or clipped sample;
- overall or per-channel peaks outside 0.12–0.92;
- overall or per-channel RMS outside −30 to −10 dBFS;
- absolute overall or per-channel DC offset above 0.005.

## Final master

`concat` streams the canonical stereo WAVs in matrix order into a seekable stereo RF64 PCM master. It uses a five-second crossfade whenever physically possible, records exact placement in `outputs/final/timeline.csv`, and then launches independent FFmpeg processes for:

- lossless FLAC at compression level 3;
- 192 kbit/s AAC/M4A;
- stereo VBR Opus at 64 kbit/s;
- stereo VBR Opus at 16 kbit/s.

`outputs/final/concat.json` records codec, duration, channel count, and file-size verification. Without `--force`, valid existing final stages are reused and only missing encodings are generated.

## Source licensing

The corpus draws from ESC-50/Freesound, Wikimedia Commons, BigSoundBank, and NASA Artemis audio. Consult `sources.tsv` before redistributing inputs or derivatives: ESC-50 sources are CC BY-NC, while CC BY and CC BY-SA sources require attribution. Downloaded media is intentionally excluded from Git.
