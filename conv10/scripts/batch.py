#!/usr/bin/env python3
"""Prepare config-defined songs and run the long-additive-synth pipeline."""

from __future__ import annotations

import argparse
import concurrent.futures
import csv
import dataclasses
import hashlib
import json
import os
import re
import resource
import shutil
import subprocess
import sys
import threading
import time
import urllib.parse
from pathlib import Path

from rank_eventful_cuts import best_offset

PROJECT_DIR = Path(__file__).resolve().parents[1]
SAMPLE_RATE = 48_000
SHORT_SECONDS = 12.0
LONG_SECONDS = 30.0
PREPARATION_VERSION = "conv10-batch-mono-f32-48k-v2"
ID_PATTERN = re.compile(r"^[a-z0-9_]+$")
DEFAULT_ALBUM = "Convolutions 10"
DEFAULT_ARTIST = "babymastodon"
METADATA_FIELDS = [
    "title",
    "album",
    "artist",
    "album_artist",
    "composer",
    "genre",
    "date",
    "track",
    "disc",
    "comment",
    "description",
]


@dataclasses.dataclass(frozen=True)
class Entry:
    identifier: str
    role: str
    trim_start: float | None
    source: str

    @property
    def seconds(self) -> float:
        return SHORT_SECONDS if self.role == "short" else LONG_SECONDS


@dataclasses.dataclass(frozen=True)
class PreparedEntry:
    entry: Entry
    trim_start: float
    raw_path: Path
    prepared_path: Path


@dataclasses.dataclass(frozen=True)
class SongDefinition:
    config_path: Path
    name: str
    entries: tuple[Entry, ...]
    metadata: dict[str, str]


@dataclasses.dataclass
class SongRun:
    song: SongDefinition
    started: float
    job_dir: Path
    manifest_path: Path
    short_count: int
    long_count: int
    prepare_seconds: float
    render_seconds: float
    verify_seconds: float
    render_cpu: dict[str, float]
    verify_cpu: dict[str, float]
    assembly_seconds: float = 0.0
    encoding_job_seconds: float = 0.0
    finalize_seconds: float = 0.0


def sanitize_identifier(value: str) -> str:
    value = urllib.parse.unquote(value).lower()
    value = re.sub(r"[^a-z0-9]+", "_", value).strip("_")
    return value or "input"


def source_identifier(source: str) -> str:
    parsed = urllib.parse.urlparse(source)
    path = parsed.path if parsed.scheme else source
    return sanitize_identifier(Path(path).stem)


def normalize_source(source: str, list_path: Path) -> str:
    parsed = urllib.parse.urlparse(source)
    if parsed.scheme in {"http", "https"}:
        return source
    if parsed.scheme == "file":
        return str(Path(urllib.parse.unquote(parsed.path)).resolve())
    path = Path(source).expanduser()
    if not path.is_absolute():
        path = list_path.parent / path
    return str(path.resolve())


def parse_trim(value: str) -> float | None:
    if value.strip().lower() == "auto":
        return None
    trim = float(value)
    if not (trim >= 0.0):
        raise ValueError(f"trim_start must be non-negative, found {value!r}")
    return trim


def parse_list(path: Path) -> list[Entry]:
    with path.open(encoding="utf-8", newline="") as source:
        records = [
            row
            for row in csv.reader(
                (
                    line
                    for line in source
                    if line.strip() and not line.lstrip().startswith("#")
                ),
                delimiter="\t",
            )
        ]
    if not records:
        raise ValueError(f"{path} has no input rows")

    header = [field.strip().lower() for field in records[0]]
    entries: list[Entry] = []
    if header == ["id", "role", "trim_start", "source"]:
        for line_number, row in enumerate(records[1:], start=2):
            if len(row) != 4:
                raise ValueError(f"{path}:{line_number}: expected four tab-separated fields")
            identifier, role, trim_start, source = (field.strip() for field in row)
            entries.append(
                Entry(
                    identifier=identifier,
                    role=role.lower(),
                    trim_start=parse_trim(trim_start),
                    source=normalize_source(source, path),
                )
            )
    elif all(len(row) == 1 for row in records):
        split = (len(records) + 1) // 2
        used: dict[str, int] = {}
        for index, row in enumerate(records):
            source = normalize_source(row[0].strip(), path)
            base = source_identifier(source)
            sequence = used.get(base, 0) + 1
            used[base] = sequence
            identifier = base if sequence == 1 else f"{base}_{sequence}"
            entries.append(
                Entry(
                    identifier=identifier,
                    role="short" if index < split else "long",
                    trim_start=None,
                    source=source,
                )
            )
    else:
        raise ValueError(
            f"{path}: use one path/URL per line or the header "
            "'id<TAB>role<TAB>trim_start<TAB>source'"
        )

    validate_entries(entries, path)
    return entries


def validate_entries(entries: list[Entry], path: Path) -> None:
    if len(entries) < 2:
        raise ValueError(f"{path} needs at least two inputs")
    identifiers: set[str] = set()
    for entry in entries:
        if not ID_PATTERN.fullmatch(entry.identifier):
            raise ValueError(
                f"{path}: invalid id {entry.identifier!r}; use lowercase ASCII, digits, and _"
            )
        if entry.identifier in identifiers:
            raise ValueError(f"{path}: duplicate id {entry.identifier!r}")
        identifiers.add(entry.identifier)
        if entry.role not in {"short", "long"}:
            raise ValueError(f"{path}: {entry.identifier}: role must be short or long")
        if not entry.source:
            raise ValueError(f"{path}: {entry.identifier}: source is empty")
    if not any(entry.role == "short" for entry in entries):
        raise ValueError(f"{path} needs at least one short input")
    if not any(entry.role == "long" for entry in entries):
        raise ValueError(f"{path} needs at least one long input")


