# conv9 performance audit

Measurements were taken on an Intel Core Ultra 7 268V with four performance cores, four efficiency cores, no SMT, and 30 GiB RAM. Release DSP measurements use the same two prepared 61-second sources before and after the optimization pass. Browser measurements use headless Chrome at 1280×860 and device-pixel ratio 2.

## Measured results

| Workload | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| 16,384-point spectrogram, 1,240 columns | 2,071 ms median | 176 ms wall | 11.8× |
| Windowed convolution, 5 s × 5 s | 748 ms | 426 ms | 1.75× |
| Windowed convolution, 0.1 s × 5 s | 19,817 ms | 6,260–6,809 ms | 2.9–3.2× |
| Windowed convolution, 0.1 s × 0.1 s | 695 ms | 557–619 ms | 1.1–1.25× |
| Full convolution, 61 s × 61 s | 348 ms | 340 ms | 1.02× |
| Source-filter vocoder, 61 s | not present | 279–281 ms warm | — |

The spectrogram now splits contiguous time ranges across four cancelable workers. Each worker receives only its PCM range plus the FFT halo, reuses one Hann window and bit-reversal plan, calculates only unique visible log-frequency bins, and returns compact `Float32` stripes. The main thread uses a cached 256-color lookup table. Resize rescales the cached spectrum and performs no FFT. The static waveform and spectrum canvases are no longer recopied on every animation frame; compositor playheads move over them instead.

The Rust convolution path now:

- reuses time and frequency workspaces instead of allocating two spectra for every grain;
- runs the two independent forward FFTs through a bounded Rayon pool;
- precomputes Tukey tapers, synthesis weights, local-power amplitudes, and chunk fades;
- estimates slowly varying grain coherence at a prime 31-frame stride, avoiding common tone-period aliasing;
- retains the original full-rate overlap, normalization, conditioning, and output checks.

The source-filter vocoder uses a fixed 1,024-frame STFT and 128-frame hop. FFT plans and all time, spectrum, envelope, prefix-sum, and smoothing buffers are reused throughout the 61-second render.

Every native response carries source-load, DSP, conditioning, encoding, and total timings. The frontend records those with decode, waveform, and spectrum timings in a bounded `state.performanceLog` and emits each record as `[conv9 performance]` in the developer console.

## Throughput ceilings

These are calculated targets, not measured promises.

The default 5 s windowed render performs 51 local convolutions: 153 real transforms of size 524,288, about 3.89 billion FFT operations before overlap and conditioning. With workspace reuse and precomputed control curves, an optimized single performance core is expected to spend roughly 0.50–0.70 seconds in this render. On this heterogeneous eight-core CPU, the measured FFT ratio gives a hard compute ceiling of about 6.8 performance-core equivalents; scheduling, overlap dependencies, and memory traffic reduce the practical ceiling to roughly 5×. Bounded grain batches could therefore reach approximately 0.15–0.25 seconds of DSP and 0.22–0.35 seconds end to end.

The asymmetric 0.1 s × 5 s case performs 2,442 local convolutions and 7,326 transforms of size 262,144, about 88.3 billion FFT operations. Its extraction, overlap, and coherence passes also touch roughly 1.8 billion sample positions. A fully optimized single-thread implementation is estimated at 13–15 seconds; full safe grain batching on this machine is estimated at 3–4 seconds. The current 6.3–6.8 seconds uses two-way FFT parallelism while preserving sequential gain/coherence state, so it sits between those bounds without the multi-gigabyte memory cost of retaining every grain.

Measured memory bandwidth was approximately 18–21 GB/s on one core, 42.5 GB/s on four, and 48.5–51 GB/s on all eight. Memory-bound stages therefore top out around 2.5–2.8× even though FFT-heavy stages can approach the higher compute ceiling. This is why unbounded grain parallelism is not appropriate.

The original browser FFT sustained about 123 million complex butterflies per second on one UI thread. A planned real-FFT implementation has a calculated single-thread target of 150–300 ms for the complete display. Four to eight optimized workers or native SIMD have a practical 70–150 ms target once transfer, merge, and image upload are included. The current 176 ms wall time uses four portable JavaScript workers and already approaches the single-thread planned-real-FFT target without adding a WASM or GPU dependency.

The real-time pitch path processes about 375 2,048-point FFTs per second for stereo, around 4.2 million butterflies or 50–100 MFLOP/s. It performs no allocation in the 128-frame audio callback and has a fixed 2,048-frame latency: 42.7 ms at 48 kHz.

## Remaining expensive cases

Very unequal legal windows, particularly 0.1 s × 30 s, remain intrinsically expensive because the short scan hop creates thousands of very large FFTs. Parallelism reduces latency but does not change that operation count. A future algorithmic improvement would need to reuse the long window’s spectrum across several short-window positions or choose a partitioned/multirate formulation; that would be a DSP change requiring new power and seam regressions, not merely another thread.
