# conv9 performance audit

Measurements were taken on an Intel Core Ultra 7 268V with four performance cores, four efficiency cores, no SMT, and 30 GiB RAM. Release DSP measurements use the same two prepared 61-second sources before and after the optimization pass. Browser measurements use headless Chrome at 1280×860 and device-pixel ratio 2.

## Measured results

| Workload | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| 16,384-point spectrogram, 1,240 columns | 2,071 ms median | 176 ms wall | 11.8× |
| Packed-real spectrogram, 1,302 columns | 176 ms wall | 67.5–76.4 ms wall | 2.30–2.61× |
| Windowed convolution, 5 s × 5 s | 748 ms | 171–175 ms | 4.3–4.4× |
| Windowed convolution, 0.1 s × 5 s, 8 threads | 9,318 ms | 2,180–2,522 ms | 3.7–4.3× |
| Windowed convolution, 0.1 s × 30 s, 8 threads | 75,257 ms | 20,907–22,492 ms | 3.35–3.60× |
| Windowed convolution, 0.1 s × 0.1 s | 695 ms | 171–183 ms | 3.8–4.1× |
| Full convolution, 61 s × 61 s | 348 ms | 340 ms | 1.02× |
| Source-filter vocoder, 61 s | 279–281 ms, weak geometric envelope | 466 ms, corrected band-power transfer | 1.66× more work |
| Predictive resonator bank, 61 s | 252–383 ms, one global model | 164 ms, parallel local models | 1.54–2.34× |
| Moving IR, 61 s A / 0.75 s IR / 0.5 s updates | 410 ms, 1 thread | 91 ms, 8 threads | 4.51× |
| Moving IR, 61 s A / 30 s IR / 0.5 s updates | 2,836–2,923 ms, 1 thread | 1,120–1,155 ms, 8 threads | 2.53× |

The spectrogram now uses six cancelable, persistent workers. Since every input frame is real, each worker packs its even/odd samples into one 8,192-point complex transform and exactly recovers the positive half of the requested 16,384-point real spectrum. This replaces the former 16,384-point complex core and reduces its butterfly count by 2.15× per column. Bit-reversal tables, Hann weights, and twiddles are precomputed once; the pool is warmed while the native audio render is already in flight, so worker startup and JavaScript tier-up are outside the visible spectrum stage. Forty-eight small contiguous time stripes are pulled dynamically by the pool, allowing faster performance cores to accept more work instead of waiting on one static stripe assigned to an efficiency core. Each stripe receives only its PCM range plus the FFT halo, calculates only unique visible log-frequency bins, and returns compact `Float32` values.

The main map now analyzes 1.05 columns per CSS pixel—1,302 rather than 1,240 at the measured viewport—while completing in 67.5–76.4 ms, 2.30–2.61× faster than the previous 176 ms implementation. The main thread uses a cached 256-color lookup table. Resize rescales the cached spectrum and performs no FFT. The static waveform and spectrum canvases are no longer recopied on every animation frame; compositor playheads move over them instead. Source previews use a separate 420×192 map with 8,192-point native real FFTs, up from 210×96 and 4,096 points; Rayon assigns columns to reusable per-thread FFT scratch buffers.

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

The source-filter vocoder uses a fixed 2,048-frame STFT and 256-frame hop. FFT plans and all time, spectrum, envelope, and smoothing buffers are reused throughout the 61-second render. Its Gaussian frequency kernel is precomputed once per render; recomputing the exponentials in the inner frame/bin loop took 1.95 seconds, while the retained precomputed kernel completes the corrected real-source DSP in 466 ms. The larger transform and normalized band-power envelope eliminate the measured hop image at the stronger default transfer.

The predictive resonator bank analyzes overlapping 8,192-frame local windows at a 2,048-frame hop. Each independent frame fits 32-pole A/B models with Levinson–Durbin recursion, whitens the windowed B frame, synthesizes it through A's corresponding stable model, preserves B's local power, and returns it for root-Hann overlap-add. Bounded 32-frame Rayon batches provide cancellation points and parallelize the expensive local autocorrelations without concurrent output writes. The corrected real-source DSP takes 164 ms, excluding source loading and common output conditioning; the former minute-long stationary fit took 252–383 ms yet could mathematically collapse to B passthrough when A and B shared a global spectrum.

