# Song configuration

Each JSON file is the complete render definition for one piece. It contains the
48 ordered source samples, delivery metadata, tuning systems, chord palettes,
scene behavior, and the scene progression. The renderer no longer has a global
scale or chord table.

## Album plan

| Piece | Primary tuning | Additional quoted tunings | Palettes | Chords | Form |
|---|---|---|---:|---:|---|
| [Field Atlas](fieldatlas.json) | 13 equal divisions of 2:1 | — | 4 | 16 | long environmental returns |
| [Melody Works](melodyworks.json) | 19 equal divisions of 2:1 | — | 4 | 16 | extended motif variations |
| [Drift](drift.json) | 14 just-ratio degrees in 2:1 | — | 4 | 16 | long main-motif returns |
| [Menagerie](menagerie.json) | 17 equal divisions of 2:1 | — | 4 | 16 | long organic returns |
| [Passage](passage.json) | 22 equal divisions of 2:1 | — | 4 | 16 | extended moving scenes |
| [Foundry](foundry.json) | 13 equal divisions of 3:1 | — | 4 | 16 | extended mechanical contrasts |
| [Commons](commons.json) | 15 equal divisions of 2:1 | — | 4 | 16 | long collective cadence |
| [Sonora](sonora.json) | 31 equal divisions of 2:1 | — | 4 | 16 | extended musical variations |
| [Signals](signals.json) | 11 equal divisions of 2:1 | — | 4 | 16 | fragmented returns and long contrasts |
| [Tempest](tempest.json) | 7 equal divisions of 2:1 | — | 4 | 16 | immediate contrast |
| [Wildwire](wildwire.json) | 16 equal divisions of 2:1 | 17-EDO and 11-EDO | 5 | 20 | hybrid quotation |
| [Tideforge](tideforge.json) | 24 equal divisions of 2:1 | just ratios and 13-ED3 | 5 | 20 | hybrid quotation |
| [Stormfolk](stormfolk.json) | 9 equal divisions of 2:1 | 7-EDO and 15-EDO | 5 | 20 | hybrid quotation |
| [Railchime](railchime.json) | 14 equal divisions of 2:1 | 22-EDO and 31-EDO | 5 | 20 | hybrid quotation |

The focused pieces use six named scenes: `a`, `a_prime`, `b`, `c`, `d`, and
`coda`. Hybrid pieces add `e`, where the parent tunings can be quoted more
directly. Each progression contains 576 pair positions, matching the complete
24×24 short/long convolution matrix. Its `pair_count` spans vary from 24 to 72
pairs, use at least three durations in every piece, and are divisible by the
selected scene's `motif_every_pairs`; scene boundaries therefore never truncate
a motif cycle. No tuning in the album uses 12 equal divisions.

## Selection model

The configuration defines musical boundaries, not a literal note-by-note
score. Before any parallel rendering starts, the renderer walks the configured
progression and creates a deterministic harmony schedule for all 576 pairs.

For each position:

1. `progression` supplies the scene and its duration in pair renders.
2. The scene's repeating `motif` may force a named chord. Deterministic free
   slots guarantee that every configured low-weight chord shape is heard.
3. Otherwise `palette_weights` select a palette and the chord's `weight`
   selects a shape.
4. A root is selected from the palette's `root_pool`.
5. An inversion is selected from `allowed_inversions`.
6. Chord degrees are realized in the palette's named tuning, shifted by whole
   tuning periods until every frequency fits `register`.

The selections use domain-separated FNV-1a hashes over the song name, scene
occurrence, position, and ordered short/long sample filenames. Identical
configs and filenames therefore reproduce identical notes, while renaming or
reordering a sample produces a new realization. The computed
`sequence_index` is stored in every matrix metrics row, so concatenation
follows the configured scene order even when filenames are not alphabetical.

## Field reference

Top-level fields:

- `schema_version`: currently `1`.
- `name`: lowercase output basename and scratch-directory name.
- `metadata`: complete tags used for FLAC, M4A/AAC, and Opus delivery files,
  including a description of themes, tunings, chord vocabulary, and form.
- `samples`: ordered input objects with `id`, `role`, `trim_start`, and
  `source`. Roles must include 24 `short` and 24 `long` entries.
- `harmony`: the musical plan.

Harmony fields:

- `register.minimum_hz` and `register.maximum_hz`: absolute frequency
  boundaries for every realized chord.
- `allowed_inversions`: a non-empty subset of `0`, `1`, and `2`.
- `tunings`: one or more named tuning definitions.
- `palettes`: named root pools and weighted three-note chord shapes.
- `scenes`: named palette mixtures plus a repeating motif.
- `progression`: ordered scene spans whose positive `pair_count` values total
  576. Checked-in album spans are 24–72 pairs and contain whole motif cycles.

An `equal_division` tuning requires `divisions` and divides `period_ratio`
evenly. A `ratios` tuning requires an increasing `ratios` array beginning at
`1/1`; ratios may be JSON numbers or readable fractions. The period endpoint
is not repeated in that array. `base_frequency_hz` anchors degree zero.
`detune_limit_fraction` limits synthesized drift to a fraction of the tuning's
smallest adjacent step.

Palette chord `degrees` are scale indices relative to the selected root. They
must contain three ascending values beginning at zero. Values may cross the
tuning period, which makes open voicings and period-spanning chords possible.
Chord names are unique across the song so scenes can reference them directly.

Scene `palette_weights` and chord `weight` values are relative integer weights.
For example, weights `6`, `3`, and `1` mean 60%, 30%, and 10% before motif
positions are applied. During each `motif_every_pairs` block, the first
`motif.length` positions use the listed chord names in order.

## Review and validation

Validate the complete checked-in set without downloading or rendering audio:

```bash
./scripts/batch.py --validate-only configs/*.json
cargo test --all-targets --offline
```

The Rust test suite loads every checked-in config, validates every configured
root/chord/inversion combination against its register, constructs all 576
filename-derived assignments, and checks the resulting frequencies.
