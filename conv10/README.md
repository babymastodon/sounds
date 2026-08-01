# Convolutions 10

`conv10` turns config-defined sets of raw recordings into long-form convolution
pieces. Each song config has 24 twelve-second inputs and 24 thirty-second
inputs, producing a complete 24×24 matrix of 576 stereo pair renders. The pairs
follow the config's scene progression and are joined with ten-second crossfades
into a 4:09:45.988 program.

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
acoustic contrasts without introducing another source. In total, the 12 pieces
contain 576 uses: 192 sources appear in one focused and one hybrid piece, and
192 appear once.

Every piece has one authoritative definition in `configs/`. Each JSON file
contains all 48 ordered samples, complete output metadata, frequency boundaries,
one or more tuning systems, weighted chord palettes, scene behavior, and the
full 576-pair progression. Concrete roots and inversions are generated
deterministically from the ordered sample filenames, so the configuration sets
the musical boundaries while preserving a reproducible realization. See
[`configs/README.md`](configs/README.md) for the album plan and field reference.
Scene spans deliberately vary from 24 to 72 pairs, always end on complete motif
cycles, and still total the exhaustive 576-pair matrix.

`SONGS.tsv` remains the ordered curation catalog and `SONG_SOURCES.tsv` is the
384-source inventory. Files in `lists/` are retained as curation intermediates;
the batch renderer reads only the JSON song configs.

## Run

Requirements: a current Rust toolchain, Python 3, curl, FFmpeg, and FFprobe.

Render the entire album using all configured workers:

```bash
cd conv10
CONV_JOBS=8 DOWNLOAD_JOBS=8 ./scripts/render_all.sh
```

Render selected pieces:

```bash
./scripts/render_all.sh configs/drift.json configs/tideforge.json
```

Force synthesis to be regenerated:

```bash
./scripts/batch.py \
  --jobs 8 \
  --prepare-jobs 8 \
  --force-render \
  configs/drift.json
```

For an album run, all matrices are rendered first, then all RF64 PCM masters
are assembled. Only after every master exists does one global queue launch
whole-file FFmpeg encoders. Preparation, rendering, RF64 assembly, encoding,
and finalization all use at least eight workers by default. This fills the
machine with independent work
because the selected FLAC, AAC, and Opus encoders do not internally parallelize
one audio stream. `render_all.sh` clamps its per-phase environment overrides to
that eight-worker floor.

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

FLAC is lossless, M4A contains AAC audio, and Opus is the third delivery
format. Every delivery file embeds the checked-in JPEG cover art; `cover.jpg`
is also suitable for copying beside the album files.
All 42 files identify the album as `Convolutions 10` and the artist, album
artist, and composer as `babymastodon`. Track title, number, disc, year, genre,
and an extended description of the sample themes, tuning, chord vocabulary,
main motif, form, and pair-count spans are embedded as well. M4A exposes this
as `DESCRIPTION`; FLAC and Opus carry the equivalent Vorbis `COMMENT`.

Audio, downloaded media, scratch matrices, assembled RF64 masters, and Rust
build artifacts are ignored by Git. Successful runs retain compact reports,
timelines, recipes, and hash files while cleaning large scratch audio unless
`--keep-work` is supplied. If a later stage fails, its assembled master remains
available for diagnosis or a resumed encoding run.

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
and records content hashes. `write` produces the 12 curation lists, album
catalog, and source inventory, then synchronizes their sample and metadata
fields into the existing JSON files without changing the reviewed harmony
plans. `seed` reuses the validated cache for focused and hybrid renders.

## Song-config format

The complete schema is documented in
[`configs/README.md`](configs/README.md). A shortened structural example is:

```json
{
  "schema_version": 1,
  "name": "drift",
  "metadata": { "title": "Drift", "album": "Convolutions 10" },
  "samples": [
    {
      "id": "drift_gentle_ocean_s",
      "role": "short",
      "trim_start": 0,
      "source": "https://example.test/ocean.flac"
    }
  ],
  "harmony": {
    "register": { "minimum_hz": 55, "maximum_hz": 440 },
    "allowed_inversions": [0, 1, 2],
    "tunings": [],
    "palettes": [],
    "scenes": [],
    "progression": []
  }
}
```

Sources may be local paths or HTTP(S) URLs. An `auto` trim asks the preparation
stage to select an eventful window. Sources shorter than their target but still
within the supported manifest range are pitch-preserving time-stretched.

Useful checks:

```bash
./scripts/batch.py --validate-only configs/*.json
cargo test --all-targets --offline
python3 -m unittest \
  scripts/test_audit_licenses.py \
  scripts/test_batch.py \
  scripts/test_curate_songs.py
python3 scripts/verify_album.py
```

Add or repair exact embedded cover art in an existing delivery without
re-encoding its audio:

```bash
python3 scripts/embed_cover_art.py
```

The batch runner validates every pair, decodes every compressed output end to
end, checks embedded metadata and exact cover art in all three formats, and
writes SHA-256 files.
`RUN_REPORT.md` records the completed v17 delivery. See `PROGRESS.md` for
current status and `PERFORMANCE.md` for the performance inventory and
historical renderer baseline.