The moving-IR method uses uniform partitioned convolution through two seconds: 1,024 input frames and 1,024-tap FIR partitions share fixed 2,048-point real FFT plans. For the default 61-second render, 2,860 A blocks and 36 IR partitions require 105.5 million interpolated frequency-bin products. Only 124 B snapshots are transformed, rather than transforming a new 0.75-second filter for all 2,860 A blocks. Input transforms, B-snapshot transforms, output-block inverse transforms, coherence calculation, and final overlap-add all use bounded Rayon batches. Every output block is independent until the final disjoint overlap-add, so one- and multi-thread output is bit-identical. The spectral workspace is approximately 80 MiB at the default and is rejected above a fixed 384 MiB ceiling.

At longer IR lengths, retaining every snapshot as a partition bank would exceed a gigabyte. The long-IR path instead uses convolution linearity to group the 1,024-frame interpolation weights by B snapshot, then convolves each compact weighted A span with that complete snapshot in one zero-padded FFT. It produces the same time-varying linear convolution as the partitioned path within `8e-5` absolute sample error while preserving the complete `A + IR - 1` tail. Centered B extraction explicitly leaves out-of-range samples at zero; it never shifts an edge window toward unrelated material. Reusable full-FFT workspaces are capped by a 384 MiB estimate. Snapshot preparation, adjacent coherence, full convolutions, and disjoint output tiles run in bounded parallel batches; each tile still adds snapshots in source order, making one- and eight-thread PCM bit-identical. Parallelizing coherence and the merge reduced the initial 30-second implementation from 1,696 ms to 1,120–1,155 ms on eight cores.

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

The original browser FFT sustained about 123 million complex butterflies per second on one UI thread. The packed-real implementation cuts the transform core from 114,688 to 53,248 complex butterflies per time column before threading, a 2.15× algorithmic reduction. Precomputed twiddles and persistent scratch arrays avoid setup and allocation on the repeated critical path. Six portable JavaScript workers compute the higher-resolution display without a WASM or GPU dependency.

The real-time pitch path processes about 375 2,048-point FFTs per second for stereo, around 4.2 million butterflies or 50–100 MFLOP/s. It performs no allocation in the 128-frame audio callback and has a fixed 2,048-frame latency: 42.7 ms at 48 kHz.

The default moving-IR render performs 4,464 filter, 2,860 input, and 2,895 output real transforms of size 2,048 plus its 105.5 million interpolated complex products. A direct sample-domain time-varying FIR would require about 105 billion tap products. Recomputing a complete filter transform at each 1,024-frame update would require 102,960 filter-partition transforms instead of 4,464. The measured optimized path sustains 148.7× real time on one thread and 671.1× on all eight heterogeneous cores: 410.3 ms and 90.9 ms respectively, using the median of three warm release renders. A forced scalar-style fused-multiply-add formulation measured 740 ms and was rejected; the retained plain complex arithmetic lets LLVM vectorize the critical bin loop. Keeping short IR normalization serial inside each already-parallel snapshot batch reduced the eight-core result from 100.7 ms to 90.9 ms by avoiding nested scheduling.

The 30-second moving-IR benchmark uses 124 time-varying snapshots over the same 61-second A input and 0.5-second update spacing. It completes in 2.84–2.92 seconds with one worker and 1.12–1.16 seconds with all eight heterogeneous cores, a measured 2.53× speedup. This case is dominated by repeated 2,097,152-point transforms and streaming roughly 30-second filter/output buffers, so its scaling approaches the machine's memory-bandwidth ratio rather than its arithmetic-core ratio.

## Remaining expensive cases

Very unequal legal windows, particularly 0.1 s × 30 s, remain intrinsically expensive because the short scan hop creates thousands of very large FFTs. The implementation is now close to the measured kernel ceiling; further material improvement must reduce the operation count by reusing the long window's spectrum across several short-window positions or adopting a partitioned/multirate formulation. Either would be a DSP change requiring new power and seam regressions rather than another threading pass.
