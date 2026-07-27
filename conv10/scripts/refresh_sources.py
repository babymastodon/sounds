#!/usr/bin/env python3

from __future__ import annotations

import argparse
import csv
import hashlib
import html
import json
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

from audit_licenses import classify_license, page_title


ROOT = Path(__file__).resolve().parents[2]
OPENVERSE_AUDIO = "https://api.openverse.org/v1/audio/"
USER_AGENT = "sounds-source-refresh/1.0"
NO_CREDIT_MARKERS = ("cc0", "creative commons 0", "creative commons zero", "public domain", "pdm")
MANIFESTS = [ROOT / f"conv{number}" / "sources.tsv" for number in range(1, 11)]
AUDITS = [
    ROOT / "conv10" / "LICENSE_AUDIT_CONV10.tsv",
    ROOT / "conv10" / "LICENSE_AUDIT_CONV8.tsv",
]
FORCED_SOURCES = {
    source
    for source in (
        (
            "https://freesound.org/people/Duisterwho/sounds/642000",
            "https://cdn.freesound.org/previews/642/642000_13590673-hq.mp3",
        ),
    )
}
LIST_OVERRIDES = {
    ("fieldatlas", "river_rapids_short"): {
        "trim_start": "10",
        "source": "https://cdn.freesound.org/previews/642/642774_457982-hq.mp3",
    },
    ("fieldatlas", "burning_fire_short"): {"trim_start": "0"},
    ("fieldatlas", "sheep_barn_short"): {"trim_start": "0"},
    ("fieldatlas", "transformer_short"): {"trim_start": "0"},
    ("fieldatlas", "printing_rhythm_short"): {"trim_start": "0"},
    ("fieldatlas", "shortwave_noise_short"): {"trim_start": "0"},
}
STOPWORDS = {
    "a",
    "an",
    "and",
    "at",
    "close",
    "distant",
    "in",
    "inside",
    "large",
    "long",
    "near",
    "of",
    "on",
    "one",
    "recording",
    "room",
    "short",
    "small",
    "sound",
    "sounds",
    "the",
    "two",
    "with",
}


