#!/usr/bin/env python3
"""Rank 61-second source cuts by foreground activity and timbral change."""

from __future__ import annotations

import argparse
import csv
import math
import os
import statistics
import subprocess
from pathlib import Path

ANALYSIS_RATE = 8_000
FRAME_SAMPLES = 4_096
FRAME_SECONDS = FRAME_SAMPLES / ANALYSIS_RATE
CLIP_SECONDS = 61


def features(path: Path) -> list[tuple[float, float, float, float]]:
    graph = (
        f"amovie=filename={path},aresample={ANALYSIS_RATE},"
        f"aformat=channel_layouts=mono,asetnsamples=n={FRAME_SAMPLES}:p=1,"
        "astats=metadata=1:reset=1,"
        f"aspectralstats=win_size={FRAME_SAMPLES}:overlap=0:"
        "measure=centroid+spread+flux"
    )
    completed = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            graph,
            "-show_frames",
            "-show_entries",
            (
                "frame_tags=lavfi.astats.Overall.RMS_level,"
                "lavfi.aspectralstats.1.centroid,"
                "lavfi.aspectralstats.1.spread,"
                "lavfi.aspectralstats.1.flux"
            ),
            "-of",
            "csv=p=0",
        ],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    output: list[tuple[float, float, float, float]] = []
    for row in csv.reader(completed.stdout.splitlines()):
        if len(row) < 4:
            continue
        values = tuple(float(value) for value in row[-4:])
        level, centroid, spread, flux = values
        output.append(
            (
                level if math.isfinite(level) else -100.0,
                max(centroid, 1.0),
                max(spread, 1.0),
                max(flux, 0.0),
            )
        )
    return output


def duration(path: Path) -> float:
    completed = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            str(path),
        ],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return float(completed.stdout.strip())


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    position = fraction * (len(ordered) - 1)
    lower = math.floor(position)
    upper = math.ceil(position)
    mix = position - lower
    return ordered[lower] * (1 - mix) + ordered[upper] * mix


def mean_difference(values: list[float]) -> float:
    return statistics.fmean(
        abs(current - previous) for previous, current in zip(values[:-1], values[1:])
    )


def best_offset(path: Path) -> tuple[float, float, float, float]:
    measurements = features(path)
    window_frames = round(CLIP_SECONDS / FRAME_SECONDS)
    maximum_start = min(
        len(measurements) - window_frames,
        math.floor((duration(path) - CLIP_SECONDS) / FRAME_SECONDS),
    )
    if maximum_start < 0:
        raise ValueError(f"source is shorter than {CLIP_SECONDS} seconds")

    all_levels = [frame[0] for frame in measurements]
    global_active_floor = max(percentile(all_levels, 0.25) + 8, -55.0)
    scores: list[tuple[float, int, float, float]] = []
    for start in range(maximum_start + 1):
        window = measurements[start : start + window_frames]
        window_levels = [frame[0] for frame in window]
        centroids = [math.log2(frame[1]) for frame in window]
        spreads = [math.log2(frame[2]) for frame in window]
        fluxes = [math.log1p(frame[3] * 100) for frame in window]
        level_motion = mean_difference(window_levels)
        level_spread = percentile(window_levels, 0.9) - percentile(window_levels, 0.1)
        spectral_motion = mean_difference(centroids) + 0.5 * mean_difference(spreads)
        flux_activity = statistics.fmean(fluxes)
        active_share = statistics.fmean(level >= global_active_floor for level in window_levels)
        median_level = statistics.median(window_levels)
        quiet_penalty = max(0.0, -55.0 - median_level)
        score = (
            0.45 * level_motion
            + 0.08 * level_spread
            + 5.0 * spectral_motion
            + 3.0 * flux_activity
            + 3.0 * active_share
            - 0.25 * quiet_penalty
        )
        scores.append((score, start, level_spread, spectral_motion))

    score, start, level_spread, spectral_motion = max(scores)
    safe_offset = math.floor(start * FRAME_SECONDS * 10) / 10
    return safe_offset, score, level_spread, spectral_motion


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=Path(__file__).parents[1] / "sources.tsv")
    parser.add_argument("--raw-dir", type=Path, default=Path(__file__).parents[1] / "samples/raw")
    parser.add_argument("--last", type=int, help="only rank the final N manifest entries")
    parser.add_argument(
        "--write-manifest",
        action="store_true",
        help="write the reviewed recommendations into the manifest",
    )
    args = parser.parse_args()

    with args.manifest.open(newline="", encoding="utf-8") as source:
        reader = csv.DictReader(source, delimiter="\t")
        fieldnames = reader.fieldnames
        all_rows = list(reader)
    rows = all_rows
    if args.last is not None:
        rows = rows[-args.last :]

    print("id\ttrim_start\tscore\tlevel_spread_db\tspectral_motion")
    recommendations: dict[str, str] = {}
    for row in rows:
        offset, score, level_spread, spectral_motion = best_offset(
            args.raw_dir / f"{row['id']}.media"
        )
        print(
            f"{row['id']}\t{offset:.1f}\t{score:.3f}\t"
            f"{level_spread:.2f}\t{spectral_motion:.3f}"
        )
        recommendations[row["id"]] = f"{offset:.1f}"

    if args.write_manifest:
        if fieldnames is None:
            raise ValueError("manifest has no header")
        for row in all_rows:
            if row["id"] in recommendations:
                row["trim_start"] = recommendations[row["id"]]
                if not row["cache_source"]:
                    row["cache_source"] = "-"
        temporary = args.manifest.with_suffix(args.manifest.suffix + ".tmp")
        with temporary.open("w", newline="", encoding="utf-8") as destination:
            writer = csv.DictWriter(
                destination,
                fieldnames=fieldnames,
                delimiter="\t",
                lineterminator="\n",
            )
            writer.writeheader()
            writer.writerows(all_rows)
        os.replace(temporary, args.manifest)


if __name__ == "__main__":
    main()
