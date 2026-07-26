# conv9: on-demand local convolution

`conv9` compares two independently selected 61-second clips without retaining rendered audio. Every UI selection invokes the Rust DSP, conditions the result, encodes a mono 48 kHz PCM16 WAV in memory, and transfers it directly to the Tauri webview. Changing a clip, method, window, or method parameter starts a new render; stale work is cancelled between FFT blocks. There is no output directory or render cache.

The 48 license-tracked sources each represent a distinct sound kind, spanning water, ice, wildlife, fire, machines, buildings, crowds, transport, weather, voice, music, radio, and footsteps. `sources.tsv` is their canonical provenance manifest. Prepare them once with:

```bash
cd conv9
./scripts/download_samples.sh
```

Prepared inputs are exact 61-second, mono, 48 kHz PCM16 WAVs. Their raw recordings are also verified to exceed one minute. They are source material, not precomputed convolutions.

## Methods and controls

The two source selectors are independent. Each method declares its own controls, so a method can have zero, two, or eventually more windows:

1. **Multiresolution** convolves complementary low, mid, and high bands with different window scales, power-normalizes their overlapping grains, then recombines their common timeline. Its panel exposes A/B windows, analysis edge taper, overlap, B timeline offset, low/high window scale and gain, both split frequencies, and crossover width.
2. **Sliding WOLA** performs lockstep local linear convolutions and power-normalized root-Hann overlap-add. It exposes A/B windows, analysis edge taper, overlap, and B timeline offset.
3. **Dual evolving IR** crops each local convolution to A- and B-sized carriers before overlap-add. In addition to the shared window controls, it exposes midpoint carrier balance, balance motion over time, and onset-to-tail crop position.
4. **Independent chunks** convolves lockstep chunks and joins them with equal-power fades. It exposes A/B windows, input edge taper, overlap, B timeline offset, and onset-to-tail crop position; the UI also reports the resulting overlap duration.
5. **Full convolution** is the smear/reference method. It exposes an offset and duration for each source, gives every selected cut a 20 ms edge fade, defaults to both complete 61-second clips, and retains the complete selected-segment linear convolution.

Windowed methods start with independent 5.00-second A/B durations and accept values from 0.10 to 30.00 seconds. The UI uses softened-log sliders—less compressed than a pure logarithmic curve while retaining useful sub-second resolution—plus numeric inputs for exact hundredths. Multiresolution, WOLA, and evolving IR default to 75% overlap, or four scan positions per shorter window. Each uses a user-controlled Tukey analysis taper, a fixed root-Hann synthesis shape, and a normalization envelope derived from the convolution of the two analysis-window powers. This keeps expected output power continuous even with sub-second windows; reducing overlap deliberately reveals more grain and periodic motion. Chunks default to a 50% power-normalized equal-power crossfade measured against the shorter window.

The parameter audit intentionally leaves FFT planning, output RMS, DC/high-pass conditioning, saturation, peak ceiling, and final edge fade automatic. They are implementation or safety invariants rather than creative controls. Dry/wet mix also remains absent because every path is convolution-only. Every exposed parameter changes a distinct analysis, convolution, crop, band, alignment, or synthesis decision and has a specific hover tooltip.

Output is never forced back to the 61-second input duration. Sliding WOLA retains the full support of its first and last local convolutions; multiresolution retains the common support of all active bands; evolving IR retains the longest active carrier; and chunks retain the actual union of their crossfaded blocks. At the defaults these are approximately 71, 67, and 66 seconds for the first three methods; chunk duration also reflects its final partial timeline slot. Full convolution has `A duration + B duration - 1 sample` frames, or just under 122 seconds when both complete clips are selected. The UI transport and visualizations follow the actual rendered length.

Every output is DC-removed, high-passed at 18 Hz, gently saturated toward the shared RMS target, held below a 0.92 peak ceiling, and faded for 20 ms. These safety/level constraints are intentionally not user-editable.

## Run

```bash
./conv9/app/run.sh
```

The desktop app uses native window decorations, disables minimize/maximize, loops audio automatically, preserves playback position across renders, and displays waveform plus a 16,384-point, up-to-2,880-column log-frequency spectrogram. See [`app/README.md`](app/README.md) for prerequisites and tests.

## Test

```bash
cd conv9
cargo test --offline

cd app/src-tauri
cargo test --offline
cargo build --offline

cd ..
npm ci
npm run test:all
```

The browser suite checks the control model, stale-selection behavior, looping, visualizations, transport, and fixed-viewport layout. The native suite drives the real Tauri app, renders both windowed and full convolution through Rust, records an isolated audio sink, and measures the captured PCM.