def read_tsv(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as source:
        return list(csv.DictReader(source, delimiter="\t"))


def write_tsv(path: Path, fieldnames: list[str], rows: list[dict[str, str]]) -> None:
    temporary = path.with_suffix(f"{path.suffix}.part")
    with temporary.open("w", encoding="utf-8", newline="") as destination:
        writer = csv.DictWriter(
            destination,
            fieldnames=fieldnames,
            delimiter="\t",
            lineterminator="\n",
        )
        writer.writeheader()
        writer.writerows(rows)
    temporary.replace(path)


def normalized_page(value: str) -> str:
    return value.rstrip("/")


def source_key(source_page: str, download_url: str) -> str:
    return f"{normalized_page(source_page)}\t{download_url}"


def no_credit(value: str) -> bool:
    lowered = value.lower()
    return any(marker in lowered for marker in NO_CREDIT_MARKERS)


def audit_by_id() -> dict[tuple[str, str], dict[str, str]]:
    result: dict[tuple[str, str], dict[str, str]] = {}
    for path in AUDITS:
        if not path.exists():
            continue
        corpus = "conv8" if "CONV8" in path.name else "conv10"
        for row in read_tsv(path):
            result[(corpus, row["id"])] = row
    return result


def collect_rows() -> list[dict[str, str]]:
    audited = audit_by_id()
    collected: list[dict[str, str]] = []
    for path in MANIFESTS:
        corpus = path.parent.name
        for row in read_tsv(path):
            item = dict(row)
            item["manifest"] = str(path.relative_to(ROOT))
            item["corpus"] = corpus
            if "license" not in item:
                audit = audited.get((corpus, item["id"]))
                if audit is None:
                    raise RuntimeError(f"missing audit for {corpus}/{item['id']}")
                item["license"] = audit["license"]
                item["license_url"] = audit["license_url"]
            collected.append(item)
    return collected


def replacement_rows() -> list[dict[str, str]]:
    grouped: dict[str, list[dict[str, str]]] = defaultdict(list)
    for row in collect_rows():
        forced = (
            normalized_page(row["source_page"]),
            row["download_url"],
        ) in FORCED_SOURCES
        if not no_credit(row["license"]) or forced:
            grouped[source_key(row["source_page"], row["download_url"])].append(row)

    result: list[dict[str, str]] = []
    for _key, rows in sorted(grouped.items()):
        representative = max(rows, key=lambda row: float(row["seconds"]))
        result.append(
            {
                "old_page": normalized_page(representative["source_page"]),
                "old_download_url": representative["download_url"],
                "old_license": representative["license"],
                "query": representative["category"],
                "fallback": representative.get("kind")
                or representative.get("domain")
                or "",
                "seconds": representative["seconds"],
                "uses": ";".join(
                    sorted(f"{row['corpus']}/{row['id']}" for row in rows)
                ),
            }
        )
    return result


def request_json(url: str, params: dict[str, str]) -> dict[str, Any]:
    encoded = urllib.parse.urlencode(params)
    request = urllib.request.Request(
        f"{url}?{encoded}",
        headers={"Accept": "application/json", "User-Agent": USER_AGENT},
    )
    for attempt in range(8):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            if error.code != 429 or attempt == 7:
                raise
            retry_after = error.headers.get("Retry-After")
            delay = float(retry_after) if retry_after else min(60, 2**attempt)
            print(f"rate limit; retrying in {delay:g}s", flush=True)
            time.sleep(delay)
        except (TimeoutError, urllib.error.URLError):
            if attempt == 7:
                raise
            time.sleep(min(30, 2**attempt))
    raise AssertionError("unreachable")


def fetch_page(url: str) -> str:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read().decode("utf-8", errors="replace")
        except (TimeoutError, urllib.error.HTTPError, urllib.error.URLError):
            if attempt == 4:
                raise
            time.sleep(2**attempt)
    raise AssertionError("unreachable")


def terms(value: str) -> set[str]:
    return {
        token
        for token in re.findall(r"[a-z0-9]+", value.lower())
        if len(token) > 2 and token not in STOPWORDS
    }


def candidate_score(query: str, candidate: dict[str, Any], rank: int) -> tuple[int, int]:
    query_terms = terms(query)
    title_terms = terms(candidate.get("title") or "")
    tag_terms = {
        token
        for tag in candidate.get("tags") or []
        for token in terms(tag.get("name") or "")
    }
    overlap = 5 * len(query_terms & title_terms) + len(query_terms & tag_terms)
    return overlap, -rank


def verified_candidate(
    query: str,
    seconds: float,
    results: list[dict[str, Any]],
    used_pages: set[str],
    preferred_page: str | None,
) -> dict[str, Any] | None:
    eligible: list[tuple[tuple[int, int], dict[str, Any]]] = []
    for rank, candidate in enumerate(results):
        page = normalized_page(candidate.get("foreign_landing_url") or "")
        duration_ms = candidate.get("duration")
        if (
            not page
            or page in used_pages
            or (
                preferred_page is not None
                and page != normalized_page(preferred_page)
            )
            or not candidate.get("url")
            or candidate.get("license") != "cc0"
            or not isinstance(duration_ms, (int, float))
            or duration_ms / 1000 < seconds + 0.25
            or "freesound.org/" not in page
        ):
            continue
        eligible.append((candidate_score(query, candidate, rank), candidate))

    for _score, candidate in sorted(eligible, reverse=True, key=lambda item: item[0]):
        page_url = candidate["foreign_landing_url"]
        try:
            page = fetch_page(page_url)
            license_name, license_url, commercial, credit, _decision = classify_license(
                page, page_url
            )
        except (RuntimeError, ValueError, urllib.error.URLError):
            continue
        if commercial and not credit and no_credit(license_name):
            item = dict(candidate)
            item["verified_title"] = page_title(page, candidate.get("title") or query)
            item["verified_license"] = license_name
            item["verified_license_url"] = license_url
            item["source_page_sha256"] = hashlib.sha256(page.encode()).hexdigest()
            return item
    return None


def find_replacements(output: Path, overrides_path: Path | None) -> None:
    targets = replacement_rows()
    overrides = (
        {row["query"]: row for row in read_tsv(overrides_path)}
        if overrides_path is not None
        else {}
    )
    retained_by_source: dict[str, dict[str, Any]] = {}
    if output.exists():
        targets_by_page: dict[str, list[dict[str, str]]] = defaultdict(list)
        for target in targets:
            targets_by_page[normalized_page(target["old_page"])].append(target)
        for row in json.loads(output.read_text(encoding="utf-8")):
            if row["query"] in overrides:
                continue
            old_download_url = row.get("old_download_url")
            if old_download_url is None:
                matching = targets_by_page[normalized_page(row["old_page"])]
                if len(matching) != 1:
                    continue
                old_download_url = matching[0]["old_download_url"]
                row["old_download_url"] = old_download_url
            retained_by_source[
                source_key(row["old_page"], old_download_url)
            ] = row
    used_pages = {
        normalized_page(row["source_page"])
        for row in collect_rows()
    }
    used_pages.update(
        normalized_page(row["new_page"]) for row in retained_by_source.values()
    )
    selected_by_source: dict[str, dict[str, Any]] = dict(retained_by_source)
    for index, target in enumerate(targets, start=1):
        old_source = source_key(target["old_page"], target["old_download_url"])
        if old_source in retained_by_source:
            print(f"{index}/{len(targets)} keep {target['query']}", flush=True)
            continue
        override = overrides.get(target["query"])
        query = (
            override["replacement_query"]
            if override is not None
            else target["query"]
        )
        preferred_page = (
            override.get("replacement_page") or None
            if override is not None
            else None
        )
        seconds = float(target["seconds"])
        print(f"{index}/{len(targets)} {target['query']} -> {query}", flush=True)
        candidate = None
        query_variants = [query]
        identifier_query = target["uses"].split(";")[0].split("/", 1)[1]
        identifier_query = identifier_query.replace("_", " ")
        query_variants.extend(
            candidate
            for candidate in (identifier_query, target["fallback"].replace("_", " "))
            if candidate and candidate.lower() != query.lower()
        )
        query_variants.extend(
            sorted(terms(" ".join(query_variants)), key=lambda token: (-len(token), token))
        )
        query_variants = list(dict.fromkeys(query_variants))
        for query_variant in query_variants:
            for page_number in (1, 2):
                response = request_json(
                    OPENVERSE_AUDIO,
                    {
                        "q": query_variant,
                        "license": "cc0",
                        "source": "freesound",
                        "page_size": "20",
                        "page": str(page_number),
                    },
                )
                candidate = verified_candidate(
                    query_variant,
                    seconds,
                    response.get("results") or [],
                    used_pages,
                    preferred_page,
                )
                if candidate is not None:
                    break
            if candidate is not None:
                break
        if candidate is None:
            raise RuntimeError(f"no verified candidate for {target}")

        duration_seconds = candidate["duration"] / 1000
        extra = duration_seconds - seconds
        trim_start = 10.0 if extra >= 20 else (1.0 if extra >= 2 else 0.0)
        source_page = normalized_page(candidate["foreign_landing_url"])
        used_pages.add(source_page)
        selected_by_source[old_source] = {
                **target,
                "new_title": candidate["verified_title"],
                "new_creator": candidate.get("creator") or "Unknown",
                "new_license": candidate["verified_license"],
                "new_license_url": candidate["verified_license_url"],
                "new_page": source_page,
                "new_download_url": candidate["url"],
                "new_duration": duration_seconds,
                "new_trim_start": trim_start,
                "source_page_sha256": candidate["source_page_sha256"],
            }
        selected = [
            selected_by_source[source_key(row["old_page"], row["old_download_url"])]
            for row in targets
            if source_key(row["old_page"], row["old_download_url"])
            in selected_by_source
        ]
        output.parent.mkdir(parents=True, exist_ok=True)
        temporary = output.with_suffix(f"{output.suffix}.part")
        temporary.write_text(json.dumps(selected, indent=2) + "\n", encoding="utf-8")
        temporary.replace(output)
        time.sleep(0.15)
    selected = [
        selected_by_source[source_key(row["old_page"], row["old_download_url"])]
        for row in targets
    ]
    print(f"wrote {output}: {len(selected)} replacements")


def load_mapping(path: Path) -> tuple[dict[str, dict[str, Any]], list[dict[str, Any]]]:
    rows = json.loads(path.read_text(encoding="utf-8"))
    mapping = {
        source_key(row["old_page"], row["old_download_url"]): row for row in rows
    }
    return mapping, rows


def apply_replacements(mapping_path: Path) -> None:
    mapping, replacement_list = load_mapping(mapping_path)
    changed = 0
    for path in MANIFESTS:
        rows = read_tsv(path)
        fieldnames = list(rows[0])
        for row in rows:
            replacement = mapping.get(
                source_key(row["source_page"], row["download_url"])
            )
            if replacement is None:
                continue
            row["provider"] = "Freesound via Openverse"
            row["creator"] = replacement["new_creator"]
            if "license" in row:
                row["license"] = "CC0 1.0"
                row["license_url"] = (
                    "https://creativecommons.org/publicdomain/zero/1.0/"
                )
            row["source_page"] = replacement["new_page"]
            row["download_url"] = replacement["new_download_url"]
            row["trim_start"] = f"{replacement['new_trim_start']:g}"
            changed += 1
        write_tsv(path, fieldnames, rows)

    sync_list(
        ROOT / "conv10" / "lists" / "melodyworks.txt",
        ROOT / "conv10" / "sources.tsv",
    )
    sync_list(
        ROOT / "conv10" / "lists" / "fieldatlas.txt",
        ROOT / "conv8" / "sources.tsv",
    )
    print(f"updated {changed} manifest rows from {len(replacement_list)} replacements")


def sync_list(list_path: Path, manifest_path: Path) -> None:
    manifest = {row["id"]: row for row in read_tsv(manifest_path)}
    rows = read_tsv(list_path)
    for row in rows:
        source = manifest[row["id"]]
        override = LIST_OVERRIDES.get((list_path.stem, row["id"]), {})
        row["trim_start"] = override.get("trim_start", source["trim_start"])
        row["source"] = override.get("source", source["download_url"])
    write_tsv(list_path, list(rows[0]), rows)


def write_inventory(mapping_path: Path, audit_path: Path, output: Path) -> None:
    _mapping, replacements = load_mapping(mapping_path)
    replacement_by_source = {
        source_key(row["new_page"], row["new_download_url"]): row
        for row in replacements
    }
    old_audits = {
        normalized_page(row["source_page"]): row
        for path in AUDITS
        if path.exists()
        for row in read_tsv(path)
    }
    repository_audit = {
        source_key(row["source_page"], row["download_url"]): row
        for row in json.loads(audit_path.read_text(encoding="utf-8"))
    }
    grouped: dict[str, list[dict[str, str]]] = defaultdict(list)
    for row in collect_rows():
        grouped[source_key(row["source_page"], row["download_url"])].append(row)
    extra_uses: dict[str, set[str]] = defaultdict(set)
    keys_by_download: dict[str, list[str]] = defaultdict(list)
    for key, rows in grouped.items():
        keys_by_download[rows[0]["download_url"]].append(key)
    for list_path in (
        ROOT / "conv10" / "lists" / "fieldatlas.txt",
        ROOT / "conv10" / "lists" / "melodyworks.txt",
    ):
        for row in read_tsv(list_path):
            keys = keys_by_download.get(row["source"], [])
            if len(keys) != 1:
                raise RuntimeError(
                    f"{list_path}: source has {len(keys)} inventory matches: "
                    f"{row['source']}"
                )
            extra_uses[keys[0]].add(
                f"conv10-list/{list_path.stem}/{row['id']}"
            )

    inventory: list[dict[str, str]] = []
    for _key, rows in sorted(grouped.items()):
        representative = rows[0]
        source_page = normalized_page(representative["source_page"])
        replacement = replacement_by_source.get(
            source_key(source_page, representative["download_url"])
        )
        verified = repository_audit.get(
            source_key(source_page, representative["download_url"])
        )
        audited = old_audits.get(source_page)
        license_name = representative.get("license") or ""
        license_url = representative.get("license_url") or ""
        page_hash = ""
        if verified is not None:
            license_name = verified["license"]
            license_url = verified["license_url"]
            page_hash = verified["source_page_sha256"]
        elif replacement is not None:
            license_name = replacement["new_license"]
            license_url = replacement["new_license_url"]
            page_hash = replacement["source_page_sha256"]
        elif audited is not None:
            license_name = audited["license"]
            license_url = audited["license_url"]
            page_hash = audited["source_page_sha256"]
        if not no_credit(license_name):
            raise RuntimeError(f"unexpected source state: {source_page} ({license_name})")
        inventory.append(
            {
                "source_page": source_page,
                "download_url": representative["download_url"],
                "creator": representative["creator"],
                "license": license_name,
                "license_url": license_url,
                "source_page_sha256": page_hash,
                "used_by": ";".join(
                    sorted(
                        {
                            *(f"{row['corpus']}/{row['id']}" for row in rows),
                            *extra_uses.get(_key, set()),
                        }
                    )
                ),
            }
        )
    write_tsv(output, list(inventory[0]), inventory)
    print(f"wrote {output}: {len(inventory)} sources")


def audit_existing_one(
    index: int,
    row: dict[str, str],
    bigsoundbank_terms_ok: bool,
) -> tuple[int, dict[str, str]]:
    source_page = row["source_page"]
    page = fetch_page(source_page)
    if "bigsoundbank.com/" in source_page:
        if not bigsoundbank_terms_ok or not (
            re.search(r"\bCC0\b", page, re.IGNORECASE)
            and re.search(r"Free and Royalty Free", page, re.IGNORECASE)
        ):
            raise ValueError(f"{source_page}: missing expected source markers")
        license_name = "CC0 1.0"
        license_url = row["license_url"]
        title = row["category"]
    else:
        license_name, license_url, commercial, credit, _decision = classify_license(
            page, source_page
        )
        if not commercial or credit or not no_credit(license_name):
            raise ValueError(f"{source_page}: unexpected source state")
        title = page_title(page, row["category"])
    return index, {
        "source_page": normalized_page(source_page),
        "download_url": row["download_url"],
        "title": title,
        "creator": row["creator"],
        "license": license_name,
        "license_url": license_url,
        "source_page_sha256": hashlib.sha256(page.encode()).hexdigest(),
    }


def audit_existing(output: Path, jobs: int) -> None:
    grouped: dict[str, dict[str, str]] = {}
    for row in collect_rows():
        if no_credit(row["license"]):
            grouped.setdefault(
                source_key(row["source_page"], row["download_url"]), row
            )
    terms_page = fetch_page("https://bigsoundbank.com/licenses.html")
    bigsoundbank_terms_ok = all(
        marker.lower() in terms_page.lower()
        for marker in ("Nothing is mandatory", "Without any restrictions")
    )
    rows = list(grouped.values())
    completed: dict[int, dict[str, str]] = {}
    errors: list[tuple[int, str]] = []
    with ThreadPoolExecutor(max_workers=jobs) as executor:
        futures = {
            executor.submit(
                audit_existing_one, index, row, bigsoundbank_terms_ok
            ): index
            for index, row in enumerate(rows, start=1)
        }
        for future in as_completed(futures):
            try:
                index, audited = future.result()
            except Exception as error:
                index = futures[future]
                errors.append((index, str(error)))
                print(
                    f"ERROR {index}/{len(rows)} {rows[index - 1]['source_page']}: {error}",
                    flush=True,
                )
                continue
            completed[index] = audited
            print(f"{len(completed)}/{len(rows)} {audited['title']}", flush=True)
    ordered = [completed[index] for index in sorted(completed)]
    temporary = output.with_suffix(f"{output.suffix}.part")
    temporary.write_text(json.dumps(ordered, indent=2) + "\n", encoding="utf-8")
    temporary.replace(output)
    print(f"audited {len(ordered)} existing sources")
    if errors:
        raise RuntimeError(
            f"{len(errors)} source checks failed: "
            + "; ".join(f"{index}: {message}" for index, message in errors)
        )


def download_file(url: str, destination: Path) -> None:
    temporary = destination.with_suffix(f"{destination.suffix}.part")
    temporary.unlink(missing_ok=True)
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                with temporary.open("wb") as output:
                    shutil.copyfileobj(response, output, length=1024 * 1024)
            temporary.replace(destination)
            return
        except (TimeoutError, urllib.error.HTTPError, urllib.error.URLError):
            temporary.unlink(missing_ok=True)
            if attempt == 4:
                raise
            time.sleep(2**attempt)


def validate_one(
    index: int, row: dict[str, Any], directory: Path
) -> tuple[int, dict[str, Any]]:
    cached = Path(row.get("cache_file") or "")
    if (
        row.get("cache_file")
        and cached.is_file()
        and row.get("media_sha256")
        and float(row.get("decoded_duration") or 0)
        + 0.001
        >= float(row["new_trim_start"]) + float(row["seconds"])
    ):
        return index, row
    digest = hashlib.sha256(row["new_download_url"].encode()).hexdigest()[:16]
    media = directory / f"{index:03d}-{digest}.media"
    if not media.is_file():
        download_file(row["new_download_url"], media)
    subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-xerror",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            str(media),
            "-map",
            "0:a:0",
            "-f",
            "null",
            "-",
        ],
        check=True,
    )
    duration = float(
        subprocess.check_output(
            [
                "ffprobe",
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=nw=1:nk=1",
                str(media),
            ],
            text=True,
        ).strip()
    )
    required = float(row["new_trim_start"]) + float(row["seconds"])
    if duration + 0.001 < required:
        raise RuntimeError(
            f"{row['query']}: decoded duration {duration} is below {required}"
        )
    updated = dict(row)
    updated["cache_file"] = str(media.resolve())
    with media.open("rb") as source:
        updated["media_sha256"] = hashlib.file_digest(source, "sha256").hexdigest()
    updated["decoded_duration"] = duration
    return index, updated


