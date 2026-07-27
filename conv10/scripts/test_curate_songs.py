#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import curate_songs


class CurateSongsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.targets = curate_songs.targets()
        self.state = {
            row["key"]: {
                **row,
                "download_url": f"https://example.test/{row['id']}",
            }
            for row in self.targets
        }

    def test_pool_defines_384_unique_sources(self) -> None:
        self.assertEqual(len(self.targets), 384)
        self.assertEqual(len({row["id"] for row in self.targets}), 384)
        self.assertEqual(len({row["key"] for row in self.targets}), 384)

    def test_each_new_song_has_24_short_and_24_long_inputs(self) -> None:
        for song in curate_songs.all_songs():
            with self.subTest(song=song):
                rows = curate_songs.song_rows(song, self.state)
                self.assertEqual(len(rows), 48)
                self.assertEqual(sum(row["role"] == "short" for row in rows), 24)
                self.assertEqual(sum(row["role"] == "long" for row in rows), 24)

    def test_hybrid_overlap_uses_192_sources_twice(self) -> None:
        uses = {row["key"]: 0 for row in self.targets}
        for song in curate_songs.all_songs():
            for row in curate_songs.song_rows(song, self.state):
                uses[row["key"]] += 1
        self.assertEqual(sum(uses.values()), 576)
        self.assertEqual(sum(count == 2 for count in uses.values()), 192)
        self.assertEqual(sum(count == 1 for count in uses.values()), 192)

    def test_album_catalog_is_complete_and_ordered(self) -> None:
        rows = curate_songs.catalog_rows()
        self.assertEqual(len(rows), 14)
        self.assertEqual(rows[0]["name"], "fieldatlas")
        self.assertEqual(rows[1]["name"], "melodyworks")
        self.assertEqual(rows[-1]["name"], "railchime")
        self.assertEqual({row["album"] for row in rows}, {"Convolutions 10"})
        self.assertEqual({row["artist"] for row in rows}, {"babymastodon"})
        self.assertEqual(
            [row["track"] for row in rows],
            [f"{index}/14" for index in range(1, 15)],
        )


if __name__ == "__main__":
    unittest.main()
