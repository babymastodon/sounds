# Convolutions 10 completed run

Completed: 2026-07-26

## Results

All 14 album tracks completed the same verified pipeline:

- 48 prepared inputs: 24 short and 24 long
- 576 unique stereo convolution pairs
- 575 full ten-second transitions
- 719,327,424 stereo frames at 48 kHz
- 14,985.988 seconds (`4:09:45.988`) per master
- FLAC, AAC/M4A, and Opus output

The album therefore contains 8,064 verified pair renders and 58.279 hours of
unique program duration. Across every matrix, measured RMS was
−21.746..−20.100 dBFS, maximum peak was 0.890, and stereo-difference RMS was
−23.192..−15.440 dBFS. No pair clipped, contained non-finite samples, or failed
the stereo checks.

| Track | Prepare | Render | Avg render CPU | Verify | Assemble/encode/check | Total |
|---|---:|---:|---:|---:|---:|---:|
| Field Atlas | 8.83 s | 28.02 s | 6.58 cores | 6.10 s | 178.95 s | 222.54 s |
| Melody Works | 3.49 s | 34.02 s | 6.38 cores | 11.48 s | 188.64 s | 238.27 s |
| Drift | 3.15 s | 32.02 s | 7.46 cores | 7.95 s | 169.45 s | 213.37 s |
| Menagerie | 3.17 s | 33.02 s | 7.34 cores | 7.31 s | 176.44 s | 220.67 s |
| Passage | 3.19 s | 34.03 s | 7.01 cores | 7.92 s | 181.77 s | 227.57 s |
| Foundry | 3.40 s | 37.03 s | 6.72 cores | 8.02 s | 180.61 s | 229.87 s |
| Commons | 3.33 s | 35.05 s | 6.96 cores | 7.75 s | 175.87 s | 222.70 s |
| Sonora | 3.09 s | 33.03 s | 7.35 cores | 7.30 s | 183.53 s | 227.58 s |
| Signals | 3.17 s | 33.02 s | 7.46 cores | 7.69 s | 174.28 s | 218.86 s |
| Tempest | 3.08 s | 33.02 s | 7.43 cores | 7.39 s | 172.27 s | 216.46 s |
| Wildwire | 3.12 s | 33.02 s | 7.48 cores | 7.36 s | 176.30 s | 220.56 s |
| Tideforge | 3.45 s | 35.04 s | 6.95 cores | 8.08 s | 179.08 s | 226.51 s |
| Stormfolk | 3.46 s | 35.03 s | 7.01 cores | 8.79 s | 190.52 s | 238.53 s |
| Railchime | 3.65 s | 37.07 s | 6.61 cores | 10.27 s | 195.91 s | 247.58 s |

Mean render time was 33.74 seconds and mean render CPU was 7.05 of 8 cores.
The complete sequential album build took 3,171.08 seconds (`52:51.08`),
including source preparation, matrix verification, encoding, compressed
end-to-end decoding, metadata checks, and hashing.

## Final delivery

| Directory | Files | Total bytes | Smallest | Largest |
|---|---:|---:|---:|---:|
| `outputs/batch/flac/` | 14 | 15,374,102,742 | 860,574,100 | 1,349,444,711 |
| `outputs/batch/m4a/` | 14 | 5,074,723,778 | 362,480,077 | 362,480,476 |
| `outputs/batch/opus/` | 14 | 3,275,968,347 | 221,641,466 | 265,369,826 |

The independent album audit confirmed exactly 14 files in each format
directory, the expected codec, two channels, 48 kHz sample rate, matching
duration, complete embedded metadata, and every recorded SHA-256 digest.
Obsolete root-level `conv8.*` and `conv10.*` generated outputs were removed.

All 42 masters identify the album as `Convolutions 10`; artist, album artist,
and composer are `babymastodon`. Titles, track numbers `1/14` through `14/14`,
disc, year, genre, and per-track descriptions also passed embedded-tag
verification.

## Source/list audit

The 12 new tracks use 384 distinct source pages and 384 distinct media URLs,
divided into eight 48-source palettes. There are 192 short and 192 long source
windows. The four hybrid tracks reuse 192 sources once, for 576 total list uses;
the other 192 sources appear once. Every source and page record has a content
hash in `SONG_SOURCES.tsv`.

Generated audio remains untracked. Git contains the album catalog, lists,
inventories, curation/build code, tests, and reports only.