def parse_song_config(path: Path) -> SongDefinition:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise ValueError(f"{path}: invalid JSON: {error}") from error
    if payload.get("schema_version") != 1:
        raise ValueError(f"{path}: schema_version must be 1")
    name = payload.get("name")
    if not isinstance(name, str) or not ID_PATTERN.fullmatch(name):
        raise ValueError(f"{path}: invalid song name {name!r}")
    raw_samples = payload.get("samples")
    if not isinstance(raw_samples, list):
        raise ValueError(f"{path}: samples must be an array")
    entries = []
    for index, sample in enumerate(raw_samples):
        if not isinstance(sample, dict):
            raise ValueError(f"{path}: samples[{index}] must be an object")
        try:
            identifier = sample["id"]
            role = sample["role"]
            source = sample["source"]
        except KeyError as error:
            raise ValueError(f"{path}: samples[{index}] is missing {error.args[0]}") from error
        trim_value = sample.get("trim_start")
        if trim_value is None or (
            isinstance(trim_value, str) and trim_value.strip().lower() == "auto"
        ):
            trim_start = None
        elif isinstance(trim_value, (int, float)) and not isinstance(trim_value, bool):
            trim_start = parse_trim(str(trim_value))
        else:
            raise ValueError(f"{path}: samples[{index}].trim_start is invalid")
        if not isinstance(identifier, str) or not isinstance(role, str):
            raise ValueError(f"{path}: samples[{index}] id and role must be strings")
        if not isinstance(source, str) or not source:
            raise ValueError(f"{path}: samples[{index}].source must be a string")
        entries.append(
            Entry(
                identifier=identifier,
                role=role,
                trim_start=trim_start,
                source=normalize_source(source, path),
            )
        )
    validate_entries(entries, path)

    raw_metadata = payload.get("metadata")
    if not isinstance(raw_metadata, dict):
        raise ValueError(f"{path}: metadata must be an object")
    metadata = {}
    for field in METADATA_FIELDS:
        value = raw_metadata.get(field)
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"{path}: metadata.{field} must be a non-empty string")
        metadata[field] = value.strip()

    harmony = payload.get("harmony")
    if not isinstance(harmony, dict):
        raise ValueError(f"{path}: harmony must be an object")
    for field in (
        "register",
        "allowed_inversions",
        "tunings",
        "palettes",
        "scenes",
        "progression",
    ):
        if field not in harmony:
            raise ValueError(f"{path}: harmony.{field} is required")
    progression = harmony["progression"]
    if not isinstance(progression, list):
        raise ValueError(f"{path}: harmony.progression must be an array")
    try:
        configured_pairs = sum(int(step["pair_count"]) for step in progression)
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"{path}: invalid harmony progression") from error
    short_count = sum(entry.role == "short" for entry in entries)
    long_count = sum(entry.role == "long" for entry in entries)
    expected_pairs = short_count * long_count
    if configured_pairs != expected_pairs:
        raise ValueError(
            f"{path}: progression has {configured_pairs} pairs, expected {expected_pairs}"
        )
    return SongDefinition(path.resolve(), name, tuple(entries), metadata)


def load_catalog(path: Path) -> dict[str, dict[str, str]]:
    if not path.is_file():
        return {}
    with path.open(encoding="utf-8", newline="") as source:
        rows = list(csv.DictReader(source, delimiter="\t"))
    if not rows:
        raise ValueError(f"{path}: catalog is empty")
    required = {"name", *METADATA_FIELDS}
    missing = required - set(rows[0])
    if missing:
        raise ValueError(f"{path}: missing catalog fields: {', '.join(sorted(missing))}")
    catalog: dict[str, dict[str, str]] = {}
    for line_number, row in enumerate(rows, start=2):
        name = row["name"].strip()
        if not ID_PATTERN.fullmatch(name):
            raise ValueError(f"{path}:{line_number}: invalid track name {name!r}")
        if name in catalog:
            raise ValueError(f"{path}:{line_number}: duplicate track name {name!r}")
        metadata = {field: row[field].strip() for field in METADATA_FIELDS}
        empty = [field for field, value in metadata.items() if not value]
        if empty:
            raise ValueError(
                f"{path}:{line_number}: empty metadata: {', '.join(empty)}"
            )
        catalog[name] = metadata
    return catalog


def default_metadata(name: str) -> dict[str, str]:
    title = name.replace("_", " ").replace("-", " ").title()
    return {
        "title": title,
        "album": DEFAULT_ALBUM,
        "artist": DEFAULT_ARTIST,
        "album_artist": DEFAULT_ARTIST,
        "composer": DEFAULT_ARTIST,
        "genre": "Experimental",
        "date": "2026",
        "track": "1/1",
        "disc": "1/1",
        "comment": f"Convolution program generated from the {name} input list.",
        "description": (
            f"Themes: {name}. Harmony and scene progression are defined by the song config."
        ),
    }


