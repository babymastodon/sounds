# conv10 completed run

Completed: 2026-07-26

## Results

| List | Inputs | Matrix | Render | Avg render CPU | Verification |
|---|---:|---:|---:|---:|---|
| conv10 | 24 short + 24 long | 576 | 34.02 s | 6.90 / 8 cores | pass |
| conv8 | 24 short + 24 long | 576 | 51.04 s | 6.20 / 8 cores | pass |

Both matrices are stereo 48 kHz PCM16. Conv10 measured −20.6 to −20.1 dBFS
with a maximum peak of 0.826. Conv8 measured −20.5 to −20.1 dBFS with a maximum
peak of 0.865. Every pair has distinct left and right channels, valid pitch
metadata, no clipped samples, and no non-finite samples.

## Final programs

Each program contains 719,327,424 stereo frames: 14,985.988 seconds, or
4:09:45.988. All 575 transitions use the full ten-second crossfade.

| List | FLAC | AAC/M4A | Opus |
|---|---:|---:|---:|
| conv10 | 839,525,346 bytes | 362,479,889 bytes | 243,090,204 bytes |
| conv8 | 1,056,278,433 bytes | 362,479,793 bytes | 226,894,369 bytes |

FFprobe confirmed FLAC, AAC, and Opus respectively, with two channels at 48 kHz
and matching durations. Every master decoded end to end after encoding.
`sha256sum -c` passed for all six files.

Generated audio remains untracked in `outputs/batch/`. Each list also has an
ignored JSON run report, concat report, recipe, timeline, and SHA-256 file.

## Timing

| Stage | conv10 | conv8 |
|---|---:|---:|
| Preparation | 453.80 s first network run | 3.60 s cached resume |
| Render | 34.02 s | 51.04 s |
| Verify | 10.88 s | 12.67 s |
| Assemble + encode + decode-check | 251.07 s | 251.56 s |
| Parallel hashing | 0.41 s | 0.56 s |

Conv10’s first-run total was 750.18 seconds. Conv8’s cached resumed run was
319.45 seconds. See `PERFORMANCE.md` for the optimization inventory.

## Source audit

All 96 source pages were fetched on 2026-07-26. The audit found 58
CC0/public-domain sources and 38 CC BY sources, all permitting commercial use.
No source required replacement, so the verified masters remain current. The CC
BY attribution block is in `YOUTUBE_ATTRIBUTION.md`.
