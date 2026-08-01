#!/usr/bin/env python3
"""Independently verify the complete Convolutions 10 delivery directory."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from batch import load_catalog, validate_embedded_cover, validate_embedded_metadata


PROJECT_DIR = Path(__file__).resolve().parents[1]
EXPECTED_DURATION_SECONDS = 14985.988
EXPECTED_SAMPLE_RATE = 48_000
EXPECTED_CHANNELS = 2
FORMATS = {
    "flac": ("flac", "flac"),
    "m4a": ("m4a", "aac"),
    "opus": ("opus", "opus"),
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def probe(path: Path) -> dict:
    return json.loads(
        subprocess.check_output(
            [
                "ffprobe",
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_name,sample_rate,channels",
                "-select_streams",
                "a:0",
                "-of",
                "json",
                str(path),
            ],
            text=True,
        )
    )


def verify_hashes(output_dir: Path, name: str) -> None:
    hash_path = output_dir / f"{name}.sha256"
    expected = {}
    for line in hash_path.read_text(encoding="utf-8").splitlines():
        digest, relative = line.split("  ", maxsplit=1)
        expected[relative] = digest
    wanted = {
        f"{directory}/{name}.{extension}"
        for directory, (extension, _codec) in FORMATS.items()
    }
    if set(expected) != wanted:
        raise ValueError(f"{hash_path}: unexpected hash entries")
    for relative, digest in expected.items():
        path = output_dir / relative
        if sha256(path) != digest:
            raise ValueError(f"{path}: SHA-256 mismatch")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--catalog",
        type=Path,
        default=PROJECT_DIR / "SONGS.tsv",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=PROJECT_DIR / "outputs" / "batch",
    )
    args = parser.parse_args()
    catalog = load_catalog(args.catalog.resolve())
    output_dir = args.output_dir.resolve()
    expected_names = set(catalog)

    root_audio = [
        path
        for path in output_dir.iterdir()
        if path.is_file() and path.suffix.lower() in {".flac", ".m4a", ".opus"}
    ]
    if root_audio:
        raise ValueError(
            "audio files must be grouped by extension: "
            + ", ".join(path.name for path in root_audio)
        )

    for directory, (extension, codec) in FORMATS.items():
        actual_names = {
            path.stem for path in (output_dir / directory).glob(f"*.{extension}")
        }
        if actual_names != expected_names:
            missing = sorted(expected_names - actual_names)
            extra = sorted(actual_names - expected_names)
            raise ValueError(
                f"{directory}: missing={missing or 'none'}, extra={extra or 'none'}"
            )

    for name, metadata in catalog.items():
        for directory, (extension, codec) in FORMATS.items():
            path = output_dir / directory / f"{name}.{extension}"
            validate_embedded_metadata(path, metadata)
            validate_embedded_cover(path)
            payload = probe(path)
            stream = payload["streams"][0]
            duration = float(payload["format"]["duration"])
            if stream["codec_name"] != codec:
                raise ValueError(f"{path}: expected {codec}, got {stream['codec_name']}")
            if int(stream["sample_rate"]) != EXPECTED_SAMPLE_RATE:
                raise ValueError(f"{path}: unexpected sample rate")
            if int(stream["channels"]) != EXPECTED_CHANNELS:
                raise ValueError(f"{path}: unexpected channel count")
            if abs(duration - EXPECTED_DURATION_SECONDS) > 0.1:
                raise ValueError(f"{path}: unexpected duration {duration:.6f}s")
        verify_hashes(output_dir, name)
        print(
            f"verified {name}: FLAC, AAC/M4A, Opus, metadata, cover art, hashes"
        )

    print(f"verified album: {len(catalog)} tracks, {len(catalog) * 3} audio files")


if __name__ == "__main__":
    main()
