# sounds

Offline FFT-convolution experiments over license-tracked sets of 48 recordings. Each experiment is an independent Rust codebase with its own manifest, downloader, renderer, exhaustive verifier, convolution matrix, crossfaded RF64 master, and parallel FLAC/AAC/Opus encoders.

## Experiment lineage

- [`conv1`](conv1/README.md) is the baseline mono pipeline. It conditions two complete mono clips, computes their linear FFT convolution, normalizes the result, and stores 1,176 canonical WAVs for the symmetric 48×48 matrix.
- [`conv2`](conv2/README.md) builds on `conv1` with stereo, role-asymmetric preprocessing. For `D = 50%` of the shorter duration, left is `convolve(A, B without its final D)` and right is `convolve(A without its final D, B)`. Both channels are convolution-only; self-pairs are dual-mono.
- [`conv3`](conv3/README.md) builds on the verified `conv2` stereo, verification, RF64, and encoding pipeline. It keeps the left channel unchanged but removes the initial `D` from track 1 for the right channel: left is `convolve(A, B without its final D)`, right is `convolve(A without its initial D, B)`. Complementary head/tail trimming makes self-pairs stereo too.
- [`conv4`](conv4/README.md) keeps the `conv3` algorithm and pipeline but replaces every input with a new open corpus: 16 busy-city recordings, 16 storms/rain recordings, and 16 slow or sustained instrumental recordings. Its excerpts span 10–60 seconds, split evenly at 35 seconds.
- [`conv5`](conv5/README.md) keeps the `conv4` implementation and duration balance but replaces every input again. It uses six recordings from each of eight groups: busy city, industrial, rain, sports, long instrumentals, speeches, train ambience, and walking.
- [`conv6`](conv6/README.md) keeps the `conv5` implementation but expands the corpus to 24 user-selected categories. Each category contributes one 10–35 second source and one 36–60 second source, for 48 entirely new inputs.
- [`conv7`](conv7/README.md) keeps the `conv6` themes and stereo algorithm but uses new duration-balanced sources: one 5–15 second and one 25–35 second clip per category. Its complete bipartite 24×24 matrix renders only cross-duration pairs, for 576 distinct outputs.
- [`conv8`](conv8/README.md) turns `conv7` into a controlled pitch-and-rhythm playground. A deterministic content hash selects one of 13 Bohlen–Pierce chords for every short/long pair, then three matched matrices compare pure convolutional resonance, rhythmic sequenced convolution, and hybrid spectral modulation.

Every newly exposed cut receives a 20 ms fade. Stereo channels are conditioned independently for RMS and then share a final peak-ceiling pass. Detailed algorithms, quality gates, output formats, source provenance, and latest audits are documented inside each experiment directory.
