# Convolutions 10 completed run

Completed: 2026-07-27

## Results

All 14 config-driven v17 pieces completed the verified album pipeline:

- 48 prepared inputs per piece: 24 short and 24 long
- 576 unique stereo convolution pairs per piece
- 575 full ten-second transitions per piece
- 719,327,424 stereo frames at 48 kHz per master
- 14,985.988 seconds (`4:09:45.988`) per master
- FLAC, AAC/M4A, and Opus delivery masters

The album therefore contains 8,064 verified pair renders and 58.279 hours of
unique program duration. Every config uses one or more non-12 tuning systems,
16 or 20 named chord shapes, filename-seeded roots/voicings/inversions, and at
least three different scene lengths. Each progression totals exactly 576 pairs
and every scene boundary falls on a complete motif cycle.

The final resumable pass used 64 preparation workers, eight render workers,
eight RF64 assembly workers, eight global FFmpeg jobs, and eight finalization
workers. It resumed from validated prepared inputs and v17 matrices, rebuilt
all 14 RF64 masters, encoded all 42 delivery files, decoded every file end to
end, checked the metadata and embedded artwork, and rewrote the hashes. The
pass completed in 1,435.466 seconds (`23:55.466`). The global eight-job encode
queue completed all 42 whole-file FFmpeg jobs in 822.546 seconds.

Per-song timing files are retained as `outputs/batch/<name>.run.json`. Their
one-second render measurements describe cache validation on this resumed pass,
not the original matrix-generation utilization, and must not be interpreted as
a synthesis scaling benchmark.

## Final delivery

| Directory | Files | Total bytes | Smallest | Largest |
|---|---:|---:|---:|---:|
| `outputs/batch/flac/` | 14 | 15,822,800,198 | 888,940,439 | 1,384,221,577 |
| `outputs/batch/m4a/` | 14 | 5,080,957,991 | 362,925,399 | 362,925,719 |
| `outputs/batch/opus/` | 14 | 3,299,610,675 | 222,139,426 | 267,589,817 |

The independent album audit confirmed exactly 14 files in each format
directory, the expected codecs, two channels, 48 kHz sample rate, matching
duration, complete embedded metadata, embedded JPEG art in every M4A, and every
recorded SHA-256 digest. Each Rust finalizer also independently decoded its
three compressed masters end to end before removing the RF64 scratch file.

All 42 masters identify the album as `Convolutions 10`; artist, album artist,
and composer are `babymastodon`. Titles, track numbers `1/14` through `14/14`,
disc, year, genre, and per-track descriptions passed embedded-tag verification.
The descriptions record the sample themes, tuning systems, chord vocabulary,
motif, form, and exact scene pair spans without adding source-administration
text to the listener-facing tags.

## Source and config audit

The 12 new tracks use 384 distinct source pages and 384 distinct media URLs,
divided into eight 48-source palettes. There are 192 short and 192 long source
windows. The four hybrid tracks reuse 192 sources once, for 576 total list uses;
the other 192 sources appear once. Every source and page record has a content
hash in `SONG_SOURCES.tsv`.

The checked inventories contain 384 accepted records for the new source pool,
48 accepted records for Field Atlas, and 48 accepted records for Melody Works,
with no attribution-required or restricted records. The two original lists and
the 12 new configs therefore need no listener-facing attribution.

Generated and downloaded audio remains untracked. Git contains the album
catalog, configs, lists, inventories, curation/build code, tests, artwork, and
reports only.
