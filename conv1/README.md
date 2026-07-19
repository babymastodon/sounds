# conv1

`conv1` renders a reproducible matrix of long-form, creative convolutions on the CPU. Its 48 ambient-oriented inputs range from 1 to 54 seconds. Every unordered pair, including self-convolutions, is rendered once: 1,176 unique WAVs represent all 2,304 cells in the symmetric 48×48 matrix.

## Run the complete pipeline

Requirements: a current Rust toolchain, `curl`, FFmpeg/FFprobe, `awk`, and `sha256sum`.

```bash
./scripts/render_all.sh
```

Downloaded and prepared inputs are written under `samples/`; rendered audio and reports are written under `outputs/`. Both media trees are ignored by Git. [`sources.tsv`](sources.tsv) is the authoritative provenance manifest and includes each acoustic domain, creator, source page, download URL, license, excerpt offset, and target duration. The manifest loader requires exactly 48 distinct sources, including exactly 24 lasting over 30 through 60 seconds, and refuses a corpus unless at least half of its entries have the explicit `industrial` domain. The checked-in set has 25 industrial recordings out of 48.

To render or verify separately:

```bash
./scripts/download_samples.sh
cargo run --release -- render
cargo run --release -- verify
cargo run --release -- concat
```

Set `DOWNLOAD_JOBS` to control concurrent downloads and `CONV_JOBS` (or `--jobs`) to control parallel convolution renders. Existing valid WAVs are reused, making interrupted renders resumable. Pass `--force` to render everything again.

The `concat` stage reads the 1,176 canonical WAVs in matrix order and writes a seekable RF64 PCM master at `outputs/final/final_mix.rf64.wav`; RF64 is used because ordinary RIFF/WAV is limited to 4 GiB. Independent FFmpeg processes then encode `final_mix.flac`, a 192 kbit/s AAC `final_mix.m4a`, and mono VBR Opus versions at 64 and 16 kbit/s (`final_mix.opus` and `final_mix_16k.opus`). FLAC compression level 3 favors speed without changing decoded audio quality; `libfdk_aac` and `libopus` are preferred when the installed FFmpeg provides them. The stage uses a full five-second crossfade whenever both the accumulated master and incoming clip are long enough; for the six convolution files shorter than five seconds, it uses the complete shorter clip as the longest physically possible fade. `outputs/final/timeline.csv` records every clip's start time and actual incoming fade, while `outputs/final/concat.json` records the encoding audit. Use `concat --force` to rebuild the RF64 and every encoding; without it, valid existing stages are reused and only missing encodings are created.

## Algorithm

For inputs of lengths `N` and `M`, the renderer computes linear—not circular—convolution using a real FFT of length `(N + M - 1).next_power_of_two()`:

1. DC removal, 18 Hz high-pass filtering, edge fades, and conservative RMS/peak normalization of each input.
2. Zero-padded real-to-complex FFTs using `realfft`.
3. Complex bin multiplication and inverse normalization by the FFT length.
4. Complex-to-real inverse FFT and truncation to exactly `N + M - 1` frames.
5. DC/high-pass cleanup, RMS targeting, and a smooth `tanh` peak limiter.
6. 16-bit mono WAV output at 48 kHz using `hound`.

Pairs are grouped by FFT length. Each input spectrum is calculated once per size group and shared by all applicable pairs. Rayon schedules independent forward transforms, inverse transforms, conditioning, WAV writes, and verification across all configured CPU cores.

`realfft` is used instead of a convenience FFT wrapper because this workload benefits from explicit reusable plans, scratch buffers, real-valued transforms, and spectrum caching. `rayon` supplies all-core scheduling and `hound` handles deterministic WAV I/O. A direct `ndarray` convolution would be quadratic for these clip lengths, while `rodio` is intentionally omitted because playback is separate from offline rendering; it can be added later as a thin preview layer without changing the DSP pipeline.

## Output and quality gates

`outputs/matrix.csv` is the complete ordered 48×48 lookup table. Because convolution is commutative, mirrored cells reference the same canonical WAV. `outputs/metrics.csv` records measurements for each unique file, and `outputs/verification.json` summarizes the exhaustive post-render audit.

Verification rejects any result with:

- a missing or unexpected frame;
- a non-finite or clipped sample;
- peak amplitude below 0.12 or above 0.92;
- RMS outside −30 to −10 dBFS;
- absolute DC offset above 0.005;
- the wrong channel count, sample rate, or PCM format.

### Latest full-run audit

The expanded pipeline was force-rendered and verified on 2026-07-19 with 8 logical CPU cores. It produced 1,176 distinct canonical WAVs (5.8 GiB) for all 2,304 ordered matrix cells. Rendering took 24.4 seconds and the complete render-plus-verification process took 39.54 seconds, with peak resident memory of 1,754,468 KiB. The manifest contributed 25 industrial inputs out of 48. Every matrix path resolved, all 48 downloaded inputs and all 1,176 WAVs had distinct SHA-256 hashes, RMS ranged from -21.07 to -20.09 dBFS, maximum peak was 0.890, and no output contained clipping or non-finite samples. The generated `outputs/verification.json` records the machine-readable result.

The final-master stage was force-run with the optimized RF64-first pipeline. It assembled 2,830,030,830 frames (16:22:38.976), applying 1,169 full five-second crossfades and six duration-limited fades. The complete assembly plus parallel FLAC/AAC encoding took 398.40 seconds with peak resident memory of 688,828 KiB. The results were a 5,660,061,740-byte RF64, 1,973,208,898-byte lossless FLAC, and 1,426,079,629-byte AAC/M4A. The retained mono VBR Opus encodings are 443,387,702 bytes at 64 kbit/s and 103,615,802 bytes at 16 kbit/s. Every retained compressed master decoded end-to-end without errors, including complete decoded-PCM SHA-256 passes for both Opus files; the PCM master measured -19.9 dB mean and -1.0 dB maximum volume.

## Source licensing

The set draws from four independent providers and many acoustic domains:

- ESC-50/Freesound environmental recordings under CC BY-NC 3.0;
- Wikimedia Commons field recordings under per-file CC0, CC BY, or CC BY-SA licenses;
- BigSoundBank factory, workshop, sawmill, nature, and city recordings released under CC0;
- NASA Artemis audio cleared for creative reuse under NASA's media usage guidelines.

The downloaded media is intentionally not redistributed from this repository. Consult [`sources.tsv`](sources.tsv) before redistributing inputs or derived material; in particular, ESC-50's CC BY-NC terms are non-commercial and CC BY/CC BY-SA sources require attribution.
