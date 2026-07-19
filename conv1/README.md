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
```

Set `DOWNLOAD_JOBS` to control concurrent downloads and `CONV_JOBS` (or `--jobs`) to control parallel convolution renders. Existing valid WAVs are reused, making interrupted renders resumable. Pass `--force` to render everything again.

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

## Source licensing

The set draws from four independent providers and many acoustic domains:

- ESC-50/Freesound environmental recordings under CC BY-NC 3.0;
- Wikimedia Commons field recordings under per-file CC0, CC BY, or CC BY-SA licenses;
- BigSoundBank factory, workshop, sawmill, nature, and city recordings released under CC0;
- NASA Artemis audio cleared for creative reuse under NASA's media usage guidelines.

The downloaded media is intentionally not redistributed from this repository. Consult [`sources.tsv`](sources.tsv) before redistributing inputs or derived material; in particular, ESC-50's CC BY-NC terms are non-commercial and CC BY/CC BY-SA sources require attribution.
