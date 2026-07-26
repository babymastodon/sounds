# conv10 progress

Last updated: 2026-07-26

## Status

- [x] Isolated all work in `conv10`; existing experiments remain untouched.
- [x] Copied the conv8 v14 additive-pitch and convolution implementation.
- [x] Built the 24 musical-short / 24 eventful-long source manifest.
- [x] Verified all 48 source pages and exact media URLs.
- [x] Replaced seven candidates and corrected one stale media URL.
- [x] Selected the most eventful valid 12- or 30-second cut from every source.
- [x] Prepared and validated all 48 local inputs from exact URL/trim recipes.
- [x] Revalidated the reduced nine-field manifest after the final text cleanup.
- [x] Passed all 30 Rust tests.
- [x] Force-rendered and independently verified both 24×24 matrices.
- [x] Assembled and decode-checked both final programs in all five formats.
- [x] Commit only source, scripts, manifests, and reports; keep audio and builds ignored.

## Expected deliverables

Each approach will contain 576 stereo matrix WAVs followed by one continuous master in RF64, FLAC, AAC/M4A, 128 kbit/s Opus, and 32 kbit/s Opus. All generated audio lives below `outputs/` and is excluded from Git.
