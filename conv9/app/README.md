# conv9 Listener

This Tauri 2 desktop interface reads `../../outputs/catalog.json` at runtime; it does not bundle the roughly 4.6 GB render corpus. The backend grants the asset protocol access only to the resolved output directory, then the frontend decodes a selected WAV for a static waveform and log-frequency spectrogram. Both plots have synchronized playback cursors and support click-to-seek.

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

Run after generating the corpus:

```bash
cd conv9/app/src-tauri
cargo run
```

Set `CONV9_OUTPUT_DIR=/absolute/path/to/outputs` if the output tree is not in the normal `conv9/outputs` location.

For frontend-only inspection from the `conv9` directory, run `python3 -m http.server 4173` and open `http://127.0.0.1:4173/app/src/`. The same UI then uses relative HTTP paths instead of the Tauri bridge; this preview mode does not replace the desktop app.
