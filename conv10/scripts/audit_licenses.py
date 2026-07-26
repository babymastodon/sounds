#!/usr/bin/env python3
"""Fetch and classify the license shown on every conv10 source page."""

from __future__ import annotations

import argparse
import csv
import hashlib
import html
import re
import urllib.request
from datetime import UTC, datetime
from pathlib import Path

PROJECT_DIR = Path(__file__).resolve().parents[1]
USER_AGENT = "conv10-license-audit/1.0"
CC_URL_PATTERN = re.compile(
    r"https?://creativecommons\.org/(?:licenses|publicdomain)/[^\"' <\\]+",
    re.IGNORECASE,
)


def fetch(url: str) -> str:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read().decode("utf-8", errors="replace")


def canonicalize_license(url: str) -> str:
    url = html.unescape(url).replace("\\/", "/")
    url = url.split("?")[0].split("#")[0]
    url = re.sub(r"/deed\.[a-z-]+$", "/", url, flags=re.IGNORECASE)
    if not url.endswith("/"):
        url += "/"
    return url.replace("http://", "https://", 1)


def classify_license(page: str, source_page: str) -> tuple[str, str, bool, bool, str]:
    normalized = html.unescape(page).replace("\\/", "/")
    candidates = [canonicalize_license(match) for match in CC_URL_PATTERN.findall(normalized)]
    if "freesound.org/" in source_page:
        license_anchor = re.search(
            r'<a[^>]*title="Go to the full license text"[^>]*'
            r'href="([^"]+)"[^>]*>([^<]+)</a>',
            normalized,
            re.IGNORECASE,
        )
        if license_anchor is None:
            license_anchor = re.search(
                r'<a[^>]*href="([^"]+)"[^>]*'
                r'title="Go to the full license text"[^>]*>([^<]+)</a>',
                normalized,
                re.IGNORECASE,
            )
        if license_anchor is None:
            raise ValueError(f"{source_page}: could not find Freesound license anchor")
        license_url = canonicalize_license(license_anchor.group(1))
        license_name = re.sub(r"\s+", " ", html.unescape(license_anchor.group(2))).strip()
    elif "commons.wikimedia.org/" in source_page:
        cc0 = next(
            (url for url in candidates if "/publicdomain/zero/" in url.lower()), None
        )
        if cc0 is None or not re.search(
            r"licensetpl(?:&#95;|_)short[^>]*>CC0<", normalized, re.IGNORECASE
        ):
            raise ValueError(f"{source_page}: expected a CC0 file-license marker")
        license_url = cc0
        license_name = "CC0 1.0"
    else:
        raise ValueError(f"{source_page}: unsupported source host")

    slug = license_url.lower()
    if "/publicdomain/zero/" in slug or "/publicdomain/mark/" in slug:
        return license_name, license_url, True, False, "accepted: public-domain tool"
    if re.search(r"/licenses/by/[0-9]", slug):
        return license_name, license_url, True, True, "accepted: commercial use with attribution"
    if "/licenses/by-nc" in slug:
        return license_name, license_url, False, True, "reject: noncommercial restriction"
    if "/licenses/by-nd" in slug:
        return license_name, license_url, False, True, "reject: no-derivatives restriction"
    if "/licenses/by-sa" in slug:
        return license_name, license_url, False, True, "reject: share-alike output constraint"
    return license_name, license_url, False, True, "reject: unsupported or unclear license"


def page_title(page: str, fallback: str) -> str:
    normalized = html.unescape(page).replace("\\/", "/")
    heading = re.search(
        r"<h1[^>]*>\s*<a[^>]*>(.*?)</a>\s*</h1>",
        normalized,
        re.IGNORECASE | re.DOTALL,
    )
    if heading is not None:
        title = re.sub(r"<[^>]+>", "", heading.group(1))
        return re.sub(r"\s+", " ", html.unescape(title)).strip()
    open_graph = re.search(
        r'<meta[^>]*property="og:title"[^>]*content="([^"]+)"',
        normalized,
        re.IGNORECASE,
    )
    if open_graph is not None:
        return re.sub(r"\s+", " ", html.unescape(open_graph.group(1))).strip()
    return fallback


def audit(manifest: Path, output: Path) -> None:
    with manifest.open(encoding="utf-8", newline="") as source:
        rows = list(csv.DictReader(source, delimiter="\t"))
    audited_at = datetime.now(UTC).replace(microsecond=0).isoformat()
    audited = []
    for index, row in enumerate(rows, start=1):
        print(f"audit {index}/{len(rows)} {row['id']}")
        page = fetch(row["source_page"])
        license_name, license_url, commercial, attribution, decision = classify_license(
            page, row["source_page"]
        )
        audited.append(
            {
                "id": row["id"],
                "title": page_title(
                    page, row.get("category") or row.get("title") or row["id"]
                ),
                "creator": row["creator"],
                "source_page": row["source_page"],
                "license": license_name,
                "license_url": license_url,
                "commercial_use": "yes" if commercial else "no",
                "attribution_required": "yes" if attribution else "no",
                "decision": decision,
                "source_page_sha256": hashlib.sha256(page.encode()).hexdigest(),
                "audited_at_utc": audited_at,
            }
        )

    temporary = output.with_suffix(f"{output.suffix}.part")
    with temporary.open("w", encoding="utf-8", newline="") as destination:
        writer = csv.DictWriter(
            destination,
            delimiter="\t",
            lineterminator="\n",
            fieldnames=list(audited[0]),
        )
        writer.writeheader()
        writer.writerows(audited)
    temporary.replace(output)
    rejected = [row["id"] for row in audited if row["commercial_use"] != "yes"]
    print(f"wrote {output}: {len(audited) - len(rejected)} accepted, {len(rejected)} rejected")
    if rejected:
        print("replace:", ", ".join(rejected))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=PROJECT_DIR / "sources.tsv")
    parser.add_argument(
        "--output", type=Path, default=PROJECT_DIR / "LICENSE_AUDIT_CONV10.tsv"
    )
    args = parser.parse_args()
    audit(args.manifest.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