def require_commands(commands: list[str]) -> None:
    missing = [command for command in commands if shutil.which(command) is None]
    if missing:
        raise RuntimeError(f"missing required commands: {', '.join(missing)}")


def source_signature(source: str) -> str:
    parsed = urllib.parse.urlparse(source)
    if parsed.scheme in {"http", "https"}:
        return source
    path = Path(source)
    metadata = path.stat()
    return f"{path}\t{metadata.st_size}\t{metadata.st_mtime_ns}"


def run_checked(command: list[str], *, cwd: Path | None = None) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def fetch_entry(entry: Entry, raw_dir: Path) -> tuple[Entry, Path, str]:
    raw_path = raw_dir / f"{entry.identifier}.media"
    recipe_path = raw_dir / f"{entry.identifier}.source"
    signature = source_signature(entry.source)
    if raw_path.is_file() and recipe_path.is_file():
        if recipe_path.read_text(encoding="utf-8").strip() == signature:
            try:
                validate_raw(raw_path, entry)
                return entry, raw_path, signature
            except (subprocess.CalledProcessError, ValueError):
                print(f"discard truncated cache {entry.identifier}", file=sys.stderr)
                raw_path.unlink(missing_ok=True)
                recipe_path.unlink(missing_ok=True)

    temporary = raw_path.with_name(
        f"{raw_path.name}.part.{os.getpid()}.{threading.get_ident()}"
    )
    temporary.unlink(missing_ok=True)
    parsed = urllib.parse.urlparse(entry.source)
    if parsed.scheme in {"http", "https"}:
        if entry.trim_start is None:
            print(f"download {entry.identifier}", file=sys.stderr)
            run_checked(
                [
                    "curl",
                    "--fail",
                    "--location",
                    "--silent",
                    "--show-error",
                    "--retry",
                    "4",
                    "--retry-delay",
                    "2",
                    "--user-agent",
                    "conv10-batch/1.0",
                    "--output",
                    str(temporary),
                    entry.source,
                ]
            )
        else:
            capture_seconds = entry.trim_start + entry.seconds + 0.1
            print(
                f"capture {entry.identifier} through {capture_seconds:.3f}s",
                file=sys.stderr,
            )
            run_checked(
                [
                    "ffmpeg",
                    "-xerror",
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-threads",
                    "1",
                    "-y",
                    "-i",
                    entry.source,
                    "-t",
                    f"{capture_seconds:.6f}",
                    "-map",
                    "0:a:0",
                    "-vn",
                    "-c:a",
                    "flac",
                    "-compression_level",
                    "0",
                    "-f",
                    "flac",
                    str(temporary),
                ]
            )
    else:
        local_source = Path(entry.source)
        if not local_source.is_file():
            raise FileNotFoundError(f"{entry.identifier}: input does not exist: {local_source}")
        print(f"copy {entry.identifier}", file=sys.stderr)
        shutil.copyfile(local_source, temporary)
    validate_raw(temporary, entry)
    temporary.replace(raw_path)
    recipe_path.write_text(f"{signature}\n", encoding="utf-8")
    return entry, raw_path, signature


def validate_raw(path: Path, entry: Entry) -> None:
    duration = probe_duration(path)
    selected_start = entry.trim_start or 0.0
    minimum_duration = (
        selected_start + entry.seconds
        if entry.trim_start is not None
        else (5.0 if entry.role == "short" else 25.0)
    )
    if duration + 0.001 < minimum_duration:
        raise ValueError(
            f"{entry.identifier}: cached input is {duration:.3f}s, "
            f"requires at least {minimum_duration:.3f}s"
        )
    run_checked(
        [
            "ffmpeg",
            "-xerror",
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            str(path),
            "-ss",
            f"{selected_start:.6f}",
            "-t",
            f"{entry.seconds:.6f}",
            "-map",
            "0:a:0",
            "-f",
            "null",
            "-",
        ]
    )


def probe_duration(path: Path) -> float:
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
    duration = float(completed.stdout.strip())
    if not (duration > 0.0):
        raise ValueError(f"{path}: invalid duration {duration}")
    return duration


def probe_frames(path: Path) -> int:
    completed = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=duration_ts",
            "-of",
            "default=nw=1:nk=1",
            str(path),
        ],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return int(completed.stdout.strip())


