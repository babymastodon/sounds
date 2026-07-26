# conv10 progress

Last updated: 2026-07-26

- [x] Keep all implementation, lists, audit evidence, and reports inside
  `conv10`.
- [x] Remove the short-additive approach and obsolete v14 output reports.
- [x] Accept arbitrary plain or explicit raw-input lists.
- [x] Check in and validate 48-input conv10 and conv8 lists.
- [x] Prepare inputs concurrently with strict first-download decoding and
  recipe-based reuse.
- [x] Support valid short sources below 12 seconds with pitch-preserving
  time-stretch.
- [x] Render only the long-input additive-synth matrix.
- [x] Optimize invariant synthesis work and oscillator recurrences.
- [x] Honor the requested worker count in input loading, rendering, and
  verification.
- [x] Add early input-aware render caching and output-recipe validation.
- [x] Encode FLAC, AAC/M4A, and Opus concurrently.
- [x] Remove persistent RF64, 32-kbit Opus, and all other final variants.
- [x] Generate and independently verify conv10: 576/576 stereo pairs and all
  three final encodings.
- [x] Generate and independently verify conv8: 576/576 stereo pairs and all
  three final encodings.
- [x] Record per-stage wall time and CPU utilization.
- [x] Audit all 96 source pages for commercial-use permission.
- [x] Confirm 58 CC0/public-domain and 38 CC BY sources; reject none.
- [x] Generate YouTube-ready attribution text for all 38 CC BY sources.
- [x] Commit only conv10 code, lists, tests, audits, and reports; never commit
  generated audio.
