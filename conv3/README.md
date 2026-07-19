# conv3

`conv3` is an independent stereo FFT-convolution experiment built from the verified `conv2` pipeline. It renders 1,176 canonical stereo WAVs for the upper triangle, including self-pairs, and writes a complete 48×48 lookup matrix whose mirrored cells reference the same canonical file.

## Stereo experiment

For canonical pair `A, B`, track 1 is the lower manifest index and track 2 is the higher index. The renderer removes half the shorter input's duration from the end of both role-specific shortened operands:

~~~text
D = round(0.5 × min(length(A), length(B)))
A_short = A without its initial D frames
B_short = B without its final D frames

left  = linear_convolution(A,       B_short)
right = linear_convolution(A_short, B)
frames_per_channel = length(A) + length(B) - D - 1
~~~

The final cut in track 2 receives a 20 ms fade-out, and the initial cut in track 1 receives a 20 ms fade-in before the FFT. No dry or original track is mixed into either output channel: every sample comes from a convolution. This deliberately retains the complete track 1 evolution only on the left and the complete track 2 evolution only on the right. Complementary head/tail trimming also makes self-pairs genuinely stereo. Mirrored matrix cells retain the canonical channel orientation rather than swapping channels.

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
- effectively identical left and right channels in any pair, including self-pairs.

## Final master

`concat` streams the canonical stereo WAVs in matrix order into a seekable stereo RF64 PCM master. It uses a five-second crossfade whenever physically possible, records exact placement in `outputs/final/timeline.csv`, and then launches independent FFmpeg processes for:

- lossless FLAC at compression level 3;
- 192 kbit/s AAC/M4A;
- stereo VBR Opus at 128 kbit/s;
- stereo VBR Opus at 32 kbit/s.

`outputs/final/concat.json` records codec, duration, channel count, and file-size verification. Without `--force`, valid existing final stages are reused and only missing encodings are generated.

## Latest full-run audit

The complete pipeline was rendered and verified on 2026-07-19 with eight logical CPU cores. It produced 1,176 canonical stereo WAVs totaling 10,414,511,040 bytes for all 2,304 matrix cells. The render itself took 49.3 seconds; release compilation, rendering, and exhaustive verification took 93.31 seconds with peak resident memory of 1,924,908 KiB. Overall RMS ranged from −20.77 to −20.08 dBFS, maximum peak was 0.890, and maximum left/right RMS imbalance was 1.374 dB. All 1,176 pairs, including all 48 self-pairs, had verified stereo differences from −34.70 to −15.86 dBFS.

The final master contains 2,322,166,832 frames (13:26:18.476), with 1,167 full five-second crossfades and eight duration-limited fades. RF64 assembly plus four parallel encoders took 626.35 seconds with peak resident memory of 247,864 KiB. Outputs are a 9,288,667,408-byte stereo RF64, 3,260,025,090-byte FLAC, 1,170,169,057-byte AAC/M4A, 751,081,424-byte 128 kbit/s Opus, and 194,419,136-byte 32 kbit/s Opus. Every compressed master decoded end to end without errors.

## Source licensing

The corpus draws from ESC-50/Freesound, Wikimedia Commons, BigSoundBank, and NASA Artemis audio. Consult `sources.tsv` before redistributing inputs or derivatives: ESC-50 sources are CC BY-NC, while CC BY and CC BY-SA sources require attribution. Downloaded media is intentionally excluded from Git.
