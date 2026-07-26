#!/usr/bin/env python3
"""Build a ready-to-paste YouTube attribution block from license audits."""

from __future__ import annotations

import argparse
import csv
from pathlib import Path

PROJECT_DIR = Path(__file__).resolve().parents[1]


def read_audit(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as source:
        rows = list(csv.DictReader(source, delimiter="\t"))
    rejected = [row["id"] for row in rows if row["commercial_use"] != "yes"]
    if rejected:
        raise ValueError(f"{path}: rejected sources remain: {', '.join(rejected)}")
    return rows


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "audits",
        nargs="*",
        type=Path,
        default=[
            PROJECT_DIR / "LICENSE_AUDIT_CONV10.tsv",
            PROJECT_DIR / "LICENSE_AUDIT_CONV8.tsv",
        ],
    )
    parser.add_argument(
        "--output", type=Path, default=PROJECT_DIR / "YOUTUBE_ATTRIBUTION.md"
    )
    args = parser.parse_args()

    lines = [
        "# YouTube attribution",
        "",
        "Paste the applicable section into the video description. CC0 sources are",
        "listed in the audit tables but do not require attribution.",
        "",
        "Changes: the source recordings were trimmed, filtered, combined with",
        "additive synthesis, convolved with other sources, leveled, and assembled",
        "into a continuous program. No source creator endorses the result.",
        "",
    ]
    for audit_path in args.audits:
        rows = read_audit(audit_path)
        label = audit_path.stem.removeprefix("LICENSE_AUDIT_").lower()
        lines.extend([f"## {label}", ""])
        for row in rows:
            if row["attribution_required"] != "yes":
                continue
            lines.append(
                f'- "{row["title"]}" by {row["creator"]} — '
                f'{row["source_page"]} — {row["license"]} '
                f'({row["license_url"]})'
            )
        lines.append("")
    args.output.write_text("\n".join(lines), encoding="utf-8")
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
