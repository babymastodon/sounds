# Convolution Playground

This Tauri 2 desktop playground renders conv9 selections on demand. Its 160 prepared source WAVs are loaded through a four-clip lazy cache, keeping startup memory bounded. A clip, method, window, or method-parameter change runs the convolution on a blocking worker, encodes the conditioned result to an in-memory PCM16 WAV, and returns it through raw binary IPC. The frontend decodes that response once into one Web Audio `AudioBuffer`, shared by playback and analysis. Nothing is written to an output directory or retained as a render cache.

The app uses environment-native window decorations with minimize and maximize disabled. Audio loops sample-accurately from the decoded buffer. One app-owned sample clock drives the sound source, numeric time, seek bar, waveform cursor, and spectrum cursor, avoiding native media-element decoder preroll and implicit loop seeks. Each A/B source control opens a searchable, category-filtered keyboard listbox with concise metadata and lazy waveform plus FFT-map previews; previews are cached, stale preview results are ignored, and opening the browser never scans all 160 WAVs. Preview waveforms use a continuous filled min/max envelope to avoid dense-line moiré. The icon between the source selectors swaps A/B roles without changing the selected method or its controls. Method changes freeze playback immediately, preserve its absolute render-head time and playing intent, validate the returned session, request, source, method, window, and parameter identity, then atomically install the new buffer. A native-issued render epoch makes cancellation safe across WebView reloads. Source replacements use short anti-click ramps, and rapid seek/speed drags coalesce restarts. Volume and octave-logarithmic 0.5×–2× playback-speed controls affect listening only and never trigger a render; speed snaps to quarter-step listening rates and 1× sits at the slider’s exact midpoint. The adjacent pitch toggle inserts a 2,048-sample real-time phase vocoder and compensates the transport for its roughly 43 ms algorithmic latency; with the toggle off, playback remains ordinary varispeed. Render duration follows the method and parameters instead of being fixed to the one-minute inputs; full convolution also provides coupled offset/duration selectors for both clips. Windowed methods expose the shared input taper and keep the A/B scan aligned. The primary full-band windowed method uses timeline-correct grain placement, root-Hann synthesis, and coherence-aware power normalization to suppress correlated-grain pumping and hop-rate buzz. The source-filter vocoder keeps A's phase, timeline, and per-frame power while transferring B's Gaussian-smoothed band-power envelope; adaptive transient protection ignores the ordinary spectral flux of stationary noise. The latent convolution bank self-supervises reusable spectro-temporal responses and sparse activations from both clips, softly routes B's activity through A's learned response bank, and retains a capacity-limited stochastic residual instead of assuming every source is one instrument or one resonator. Zero-control dry A/B methods play either conditioned input without convolution or output reconditioning. RMS and peak appear as a waveform overlay instead of consuming their own row. The interface is fixed to the available viewport. Six persistent cancelable workers dynamically pull fine-grained stripes, compute the 16,384-point spectrum with packed-real FFTs, retain only visible log-frequency bins, and cache the completed image across resize; compositor playheads move without repainting either complete visualization.

Install the [official Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/) first. On Fedora:

```bash
sudo dnf install webkit2gtk4.1-devel \
  openssl-devel \
  curl wget file \
  libappindicator-gtk3-devel \
  librsvg2-devel \
  libxdo-devel
sudo dnf group install "c-development"
```

Prepare all 160 source clips once, then launch from the repository root:

```bash
cd conv9
./scripts/download_samples.sh
cd ..
./conv9/app/run.sh
```

The launcher verifies that every manifest entry has a prepared WAV before compiling or opening Tauri and prints the exact preparation command when the library is incomplete. The app also reports startup errors without a Rust panic. The launcher uses installed system libraries when available. In this development environment it can also use an extracted sysroot at `/tmp/conv9-tauri-devel`, or one selected with `CONV9_TAURI_SYSROOT`. Override input locations with `CONV9_MANIFEST` and `CONV9_INPUT_DIR`.

## Tests

The Playwright suite hosts only the frontend and prepared input fixture. It injects an on-demand test bridge to cover independent searchable source browsers, lazy preview caching, category filters, listbox keyboard control, continuous window sliders/numeric inputs, every dynamic method panel, rapid stale selections, looping transport, parallel high-resolution visualization, cached resize behavior, pitch-worklet insertion, and 1280×860 plus 900×640 layouts. A deterministic worklet test also verifies that inverse pitch compensation moves a synthetic 660 Hz varispeed tone back to 440 Hz at a sane output level:

```bash
cd conv9/app
npm ci
npm run test:browser
```

The native test is the authoritative integration test. It launches the real desktop binary through Tauri’s official WebDriver, invokes real windowed, source-filter-vocoder, and full-convolution renders, starts Web Audio playback, and records an isolated PipeWire/PulseAudio sink with FFmpeg. It compares first-start and replay speaker output to the exact decoded buffer, rejects a startup burst during the quiet first 500 ms, checks repeated speaker samples across short loop seams, confirms the AudioWorklet pitch path produces audible native output, and verifies render transitions, output power, and window-hop ripple:

```bash
cargo install tauri-driver --locked
cd conv9/app/src-tauri
cargo build --offline
cd ..
npm run test:native-audio
```

It requires `WebKitWebDriver`, `pactl`, and FFmpeg. Set `TAURI_DRIVER_BIN` or `CONV9_TAURI_SYSROOT` when they are in nonstandard locations. `npm run test:all` runs both suites after the desktop binary has been built.
