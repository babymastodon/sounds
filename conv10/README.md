# Convolutions 10

`conv10` turns lists of raw recordings into long-form convolution pieces. Each
48-input list has 24 twelve-second inputs and 24 thirty-second inputs, producing
a complete 24×24 matrix of 576 stereo pair renders. The pairs are sequenced with
ten-second crossfades into a 4:09:45.988 program.

The album contains 14 pieces:

| # | Name | Character |
|---:|---|---|
| 1 | Field Atlas | Water, animals, transport, public spaces, machinery, and collective sound |
| 2 | Melody Works | Instrumental gestures crossed with vehicles, tools, alarms, crowds, and weather |
| 3 | Drift | Calm water, weather, vegetation, and spacious natural environments |
| 4 | Menagerie | Birds, insects, farm animals, wildlife, and underwater voices |
| 5 | Passage | Rail, road, air, water, and human-powered motion |
| 6 | Foundry | Factories, workshops, tools, engines, and heavy mechanical processes |
| 7 | Commons | Markets, gatherings, institutions, celebrations, and shared public rooms |
| 8 | Sonora | Acoustic instruments, voices, tuned percussion, and improvisation |
| 9 | Signals | Radio, electrical systems, alarms, computers, machines, and synthetic tones |
| 10 | Tempest | Storms, fire, ice, impacts, explosions, and extreme natural force |
| 11 | Wildwire | Animal communication crossed with electronic signaling and machine language |
| 12 | Tideforge | Calm environmental flow crossed with sharp industrial force |
| 13 | Stormfolk | Extreme weather and impacts crossed with crowds, rituals, and public life |
| 14 | Railchime | Transport rhythms crossed with melodic instruments and tuned percussion |

Field Atlas and Melody Works are the renamed and regenerated original programs.
The other 12 pieces draw from 384 distinct online snippets spanning 192 topics.
The eight focused pieces each use a separate 48-source palette. The four hybrid
pieces each combine alternating topics from two palettes, creating semantic and
acoustic contrasts without introducing another source. In total, the 12 lists
contain 576 uses: 192 sources appear in one focused and one hybrid piece, and
192 appear once.

`SONGS.tsv` is the ordered album catalog and metadata record.
`SONG_SOURCES.tsv` is the 384-source inventory. Every piece has its own explicit
input definition in `lists/`.

## Run

Requirements: a current Rust toolchain, Python 3, curl, FFmpeg, and FFprobe.

Render the entire album using all configured workers:

```bash
cd conv10
CONV_JOBS=8 DOWNLOAD_JOBS=8 ./scripts/render_all.sh
```

Render selected pieces:

```bash
./scripts/render_all.sh lists/drift.txt lists/tideforge.txt
```

Force synthesis and final encoding to be regenerated:

```bash
./scripts/batch.py \
  --jobs 8 \
  --prepare-jobs 8 \
  --force-render \
  --force-output \
  lists/drift.txt
```

The final audio is grouped by format for easy copying:

```text
conv10/
├── cover.jpg
└── outputs/batch/
    ├── flac/
    │   ├── fieldatlas.flac
    │   └── ...
    ├── m4a/
    │   ├── fieldatlas.m4a
    │   └── ...
    └── opus/
        ├── fieldatlas.opus
        └── ...
```

FLAC is lossless, M4A contains AAC audio and embedded JPEG cover art, and Opus
is the third delivery format. The checked-in `cover.jpg` is also suitable for
copying beside the album files.
All 42 files identify the album as `Convolutions 10` and the artist, album
artist, and composer as `babymastodon`. Track title, number, disc, year, genre,
and a short piece description are embedded as well.

Audio, downloaded media, scratch matrices, and Rust build artifacts are ignored
by Git. Successful runs retain compact reports, timelines, recipes, and hash
files while cleaning raw, prepared, and matrix WAVs unless `--keep-work` is
supplied.

## Source curation

The source pool is deterministic and resumable:

```bash
python3 scripts/curate_songs.py discover
python3 scripts/curate_songs.py validate --jobs 8
python3 scripts/curate_songs.py write
python3 scripts/curate_songs.py seed
python3 scripts/curate_songs.py check
```

Discovery verifies source pages and prevents overlap with the existing
repository inventory. Validation captures each configured source prefix as
lossless audio, strictly decodes the selected window, checks usable duration,
and records content hashes. `write` produces the 12 new lists, album catalog,
and source inventory; `seed` reuses the validated cache for focused and hybrid
renders.

## Input-list format

Explicit lists are tab-separated:

```text
id	role	trim_start	source
short_one	short	0	/path/to/short.wav
long_one	long	auto	https://example.test/long.flac
```

A plain text file may instead contain one local path or HTTP(S) URL per
non-comment line. Its first ceiling-half becomes the short side and its
remainder becomes the long side; eventful trims are selected automatically.
Sources shorter than their target but still within the supported manifest range
are pitch-preserving time-stretched.

Useful checks:

```bash
./scripts/batch.py --validate-only lists/*.txt
cargo test --all-targets
python3 -m unittest scripts/test_batch.py scripts/test_curate_songs.py
python3 scripts/verify_album.py
```

The batch runner validates every pair, decodes every compressed output end to
end, checks embedded metadata, and writes SHA-256 files. See `RUN_REPORT.md` for
the completed album results and `PERFORMANCE.md` for the performance inventory.
