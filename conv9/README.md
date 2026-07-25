# conv9: on-demand local convolution

`conv9` compares two independently selected one-minute clips without retaining rendered audio. Every UI selection invokes the Rust DSP, conditions the result, encodes a mono 48 kHz PCM16 WAV in memory, and transfers it directly to the Tauri webview. Changing a clip, method, window, or method parameter starts a new render; stale work is cancelled between FFT blocks. There is no output directory or render cache.

The 12 license-tracked sources span music, percussion, mechanical rhythm, activities, speech, city, weather, water, wildlife, and transit. `sources.tsv` is their canonical provenance manifest. Prepare them once with:

```bash
cd conv9
./scripts/download_samples.sh
```

Prepared inputs are exact 60-second, mono, 48 kHz WAVs. They are source material, not precomputed convolutions.

## Methods and controls

The two source selectors are independent. Each method declares its own controls, so a method can have zero, two, or eventually more windows:

1. **Multiresolution** convolves complementary low, mid, and high bands with different window scales, then recombines them. Its panel exposes A/B windows, Tukey taper, low/high window scale, low/high band gain, and both frequency splits.
2. **Sliding WOLA** performs lockstep local linear convolutions and Hann weighted overlap-add. It exposes A/B windows and Tukey taper.
3. **Dual evolving IR** crops each local convolution to A- and B-sized carriers before overlap-add. It exposes A/B windows, Tukey taper, and the A/B carrier balance.
4. **Independent chunks** convolves lockstep chunks and joins them with equal-power fades. It exposes A/B windows, Tukey taper, and crossfade percentage; the UI also reports the resulting duration.
5. **Full convolution** is the smear/reference method. It linearly convolves both complete 60-second clips, then retains the final 60 seconds. It has no window controls.

Windowed methods start with independent 5.00-second A/B durations and accept values from 0.10 to 30.00 seconds. The UI uses softened-log sliders—less compressed than a pure logarithmic curve while retaining useful sub-second resolution—plus numeric inputs for exact hundredths. Their hop is derived as 80% of the longer window, ensuring overlap without another global control. Every interactive control has a specific hover tooltip describing its DSP or playback effect. All local paths contain convolution only; no dry source is mixed in.

Every output is DC-removed, high-passed at 18 Hz, gently saturated toward the shared RMS target, held below a 0.92 peak ceiling, and faded for 20 ms. These safety/level constraints are intentionally not user-editable.

## Run

```bash
./conv9/app/run.sh
```

The desktop app uses native window decorations, disables minimize/maximize, loops audio automatically, preserves playback position across renders, and displays waveform plus an 8,192-point, up-to-2,880-column log-frequency spectrogram. See [`app/README.md`](app/README.md) for prerequisites and tests.

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
