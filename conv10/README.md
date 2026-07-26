# conv10

conv10 turns arbitrary raw-audio lists into one complete short-to-long
convolution matrix, applies the long-input additive synth, verifies every pair,
and assembles a continuous program in exactly three final encodings:

- lossless FLAC
- AAC in an M4A container
- Opus

The checked-in `lists/conv10.txt` and `lists/conv8.txt` each contain 24 short and
24 long inputs, producing 576 stereo pairs and a 4:09:45.988 final program.
There is no short-additive variant, persistent RF64 master, low-bitrate Opus
variant, or other final encoding.

## Run

Requirements: a current Rust toolchain, Python 3, curl, FFmpeg, and FFprobe.

```bash
cd conv10
./scripts/render_all.sh
```

Equivalent explicit command:

```bash
./scripts/batch.py \
  --jobs 8 \
  --prepare-jobs 8 \
  lists/conv10.txt lists/conv8.txt
```

Outputs use the input-list stem:

```text
outputs/batch/conv10.flac
outputs/batch/conv10.m4a
outputs/batch/conv10.opus
outputs/batch/conv8.flac
outputs/batch/conv8.m4a
outputs/batch/conv8.opus
```

Audio, scratch media, matrices, and Rust build artifacts are ignored by Git.
Successful runs retain reports, timelines, recipes, and hashes but clean raw,
prepared, and matrix WAVs unless `--keep-work` is supplied.

## Input-list formats

A plain text file may contain one local path or HTTP(S) URL per non-comment
line. The first ceiling-half becomes the short side and the remainder becomes
the long side; eventful trims are selected automatically.

For explicit control, use tab-separated rows:

```text
id	role	trim_start	source
short_one	short	0	/path/to/short.wav
long_one	long	auto	https://example.test/long.flac
```

Short targets are 12 seconds and long targets are 30 seconds. Valid sources
shorter than their target but still within the manifest range are
pitch-preserving time-stretched. Preparation is content-recipe cached, and
render/final-output recipes prevent stale output reuse.

Useful modes:

```bash
./scripts/batch.py --validate-only lists/conv10.txt lists/conv8.txt
./scripts/batch.py --prepare-only my-list.txt
./scripts/batch.py --keep-work my-list.txt
```

## Verification and source use

The 2026-07-26 runs passed matrix validation, full compressed decode checks, and
SHA-256 verification for both lists. See `RUN_REPORT.md` and `PERFORMANCE.md`.

All 96 conv10-plus-conv8 source pages were audited. They comprise 58 CC0/public
domain and 38 CC BY recordings; no NC, ND, SA, Sampling+, or unclear source
remains. See `SOURCE_USE.md`, the two `LICENSE_AUDIT_*.tsv` evidence tables, and
`YOUTUBE_ATTRIBUTION.md`. CC BY entries must be credited.
