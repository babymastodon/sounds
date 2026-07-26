# conv9 performance audit

Measurements were taken on an Intel Core Ultra 7 268V with four performance cores, four efficiency cores, no SMT, and 30 GiB RAM. Release DSP measurements use the same two prepared 61-second sources before and after the optimization pass. Browser measurements use headless Chrome at 1280×860 and device-pixel ratio 2.

## Measured results

| Workload | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| 16,384-point spectrogram, 1,240 columns | 2,071 ms median | 176 ms wall | 11.8× |
| Windowed convolution, 5 s × 5 s | 748 ms | 171–175 ms | 4.3–4.4× |
| Windowed convolution, 0.1 s × 5 s, 8 threads | 9,318 ms | 2,180–2,522 ms | 3.7–4.3× |
| Windowed convolution, 0.1 s × 30 s, 8 threads | 75,257 ms | 20,907–22,492 ms | 3.35–3.60× |
| Windowed convolution, 0.1 s × 0.1 s | 695 ms | 171–183 ms | 3.8–4.1× |
| Full convolution, 61 s × 61 s | 348 ms | 340 ms | 1.02× |
| Source-filter vocoder, 61 s | not present | 279–281 ms warm | — |
| Predictive resonator bank, 61 s | not present | 252–383 ms warm | — |

The spectrogram now splits contiguous time ranges across four cancelable workers. Each worker receives only its PCM range plus the FFT halo, reuses one Hann window and bit-reversal plan, calculates only unique visible log-frequency bins, and returns compact `Float32` stripes. The main thread uses a cached 256-color lookup table. Resize rescales the cached spectrum and performs no FFT. The static waveform and spectrum canvases are no longer recopied on every animation frame; compositor playheads move over them instead.

The Rust convolution path now:

- reuses time and frequency workspaces instead of allocating two spectra for every grain;
- gives each safe Rayon worker one persistent FFT workspace and renders independent grains in bounded, source-ordered batches;
- keeps each worker's two forward transforms serial, preventing nested parallelism and oversubscription;
- calculates the sequential smoothed gain from RMS accumulated while copying the inverse FFT, avoiding another complete grain pass;
- merges each batch across disjoint 32,768-frame output tiles on all available cores while preserving the original grain-addition order at every sample;
- precomputes Tukey tapers, synthesis weights, local-power amplitudes, and chunk fades;
- estimates slowly varying grain coherence at a prime 31-frame stride, avoiding common tone-period aliasing;
- retains the original full-rate overlap, normalization, conditioning, and output checks.

The batch pool is also memory-bounded. A 0.1 s × 30 s worker needs approximately 51 MiB for FFT buffers, extracted inputs, and its result. All eight cores are used on the measured machine, while a fixed 768 MiB worker-workspace ceiling prevents machines with very high core counts from creating an unbounded number of giant FFT buffers. Including the five complete overlap arrays, the measured eight-worker case remains near 0.55 GiB rather than retaining thousands of multi-megabyte grains.

The source-filter vocoder uses a fixed 1,024-frame STFT and 128-frame hop. FFT plans and all time, spectrum, envelope, prefix-sum, and smoothing buffers are reused throughout the 61-second render.

The predictive resonator bank computes each source's 65 biased autocorrelation lags in bounded eight-lag Rayon batches, solves the resulting Toeplitz systems with Levinson–Durbin recursion, and performs one allocation-free, sample-continuous analysis/synthesis pass after reserving its exact output. Cancellation is checked between autocorrelation batches and every 16,384 synthesis frames. The reported 252–383 ms excludes source loading and common output conditioning.

Every native response carries source-load, DSP, conditioning, encoding, and total timings. The frontend records those with decode, waveform, and spectrum timings in a bounded `state.performanceLog` and emits each record as `[conv9 performance]` in the developer console.

## Throughput ceilings

The ceilings below distinguish three different limits: the processor's arithmetic peak, the measured single-worker FFT kernel, and achieved complete-render throughput. FFT operation counts use the common approximate cost of 2.5 N log2(N) floating operations for one real transform. Actual FFTs are permutation- and memory-heavy, so the hardware arithmetic peak is intentionally only an order-of-magnitude bound.

