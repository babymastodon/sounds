# conv9: lockstep windowed convolution

`conv9` replaces whole-clip convolution with local convolution. Two one-minute clips advance in normalized lockstep, a short window is extracted from each, only those windows are linearly FFT-convolved, and the local results are merged onto a fixed one-minute timeline. This retains changes of scene, rhythm, and texture that a 119.999-second whole-clip convolution would blur into one stationary tail.

The complete experiment is:

- 12 license-tracked, open/free one-minute mono sources spanning music, percussion, mechanical rhythm, activities, speech, city, weather, water, wildlife, and transit;
- all 66 unordered source pairs;
- four local-convolution algorithms;
- short, medium, and long window presets;
- 792 one-minute, mono, 48 kHz PCM16 WAVs;
- an exhaustive verifier and generated `metrics.csv` / `catalog.json`;
- a Tauri comparison app with position-preserving variant switching, waveform, log-frequency spectrogram, and a synchronized seek cursor in both views.

## Ranked algorithms

The ranking is an expected-result ranking, not a baked-in test result:

1. **Multiresolution convolution** should give the best balance. It convolves low frequencies with windows 1.6× the selected preset, mid frequencies at 1×, and highs at 0.6×. Complementary raised-cosine masks split the local convolution spectrum at 160–300 Hz and 1.7–2.6 kHz before the bands are overlap-added and recombined. Bass gets enough context to sound stable while transients get shorter windows and less smear.
2. **Sliding WOLA convolution** is the strongest neutral baseline. Tukey-windowed source frames advance in lockstep, each full local linear convolution is centered at the corresponding output time, and Hann weighted overlap-add divides by the accumulated synthesis weights. It should be smooth and preserve local evolution, with less special treatment than the first method.
3. **Dual evolving-IR convolution** treats each source in turn as the local carrier. It center-crops each local convolution to the A-window and B-window lengths, overlap-adds both directional views, and averages them. It should retain tighter event timing, but it intentionally discards more convolution tails.
4. **Independent chunks + crossfade** is the simple comparator. Non-overlapping lockstep chunks are independently convolved, center-cropped to their timeline slot plus a 25% transition, and merged with equal-power edge crossfades. It is likely to make block boundaries or abrupt texture changes more evident.

All four paths are convolution-only; there is no dry source mixed into any render.

## Window presets

| Preset | clip A window | clip B window | lockstep hop |
|---|---:|---:|---:|
| short | 0.30 s | 0.45 s | 0.55 s |
| medium | 0.90 s | 1.30 s | 1.40 s |
| long | 2.25 s | 3.25 s | 3.20 s |

The unequal A/B lengths make the requested per-clip window control explicit. Presets live in `WindowPreset::config` in `src/dsp.rs`; changing those three definitions changes every algorithm consistently. At a timeline phase `p`, each extractor reads the window centered at `p × (source_frames − 1)`, with reflection at the boundaries.

Local convolutions receive smoothed RMS gains whose adjacent changes are limited to 1 dB. Completed files are DC-removed, high-passed at 18 Hz, gently saturated toward −20.4 dBFS RMS, peak-limited below 0.92, and faded for 20 ms. These shared stages keep comparisons level-matched without normalizing each algorithm differently after encoding.

## Build and generate

Requirements: Rust, FFmpeg/FFprobe, `curl`, and about 5 GB free for the finished output tree.

```bash
cd conv9
./scripts/download_samples.sh
cargo test --offline
cargo build --release --offline
target/release/conv9 render --jobs 4
target/release/conv9 verify --jobs 4
```

Renders resume safely: an existing output is reused only after its format, duration, level, finiteness, and ceiling are validated. Use `--force` to replace selected valid files. Development filters make it possible to render one comparison:

```bash
target/release/conv9 render \
  --pair ambient_guitar__drumland_ambient \
  --algorithm sliding_wola \
  --preset short \
  --force
```

The full hierarchy is:

```text
outputs/
  multiresolution/{short,medium,long}/*.wav
  sliding_wola/{short,medium,long}/*.wav
  evolving_ir/{short,medium,long}/*.wav
  chunk_crossfade/{short,medium,long}/*.wav
  catalog.json
  metrics.csv
```

`sources.tsv` is the canonical provenance manifest. The download script prefers a byte-identical source already cached by an earlier experiment and otherwise uses the recorded HTTPS URL. It always prepares a new exact 60-second, mono, 48 kHz float WAV for this experiment and records source/prepared SHA-256 files.

## Listener app

The app deliberately leaves the multi-gigabyte output hierarchy outside its bundle. After generation:

```bash
./app/run.sh
```

The Rust backend resolves `conv9/outputs`, grants the Tauri asset protocol access only to that directory, and returns the catalog. See [`app/README.md`](app/README.md) for relocation details.
