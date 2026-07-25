# conv9 Listener

This Tauri 2 desktop app renders conv9 selections on demand. At startup it loads the 12 prepared source WAVs into the Rust backend. A clip, method, window, or method-parameter change runs the convolution on a blocking worker, encodes the conditioned result to an in-memory PCM16 WAV, and returns it through raw binary IPC. The frontend creates one temporary `blob:` URL for playback and analysis; superseded blobs are revoked. Nothing is written to an output directory or retained as a render cache.

The app uses environment-native window decorations with minimize and maximize disabled. Audio loops automatically. Method changes preserve playback phase and playing state. Render duration follows the method and parameters instead of being fixed to the one-minute inputs; full convolution also provides coupled offset/duration selectors for both clips. Each windowed method exposes overlap and its relevant alignment, crossfade, band, carrier, or crop controls without exposing safety/normalization internals. The interface is fixed to the available viewport and uses a 16,384-point spectrogram analysis with up to 2,880 time columns.

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

Prepare the source material once, then launch from the repository root:

```bash
cd conv9
./scripts/download_samples.sh
cd ..
./conv9/app/run.sh
```

The launcher uses installed system libraries when available. In this development environment it can also use an extracted sysroot at `/tmp/conv9-tauri-devel`, or one selected with `CONV9_TAURI_SYSROOT`. Override input locations with `CONV9_MANIFEST` and `CONV9_INPUT_DIR`.

## Tests

The Playwright suite hosts only the frontend and prepared input fixture. It injects an on-demand test bridge to cover independent clip selection, continuous window sliders/numeric inputs, every dynamic method panel, rapid stale selections, looping transport, high-resolution visualizations, and 1280×860 plus 900×640 layouts:

```bash
cd conv9/app
npm ci
npm run test:browser
```

The native test is the authoritative integration test. It launches the real desktop binary through Tauri’s official WebDriver, invokes real windowed and full-convolution renders, starts WebKit playback, records an isolated PipeWire/PulseAudio sink with FFmpeg, and asserts that the captured PCM is non-silent:

```bash
cargo install tauri-driver --locked
cd conv9/app/src-tauri
cargo build --offline
cd ..
npm run test:native-audio
```

It requires `WebKitWebDriver`, `pactl`, and FFmpeg. Set `TAURI_DRIVER_BIN` or `CONV9_TAURI_SYSROOT` when they are in nonstandard locations. `npm run test:all` runs both suites after the desktop binary has been built.
