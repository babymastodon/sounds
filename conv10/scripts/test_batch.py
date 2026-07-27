#!/usr/bin/env python3
"""Tests for the config-driven batch front end."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
import json
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from batch import (
    SongDefinition,
    SongRun,
    PROJECT_DIR,
    default_metadata,
    encode_one,
    encoder_available,
    load_catalog,
    parse_list,
    parse_song_config,
    validate_embedded_cover,
    validate_embedded_metadata,
)


class ParseListTests(unittest.TestCase):
    def test_plain_paths_split_into_short_and_long_halves(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            list_path = root / "plain.txt"
            list_path.write_text(
                "alpha.wav\nbeta.wav\ngamma.wav\ndelta.wav\n", encoding="utf-8"
            )

            entries = parse_list(list_path)

            self.assertEqual([entry.role for entry in entries], ["short", "short", "long", "long"])
            self.assertEqual(
                [entry.identifier for entry in entries],
                ["alpha", "beta", "gamma", "delta"],
            )
            self.assertTrue(all(entry.trim_start is None for entry in entries))
            self.assertTrue(all(Path(entry.source).is_absolute() for entry in entries))

    def test_explicit_rows_preserve_roles_offsets_and_sources(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            list_path = root / "explicit.txt"
            list_path.write_text(
                "id\trole\ttrim_start\tsource\n"
                "one\tshort\t1.25\tone.wav\n"
                "two\tlong\tauto\thttps://example.test/two.ogg\n",
                encoding="utf-8",
            )

            entries = parse_list(list_path)

            self.assertEqual(entries[0].trim_start, 1.25)
            self.assertIsNone(entries[1].trim_start)
            self.assertEqual(entries[1].source, "https://example.test/two.ogg")

    def test_rejects_a_list_without_both_roles(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            list_path = Path(temporary) / "bad.txt"
            list_path.write_text(
                "id\trole\ttrim_start\tsource\n"
                "one\tshort\t0\tone.wav\n"
                "two\tshort\t0\ttwo.wav\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "long input"):
                parse_list(list_path)


class ParseSongConfigTests(unittest.TestCase):
    def test_checked_in_configs_are_complete_album_definitions(self) -> None:
        paths = sorted((PROJECT_DIR / "configs").glob("*.json"))
        songs = [parse_song_config(path) for path in paths]
        catalog = load_catalog(PROJECT_DIR / "SONGS.tsv")

        self.assertEqual(len(songs), 14)
        self.assertEqual(len({song.name for song in songs}), 14)
        self.assertTrue(all(len(song.entries) == 48 for song in songs))
        self.assertTrue(
            all(
                sum(entry.role == "short" for entry in song.entries) == 24
                and sum(entry.role == "long" for entry in song.entries) == 24
                for song in songs
            )
        )
        self.assertEqual(
            {song.metadata["album"] for song in songs}, {"Convolutions 10"}
        )
        self.assertEqual(
            {song.metadata["artist"] for song in songs}, {"babymastodon"}
        )
        for song in songs:
            with self.subTest(song=song.name):
                self.assertEqual(song.metadata, catalog[song.name])
                self.assertEqual(
                    song.entries,
                    tuple(parse_list(PROJECT_DIR / "lists" / f"{song.name}.txt")),
                )
                payload = json.loads(song.config_path.read_text(encoding="utf-8"))
                scenes = {
                    scene["name"]: scene
                    for scene in payload["harmony"]["scenes"]
                }
                progression = payload["harmony"]["progression"]
                pair_counts = [step["pair_count"] for step in progression]
                self.assertEqual(sum(pair_counts), 24 * 24)
                self.assertGreaterEqual(len(set(pair_counts)), 3)
                self.assertTrue(all(24 <= count <= 72 for count in pair_counts))
                self.assertTrue(
                    all(
                        step["pair_count"]
                        % scenes[step["scene"]]["motif_every_pairs"]
                        == 0
                        for step in progression
                    )
                )
                self.assertIn(
                    "/".join(str(count) for count in pair_counts),
                    song.metadata["description"],
                )


class MetadataTests(unittest.TestCase):
    def test_catalog_preserves_complete_track_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            catalog_path = Path(temporary) / "songs.tsv"
            catalog_path.write_text(
                "name\ttitle\talbum\tartist\talbum_artist\tcomposer\tgenre\tdate"
                "\ttrack\tdisc\tcomment\tdescription\n"
                "drift\tDrift\tConvolutions 10\tbabymastodon\tbabymastodon"
                "\tbabymastodon\tExperimental\t2026\t3/14\t1/1"
                "\tCalm environmental flow.\tCalm environmental flow.\n",
                encoding="utf-8",
            )

            catalog = load_catalog(catalog_path)

            self.assertEqual(catalog["drift"]["album"], "Convolutions 10")
            self.assertEqual(catalog["drift"]["track"], "3/14")
            self.assertEqual(catalog["drift"]["artist"], "babymastodon")

    def test_default_metadata_is_complete(self) -> None:
        metadata = default_metadata("new_song")

        self.assertEqual(metadata["title"], "New Song")
        self.assertEqual(metadata["album"], "Convolutions 10")
        self.assertTrue(all(metadata.values()))

    def test_embedded_jpeg_cover_is_required(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            plain_path = root / "plain.m4a"
            covered_path = root / "covered.m4a"
            subprocess.run(
                [
                    "ffmpeg",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=48000:cl=stereo",
                    "-t",
                    "0.1",
                    "-c:a",
                    "aac",
                    str(plain_path),
                ],
                check=True,
            )
            subprocess.run(
                [
                    "ffmpeg",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=48000:cl=stereo",
                    "-i",
                    str(PROJECT_DIR / "cover.jpg"),
                    "-t",
                    "0.1",
                    "-map",
                    "0:a:0",
                    "-map",
                    "1:v:0",
                    "-c:a",
                    "aac",
                    "-c:v",
                    "copy",
                    "-disposition:v:0",
                    "attached_pic",
                    str(covered_path),
                ],
                check=True,
            )

            with self.assertRaisesRegex(ValueError, "missing embedded cover"):
                validate_embedded_cover(plain_path)
            validate_embedded_cover(covered_path)

    def test_parallel_queue_encoder_preserves_metadata_and_cover(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            master = root / "work" / "concat" / "test.part.rf64.wav"
            master.parent.mkdir(parents=True)
            subprocess.run(
                [
                    "ffmpeg",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=48000:cl=stereo",
                    "-t",
                    "0.1",
                    "-c:a",
                    "pcm_s16le",
                    str(master),
                ],
                check=True,
            )
            metadata = {
                **default_metadata("test"),
                "comment": "Themes: rain. Tuning: 15-EDO. Form: A-B-A.",
                "description": "Themes: rain. Tuning: 15-EDO. Form: A-B-A.",
            }
            config_path = root / "test.json"
            config_path.write_text("{}\n", encoding="utf-8")
            song = SongDefinition(config_path, "test", (), metadata)
            run = SongRun(
                song,
                0.0,
                root / "work",
                root / "manifest.tsv",
                1,
                1,
                0.0,
                0.0,
                0.0,
                {},
                {},
            )
            aac_encoder = (
                "libfdk_aac" if encoder_available("libfdk_aac") else "aac"
            )
            opus_encoder = "libopus" if encoder_available("libopus") else "opus"
            output_dir = root / "outputs"
            for kind in ("flac", "aac", "opus"):
                encode_one(
                    run,
                    kind,
                    output_dir,
                    aac_encoder,
                    opus_encoder,
                    192,
                    128,
                )
            for path in (
                output_dir / "flac" / "test.flac",
                output_dir / "m4a" / "test.m4a",
                output_dir / "opus" / "test.opus",
            ):
                validate_embedded_metadata(path, metadata)
            validate_embedded_cover(output_dir / "m4a" / "test.m4a")


if __name__ == "__main__":
    unittest.main()
