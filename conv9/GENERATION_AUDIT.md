# Generation audit

Completed 2026-07-25 on Fedora 44 with `conv9` release mode and four Rayon workers.

## Corpus

- 12 manifest entries, all exactly 2,880,000 frames (60.000 s), mono, 48 kHz float PCM after preparation
- 12 distinct source/download URLs
- 8 source-kind labels spanning musical, rhythmic, ambient, speech, wildlife, transit, and other activities
- all licenses restricted by the manifest validator to CC0 1.0, CC BY 4.0, or CC BY-SA 3.0
- raw and prepared SHA-256 manifests generated under the ignored `samples/` tree

## Matrix

- 66 / 66 unordered pairs
- 4 / 4 algorithms
- 3 / 3 window presets
- 792 / 792 WAVs
- every file exactly 5,760,044 bytes: a 44-byte WAV header plus 2,880,000 mono PCM16 samples
- 66 files in every algorithm/preset leaf directory
- total output tree: 4.3 GiB (`du -sh`)
- release render wall time: 1:03.83
- peak renderer resident memory: 351,160 KiB

## Exhaustive signal verification

- every output: mono, 48,000 Hz, signed PCM16
- every output: exactly 2,880,000 frames / 60.000 seconds
- RMS range: −23.721 through −20.553 dBFS
- maximum peak: 0.919983
- clipped samples: 0
- non-finite samples: 0
- verification wall time: 11.24 seconds
- verification peak resident memory: 185,432 KiB

Generated index checksums:

```text
310e5f3d23b444d9636432acd0ad1ca5fffcdfef8a5d37c3a583e89a4ba7026a  outputs/catalog.json
760313b3552ef68a0fa49b08663e373f396e95a300a7e8067b9b411a064e7cb0  outputs/metrics.csv
```

The output tree is intentionally ignored by Git and remains locally available. `conv9 verify` regenerates both indexes deterministically from the audio files and fails for any missing, extra, malformed, silent, non-finite, or clipped WAV.

