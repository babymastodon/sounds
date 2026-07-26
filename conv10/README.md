# conv10

conv10 applies the complete conv8 `sparse-hashed-13edo-gradual-drones-v14` algorithm to a different 48-recording corpus. The DSP implementation is copied intact: the 24×24 short-to-long convolution matrix, complementary stereo trims, both additive-synth placements, deterministic 13-EDO chords and gestures, convolution-domain tone calibration, output verification, ten-second final crossfades, and five master formats are unchanged.

The corpus uses 24 twelve-second musical excerpts as the short side and 24 thirty-second action/process recordings as the long side. `sources.tsv` is authoritative. Seven candidate recordings were replaced after source review. The final manifest contains only the approved 48-recording set.

## Run

Requirements: a current Rust toolchain, curl, FFmpeg/FFprobe, awk, and sha256sum.

```bash
cd conv10
./scripts/render_all.sh
```

The pipeline downloads and prepares the 48 inputs, renders and verifies both 576-file matrices, then concatenates and validates both final programs. `DOWNLOAD_JOBS` controls preparation concurrency and `CONV_JOBS` controls rendering concurrency.

Stages may also be run separately:

```bash
./scripts/download_samples.sh
cargo run --release -- render
cargo run --release -- verify
cargo run --release -- concat
```

Local media, prepared WAVs, matrix WAVs, final masters, and Rust build artifacts are ignored by Git. The checked-in code, manifest, scripts, chord table, and progress report are sufficient to reproduce the work from the published source URLs.

## Completed masters

The 2026-07-26 run produced and validated both 4:09:45.988 programs:

```text
outputs/final/long_additive_synth/
outputs/final/short_additive_synth/
```

Each directory contains RF64 PCM, FLAC, AAC/M4A, 128 kbit/s Opus, and 32 kbit/s Opus. See `RUN_REPORT.md` for the matrix and master audit.
