# conv9: on-demand local convolution

`conv9` compares two independently selected 61-second clips without retaining rendered audio. Every UI selection invokes the Rust DSP, conditions the result, encodes a mono 48 kHz PCM16 WAV in memory, and transfers it directly to the Tauri webview. Changing a clip, method, window, or method parameter starts a new render; stale work is cancelled between FFT blocks. There is no output directory or render cache.

The 96 license-tracked sources each represent a distinct sound kind. The original broad field-recording set is joined by 48 deliberately foreground-focused additions: 24 music or instrument recordings with clear notes, attacks, and phrases, and 24 non-musical recordings built around identifiable voices, impacts, vehicles, tools, alarms, or physical processes. The additions avoid drones and generic atmosphere beds, and favor recordings whose content changes substantially during the selected minute. `sources.tsv` is their canonical provenance manifest. Prepare them once with:

```bash
cd conv9
./scripts/download_samples.sh
```

Prepared inputs are exact 61-second, mono, 48 kHz PCM16 WAVs. Their raw recordings are also verified to exceed one minute. They are source material, not precomputed convolutions.

For additions with more than a minute of material, `scripts/rank_eventful_trims.py` analyzes half-second level, spectral-centroid, spread, and flux changes and reports the most eventful valid cut. It remains a review tool by default; after listening and rejecting weak sources, pass `--write-manifest` to apply the reviewed offsets. Raw-source and preparation recipes include each source URL, cache source, and trim, so changing any of them refreshes only the affected local files.

In `sources.tsv`, a `cache_source` of `-` means the source is downloaded directly; older entries can name a compatible raw file from a previous experiment for local reuse.

## Methods and controls

The two source selectors are independent. Each method declares its own controls, so a method can have zero, two, or eventually more windows:

1. **Windowed convolution** replaces the former multiresolution and WOLA methods. It scans A/B windows in lockstep, performs exactly one full-band linear FFT convolution per pair, and merges the complete grains with root-Hann crossfades. Its panel contains A/B windows, input taper, and overlap.
2. **Dual evolving IR** crops each local convolution to A- and B-sized carriers before overlap-add. In addition to the shared window and input-taper controls, it exposes midpoint carrier balance, balance motion over time, and onset-to-tail crop position.
3. **Independent chunks** convolves lockstep chunks and joins them with equal-power fades. It exposes A/B windows, input taper, overlap, and onset-to-tail crop position; the UI also reports the resulting overlap duration.
4. **Full convolution** is the smear/reference method. It exposes an offset and duration for each source, gives every selected cut a 20 ms edge fade, defaults to both complete 61-second clips, and retains the complete selected-segment linear convolution.
5. **Dry A / dry B** play either complete conditioned input exactly as it enters the convolution engine. They apply no convolution, output saturation, or second normalization pass and have no controls.

Windowed methods start with independent 5.00-second A/B durations and accept values from 0.10 to 30.00 seconds. The UI uses softened-log sliders—less compressed than a pure logarithmic curve while retaining useful sub-second resolution—plus numeric inputs for exact hundredths. Their shared input taper is a Tukey shape from nearly rectangular at `0.05` to full Hann at `1.0`, defaulting to `0.50`. Windowed convolution and evolving IR default to 75% source-scan overlap. Because each convolution is placed at `tA+tB`, that gives four-way synthesis coverage for equal windows; native audio testing found that 50% left a 2.95 dB hop-period ripple, while 75% measured 1.95 dB at 0.25×0.25 seconds and 1.42 dB at 0.1×5 seconds. Chunks use a separate 50% overlap measured against the shorter chunk.

The primary windowed method is specifically designed not to become a hop-rate “buzzfest.” Scan centers are distributed evenly from the first through the final source frame, so there is no short special-case final hop. Each local result is placed at its physical convolution time `tA + tB`; putting it at only one source time would phase-reset steady tones on every hop. Grain gains follow a four-second log-domain smoother rather than jumping independently.

Synthesis applies the selected input taper to both analysis windows and uses fixed root-Hann weights for overlap-add. Its denominator blends between two normalizations using the measured positive coherence `c` of adjacent, timeline-aligned grains: `(1-c) × sum(weight² × local power) + c × sum(weight × sqrt(local power))²`. Independent noise therefore receives equal-power normalization, while identical grains receive constant-amplitude normalization instead of the usual +3.01 dB midpoint swell. Negative correlation is clamped to zero only in the denominator: real opposite-phase cancellation remains, but it stays gradual and cannot trigger normalization gain. One half-Hann fade is applied only at each end of the complete result.

The parameter audit intentionally leaves the aligned A/B scan, FFT planning, output RMS, DC/high-pass conditioning, saturation, peak ceiling, and final edge fade automatic. They are implementation or safety invariants rather than creative controls. Every exposed parameter changes a distinct window, input taper, overlap, crop, carrier, segment, or synthesis decision and has a specific hover tooltip.

Output is never forced back to the 61-second input duration. Windowed convolution retains its physical two-source timeline plus the complete first/last local support—about 132 seconds for 5×5-second windows and 122.5 seconds for 0.25×0.25-second windows. Evolving IR retains the longest active carrier, and chunks retain the actual union of their crossfaded blocks. Full convolution has `A duration + B duration - 1 sample` frames, or just under 122 seconds when both complete clips are selected. The UI transport and visualizations follow the actual rendered length.

Every output is DC-removed, high-passed at 18 Hz, gently saturated toward the shared RMS target, held below a 0.92 peak ceiling, and faded for 20 ms. These safety/level constraints are intentionally not user-editable.

## Run

```bash
./conv9/app/run.sh
```

The desktop app uses native window decorations, disables minimize/maximize, loops audio automatically, preserves playback position across renders, provides independent volume and octave-logarithmic 0.5×–2× listening-speed controls with 1× centered, and displays waveform plus a 16,384-point, up-to-2,880-column log-frequency spectrogram. See [`app/README.md`](app/README.md) for prerequisites and tests.

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

The browser suite checks the control model, stale-selection behavior, looping, visualizations, transport, and fixed-viewport layout. Rust regressions exercise independent noise, correlated tones, identical and opposite-polarity grains, sparse impulses, silence, slow ramps, unequal windows down to the scaled equivalent of 0.1×5 seconds, non-divisible timelines, power ripple, hop-phase modulation, and seam derivatives. The native suite drives the real Tauri app, renders both windowed and full convolution through Rust, records an isolated audio sink, and measures the captured PCM.