def prepare_entry(
    fetched: tuple[Entry, Path, str], prepared_dir: Path
) -> PreparedEntry:
    entry, raw_path, source_recipe = fetched
    duration = probe_duration(raw_path)
    minimum_source_seconds = 5.0 if entry.role == "short" else 25.0
    if duration + 0.001 < minimum_source_seconds:
        raise ValueError(
            f"{entry.identifier}: {duration:.3f}s input is shorter than "
            f"the minimum {minimum_source_seconds:.0f}s {entry.role} source"
        )
    trim_cache_path = prepared_dir / f"{entry.identifier}.trim.json"
    trim_recipe = {
        "version": PREPARATION_VERSION,
        "source": source_recipe,
        "role": entry.role,
        "seconds": entry.seconds,
    }
    trim_start = entry.trim_start
    if trim_start is None and trim_cache_path.is_file():
        try:
            cached_trim = json.loads(trim_cache_path.read_text(encoding="utf-8"))
            if cached_trim.get("recipe") == trim_recipe:
                trim_start = float(cached_trim["trim_start"])
        except (OSError, TypeError, ValueError, json.JSONDecodeError):
            trim_start = None
    if trim_start is None:
        trim_start = best_offset(raw_path, entry.seconds)[0]
        trim_cache_path.write_text(
            json.dumps({"recipe": trim_recipe, "trim_start": trim_start}, indent=2)
            + "\n",
            encoding="utf-8",
        )
    if duration + 0.001 < entry.seconds and trim_start > 0.0:
        raise ValueError(
            f"{entry.identifier}: a {duration:.3f}s source shorter than the "
            f"{entry.seconds:.0f}s target must use trim_start 0"
        )
    if duration + 0.001 >= entry.seconds and trim_start + entry.seconds > duration + 0.001:
        raise ValueError(
            f"{entry.identifier}: {trim_start:.3f}+{entry.seconds:.3f}s "
            f"exceeds the {duration:.3f}s input"
        )

    expected_frames = round(entry.seconds * SAMPLE_RATE)
    start_frame = round(trim_start * SAMPLE_RATE)
    end_frame = start_frame + expected_frames
    prepared_path = prepared_dir / f"{entry.identifier}.wav"
    recipe_path = prepared_dir / f"{entry.identifier}.recipe"
    recipe = (
        f"{PREPARATION_VERSION}\t{source_recipe}\t{entry.role}\t"
        f"{trim_start:.6f}\t{entry.seconds:.6f}"
    )
    reusable = (
        prepared_path.is_file()
        and recipe_path.is_file()
        and recipe_path.read_text(encoding="utf-8").strip() == recipe
    )
    if reusable:
        reusable = probe_frames(prepared_path) == expected_frames
    if not reusable:
        print(
            f"prepare {entry.identifier} ({entry.role}, {entry.seconds:.0f}s "
            f"from {trim_start:.3f}s)",
            file=sys.stderr,
        )
        temporary = prepared_path.with_name(
            f"{prepared_path.name}.part.{os.getpid()}.{threading.get_ident()}.wav"
        )
        temporary.unlink(missing_ok=True)
        fade_out = max(0.0, entry.seconds - 0.02)
        tempo_filter = ""
        if duration + 0.001 < entry.seconds:
            tempo = duration / entry.seconds
            tempo_filter = f"atempo={tempo:.9f},"
            print(
                f"stretch {entry.identifier} from {duration:.3f}s to "
                f"{entry.seconds:.0f}s",
                file=sys.stderr,
            )
        audio_filter = (
            f"aresample={SAMPLE_RATE},asetpts=N/SR/TB,"
            f"atrim=start_sample={start_frame}:end_sample={end_frame},asetpts=N/SR/TB,"
            f"{tempo_filter}highpass=f=15,lowpass=f=21000,"
            f"apad,atrim=end_sample={expected_frames},"
            f"afade=t=in:st=0:d=0.02,afade=t=out:st={fade_out:.6f}:d=0.02,"
            "asetpts=N/SR/TB"
        )
        run_checked(
            [
                "ffmpeg",
                "-xerror",
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-y",
                "-i",
                str(raw_path),
                "-vn",
                "-af",
                audio_filter,
                "-ar",
                str(SAMPLE_RATE),
                "-ac",
                "1",
                "-c:a",
                "pcm_f32le",
                "-f",
                "wav",
                str(temporary),
            ]
        )
        temporary.replace(prepared_path)
        recipe_path.write_text(f"{recipe}\n", encoding="utf-8")
    actual_frames = probe_frames(prepared_path)
    if actual_frames != expected_frames:
        raise ValueError(
            f"{entry.identifier}: prepared input has {actual_frames} frames, "
            f"expected {expected_frames}"
        )
    return PreparedEntry(entry, trim_start, raw_path, prepared_path)


def prepare_song(
    song: SongDefinition, scratch_root: Path, jobs: int
) -> tuple[list[PreparedEntry], Path, Path]:
    entries = list(song.entries)
    job_name = sanitize_identifier(song.name)
    job_dir = scratch_root / job_name
    raw_dir = job_dir / "raw"
    prepared_dir = job_dir / "prepared"
    raw_dir.mkdir(parents=True, exist_ok=True)
    prepared_dir.mkdir(parents=True, exist_ok=True)

    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as executor:
        fetches = {
            executor.submit(fetch_entry, entry, raw_dir): entry for entry in entries
        }
        preparations = []
        for future in concurrent.futures.as_completed(fetches):
            preparations.append(
                executor.submit(prepare_entry, future.result(), prepared_dir)
            )
        prepared_by_id = {
            item.entry.identifier: item
            for item in (
                future.result()
                for future in concurrent.futures.as_completed(preparations)
            )
        }
        prepared = [prepared_by_id[entry.identifier] for entry in entries]

    manifest_path = job_dir / "manifest.tsv"
    with manifest_path.open("w", encoding="utf-8", newline="") as destination:
        writer = csv.writer(destination, delimiter="\t", lineterminator="\n")
        writer.writerow(
            [
                "id",
                "category",
                "domain",
                "seconds",
                "trim_start",
                "provider",
                "creator",
                "source_page",
                "download_url",
            ]
        )
        for item in prepared:
            writer.writerow(
                [
                    item.entry.identifier,
                    item.entry.identifier,
                    item.entry.role,
                    f"{item.entry.seconds:.0f}",
                    f"{item.trim_start:.6f}",
                    "batch",
                    "batch",
                    item.entry.source,
                    item.entry.source,
                ]
            )
    return prepared, manifest_path, job_dir


