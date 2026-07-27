# conv10 completed run

Completed: 2026-07-26

## Results

| List | Inputs | Matrix | Render | Avg render CPU | Verification |
|---|---:|---:|---:|---:|---|
| conv10 | 24 short + 24 long | 576 | 31.04 s | 5.99 / 8 cores | pass |
| conv8 | 24 short + 24 long | 576 | 29.03 s | 6.55 / 8 cores | pass |

Both matrices are stereo 48 kHz PCM16. Conv10 measured −20.6 to −20.1 dBFS
with a maximum peak of 0.826. Conv8 measured −20.5 to −20.1 dBFS with a maximum
peak of 0.840. Every pair has distinct left and right channels, valid pitch
metadata, no clipped samples, and no non-finite samples.

## Final programs

Each program contains 719,327,424 stereo frames: 14,985.988 seconds, or
4:09:45.988. All 575 transitions use the full ten-second crossfade.

| List | FLAC | AAC/M4A | Opus |
|---|---:|---:|---:|
| conv10 | 860,573,786 bytes | 362,479,793 bytes | 242,912,270 bytes |
| conv8 | 1,058,343,941 bytes | 362,480,033 bytes | 232,224,702 bytes |

FFprobe confirmed FLAC, AAC, and Opus respectively, with two channels at 48 kHz
and matching durations. Every master decoded end to end after encoding.
`sha256sum -c` passed for all six files.

Generated audio remains untracked in `outputs/batch/`. Each list also has an
ignored JSON run report, concat report, recipe, timeline, and SHA-256 file.

## Timing

| Stage | conv10 | conv8 |
|---|---:|---:|
| Preparation | 3.49 s cached | 23.18 s partially cached |
| Render | 31.04 s | 29.03 s |
| Verify | 6.43 s | 6.24 s |
| Assemble + encode + decode-check | 187.74 s | 201.35 s |
| Parallel hashing | 0.37 s | 0.43 s |

The final cached conv10 run took 229.07 seconds. The imported conv8 run took
260.23 seconds. See `PERFORMANCE.md` for the optimization inventory and
`SOURCE_INVENTORY.tsv` for the repository source record.
