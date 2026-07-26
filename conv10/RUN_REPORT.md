# conv10 completed run

Completed: 2026-07-26

## Inputs and implementation

- 48 source pages and exact media URLs passed `scripts/verify_sources.sh`.
- 24 musical inputs are exactly 12 seconds; 24 action/process inputs are exactly 30 seconds.
- Every raw recording decodes end to end under FFmpeg strict error handling.
- Every prepared input is mono 48 kHz with the exact manifest frame count and a matching URL/trim/preparation recipe.
- The conv8 v14 DSP core files `audio.rs`, `concat.rs`, `convolution.rs`, `lib.rs`, and `pitch.rs` are byte-identical. `render.rs` differs only in its worker-thread label, and `main.rs` differs only in the crate import.
- All 30 Rust tests pass.

## Matrix results

| Approach | Pair WAVs | RMS range | Max peak | Tone floor | Verification |
|---|---:|---:|---:|---:|---|
| Long additive synth | 576 | −20.590 to −20.124 dBFS | 0.826 | −1.500 dB | pass |
| Short additive synth | 576 | −20.673 to −20.122 dBFS | 0.866 | −1.500 dB | pass |

Both approaches use 181 modal-noise, 191 inharmonic-FM, and 204 destroyed-saw pair assignments. The complete matrix contains 1,152 stereo 48 kHz PCM16 WAVs.

## Final programs

Each final program contains 719,327,424 stereo frames, or 14,985.988 seconds (4:09:45.988). All 575 transitions use the full ten-second crossfade.

| Approach | RF64 | FLAC | AAC/M4A | Opus 128k | Opus 32k |
|---|---:|---:|---:|---:|---:|
| Long additive synth | 2,877,309,776 | 839,531,731 | 362,479,961 | 243,088,082 | 61,768,847 |
| Short additive synth | 2,877,309,776 | 986,849,047 | 362,479,925 | 234,136,570 | 57,961,959 |

Sizes are bytes. Every compressed master was decoded end to end after encoding and passed codec, channel, sample-rate, and duration checks. Master hashes are recorded in `outputs/final/SHA256SUMS`.
