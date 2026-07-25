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

Run from the repository root after generating the corpus:

```bash
./conv9/app/run.sh
```

The launcher uses installed system libraries when available. In the development
environment it can also use an extracted sysroot at `/tmp/conv9-tauri-devel`, or
one selected with `CONV9_TAURI_SYSROOT`. If neither is available, it prints the
required Fedora installation command. Set
`CONV9_OUTPUT_DIR=/absolute/path/to/outputs` if the output tree is not in the
normal `conv9/outputs` location.

For frontend-only inspection:

```bash
cd conv9/app
npm run preview
```

Open `http://127.0.0.1:4173/app/src/`. This preview server supports HTTP byte
ranges so playback seeking works correctly; a generic static server may not.
The same UI uses relative HTTP paths instead of the Tauri bridge, so preview
mode does not replace the desktop app.

The functional test uses Playwright with an installed Chrome or Chromium and
the real output catalog:

```bash
cd conv9/app
npm ci
npm test
```

Set `CHROME_BIN` if the browser is not in a standard Linux location.
