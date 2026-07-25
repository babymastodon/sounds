# conv9 Listener

This Tauri 2 desktop interface reads `../../outputs/catalog.json` at runtime; it does not bundle the roughly 4.6 GB render corpus. The backend grants the asset protocol access only to the resolved output directory, then the frontend decodes a selected WAV for a static waveform and log-frequency spectrogram. Both plots have synchronized playback cursors and support click-to-seek.

Run after generating the corpus:

```bash
cd conv9/app/src-tauri
cargo run
```

Set `CONV9_OUTPUT_DIR=/absolute/path/to/outputs` if the output tree is not in the normal `conv9/outputs` location.

