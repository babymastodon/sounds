# Performance inventory

Measured on an 8-core Intel Core Ultra 7 268V with the release build.

## Outcome

The final conv10 24×24 render completed in 31.04 seconds while averaging 5.99
cores. The imported conv8 list completed in 29.03 seconds while averaging 6.55
cores. The earlier
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
| conv10 render | 31.04 s | 5.99 | 4 s |
| conv10 verify | 6.43 s | 6.90 | — |
| conv8 render | 29.03 s | 6.55 | 1 s |
| conv8 verify | 6.24 s | 7.09 | — |
| conv10 assemble/encode/check | 187.74 s | 2.04 | expected codec tail |
| conv8 assemble/encode/check | 201.35 s | 2.01 | expected codec tail |

Pair cost depends on the selected instrument and gesture, explaining the
content-dependent conv8/conv10 difference and short queue-drain tail. The
renderer is no longer behaving like a single-core workload.

## Remaining optimization inventory

1. Master assembly is serial and reads roughly four GiB of pair WAV data,
   crossfades it, and writes a temporary RF64 file. The RF64 is scratch-only and
   removed after encoding, but this is now the dominant wall-time stage.
2. This FFmpeg build reports no internal threading for the selected FLAC, AAC,
   or Opus encoders. Three encoders run concurrently, then usage drops as the
   slowest single-threaded encoder remains. Benchmark lower Opus complexity
   before attempting segmented encoding.
3. Pair tasks are indivisible and vary by instrument. Estimated-heavy-first
   scheduling, `rayon::join` inside large pairs, or both could reduce the final
   queue-drain tail.
4. FFT input, output, and scratch buffers can be reused per Rayon worker.
   A full dry-trim spectrum cache was slower in testing because its extra
   hundreds of MiB increased pressure on the 12 MiB shared L3 cache; a bounded
   worker-local or one-sided cache still needs measurement.
5. Preparation fully decodes cached raw files, may scan them again for automatic
   cuts, and can decode them again for conversion. Its fetch/prepare pipeline
   still has expensive single-file tails, and FFmpeg children have no shared
   CPU-token budget.
6. A shared content-addressed raw/prepared cache would reuse identical sources
   across lists and avoid the per-list cache gap observed by the imported conv8
   run.
7. List processing remains sequential. Add `--list-jobs` with a global CPU and
   memory budget for multiple small lists.
8. Output names use only the list stem. Reject same-basename lists or add a
   stable list digest, and keep list/source/algorithm/crossfade/bitrate
   fingerprints in every reuse recipe.
9. Streaming PCM into a single multi-output FFmpeg process could remove the
   temporary RF64 and three rereads. Segmented AAC/Opus encoding needs careful
   boundary testing because encoder delay can introduce gaps.
10. Compressed-output hashes are still read sequentially after the decode
    checks. Hashing during encoding or decode validation would eliminate the
    final read pass.

The next high-value benchmark is a streamed master-assembly prototype, followed
by jobs 1/2/4/6/8 scaling on the same warm-cache 24×24 corpus. Record p50, p95,
maximum pair time, final-tail time, process-tree CPU, RSS, and read/write bytes
for render, assembly, each encoder, decode validation, and hashing.