def validate_replacements(mapping_path: Path, directory: Path, jobs: int) -> None:
    rows = json.loads(mapping_path.read_text(encoding="utf-8"))
    directory.mkdir(parents=True, exist_ok=True)
    completed: dict[int, dict[str, Any]] = {}
    with ThreadPoolExecutor(max_workers=jobs) as executor:
        futures = {
            executor.submit(validate_one, index, row, directory): index
            for index, row in enumerate(rows, start=1)
        }
        for future in as_completed(futures):
            index, updated = future.result()
            completed[index] = updated
            print(f"{len(completed)}/{len(rows)} {updated['query']}", flush=True)
    ordered = [completed[index] for index in range(1, len(rows) + 1)]
    temporary = mapping_path.with_suffix(f"{mapping_path.suffix}.part")
    temporary.write_text(json.dumps(ordered, indent=2) + "\n", encoding="utf-8")
    temporary.replace(mapping_path)
    print(f"validated {len(ordered)} media files")


def seed_replacements(
    mapping_path: Path,
    query: str | None,
    preserve_outputs: bool,
) -> None:
    rows = json.loads(mapping_path.read_text(encoding="utf-8"))
    if query is not None:
        rows = [row for row in rows if row["query"] == query]
        if not rows:
            raise ValueError(f"mapping has no query: {query}")
    for row in rows:
        cache_file = Path(row["cache_file"])
        if not cache_file.is_file():
            raise FileNotFoundError(cache_file)
        for use in row["uses"].split(";"):
            corpus, identifier = use.split("/", 1)
            projects = [ROOT / corpus]
            if corpus == "conv8":
                projects.append(ROOT / "conv10")
            for project in projects:
                raw = project / "samples" / "raw" / f"{identifier}.media"
                prepared = project / "samples" / "prepared" / f"{identifier}.wav"
                for path in (
                    raw,
                    raw.with_suffix(".source"),
                    prepared,
                    prepared.with_suffix(".recipe"),
                ):
                    path.unlink(missing_ok=True)
                raw.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(cache_file, raw)
                if project.name == "conv9":
                    manifest_row = next(
                        item
                        for item in read_tsv(project / "sources.tsv")
                        if item["id"] == identifier
                    )
                    cache_source = manifest_row["cache_source"]
                    if cache_source == "-":
                        cache_source = ""
                    raw.with_suffix(".source").write_text(
                        f"{row['new_download_url']}\t{cache_source}\n",
                        encoding="utf-8",
                    )
                elif project.name == "conv10":
                    raw.with_suffix(".source").write_text(
                        f"{row['new_download_url']}\n",
                        encoding="utf-8",
                    )

    if not preserve_outputs:
        for corpus_number in range(1, 11):
            output_dir = ROOT / f"conv{corpus_number}" / "outputs"
            if not output_dir.exists():
                continue
            for path in output_dir.rglob("*"):
                if path.is_file() and path.suffix.lower() in {
                    ".aac",
                    ".flac",
                    ".m4a",
                    ".mp3",
                    ".opus",
                    ".wav",
                }:
                    path.unlink()
    print(f"seeded {sum(len(row['uses'].split(';')) for row in rows)} corpus clips")