def read_process_ticks(pid: int) -> int:
    fields = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8").split()
    return int(fields[13]) + int(fields[14])


def run_monitored(command: list[str], csv_path: Path) -> tuple[float, dict[str, float]]:
    started = time.monotonic()
    process = subprocess.Popen(command, cwd=PROJECT_DIR)
    logical_cpus = os.cpu_count() or 1
    clock_ticks = os.sysconf("SC_CLK_TCK")
    samples: list[tuple[float, float, float]] = []
    previous_time = started
    try:
        previous_ticks = read_process_ticks(process.pid)
    except (FileNotFoundError, ProcessLookupError):
        previous_ticks = 0

    while process.poll() is None:
        time.sleep(1.0)
        now = time.monotonic()
        try:
            ticks = read_process_ticks(process.pid)
        except (FileNotFoundError, ProcessLookupError):
            break
        elapsed = now - previous_time
        cores = ((ticks - previous_ticks) / clock_ticks) / max(elapsed, 1.0e-9)
        utilization = 100.0 * cores / logical_cpus
        samples.append((now - started, cores, utilization))
        previous_time = now
        previous_ticks = ticks

    status = process.wait()
    elapsed = time.monotonic() - started
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with csv_path.open("w", encoding="utf-8", newline="") as destination:
        writer = csv.writer(destination, lineterminator="\n")
        writer.writerow(["elapsed_seconds", "cores_used", "cpu_utilization_percent"])
        for sample in samples:
            writer.writerow([f"{value:.3f}" for value in sample])
    if status != 0:
        raise subprocess.CalledProcessError(status, command)

    average_cores = (
        sum(sample[1] for sample in samples) / len(samples) if samples else 0.0
    )
    average_utilization = (
        sum(sample[2] for sample in samples) / len(samples) if samples else 0.0
    )
    maximum_cores = max((sample[1] for sample in samples), default=0.0)
    low_run = 0
    longest_low_run = 0
    for _, _, utilization in samples:
        if utilization < 75.0:
            low_run += 1
            longest_low_run = max(longest_low_run, low_run)
        else:
            low_run = 0
    return elapsed, {
        "logical_cpus": float(logical_cpus),
        "samples": float(len(samples)),
        "average_cores_used": average_cores,
        "maximum_cores_used": maximum_cores,
        "average_cpu_utilization_percent": average_utilization,
        "longest_below_75_percent_seconds": float(longest_low_run),
    }


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(4 * 1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def run_profiled(
    command: list[str], *, cwd: Path | None = None
) -> tuple[float, dict[str, float]]:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    run_checked(command, cwd=cwd)
    elapsed = time.monotonic() - started
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    user_seconds = after.ru_utime - before.ru_utime
    system_seconds = after.ru_stime - before.ru_stime
    return elapsed, {
        "user_seconds": user_seconds,
        "system_seconds": system_seconds,
        "average_cores_used": (user_seconds + system_seconds) / max(elapsed, 1.0e-9),
    }


def clean_large_scratch(job_dir: Path) -> None:
    for path in [job_dir / "raw", job_dir / "prepared", job_dir / "matrix" / "wav"]:
        if path.exists():
            shutil.rmtree(path)


def render_song(
    song: SongDefinition,
    scratch_root: Path,
    prepare_jobs: int,
    render_jobs: int,
    force_render: bool,
    prepare_only: bool,
) -> SongRun:
    started = time.monotonic()
    prepare_started = time.monotonic()
    prepared, manifest_path, job_dir = prepare_song(song, scratch_root, prepare_jobs)
    prepare_seconds = time.monotonic() - prepare_started
    short_count = sum(item.entry.role == "short" for item in prepared)
    long_count = sum(item.entry.role == "long" for item in prepared)
    print(
        f"prepared {len(prepared)} inputs ({short_count} short, {long_count} long) "
        f"in {job_dir}",
        file=sys.stderr,
    )
    empty_cpu = {
        "user_seconds": 0.0,
        "system_seconds": 0.0,
        "average_cores_used": 0.0,
    }
    if prepare_only:
        return SongRun(
            song,
            started,
            job_dir,
            manifest_path,
            short_count,
            long_count,
            prepare_seconds,
            0.0,
            0.0,
            empty_cpu,
            empty_cpu,
        )

    matrix_dir = job_dir / "matrix"
    matrix_dir.mkdir(parents=True, exist_ok=True)
    binary = PROJECT_DIR / "target" / "release" / "conv10"
    render_command = [
        str(binary),
        "render",
        "--song-config",
        str(song.config_path),
        "--manifest",
        str(manifest_path),
        "--input-dir",
        str(job_dir / "prepared"),
        "--output-dir",
        str(matrix_dir),
        "--jobs",
        str(render_jobs),
    ]
    if force_render:
        render_command.append("--force")
    render_seconds, cpu_summary = run_monitored(
        render_command, job_dir / "render_cpu.csv"
    )
    verify_seconds, verify_cpu = run_profiled(
        [
            str(binary),
            "verify",
            "--song-config",
            str(song.config_path),
            "--manifest",
            str(manifest_path),
            "--input-dir",
            str(job_dir / "prepared"),
            "--output-dir",
            str(matrix_dir),
            "--jobs",
            str(render_jobs),
        ],
        cwd=PROJECT_DIR,
    )
    return SongRun(
        song,
        started,
        job_dir,
        manifest_path,
        short_count,
        long_count,
        prepare_seconds,
        render_seconds,
        verify_seconds,
        cpu_summary,
        verify_cpu,
    )


def concat_command(
    run: SongRun,
    output_dir: Path,
    aac_bitrate_kbps: int,
    opus_bitrate_kbps: int,
    crossfade_seconds: float,
) -> list[str]:
    binary = PROJECT_DIR / "target" / "release" / "conv10"
    command = [
        str(binary),
        "concat",
        "--manifest",
        str(run.manifest_path),
        "--matrix-dir",
        str(run.job_dir / "matrix"),
        "--output-dir",
        str(output_dir),
        "--scratch-dir",
        str(run.job_dir / "concat"),
        "--cover-art",
        str(PROJECT_DIR / "cover.jpg"),
        "--output-name",
        run.song.name,
        "--crossfade-seconds",
        str(crossfade_seconds),
        "--aac-bitrate-kbps",
        str(aac_bitrate_kbps),
        "--opus-bitrate-kbps",
        str(opus_bitrate_kbps),
    ]
    for field in METADATA_FIELDS:
        command.extend(
            [f"--{field.replace('_', '-')}", run.song.metadata[field]]
        )
    return command


def assemble_song(
    run: SongRun,
    output_dir: Path,
    aac_bitrate_kbps: int,
    opus_bitrate_kbps: int,
    crossfade_seconds: float,
) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    command = concat_command(
        run,
        output_dir,
        aac_bitrate_kbps,
        opus_bitrate_kbps,
        crossfade_seconds,
    )
    command.extend(["--assemble-only", "--force"])
    started = time.monotonic()
    run_checked(command, cwd=PROJECT_DIR)
    run.assembly_seconds = time.monotonic() - started


def encoder_available(name: str) -> bool:
    listing = subprocess.check_output(
        ["ffmpeg", "-hide_banner", "-encoders"],
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return any(
        len(fields) >= 2 and fields[1] == name
        for line in listing.splitlines()
        if (fields := line.split())
    )


def metadata_arguments(
    metadata: dict[str, str], *, include_description: bool
) -> list[str]:
    arguments: list[str] = []
    for field in METADATA_FIELDS:
        if field == "description" and not include_description:
            continue
        arguments.extend(["-metadata", f"{field}={metadata[field]}"])
    return arguments


def encode_one(
    run: SongRun,
    kind: str,
    output_dir: Path,
    aac_encoder: str,
    opus_encoder: str,
    aac_bitrate_kbps: int,
    opus_bitrate_kbps: int,
) -> tuple[str, str, float]:
    output_subdir, extension = {
        "flac": ("flac", "flac"),
        "aac": ("m4a", "m4a"),
        "opus": ("opus", "opus"),
    }[kind]
    target_dir = output_dir / output_subdir
    target_dir.mkdir(parents=True, exist_ok=True)
    target = target_dir / f"{run.song.name}.{extension}"
    temporary = target.with_name(
        f".{target.stem}.part.{os.getpid()}.{threading.get_ident()}{target.suffix}"
    )
    temporary.unlink(missing_ok=True)
    master = run.job_dir / "concat" / f"{run.song.name}.part.rf64.wav"
    if not master.is_file():
        raise FileNotFoundError(f"assembled RF64 master not found: {master}")

    command = [
        "ffmpeg",
        "-xerror",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        str(master),
    ]
    if kind == "flac":
        command.extend(
            [
                "-map",
                "0:a:0",
                "-c:a",
                "flac",
                "-compression_level",
                "3",
                *metadata_arguments(run.song.metadata, include_description=False),
            ]
        )
    elif kind == "aac":
        command.extend(
            [
                "-i",
                str(PROJECT_DIR / "cover.jpg"),
                "-map",
                "0:a:0",
                "-map",
                "1:v:0",
                "-c:a",
                aac_encoder,
                "-b:a",
                f"{aac_bitrate_kbps}k",
                "-c:v",
                "copy",
                "-disposition:v:0",
                "attached_pic",
                "-metadata:s:v:0",
                "title=Album cover",
                "-metadata:s:v:0",
                "comment=Cover (front)",
                *metadata_arguments(run.song.metadata, include_description=True),
            ]
        )
    else:
        command.extend(
            [
                "-map",
                "0:a:0",
                "-c:a",
                opus_encoder,
                "-b:a",
                f"{opus_bitrate_kbps}k",
                "-ac",
                "2",
                "-vbr",
                "on",
                "-application",
                "audio",
                "-compression_level",
                "10",
                *metadata_arguments(run.song.metadata, include_description=False),
            ]
        )
    command.append(str(temporary))
    started = time.monotonic()
    try:
        run_checked(command, cwd=PROJECT_DIR)
        temporary.replace(target)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise
    elapsed = time.monotonic() - started
    print(
        f"encoded {run.song.name}.{extension} in {elapsed:.1f}s",
        file=sys.stderr,
    )
    return run.song.name, kind, elapsed


def encode_all(
    runs: list[SongRun],
    output_dir: Path,
    jobs: int,
    aac_bitrate_kbps: int,
    opus_bitrate_kbps: int,
) -> float:
    aac_encoder = "libfdk_aac" if encoder_available("libfdk_aac") else "aac"
    opus_encoder = "libopus" if encoder_available("libopus") else "opus"
    work = [(run, kind) for run in runs for kind in ("flac", "aac", "opus")]
    started = time.monotonic()
    print(
        f"encoding {len(work)} files with {jobs} concurrent FFmpeg processes "
        f"({aac_encoder}, {opus_encoder})",
        file=sys.stderr,
    )
    elapsed_by_song: dict[str, float] = {run.song.name: 0.0 for run in runs}
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as executor:
        futures = [
            executor.submit(
                encode_one,
                run,
                kind,
                output_dir,
                aac_encoder,
                opus_encoder,
                aac_bitrate_kbps,
                opus_bitrate_kbps,
            )
            for run, kind in work
        ]
        for future in concurrent.futures.as_completed(futures):
            song_name, _kind, elapsed = future.result()
            elapsed_by_song[song_name] += elapsed
    for run in runs:
        run.encoding_job_seconds = elapsed_by_song[run.song.name]
    return time.monotonic() - started


def finalize_song(
    run: SongRun,
    output_dir: Path,
    aac_bitrate_kbps: int,
    opus_bitrate_kbps: int,
    crossfade_seconds: float,
    encoding_wall_seconds: float,
    keep_work: bool,
) -> None:
    output_name = run.song.name
    command = concat_command(
        run,
        output_dir,
        aac_bitrate_kbps,
        opus_bitrate_kbps,
        crossfade_seconds,
    )
    command.append("--finalize-only")
    started = time.monotonic()
    run_checked(command, cwd=PROJECT_DIR)
    run.finalize_seconds = time.monotonic() - started

    flac_path = output_dir / "flac" / f"{output_name}.flac"
    aac_path = output_dir / "m4a" / f"{output_name}.m4a"
    opus_path = output_dir / "opus" / f"{output_name}.opus"
    for path in (flac_path, aac_path, opus_path):
        validate_embedded_metadata(path, run.song.metadata)
    validate_embedded_cover(aac_path)
    hash_started = time.monotonic()
    output_paths = [flac_path, aac_path, opus_path]
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(output_paths)) as executor:
        hashes = dict(
            executor.map(
                lambda path: (
                    str(path.relative_to(output_dir)),
                    file_sha256(path),
                ),
                output_paths,
            )
        )
    hash_seconds = time.monotonic() - hash_started
    hash_path = output_dir / f"{output_name}.sha256"
    hash_path.write_text(
        "".join(f"{digest}  {name}\n" for name, digest in hashes.items()),
        encoding="utf-8",
    )
    report = {
        "status": "pass",
        "config": str(run.song.config_path),
        "config_sha256": hashlib.sha256(
            run.song.config_path.read_bytes()
        ).hexdigest(),
        "short_inputs": run.short_count,
        "long_inputs": run.long_count,
        "matrix_pairs": run.short_count * run.long_count,
        "prepare_seconds": run.prepare_seconds,
        "render_seconds": run.render_seconds,
        "verify_seconds": run.verify_seconds,
        "assembly_seconds": run.assembly_seconds,
        "encoding_job_seconds": run.encoding_job_seconds,
        "global_encoding_wall_seconds": encoding_wall_seconds,
        "finalize_seconds": run.finalize_seconds,
        "hash_seconds": hash_seconds,
        "total_seconds": time.monotonic() - run.started,
        "cpu": {
            "render": run.render_cpu,
            "verify": run.verify_cpu,
        },
        "outputs": {
            "flac": str(flac_path),
            "aac": str(aac_path),
            "opus": str(opus_path),
            "hashes": str(hash_path),
        },
        "metadata": run.song.metadata,
    }
    (output_dir / f"{output_name}.run.json").write_text(
        json.dumps(report, indent=2) + "\n", encoding="utf-8"
    )
    if not keep_work:
        clean_large_scratch(run.job_dir)
    print(
        f"completed {run.song.config_path.name}: {flac_path.name}, {aac_path.name}, "
        f"{opus_path.name}; "
        f"render CPU averaged {run.render_cpu['average_cores_used']:.2f} cores",
        file=sys.stderr,
    )


def validate_embedded_metadata(path: Path, expected: dict[str, str]) -> None:
    payload = json.loads(
        subprocess.check_output(
            [
                "ffprobe",
                "-v",
                "error",
                "-show_entries",
                "format_tags:stream_tags",
                "-of",
                "json",
                str(path),
            ],
            text=True,
        )
    )
    raw_tags: dict[str, str] = {}
    for stream in payload.get("streams", []):
        raw_tags.update(stream.get("tags", {}))
    raw_tags.update(payload.get("format", {}).get("tags", {}))
    tags = {key.lower(): str(value) for key, value in raw_tags.items()}
    aliases = {
        "album_artist": ("album_artist", "albumartist"),
        "track": ("track", "tracknumber"),
        "disc": ("disc", "discnumber"),
        # FFmpeg maps DESCRIPTION to COMMENT in Vorbis comments (FLAC/Opus).
        "description": ("description", "comment"),
    }
    mismatches = []
    for field, value in expected.items():
        keys = aliases.get(field, (field,))
        actual = next((tags[key] for key in keys if key in tags), None)
        if actual != value:
            mismatches.append(f"{field}={actual!r}, expected {value!r}")
    if mismatches:
        raise ValueError(f"{path}: metadata mismatch: {'; '.join(mismatches)}")


def validate_embedded_cover(path: Path) -> None:
    payload = json.loads(
        subprocess.check_output(
            [
                "ffprobe",
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name:stream_disposition=attached_pic",
                "-of",
                "json",
                str(path),
            ],
            text=True,
        )
    )
    streams = payload.get("streams", [])
    if not streams:
        raise ValueError(f"{path}: missing embedded cover art")
    stream = streams[0]
    attached = stream.get("disposition", {}).get("attached_pic") == 1
    if stream.get("codec_name") != "mjpeg" or not attached:
        raise ValueError(f"{path}: invalid embedded JPEG cover art")


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Turn one or more config-defined songs into same-named FLAC, AAC/M4A, "
            "and Opus long-additive-synth programs."
        )
    )
    parser.add_argument("configs", type=Path, nargs="+")
    parser.add_argument(
        "--scratch-dir", type=Path, default=PROJECT_DIR / ".scratch"
    )
    parser.add_argument(
        "--output-dir", type=Path, default=PROJECT_DIR / "outputs" / "batch"
    )
    parser.add_argument(
        "--prepare-jobs",
        type=positive_integer,
        default=max(os.cpu_count() or 1, 8),
    )
    parser.add_argument(
        "--jobs", type=positive_integer, default=max(os.cpu_count() or 1, 8)
    )
    parser.add_argument(
        "--assemble-jobs",
        type=positive_integer,
        default=max(os.cpu_count() or 1, 8),
        help="number of RF64 masters to assemble concurrently",
    )
    parser.add_argument(
        "--encode-jobs",
        type=positive_integer,
        default=max(os.cpu_count() or 1, 8),
        help="global limit for concurrent whole-file FFmpeg encoders",
    )
    parser.add_argument(
        "--finalize-jobs",
        type=positive_integer,
        default=max(os.cpu_count() or 1, 8),
        help="songs to decode-validate and hash concurrently",
    )
    parser.add_argument("--aac-bitrate-kbps", type=positive_integer, default=192)
    parser.add_argument("--opus-bitrate-kbps", type=positive_integer, default=128)
    parser.add_argument("--crossfade-seconds", type=float, default=10.0)
    parser.add_argument("--force-render", action="store_true")
    parser.add_argument("--keep-work", action="store_true")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument(
        "--validate-only",
        action="store_true",
        help="parse and validate configs without downloading or generating audio",
    )
    args = parser.parse_args()

    config_paths = [path.expanduser().resolve() for path in args.configs]
    songs = [parse_song_config(path) for path in config_paths]
    output_names: dict[str, Path] = {}
    scratch_names: dict[str, Path] = {}
    for song in songs:
        if previous := output_names.get(song.name):
            parser.error(
                f"{song.config_path} and {previous} have the same song name {song.name!r}"
            )
        output_names[song.name] = song.config_path
        scratch_name = sanitize_identifier(song.name)
        if previous := scratch_names.get(scratch_name):
            parser.error(
                f"{song.config_path} and {previous} map to the same scratch name "
                f"{scratch_name!r}"
            )
        scratch_names[scratch_name] = song.config_path
        print(
            f"{song.config_path}: {len(song.entries)} inputs, "
            f"{sum(entry.role == 'short' for entry in song.entries)} short, "
            f"{sum(entry.role == 'long' for entry in song.entries)} long"
        )
    if args.validate_only:
        return

    require_commands(["cargo", "curl", "ffmpeg", "ffprobe"])
    args.scratch_dir.mkdir(parents=True, exist_ok=True)
    if not args.prepare_only:
        run_checked(["cargo", "build", "--release", "--offline"], cwd=PROJECT_DIR)
    runs = [
        render_song(
            song,
            args.scratch_dir.resolve(),
            args.prepare_jobs,
            args.jobs,
            args.force_render,
            args.prepare_only,
        )
        for song in songs
    ]
    if args.prepare_only:
        return

    output_dir = args.output_dir.resolve()
    print(
        f"assembling {len(runs)} RF64 masters with {args.assemble_jobs} workers",
        file=sys.stderr,
    )
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=args.assemble_jobs
    ) as executor:
        futures = [
            executor.submit(
                assemble_song,
                run,
                output_dir,
                args.aac_bitrate_kbps,
                args.opus_bitrate_kbps,
                args.crossfade_seconds,
            )
            for run in runs
        ]
        for future in concurrent.futures.as_completed(futures):
            future.result()

    encoding_wall_seconds = encode_all(
        runs,
        output_dir,
        args.encode_jobs,
        args.aac_bitrate_kbps,
        args.opus_bitrate_kbps,
    )
    print(
        f"global encoding queue completed in {encoding_wall_seconds:.1f}s",
        file=sys.stderr,
    )

    print(
        f"finalizing {len(runs)} songs with {args.finalize_jobs} workers",
        file=sys.stderr,
    )
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=args.finalize_jobs
    ) as executor:
        futures = [
            executor.submit(
                finalize_song,
                run,
                output_dir,
                args.aac_bitrate_kbps,
                args.opus_bitrate_kbps,
                args.crossfade_seconds,
                encoding_wall_seconds,
                args.keep_work,
            )
            for run in runs
        ]
        for future in concurrent.futures.as_completed(futures):
            future.result()


if __name__ == "__main__":
    main()
