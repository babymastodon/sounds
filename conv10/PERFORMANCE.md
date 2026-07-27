# Performance inventory

Measured on an 8-core Intel Core Ultra 7 268V with the release build.

## Outcome

The completed 14-track album rendered 8,064 convolution pairs in 472.42
aggregate seconds. A 24×24 matrix averaged 33.74 seconds while using 7.05 of 8
cores; individual tracks ranged from 28.02 to 37.07 seconds and 6.38 to 7.48
average cores. The earlier pre-optimization renderer took 69.7 seconds for one
576-pair matrix, so current matrix wall time is roughly half that baseline.

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
- Captured only the lossless source prefix needed for explicit trims instead of
  downloading arbitrarily long recordings in full.
- Changed source validation from full-file decoding to strict decoding of the
  exact selected window.
- Recorded render samples once per second plus per-stage wall and aggregate CPU
  time.

## Measured parallel behavior

| Workload | Wall time | Average cores |
|---|---:|---:|
| 14 matrix renders, mean | 33.74 s | 7.05 |
| Fastest matrix: Field Atlas | 28.02 s | 6.58 |
| Highest utilization: Wildwire | 33.02 s | 7.48 |
| Slowest matrix: Railchime | 37.07 s | 6.61 |
| 14 verification passes, mean | 8.10 s | recorded per run |
| 14 assemble/encode/decode-check stages, mean | 180.26 s | codec-limited |

Pair cost depends on the selected instrument and gesture, explaining the
track-to-track difference and short queue-drain tail. The renderer is no longer
behaving like a single-core workload.

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
5. Automatic-trim inputs still require an activity scan before conversion.
   Explicit-trim downloads now capture bounded lossless prefixes, but FFmpeg
   children still have no global CPU-token budget.
6. The source-pool seeder reuses downloaded media across focused and hybrid
   lists, but a shared content-addressed prepared cache could also eliminate
   their repeated preparation.
7. List processing remains sequential. Add `--list-jobs` with a global CPU and
   memory budget for multiple small lists.
8. The batch runner rejects same-basename collisions within one invocation, but
   independent invocations can still target the same stem. A stable list digest
   would close that remaining naming gap.
9. Streaming PCM into a single multi-output FFmpeg process could remove the
   temporary RF64 and three rereads. Segmented AAC/Opus encoding needs careful
   boundary testing because encoder delay can introduce gaps.
10. The three compressed-output hashes run in parallel but still reread every
    file after decode validation. Hashing during encoding or decode validation
    would eliminate the final read pass.

The next high-value benchmark is a streamed master-assembly prototype, followed
by jobs 1/2/4/6/8 scaling on the same warm-cache 24×24 corpus. Record p50, p95,
maximum pair time, final-tail time, process-tree CPU, RSS, and read/write bytes
for render, assembly, each encoder, decode validation, and hashing.