def print_inventory() -> None:
    rows = collect_rows()
    replacements = replacement_rows()
    print(f"manifest_rows\t{len(rows)}")
    print(f"unique_sources\t{len({normalized_page(row['source_page']) for row in rows})}")
    print(f"replacement_sources\t{len(replacements)}")
    print(f"replacement_rows\t{sum(len(row['uses'].split(';')) for row in replacements)}")
    if not replacements:
        return
    writer = csv.DictWriter(
        sys.stdout,
        fieldnames=list(replacements[0]),
        delimiter="\t",
        lineterminator="\n",
    )
    writer.writeheader()
    writer.writerows(replacements)


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("scan")

    find_parser = subparsers.add_parser("find")
    find_parser.add_argument("--output", type=Path, required=True)
    find_parser.add_argument("--overrides", type=Path)

    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("--mapping", type=Path, required=True)
    validate_parser.add_argument("--directory", type=Path, required=True)
    validate_parser.add_argument("--jobs", type=int, default=8)

    audit_parser = subparsers.add_parser("audit")
    audit_parser.add_argument("--output", type=Path, required=True)
    audit_parser.add_argument("--jobs", type=int, default=8)

    apply_parser = subparsers.add_parser("apply")
    apply_parser.add_argument("--mapping", type=Path, required=True)

    seed_parser = subparsers.add_parser("seed")
    seed_parser.add_argument("--mapping", type=Path, required=True)
    seed_parser.add_argument("--query")
    seed_parser.add_argument("--preserve-outputs", action="store_true")

    inventory_parser = subparsers.add_parser("inventory")
    inventory_parser.add_argument("--mapping", type=Path, required=True)
    inventory_parser.add_argument("--audit", type=Path, required=True)
    inventory_parser.add_argument("--output", type=Path, required=True)

    args = parser.parse_args()
    if args.command == "scan":
        print_inventory()
    elif args.command == "find":
        find_replacements(
            args.output.resolve(),
            args.overrides.resolve() if args.overrides is not None else None,
        )
    elif args.command == "validate":
        validate_replacements(
            args.mapping.resolve(), args.directory.resolve(), args.jobs
        )
    elif args.command == "audit":
        audit_existing(args.output.resolve(), args.jobs)
    elif args.command == "apply":
        apply_replacements(args.mapping.resolve())
    elif args.command == "seed":
        seed_replacements(
            args.mapping.resolve(), args.query, args.preserve_outputs
        )
    else:
        write_inventory(
            args.mapping.resolve(), args.audit.resolve(), args.output.resolve()
        )


if __name__ == "__main__":
    main()
