# Performance inventory

Measured on an 8-core Intel Core Ultra 7 268V with the release build.

## Outcome

The conv10 24×24 render now completes in 33.3 seconds internally (34.02 seconds
including process monitoring) while averaging 6.90 cores. The earlier
pre-optimization renderer took 69.7 seconds for the same 576 pairs, so matrix
render wall time fell by about 52 percent.

A controlled 60-second single-worker partial run advanced from 140/576 pairs in
the old build to 190/576 in the optimized build, a 36 percent throughput gain.
This addresses the single-thread hot loop as well as parallel scheduling. An
earlier 8-worker benchmark peaked near 1.46 GiB RSS.

## Implemented

- Hoisted modal, FM, and saw invariants out of per-sample loops.
- Replaced most oscillator trigonometry and modal exponential work with stable
  recurrences plus periodic normalization.
- Reduced tone/dry convolution from four inverse FFTs to two and used spectral
  energy for calibration.
- Avoided redundant output-conditioning allocations.
- Built the custom Rayon pool before loading inputs and used it for every Rust
  parallel stage.
- Removed duplicate verification from rendering.
- Added an input-fingerprint and algorithm recipe checked before pair synthesis.
- Parallelized input preparation, pair rendering, verification, the three
  encoders, compressed decode checks, and output hashing.
- Pipelined preparation behind completed downloads and cached automatic trims.
- Recorded render samples once per second plus per-stage wall and aggregate CPU
  time.

## Measured parallel behavior

| Workload | Wall time | Average cores | Longest low-use render run |
|---|---:|---:|---:|
| conv10 render | 34.02 s | 6.90 | 1 s |
| conv10 verify | 10.88 s | 6.59 | — |
| conv8 render | 51.04 s | 6.20 | 2 s |
| conv8 verify | 12.67 s | 6.35 | — |
| conv10 assemble/encode/check | 251.07 s | 1.98 | expected codec tail |
| conv8 assemble/encode/check | 251.56 s | 2.00 | expected codec tail |

Pair cost depends on the selected instrument and gesture, explaining the
content-dependent conv8/conv10 difference and short queue-drain tail. The
renderer is no longer behaving like a single-core workload.

## Remaining optimization inventory

1. Master assembly is serial and reads roughly four GiB of pair WAV data,
   crossfades it, and writes a temporary RF64 file. The RF64 is scratch-only and
   removed after encoding, but this is now the dominant wall-time stage.
2. This FFmpeg build reports no internal threading for the selected FLAC, AAC,
   or Opus encoders. Three encoders run concurrently, then usage drops as the
   slowest single-threaded encoder remains.
3. Pair tasks are indivisible and vary by instrument. Estimated-heavy-first
   scheduling or intra-pair joins could reduce the final render tail.
4. FFT buffers and scratch allocations could be reused per Rayon worker.
5. A full dry-trim spectrum cache reduces transforms but was slower in testing:
   its extra hundreds of MiB increased pressure on the 12 MiB shared L3 cache.
   A one-sided or bounded worker-local cache needs measurement before adoption.
6. A shared content-addressed raw/prepared cache would reuse identical sources
   across differently named lists.
7. Streaming PCM into a single multi-output FFmpeg process could remove the
   temporary RF64 and three rereads. Segmented AAC/Opus encoding needs careful
   boundary testing before use because encoder delay can introduce gaps.

The next high-value benchmark is a streamed master-assembly prototype, followed
by jobs 1/2/4/6/8 scaling on the same prepared 24×24 corpus and build.
