#!/usr/bin/env python3
"""Prepare raw audio lists and run the long-additive-synth pipeline."""

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
    return entries


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
            return entry, raw_path, signature

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
    temporary.replace(raw_path)
    recipe_path.write_text(f"{signature}\n", encoding="utf-8")
    validate_raw(raw_path, entry.seconds)
    return entry, raw_path, signature


def validate_raw(path: Path, required_seconds: float) -> None:
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
            "-t",
            f"{required_seconds:.6f}",
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


def prepare_list(
    list_path: Path, scratch_root: Path, jobs: int
) -> tuple[list[PreparedEntry], Path, Path]:
    entries = parse_list(list_path)
    job_name = sanitize_identifier(list_path.stem)
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


def run_pipeline(
    list_path: Path,
    scratch_root: Path,
    output_dir: Path,
    prepare_jobs: int,
    render_jobs: int,
    aac_bitrate_kbps: int,
    opus_bitrate_kbps: int,
    crossfade_seconds: float,
    force_render: bool,
    force_output: bool,
    keep_work: bool,
    prepare_only: bool,
    metadata: dict[str, str],
) -> None:
    started = time.monotonic()
    prepare_started = time.monotonic()
    prepared, manifest_path, job_dir = prepare_list(
        list_path, scratch_root, prepare_jobs
    )
    prepare_seconds = time.monotonic() - prepare_started
    short_count = sum(item.entry.role == "short" for item in prepared)
    long_count = sum(item.entry.role == "long" for item in prepared)
    print(
        f"prepared {len(prepared)} inputs ({short_count} short, {long_count} long) "
        f"in {job_dir}",
        file=sys.stderr,
    )
    if prepare_only:
        return

    matrix_dir = job_dir / "matrix"
    matrix_dir.mkdir(parents=True, exist_ok=True)
    binary = PROJECT_DIR / "target" / "release" / "conv10"
    render_command = [
        str(binary),
        "render",
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

    output_dir.mkdir(parents=True, exist_ok=True)
    output_name = list_path.stem
    concat_command = [
        str(binary),
        "concat",
        "--manifest",
        str(manifest_path),
        "--matrix-dir",
        str(matrix_dir),
        "--output-dir",
        str(output_dir),
        "--scratch-dir",
        str(job_dir / "concat"),
        "--cover-art",
        str(PROJECT_DIR / "cover.jpg"),
        "--output-name",
        output_name,
        "--crossfade-seconds",
        str(crossfade_seconds),
        "--aac-bitrate-kbps",
        str(aac_bitrate_kbps),
        "--opus-bitrate-kbps",
        str(opus_bitrate_kbps),
    ]
    for field in METADATA_FIELDS:
        concat_command.extend([f"--{field.replace('_', '-')}", metadata[field]])
    if force_output:
        concat_command.append("--force")
    concat_seconds, concat_cpu = run_profiled(concat_command, cwd=PROJECT_DIR)

    flac_path = output_dir / "flac" / f"{output_name}.flac"
    aac_path = output_dir / "m4a" / f"{output_name}.m4a"
    opus_path = output_dir / "opus" / f"{output_name}.opus"
    for path in (flac_path, aac_path, opus_path):
        validate_embedded_metadata(path, metadata)
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
        "list": str(list_path),
        "list_sha256": hashlib.sha256(list_path.read_bytes()).hexdigest(),
        "short_inputs": short_count,
        "long_inputs": long_count,
        "matrix_pairs": short_count * long_count,
        "prepare_seconds": prepare_seconds,
        "render_seconds": render_seconds,
        "verify_seconds": verify_seconds,
        "concat_and_encoding_seconds": concat_seconds,
        "hash_seconds": hash_seconds,
        "total_seconds": time.monotonic() - started,
        "cpu": {
            "render": cpu_summary,
            "verify": verify_cpu,
            "concat_and_encoding": concat_cpu,
        },
        "outputs": {
            "flac": str(flac_path),
            "aac": str(aac_path),
            "opus": str(opus_path),
            "hashes": str(hash_path),
        },
        "metadata": metadata,
    }
    (output_dir / f"{output_name}.run.json").write_text(
        json.dumps(report, indent=2) + "\n", encoding="utf-8"
    )
    if not keep_work:
        clean_large_scratch(job_dir)
    print(
        f"completed {list_path.name}: {flac_path.name}, {aac_path.name}, "
        f"{opus_path.name}; "
        f"render CPU averaged {cpu_summary['average_cores_used']:.2f} cores",
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
            "Turn one or more raw-input text lists into same-named FLAC, AAC/M4A, "
            "and Opus long-additive-synth programs."
        )
    )
    parser.add_argument("lists", type=Path, nargs="+")
    parser.add_argument(
        "--scratch-dir", type=Path, default=PROJECT_DIR / ".scratch"
    )
    parser.add_argument(
        "--output-dir", type=Path, default=PROJECT_DIR / "outputs" / "batch"
    )
    parser.add_argument(
        "--catalog", type=Path, default=PROJECT_DIR / "SONGS.tsv"
    )
    parser.add_argument(
        "--prepare-jobs",
        type=positive_integer,
        default=min(os.cpu_count() or 1, 8),
    )
    parser.add_argument(
        "--jobs", type=positive_integer, default=os.cpu_count() or 1
    )
    parser.add_argument("--aac-bitrate-kbps", type=positive_integer, default=192)
    parser.add_argument("--opus-bitrate-kbps", type=positive_integer, default=128)
    parser.add_argument("--crossfade-seconds", type=float, default=10.0)
    parser.add_argument("--force-render", action="store_true")
    parser.add_argument("--force-output", action="store_true")
    parser.add_argument("--keep-work", action="store_true")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument(
        "--validate-only",
        action="store_true",
        help="parse and validate lists without downloading or generating audio",
    )
    args = parser.parse_args()

    list_paths = [path.expanduser().resolve() for path in args.lists]
    catalog = load_catalog(args.catalog.expanduser().resolve())
    output_names: dict[str, Path] = {}
    scratch_names: dict[str, Path] = {}
    for path in list_paths:
        entries = parse_list(path)
        if previous := output_names.get(path.stem):
            parser.error(
                f"{path} and {previous} have the same output stem {path.stem!r}"
            )
        output_names[path.stem] = path
        scratch_name = sanitize_identifier(path.stem)
        if previous := scratch_names.get(scratch_name):
            parser.error(
                f"{path} and {previous} map to the same scratch name {scratch_name!r}"
            )
        scratch_names[scratch_name] = path
        print(
            f"{path}: {len(entries)} inputs, "
            f"{sum(entry.role == 'short' for entry in entries)} short, "
            f"{sum(entry.role == 'long' for entry in entries)} long"
        )
    if args.validate_only:
        return

    require_commands(["cargo", "curl", "ffmpeg", "ffprobe"])
    args.scratch_dir.mkdir(parents=True, exist_ok=True)
    if not args.prepare_only:
        run_checked(["cargo", "build", "--release", "--offline"], cwd=PROJECT_DIR)
    for path in list_paths:
        run_pipeline(
            path,
            args.scratch_dir.resolve(),
            args.output_dir.resolve(),
            args.prepare_jobs,
            args.jobs,
            args.aac_bitrate_kbps,
            args.opus_bitrate_kbps,
            args.crossfade_seconds,
            args.force_render,
            args.force_output,
            args.keep_work,
            args.prepare_only,
            catalog.get(path.stem, default_metadata(path.stem)),
        )


if __name__ == "__main__":
    main()
