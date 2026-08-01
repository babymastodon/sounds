#!/usr/bin/env python3
"""Atomically add the album cover to existing FLAC and Opus masters."""

from __future__ import annotations

import argparse
import concurrent.futures
import os
import subprocess
import threading
from pathlib import Path

from batch import (
    PROJECT_DIR,
    cover_picture_metadata,
    ffmetadata_escape,
    file_sha256,
    load_catalog,
    metadata_arguments,
    validate_embedded_cover,
    validate_embedded_metadata,
)


def remux_cover(path: Path, metadata: dict[str, str], cover: Path) -> str:
    try:
        validate_embedded_cover(path, cover)
        validate_embedded_metadata(path, metadata)
        return f"already covered: {path}"
    except ValueError:
        pass

    temporary = path.with_name(
        f".{path.stem}.cover.{os.getpid()}.{threading.get_ident()}{path.suffix}"
    )
    sidecar = temporary.with_suffix(".picture.ffmetadata")
    temporary.unlink(missing_ok=True)
    sidecar.unlink(missing_ok=True)
    command = [
        "ffmpeg",
        "-xerror",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        str(path),
    ]
    if path.suffix.lower() == ".flac":
        command.extend(
            [
                "-i",
                str(cover),
                "-map",
                "0:a:0",
                "-map",
                "1:v:0",
                "-map_metadata",
                "0",
                "-c:a",
                "copy",
                "-c:v",
                "copy",
                "-disposition:v:0",
                "attached_pic",
                "-metadata:s:v:0",
                "title=Album cover",
                "-metadata:s:v:0",
                "comment=Cover (front)",
            ]
        )
    elif path.suffix.lower() == ".opus":
        sidecar.write_text(
            ";FFMETADATA1\nMETADATA_BLOCK_PICTURE="
            + ffmetadata_escape(cover_picture_metadata(cover))
            + "\n",
            encoding="utf-8",
        )
        command.extend(
            [
                "-f",
                "ffmetadata",
                "-i",
                str(sidecar),
                "-map",
                "0:a:0",
                "-map_metadata",
                "1",
                "-c:a",
                "copy",
                *metadata_arguments(metadata, include_description=False),
            ]
        )
    else:
        raise ValueError(f"unsupported cover-art target: {path}")
    command.append(str(temporary))

    try:
        subprocess.run(command, cwd=PROJECT_DIR, check=True)
        validate_embedded_metadata(temporary, metadata)
        validate_embedded_cover(temporary, cover)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)
        sidecar.unlink(missing_ok=True)
    return f"embedded cover: {path}"


def rewrite_hashes(output_dir: Path, name: str) -> None:
    paths = [
        output_dir / "flac" / f"{name}.flac",
        output_dir / "m4a" / f"{name}.m4a",
        output_dir / "opus" / f"{name}.opus",
    ]
    hash_path = output_dir / f"{name}.sha256"
    hash_path.write_text(
        "".join(
            f"{file_sha256(path)}  {path.relative_to(output_dir)}\n" for path in paths
        ),
        encoding="utf-8",
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--catalog", type=Path, default=PROJECT_DIR / "SONGS.tsv"
    )
    parser.add_argument(
        "--output-dir", type=Path, default=PROJECT_DIR / "outputs" / "batch"
    )
    parser.add_argument("--cover", type=Path, default=PROJECT_DIR / "cover.jpg")
    parser.add_argument("--jobs", type=int, default=min(os.cpu_count() or 1, 4))
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be at least 1")

    catalog = load_catalog(args.catalog.resolve())
    output_dir = args.output_dir.resolve()
    cover = args.cover.resolve()
    work = [
        (output_dir / directory / f"{name}.{extension}", metadata)
        for name, metadata in catalog.items()
        for directory, extension in (("flac", "flac"), ("opus", "opus"))
    ]
    missing = [str(path) for path, _metadata in work if not path.is_file()]
    if missing:
        raise FileNotFoundError("missing delivery files: " + ", ".join(missing))

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = [
            executor.submit(remux_cover, path, metadata, cover)
            for path, metadata in work
        ]
        for future in concurrent.futures.as_completed(futures):
            print(future.result())

    for name, metadata in catalog.items():
        for directory, extension in (
            ("flac", "flac"),
            ("m4a", "m4a"),
            ("opus", "opus"),
        ):
            path = output_dir / directory / f"{name}.{extension}"
            validate_embedded_metadata(path, metadata)
            validate_embedded_cover(path, cover)
        rewrite_hashes(output_dir, name)
    print(f"verified exact cover art and rewrote hashes for {len(catalog)} tracks")


if __name__ == "__main__":
    main()
