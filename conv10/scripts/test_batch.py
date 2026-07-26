#!/usr/bin/env python3
"""Tests for the list-driven batch front end."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from batch import parse_list


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


if __name__ == "__main__":
    unittest.main()
