# Convolutions 10 progress

Last updated: 2026-08-01

- [x] Keep implementation, lists, inventories, tests, and reports inside
  `conv10`.
- [x] Rename the original two programs to Field Atlas and Melody Works.
- [x] Define eight focused and four hybrid new pieces.
- [x] Select 384 distinct online source snippets across 192 topics.
- [x] Validate 384 unique source pages, media URLs, durations, selected audio
  windows, and content hashes.
- [x] Write one explicit 48-input list for each new piece.
- [x] Confirm all 14 lists contain 24 short and 24 long inputs.
- [x] Reuse 192 sources in one focused and one hybrid piece, producing 576 total
  list uses without adding duplicate source records.
- [x] Add the ordered 14-track `SONGS.tsv` album catalog.
- [x] Embed title, album, artist, album artist, composer, genre, year, track,
  disc, and description in every output.
- [x] Group final audio under `flac/`, `m4a/`, and `opus/`.
- [x] Capture bounded lossless prefixes for explicit online trims instead of
  downloading arbitrarily long recordings in full.
- [x] Decode-validate exact selected source windows concurrently.
- [x] Render and verify all 8,064 convolution pairs across 14 matrices.
- [x] Generate 14 FLAC, 14 AAC/M4A, and 14 Opus masters.
- [x] Decode every compressed master end to end.
- [x] Independently verify all 42 codecs, durations, channel/rate properties,
  metadata records, directory membership, and SHA-256 hashes.
- [x] Remove obsolete generated `conv8.*` and `conv10.*` outputs.
- [x] Build the release binary and pass the original 31 Rust tests plus 9
  Python tests.
- [x] Record complete run and performance results.
- [x] Keep all generated and downloaded audio out of Git.

## Config-driven harmony revision

- [x] Replace the global 13-EDO chord table with per-song tuning, register,
  inversion, palette, scene, motif, and progression settings.
- [x] Consolidate samples, output metadata, and harmony into one authoritative
  JSON config per piece.
- [x] Generate and validate configs for all 14 album pieces, each with 48
  samples and a 576-position scene progression.
- [x] Derive concrete palette, chord, root, and inversion choices
  deterministically from ordered sample filenames before parallel rendering.
- [x] Support equal divisions of arbitrary periods and explicit ratio scales,
  including multiple tunings in one hybrid piece.
- [x] Exclude 12-EDO entirely; Commons uses 15-EDO and Stormfolk quotes that
  same 15-EDO parent palette.
- [x] Record scene and harmony decisions in pair metrics and preserve configured
  progression order during concatenation.
- [x] Pass 34 Rust tests and 15 Python tests, including exhaustive schedule
  construction for every checked-in config.
- [x] Audit `pair_count`: retain all 576 exhaustive pairs, vary every song
  across at least three 24–72-pair scene lengths, and align every boundary to a
  complete motif cycle.
- [x] Add config-driven descriptions covering sample themes, tuning systems,
  chord vocabulary, main motif, full scene form, and pair-count spans.
- [x] Stage album production as render-all, assemble-all, globally parallel
  whole-file encoding, then bounded parallel validation and hashing.
- [x] Replace all old v15 masters with config-driven v17 renders and verify
  embedded cover art in every M4A.
- [x] Run source/config audits, full compressed end-to-end decoding, the
  independent 42-file album verifier, and SHA-256 checks.
- [x] Use 64 preparation workers and at least eight workers for rendering,
  RF64 assembly, whole-file encoding, and finalization.
- [x] When `/run/media/babymastodon/FBFA-9088` is mounted, refresh its `conv`
  directory, `cover.jpg`, and `conv` playlist from the verified masters.
- [x] Embed the exact checked-in JPEG cover in FLAC, M4A, and Opus; require
  byte-identical cover validation in the batch finalizer and independent album
  verifier.