The default 5 s windowed render performs 50 local convolutions: 150 real transforms of size 524,288, about 3.74 billion FFT operations before overlap and conditioning. Bounded grain batches now complete that DSP in 171–175 ms on the heterogeneous eight-core CPU; total warm render time, including conditioning and PCM encoding, is 278–282 ms.

At 75% overlap, the asymmetric 0.1 s cases use a 1,200-frame scan hop and 2,441 grains over the 61-second source.

For 0.1 s × 5 s, each grain uses three 262,144-point real transforms. The complete render therefore represents 7,323 transforms, approximately 86.4 billion FFT operations, plus 0.60 billion grain/output positions. The optimized one-worker kernel measures 1.71 ms per grain, or about 20.7 effective GFLOP/s. That predicts 4.17 seconds of FFT work; the measured complete one-thread render is 4.80–5.08 seconds, down from 10.00 seconds before the serial cleanup. Eight workers complete the DSP in 2.18–2.52 seconds. The repeated PCM output is bit stable across one and eight threads (`9afd8c39f13ce123`, 64-bit FNV-1a over the encoded WAV).

For 0.1 s × 30 s, each grain uses three 2,097,152-point real transforms: approximately 806 billion FFT operations in total. The optimized one-worker kernel measures 13.71 ms per grain, or about 24.1 effective GFLOP/s, giving a 33.47-second empirical FFT floor. Overlap and coherence visit 3.53 billion grain positions; a conservative 183 GB of associated streaming traffic needs another 8.7–10.2 seconds at the measured 18–21 GB/s single-core bandwidth. The resulting practical one-thread floor is about 42–44 seconds, and the complete measured result is 43.28 seconds. In other words, the single-thread critical path is already within measurement noise of its kernel-plus-bandwidth ceiling.

With eight concurrent large-FFT workers, the measured kernel ceiling is 8.09 ms effective time per grain, or 19.75 seconds for all FFTs. The complete eight-thread render takes 20.91–22.49 seconds, within roughly 6–14% of that empirical ceiling and 3.35–3.60× faster than the previous 75.26-second path. The WAV hash is stable across one and eight threads (`8481759954fe6764`).

At 5 GHz, a single AVX2 performance core has an order-of-magnitude arithmetic peak of roughly 80–160 GFLOP/s depending instruction mix, and the heterogeneous package has several times that aggregate peak. A purely arithmetic lower bound would therefore be much smaller. The measured 20–24 effective GFLOP/s FFT kernels are the defensible achievable ceiling here because cache-unfriendly FFT permutations and shared memory bandwidth dominate long before arithmetic units saturate.

Measured memory bandwidth was approximately 18–21 GB/s on one core, 42.5 GB/s on four, and 48.5–51 GB/s on all eight. Memory-bound stages therefore top out around 2.5–2.8× even though FFT-heavy stages can approach the higher compute ceiling. This is why unbounded grain parallelism is not appropriate.

Correctness coverage compares the batched renderer against the prior sequential formulation on deterministic synthetic input; its measured maximum absolute difference is 1.49e-8 at 0.0931 reference RMS, far inside the 0.02%-of-reference-RMS regression limit. It also verifies two parallel renders bit for bit. Separate tests verify cancellation between bounded batches and enforce the per-worker memory ceiling. The manual `windowed_perf` example reports DSP/total time and a WAV hash for repeatable release measurements.

The original browser FFT sustained about 123 million complex butterflies per second on one UI thread. A planned real-FFT implementation has a calculated single-thread target of 150–300 ms for the complete display. Four to eight optimized workers or native SIMD have a practical 70–150 ms target once transfer, merge, and image upload are included. The current 176 ms wall time uses four portable JavaScript workers and already approaches the single-thread planned-real-FFT target without adding a WASM or GPU dependency.

The real-time pitch path processes about 375 2,048-point FFTs per second for stereo, around 4.2 million butterflies or 50–100 MFLOP/s. It performs no allocation in the 128-frame audio callback and has a fixed 2,048-frame latency: 42.7 ms at 48 kHz.

## Remaining expensive cases

Very unequal legal windows, particularly 0.1 s × 30 s, remain intrinsically expensive because the short scan hop creates thousands of very large FFTs. The implementation is now close to the measured kernel ceiling; further material improvement must reduce the operation count by reusing the long window's spectrum across several short-window positions or adopting a partitioned/multirate formulation. Either would be a DSP change requiring new power and seam regressions rather than another threading pass.
