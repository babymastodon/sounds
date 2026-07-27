#!/usr/bin/env python3
"""Curate, validate, inventory, and seed the twelve conv10 song lists."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import time
from collections import OrderedDict, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

from audit_licenses import classify_license, page_title
from refresh_sources import (
    OPENVERSE_AUDIO,
    fetch_page,
    no_credit,
    normalized_page,
    request_json,
    verified_candidate,
)


PROJECT_DIR = Path(__file__).resolve().parents[1]
ROOT = PROJECT_DIR.parent
USER_AGENT = "conv10-song-curator/1.0"
SHORT_SECONDS = 12.0
LONG_SECONDS = 30.0
TOPICS_PER_PALETTE = 24
EXPECTED_SOURCES = 384
ALBUM = "Convolutions 10"
ARTIST = "babymastodon"

PINNED_SOURCES = {
    "tempest/burning_brush/short": {
        "source_page": "https://freesound.org/people/goochiano/sounds/755367",
        "download_url": "https://cdn.freesound.org/previews/755/755367_1185437-hq.mp3",
        "creator": "goochiano",
        "duration_seconds": 50.666,
    }
}


PALETTES: OrderedDict[str, list[tuple[str, str]]] = OrderedDict(
    {
        "drift": [
            ("gentle_ocean", "gentle ocean waves"),
            ("rocky_surf", "rocky beach surf"),
            ("quiet_river", "quiet river stream"),
            ("forest_creek", "forest creek water"),
            ("waterfall", "natural waterfall"),
            ("light_rain", "light rain ambience"),
            ("rain_roof", "rain on roof"),
            ("pine_wind", "pine forest wind"),
            ("rustling_leaves", "leaves rustling wind"),
            ("meadow", "quiet meadow ambience"),
            ("mountain_wind", "mountain wind ambience"),
            ("desert_wind", "desert wind ambience"),
            ("lake_shore", "calm lake shore"),
            ("pond", "quiet pond ambience"),
            ("marsh", "marsh wetland ambience"),
            ("cave_drops", "cave water drops"),
            ("underwater", "underwater ambience"),
            ("harbor_water", "harbor water ambience"),
            ("bamboo", "bamboo forest ambience"),
            ("rainforest", "tropical rainforest ambience"),
            ("snow_forest", "snow forest ambience"),
            ("countryside_dawn", "countryside dawn ambience"),
            ("countryside_night", "quiet countryside night"),
            ("garden", "quiet garden ambience"),
        ],
        "menagerie": [
            ("songbirds", "songbirds field recording"),
            ("owls", "owl night field recording"),
            ("frogs", "frog pond chorus"),
            ("crickets", "cricket night chorus"),
            ("cicadas", "cicada chorus"),
            ("beehive", "beehive buzzing"),
            ("sheep", "sheep flock ambience"),
            ("cattle", "cattle pasture ambience"),
            ("horses", "horse stable ambience"),
            ("dogs", "dogs park ambience"),
            ("cats", "cat purring"),
            ("chickens", "chicken coop ambience"),
            ("ducks", "ducks pond ambience"),
            ("geese", "geese flock ambience"),
            ("seagulls", "seagulls harbor ambience"),
            ("crows", "crow flock ambience"),
            ("monkeys", "monkey jungle calls"),
            ("elephants", "elephant herd ambience"),
            ("lions", "lion zoo ambience"),
            ("wolves", "wolves howling"),
            ("whales", "whale underwater calls"),
            ("dolphins", "dolphin underwater sounds"),
            ("pigs", "pig farm ambience"),
            ("goats", "goat herd ambience"),
        ],
        "passage": [
            ("train_station", "train station ambience"),
            ("train_ride", "train interior ride"),
            ("subway", "subway train ride"),
            ("tram", "tram ride ambience"),
            ("bus", "bus interior ride"),
            ("highway", "highway car traffic"),
            ("motorcycle", "motorcycle road ride"),
            ("bicycle", "bicycle ride ambience"),
            ("airplane_cabin", "airplane cabin ambience"),
            ("airport_runway", "airport runway ambience"),
            ("helicopter", "helicopter flight"),
            ("ferry", "ferry boat ride"),
            ("sailboat", "sailboat sailing ambience"),
            ("speedboat", "speedboat ride"),
            ("tractor", "tractor field recording"),
            ("truck", "truck driving ambience"),
            ("excavator", "excavator construction vehicle"),
            ("elevator", "elevator ride"),
            ("escalator", "escalator ambience"),
            ("rollercoaster", "roller coaster ride"),
            ("skiing", "skiing downhill ambience"),
            ("skateboard", "skateboard park ambience"),
            ("carriage", "horse carriage ride"),
            ("cablecar", "cable car ride"),
        ],
        "foundry": [
            ("factory", "factory machinery ambience"),
            ("metal_shop", "metal workshop ambience"),
            ("blacksmith", "blacksmith workshop"),
            ("welding", "welding workshop"),
            ("grinder", "angle grinder metal"),
            ("circular_saw", "circular saw workshop"),
            ("chainsaw", "chainsaw cutting wood"),
            ("woodshop", "woodworking shop ambience"),
            ("printing_press", "printing press machinery"),
            ("sewing", "sewing machine workshop"),
            ("laundromat", "laundromat machines"),
            ("dishwasher", "industrial dishwasher kitchen"),
            ("coffee_machine", "coffee machine cafe"),
            ("pneumatic_drill", "pneumatic drill construction"),
            ("jackhammer", "jackhammer construction"),
            ("demolition", "building demolition machinery"),
            ("hammering", "construction hammering"),
            ("power_drill", "power drill workshop"),
            ("compressor", "industrial air compressor"),
            ("generator", "diesel generator ambience"),
            ("turbine", "industrial turbine ambience"),
            ("pump_room", "industrial pump room"),
            ("conveyor", "conveyor belt factory"),
            ("quarry", "quarry machinery ambience"),
        ],
        "commons": [
            ("market", "busy market crowd"),
            ("restaurant", "restaurant ambience crowd"),
            ("cafe", "cafe ambience conversation"),
            ("cafeteria", "school cafeteria ambience"),
            ("playground", "playground children ambience"),
            ("stadium", "sports stadium crowd"),
            ("protest", "protest march crowd"),
            ("parade", "street parade crowd"),
            ("festival", "street festival ambience"),
            ("carnival", "carnival crowd ambience"),
            ("church", "church interior ambience"),
            ("choir_rehearsal", "choir rehearsal ambience"),
            ("theater", "theater rehearsal ambience"),
            ("orchestra_tuning", "orchestra tuning ambience"),
            ("library", "library ambience"),
            ("museum", "museum interior ambience"),
            ("hospital", "hospital corridor ambience"),
            ("office", "busy office ambience"),
            ("airport_terminal", "airport terminal crowd"),
            ("shopping_mall", "shopping mall ambience"),
            ("hotel_lobby", "hotel lobby ambience"),
            ("auction", "auction crowd ambience"),
            ("city_square", "city square crowd ambience"),
            ("wedding", "wedding reception ambience"),
        ],
        "sonora": [
            ("piano", "piano improvisation"),
            ("acoustic_guitar", "acoustic guitar improvisation"),
            ("electric_guitar", "electric guitar improvisation"),
            ("violin", "violin practice"),
            ("cello", "cello improvisation"),
            ("flute", "flute improvisation"),
            ("clarinet", "clarinet practice"),
            ("saxophone", "saxophone improvisation"),
            ("trumpet", "trumpet practice"),
            ("trombone", "trombone practice"),
            ("accordion", "accordion street music"),
            ("harmonica", "harmonica improvisation"),
            ("harp", "harp improvisation"),
            ("ukulele", "ukulele improvisation"),
            ("sitar", "sitar improvisation"),
            ("kalimba", "kalimba improvisation"),
            ("tabla", "tabla percussion"),
            ("taiko", "taiko drum performance"),
            ("handpan", "handpan improvisation"),
            ("gamelan", "gamelan performance"),
            ("marimba", "marimba improvisation"),
            ("xylophone", "xylophone performance"),
            ("pipe_organ", "pipe organ improvisation"),
            ("vocal_notes", "vocal choir notes"),
        ],
        "signals": [
            ("shortwave", "shortwave radio static"),
            ("morse", "morse code radio"),
            ("modem", "dial up modem data"),
            ("computer_fan", "computer fan ambience"),
            ("server_room", "server room ambience"),
            ("electrical_hum", "electrical hum ambience"),
            ("transformer", "electrical transformer hum"),
            ("fluorescent", "fluorescent light hum"),
            ("alarm_siren", "alarm siren ambience"),
            ("fire_alarm", "fire alarm ringing"),
            ("telephone", "telephone ringing sequence"),
            ("radio_static", "radio static tuning"),
            ("synth_drone", "synthesizer drone"),
            ("analog_synth", "analog synthesizer sequence"),
            ("arcade", "arcade machine ambience"),
            ("chiptune", "chiptune video game music"),
            ("robot", "robot servo movement"),
            ("printer", "office printer scanner"),
            ("hard_drive", "computer hard drive sounds"),
            ("typing", "computer keyboard typing"),
            ("sonar", "sonar ping sequence"),
            ("laboratory", "laboratory equipment ambience"),
            ("vending", "vending machine sounds"),
            ("medical_monitor", "hospital medical monitor beeps"),
        ],
        "tempest": [
            ("thunderstorm", "heavy thunderstorm"),
            ("hail", "hail storm"),
            ("gale", "strong wind gale"),
            ("blizzard", "blizzard wind"),
            ("hurricane", "hurricane wind"),
            ("ocean_storm", "ocean storm waves"),
            ("crashing_waves", "violent crashing waves"),
            ("avalanche", "avalanche rumble"),
            ("rockfall", "rockfall landslide"),
            ("earthquake", "earthquake rumble"),
            ("volcano", "volcano eruption rumble"),
            ("burning_brush", "burning brush fire"),
            ("bonfire", "large bonfire roaring"),
            ("explosions", "explosion sequence"),
            ("fireworks", "fireworks barrage"),
            ("cannon", "cannon fire sequence"),
            ("demolition_blast", "demolition explosion"),
            ("glass_breaking", "glass breaking sequence"),
            ("tree_falling", "tree falling forest"),
            ("ice_cracking", "lake ice cracking"),
            ("flood", "flood water torrent"),
            ("tornado", "tornado wind"),
            ("geyser", "geyser eruption"),
            ("storm_siren", "storm warning siren"),
        ],
    }
)


HYBRIDS: OrderedDict[str, tuple[tuple[str, int], tuple[str, int]]] = OrderedDict(
    {
        "wildwire": (("menagerie", 0), ("signals", 1)),
        "tideforge": (("drift", 0), ("foundry", 1)),
        "stormfolk": (("tempest", 0), ("commons", 1)),
        "railchime": (("passage", 0), ("sonora", 1)),
    }
)

TRACKS: OrderedDict[str, tuple[str, str]] = OrderedDict(
    {
        "fieldatlas": (
            "Field Atlas",
            "Environmental fields moving between water, animals, transport, public space, machinery, and collective sound.",
        ),
        "melodyworks": (
            "Melody Works",
            "Instrumental gestures convolved with vehicles, tools, alarms, crowds, weather, and physical processes.",
        ),
        "drift": (
            "Drift",
            "Calm water, weather, vegetation, and spacious natural environments.",
        ),
        "menagerie": (
            "Menagerie",
            "Birds, insects, farm animals, wildlife, and underwater voices.",
        ),
        "passage": (
            "Passage",
            "Rail, road, air, water, and human-powered motion.",
        ),
        "foundry": (
            "Foundry",
            "Factories, workshops, tools, engines, and heavy mechanical processes.",
        ),
        "commons": (
            "Commons",
            "Markets, gatherings, institutions, celebrations, and shared public rooms.",
        ),
        "sonora": (
            "Sonora",
            "Acoustic instruments, voices, tuned percussion, and improvisation.",
        ),
        "signals": (
            "Signals",
            "Radio, electrical systems, alarms, computers, machines, and synthetic tones.",
        ),
        "tempest": (
            "Tempest",
            "Storms, fire, ice, impacts, explosions, and extreme natural force.",
        ),
        "wildwire": (
            "Wildwire",
            "Animal communication crossed with electronic signaling and machine language.",
        ),
        "tideforge": (
            "Tideforge",
            "Calm environmental flow crossed with sharp industrial force.",
        ),
        "stormfolk": (
            "Stormfolk",
            "Extreme weather and impacts crossed with crowds, rituals, and public life.",
        ),
        "railchime": (
            "Railchime",
            "Transport rhythms crossed with melodic instruments and tuned percussion.",
        ),
    }
)

QUERY_FALLBACKS = {
    ("passage", "speedboat"): [
        "motorboat engine",
        "powerboat ride",
        "motor boat field recording",
    ],
    ("passage", "truck"): [
        "truck cab driving",
        "semi truck engine road",
        "lorry interior driving",
    ],
    ("passage", "cablecar"): [
        "cable car gondola ride",
        "aerial tramway gondola",
        "funicular ride",
    ],
    ("commons", "choir_rehearsal"): [
        "choir practice",
        "choir singing rehearsal",
        "vocal ensemble warmup",
        "group singing together",
        "choral singing",
        "crowd singing",
    ],
    ("commons", "theater"): [
        "stage rehearsal",
        "theatre performance ambience",
        "actors rehearsal",
    ],
    ("sonora", "kalimba"): ["kalimba music", "thumb piano improvisation"],
    ("sonora", "pipe_organ"): ["church organ music", "organ improvisation"],
    ("signals", "medical_monitor"): [
        "heart monitor beeps",
        "patient monitor alarm",
        "medical equipment beeping",
    ],
    ("signals", "server_room"): [
        "data center server fans",
        "server rack room ambience",
        "computer server room",
    ],
    ("signals", "chiptune"): [
        "8 bit chiptune music",
        "retro video game music",
        "chip music sequence",
    ],
    ("tempest", "avalanche"): ["snow avalanche", "snow slide rumble"],
    ("tempest", "rockfall"): [
        "rocks falling",
        "stone debris falling",
        "quarry rock slide",
    ],
    ("tempest", "earthquake"): ["earth rumble", "ground rumble"],
    ("tempest", "volcano"): ["volcanic eruption", "lava rumble"],
    ("tempest", "tornado"): [
        "tornado wind ambience",
        "extreme wind debris",
        "tornado storm",
    ],
    ("drift", "bamboo"): ["wind through bamboo forest", "bamboo grove ambience"],
    ("menagerie", "songbirds"): ["songbirds forest", "birdsong chorus"],
    ("menagerie", "dogs"): ["dogs barking park", "dog kennel ambience"],
    ("menagerie", "goats"): ["goat herd bells", "goats farm ambience"],
    ("menagerie", "seagulls"): ["seagulls calling harbor", "gulls seaside"],
    ("passage", "airplane_cabin"): [
        "airplane interior passenger ambience",
        "aircraft cabin flight",
    ],
    ("passage", "excavator"): ["excavator engine digging", "digger construction"],
    ("passage", "skateboard"): ["skateboard rolling park", "skateboarding ambience"],
    ("foundry", "metal_shop"): ["metal workshop grinder", "metalworking machinery"],
    ("foundry", "circular_saw"): [
        "circular saw cutting wood",
        "table saw workshop",
    ],
    ("foundry", "conveyor"): ["conveyor belt machine", "factory conveyor"],
    ("commons", "church"): ["church interior room tone", "cathedral interior ambience"],
    ("sonora", "harp"): ["harp performance", "harp improvisation music"],
    ("sonora", "marimba"): ["marimba performance", "marimba music"],
    ("tempest", "burning_brush"): [
        "brush fire sound",
        "burning brush outdoors",
        "controlled brush burn",
    ],
    ("tempest", "cannon"): ["cannon fire", "artillery cannon shots"],
    ("tempest", "flood"): [
        "flash flood rushing water",
        "flood water torrent",
        "river flood overflow",
    ],
    ("tempest", "explosions"): [
        "explosion boom sequence",
        "detonation blast",
        "multiple explosions",
    ],
    ("tempest", "tree_falling"): ["tree falling impact", "falling tree trunk"],
}

TITLE_RULES = {
    ("drift", "bamboo"): (
        ("bamboo grove", "bamboo forest", "wind in the bamboo", "bamboo wind"),
        ("chime", "maraca", "pluck"),
    ),
    ("menagerie", "songbirds"): (("bird", "songbird", "dawn chorus"), ("jackhammer",)),
    ("menagerie", "dogs"): (("dog", "bark", "kennel"), ()),
    ("menagerie", "goats"): (("goat",), ("sheep",)),
    ("menagerie", "seagulls"): (("gull", "seagull"), ()),
    ("passage", "airplane_cabin"): (("airplane", "aircraft", "plane cabin"), ("subway",)),
    ("passage", "speedboat"): (
        ("speedboat", "powerboat", "motorboat", "motor boat", "boat engine"),
        ("sailboat",),
    ),
    ("passage", "truck"): (("truck", "lorry", "semi", "pickup"), ("car driving",)),
    ("passage", "cablecar"): (
        ("cable car", "cablecar", "gondola", "funicular", "aerial tram"),
        ("subway",),
    ),
    ("passage", "excavator"): (("excavator", "digger"), ("truck",)),
    ("passage", "skateboard"): (("skateboard", "skating"), ("walk in the park",)),
    ("foundry", "metal_shop"): (("metal", "grinder", "machinery"), ("artists workshop",)),
    ("foundry", "circular_saw"): (("saw",), ("wood beams",)),
    ("foundry", "conveyor"): (("conveyor",), ()),
    ("commons", "church"): (("church", "cathedral"), ("playground",)),
    ("commons", "choir_rehearsal"): (
        ("choir", "vocal", "singing", "chorus", "chant"),
        ("peeper",),
    ),
    ("sonora", "harp"): (("harp",), ("ghatam",)),
    ("sonora", "marimba"): (("marimba",), ()),
    ("signals", "medical_monitor"): (
        ("monitor", "medical", "hospital", "ecg", "ekg", "heart rate"),
        ("police chatter",),
    ),
    ("signals", "server_room"): (
        ("server", "data center", "data centre", "server rack"),
        ("spaceship",),
    ),
    ("signals", "chiptune"): (
        ("chiptune", "chip tune", "8 bit", "8-bit", "video game", "chip music"),
        (),
    ),
    ("tempest", "avalanche"): (("avalanche", "snow slide"), ("earthquake",)),
    ("tempest", "volcano"): (("volcano", "volcanic", "lava", "mud volcano"), ("train",)),
    ("tempest", "burning_brush"): (
        ("burning brush", "brush fire", "brush burn", "wild fire", "wildfire"),
        ("digital", "haptic"),
    ),
    ("tempest", "explosions"): (("explosion", "blast", "boom", "detonation"), ("crazy run",)),
    ("tempest", "cannon"): (("cannon", "artillery"), ("water cannon",)),
    ("tempest", "flood"): (
        ("flood", "overflow", "torrent", "rushing water"),
        ("rain-on-cabin",),
    ),
    ("tempest", "tree_falling"): (
        ("tree falling", "falling tree", "felled tree", "tree crash", "tree trunk"),
        ("falling snow", "drops falling"),
    ),
    ("tempest", "tornado"): (
        ("tornado", "high wind", "howling wind", "wind debris"),
        ("warning", "siren"),
    ),
}


def slug(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", value.lower()).strip("_")


def targets() -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for palette, topics in PALETTES.items():
        if len(topics) != TOPICS_PER_PALETTE:
            raise RuntimeError(f"{palette} has {len(topics)} topics")
        for topic_index, (topic, query) in enumerate(topics):
            for role, seconds in (("long", LONG_SECONDS), ("short", SHORT_SECONDS)):
                result.append(
                    {
                        "key": f"{palette}/{topic}/{role}",
                        "id": f"{palette}_{topic}_{role[0]}",
                        "palette": palette,
                        "topic": topic,
                        "topic_index": topic_index,
                        "query": query,
                        "role": role,
                        "seconds": seconds,
                    }
                )
    if len(result) != EXPECTED_SOURCES:
        raise RuntimeError(f"defined {len(result)} targets, expected {EXPECTED_SOURCES}")
    identifiers = {row["id"] for row in result}
    if len(identifiers) != EXPECTED_SOURCES:
        raise RuntimeError("target identifiers are not globally unique")
    return result


def read_tsv(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as source:
        return list(csv.DictReader(source, delimiter="\t"))


def write_tsv(path: Path, rows: list[dict[str, Any]]) -> None:
    temporary = path.with_suffix(f"{path.suffix}.part")
    with temporary.open("w", encoding="utf-8", newline="") as destination:
        writer = csv.DictWriter(
            destination,
            fieldnames=list(rows[0]),
            delimiter="\t",
            lineterminator="\n",
        )
        writer.writeheader()
        writer.writerows(rows)
    temporary.replace(path)


def load_state(path: Path) -> dict[str, dict[str, Any]]:
    if not path.exists():
        return {}
    rows = json.loads(path.read_text(encoding="utf-8"))
    return {row["key"]: row for row in rows}


def save_state(path: Path, state: dict[str, dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    order = {row["key"]: index for index, row in enumerate(targets())}
    rows = sorted(state.values(), key=lambda row: order.get(row["key"], math.inf))
    temporary = path.with_suffix(f"{path.suffix}.part")
    temporary.write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def existing_pages() -> set[str]:
    inventory = PROJECT_DIR / "SOURCE_INVENTORY.tsv"
    if not inventory.exists():
        return set()
    return {normalized_page(row["source_page"]) for row in read_tsv(inventory)}


def discover(state_path: Path) -> None:
    state = load_state(state_path)
    defined = targets()
    defined_by_key = {row["key"]: row for row in defined}
    state = {
        key: row
        for key, row in state.items()
        if key in defined_by_key
        and all(row.get(field) == defined_by_key[key][field] for field in ("id", "query", "role"))
    }
    used_pages = existing_pages()
    used_downloads: set[str] = set()
    for row in state.values():
        page = normalized_page(row["source_page"])
        if page in used_pages or row["download_url"] in used_downloads:
            continue
        used_pages.add(page)
        used_downloads.add(row["download_url"])

    for index, target in enumerate(defined, start=1):
        retained = state.get(target["key"])
        if retained is not None:
            print(f"{index}/{len(defined)} keep {target['key']}", flush=True)
            continue
        print(f"{index}/{len(defined)} find {target['key']}", flush=True)
        candidate = pinned_candidate(target, used_pages, used_downloads)
        if candidate is None and target["key"] not in PINNED_SOURCES:
            query_variants = [
                target["query"],
                f"{target['query']} field recording",
                target["topic"].replace("_", " "),
                *QUERY_FALLBACKS.get(
                    (target["palette"], target["topic"]),
                    [],
                ),
            ]
            for query in dict.fromkeys(query_variants):
                for page_number in range(1, 5):
                    response = request_json(
                        OPENVERSE_AUDIO,
                        {
                            "q": query,
                            "license": "cc0",
                            "source": "freesound",
                            "page_size": "20",
                            "page": str(page_number),
                            "mature": "false",
                        },
                    )
                    while True:
                        candidate = verified_candidate(
                            query,
                            float(target["seconds"]),
                            response.get("results") or [],
                            used_pages,
                            None,
                        )
                        if candidate is None:
                            break
                        page = normalized_page(candidate["foreign_landing_url"])
                        if (
                            candidate["url"] in used_downloads
                            or not title_matches(target, candidate["verified_title"])
                        ):
                            used_pages.add(page)
                            candidate = None
                            continue
                        break
                    if candidate is not None:
                        break
                if candidate is not None:
                    break
        if candidate is None:
            save_state(state_path, state)
            raise RuntimeError(f"no source found for {target['key']}: {target['query']}")

        duration = float(candidate["duration"]) / 1000.0
        headroom = duration - float(target["seconds"])
        preferred_trim = 10.0 if target["role"] == "long" else 5.0
        trim_start = round(min(preferred_trim, max(0.0, headroom / 3.0)), 3)
        page_url = normalized_page(candidate["foreign_landing_url"])
        selected = {
            **target,
            "title": candidate["verified_title"],
            "creator": candidate.get("creator") or "Unknown",
            "source_page": page_url,
            "download_url": candidate["url"],
            "license": candidate["verified_license"],
            "license_url": candidate["verified_license_url"],
            "source_page_sha256": candidate["source_page_sha256"],
            "advertised_duration_seconds": duration,
            "trim_start": trim_start,
        }
        state[target["key"]] = selected
        used_pages.add(page_url)
        used_downloads.add(candidate["url"])
        save_state(state_path, state)
        time.sleep(0.1)
    print(f"discovered {len(state)} distinct sources")


def pinned_candidate(
    target: dict[str, Any],
    used_pages: set[str],
    used_downloads: set[str],
) -> dict[str, Any] | None:
    pinned = PINNED_SOURCES.get(target["key"])
    if pinned is None:
        return None
    source_page = normalized_page(pinned["source_page"])
    if source_page in used_pages or pinned["download_url"] in used_downloads:
        return None
    page = fetch_page(source_page)
    license_name, license_url, commercial, credit, _decision = classify_license(
        page, source_page
    )
    title = page_title(page, target["query"])
    if (
        not commercial
        or credit
        or not no_credit(license_name)
        or float(pinned["duration_seconds"]) < float(target["seconds"]) + 0.25
        or not title_matches(target, title)
    ):
        raise RuntimeError(f"pinned source no longer matches {target['key']}")
    return {
        "foreign_landing_url": source_page,
        "url": pinned["download_url"],
        "creator": pinned["creator"],
        "duration": float(pinned["duration_seconds"]) * 1000.0,
        "verified_title": title,
        "verified_license": license_name,
        "verified_license_url": license_url,
        "source_page_sha256": hashlib.sha256(page.encode()).hexdigest(),
    }


def title_matches(target: dict[str, Any], title: str) -> bool:
    rule = TITLE_RULES.get((target["palette"], target["topic"]))
    if rule is None:
        return True
    included, excluded = rule
    lowered = title.lower()
    return any(term in lowered for term in included) and not any(
        term in lowered for term in excluded
    )


def redo(state_path: Path, keys: list[str]) -> None:
    state = load_state(state_path)
    defined = {row["key"] for row in targets()}
    unknown = [key for key in keys if key not in defined]
    if unknown:
        raise ValueError(f"unknown target keys: {', '.join(unknown)}")
    removed = 0
    for key in keys:
        if state.pop(key, None) is not None:
            removed += 1
    save_state(state_path, state)
    print(f"removed {removed} selections for semantic reselection")


def validate_one(row: dict[str, Any], media_dir: Path) -> dict[str, Any]:
    cached = Path(row.get("cache_file") or "")
    if (
        cached.is_file()
        and row.get("media_sha256")
        and float(row.get("decoded_duration_seconds") or 0)
        >= float(row["seconds"])
    ):
        return row
    digest = hashlib.sha256(row["download_url"].encode()).hexdigest()[:16]
    media = media_dir / f"{row['id']}-{digest}.media"
    if not media.exists():
        download_excerpt(row, media)
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
    seconds = float(row["seconds"])
    if duration + 0.001 < seconds:
        raise RuntimeError(
            f"{row['key']}: decoded duration {duration:.3f}s is below {seconds:.3f}s"
        )
    trim_start = min(
        float(row["trim_start"]),
        max(0.0, duration - seconds - 0.05),
    )
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
            "-ss",
            f"{trim_start:.6f}",
            "-t",
            f"{seconds:.6f}",
            "-map",
            "0:a:0",
            "-f",
            "null",
            "-",
        ],
        check=True,
    )
    with media.open("rb") as source:
        media_sha256 = hashlib.file_digest(source, "sha256").hexdigest()
    return {
        **row,
        "trim_start": round(trim_start, 3),
        "cache_file": str(media.resolve()),
        "media_sha256": media_sha256,
        "decoded_duration_seconds": duration,
    }


def download_excerpt(row: dict[str, Any], destination: Path) -> None:
    capture_seconds = (
        float(row["trim_start"]) + float(row["seconds"]) + 0.1
    )
    temporary = destination.with_suffix(f"{destination.suffix}.part")
    temporary.unlink(missing_ok=True)
    subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-xerror",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-y",
            "-i",
            row["download_url"],
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
        ],
        check=True,
    )
    temporary.replace(destination)


def validate(state_path: Path, media_dir: Path, jobs: int) -> None:
    state = load_state(state_path)
    defined = targets()
    missing = [row["key"] for row in defined if row["key"] not in state]
    if missing:
        raise RuntimeError(f"discover sources first; missing {len(missing)} targets")
    media_dir.mkdir(parents=True, exist_ok=True)
    with ThreadPoolExecutor(max_workers=jobs) as executor:
        futures = {
            executor.submit(validate_one, state[row["key"]], media_dir): row["key"]
            for row in defined
        }
        complete = 0
        for future in as_completed(futures):
            key = futures[future]
            state[key] = future.result()
            complete += 1
            save_state(state_path, state)
            print(f"{complete}/{len(defined)} validated {key}", flush=True)
    print(f"validated {len(state)} media files")


def palette_rows(
    palette: str, state: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    return [
        state[row["key"]]
        for row in targets()
        if row["palette"] == palette
    ]


def song_rows(
    song: str, state: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    if song in PALETTES:
        selected = palette_rows(song, state)
    else:
        selected = []
        for palette, parity in HYBRIDS[song]:
            selected.extend(
                row
                for row in palette_rows(palette, state)
                if int(row["topic_index"]) % 2 == parity
            )
    selected.sort(key=lambda row: (row["role"] != "short", row["palette"], row["topic_index"]))
    short_count = sum(row["role"] == "short" for row in selected)
    long_count = sum(row["role"] == "long" for row in selected)
    if len(selected) != 48 or short_count != 24 or long_count != 24:
        raise RuntimeError(
            f"{song}: {len(selected)} rows, {short_count} short, {long_count} long"
        )
    return selected


def all_songs() -> list[str]:
    return [*PALETTES, *HYBRIDS]


def render_songs() -> list[str]:
    return list(TRACKS)


def catalog_rows() -> list[dict[str, str]]:
    total = len(TRACKS)
    return [
        {
            "name": name,
            "title": title,
            "album": ALBUM,
            "artist": ARTIST,
            "album_artist": ARTIST,
            "composer": ARTIST,
            "genre": "Experimental",
            "date": "2026",
            "track": f"{index}/{total}",
            "disc": "1/1",
            "comment": comment,
        }
        for index, (name, (title, comment)) in enumerate(TRACKS.items(), start=1)
    ]


def write_configs(state_path: Path) -> None:
    state = load_state(state_path)
    if len(state) != EXPECTED_SOURCES:
        raise RuntimeError(f"state has {len(state)} sources, expected {EXPECTED_SOURCES}")
    for row in state.values():
        if not Path(row.get("cache_file") or "").is_file():
            raise RuntimeError(f"validate media first: {row['key']}")

    uses: dict[str, list[str]] = defaultdict(list)
    lists_dir = PROJECT_DIR / "lists"
    lists_dir.mkdir(parents=True, exist_ok=True)
    for song in all_songs():
        rows = song_rows(song, state)
        output = lists_dir / f"{song}.txt"
        write_tsv(
            output,
            [
                {
                    "id": row["id"],
                    "role": row["role"],
                    "trim_start": f"{float(row['trim_start']):g}",
                    "source": row["download_url"],
                }
                for row in rows
            ],
        )
        for row in rows:
            uses[row["key"]].append(song)
        print(f"wrote {output}: 48 inputs")

    inventory: list[dict[str, Any]] = []
    for target in targets():
        row = state[target["key"]]
        inventory.append(
            {
                "id": row["id"],
                "palette": row["palette"],
                "topic": row["topic"],
                "role": row["role"],
                "seconds": f"{float(row['seconds']):g}",
                "trim_start": f"{float(row['trim_start']):g}",
                "title": row["title"],
                "creator": row["creator"],
                "source_page": row["source_page"],
                "download_url": row["download_url"],
                "license": row["license"],
                "license_url": row["license_url"],
                "source_page_sha256": row["source_page_sha256"],
                "media_sha256": row["media_sha256"],
                "advertised_duration_seconds": (
                    f"{float(row['advertised_duration_seconds']):.6f}"
                ),
                "decoded_duration_seconds": f"{float(row['decoded_duration_seconds']):.6f}",
                "used_by": ";".join(uses[row["key"]]),
            }
        )
    write_tsv(PROJECT_DIR / "SONG_SOURCES.tsv", inventory)
    write_tsv(PROJECT_DIR / "SONGS.tsv", catalog_rows())
    use_count = sum(len(value) for value in uses.values())
    reused = sum(len(value) == 2 for value in uses.values())
    if use_count != 576 or reused != 192:
        raise RuntimeError(f"unexpected overlap: {use_count} uses, {reused} reused")
    print(
        f"wrote SONG_SOURCES.tsv: {len(inventory)} sources, "
        f"{use_count} list uses, {reused} sources reused once"
    )
    print(f"wrote SONGS.tsv: {len(TRACKS)} album tracks")


def seed(state_path: Path, scratch_dir: Path) -> None:
    state = load_state(state_path)
    for song in all_songs():
        raw_dir = scratch_dir / song / "raw"
        raw_dir.mkdir(parents=True, exist_ok=True)
        for row in song_rows(song, state):
            source = Path(row["cache_file"])
            media = raw_dir / f"{row['id']}.media"
            recipe = raw_dir / f"{row['id']}.source"
            media.unlink(missing_ok=True)
            try:
                os.link(source, media)
            except OSError:
                shutil.copyfile(source, media)
            recipe.write_text(f"{row['download_url']}\n", encoding="utf-8")
        print(f"seeded {song}: 48 raw inputs")
    seed_existing_lists(scratch_dir)


def seed_existing_lists(scratch_dir: Path) -> None:
    imported_sources = {
        row["id"]: row["download_url"]
        for row in read_tsv(ROOT / "conv8" / "sources.tsv")
    }
    cache_roots = (
        (PROJECT_DIR / "samples" / "raw", None),
        (ROOT / "conv8" / "samples" / "raw", imported_sources),
    )
    for song in ("fieldatlas", "melodyworks"):
        rows = read_tsv(PROJECT_DIR / "lists" / f"{song}.txt")
        raw_dir = scratch_dir / song / "raw"
        raw_dir.mkdir(parents=True, exist_ok=True)
        seeded = 0
        for row in rows:
            media = raw_dir / f"{row['id']}.media"
            recipe = raw_dir / f"{row['id']}.source"
            media.unlink(missing_ok=True)
            recipe.unlink(missing_ok=True)
            for cache_root, inventory in cache_roots:
                candidate = cache_root / f"{row['id']}.media"
                if not candidate.is_file():
                    continue
                if inventory is None:
                    candidate_recipe = candidate.with_suffix(".source")
                    if (
                        not candidate_recipe.is_file()
                        or candidate_recipe.read_text(encoding="utf-8").strip()
                        != row["source"]
                    ):
                        continue
                elif inventory.get(row["id"]) != row["source"]:
                    continue
                try:
                    os.link(candidate, media)
                except OSError:
                    shutil.copyfile(candidate, media)
                recipe.write_text(f"{row['source']}\n", encoding="utf-8")
                seeded += 1
                break
        print(f"seeded {song}: {seeded}/{len(rows)} cached raw inputs")


def check(state_path: Path) -> None:
    state = load_state(state_path)
    defined = targets()
    if len(state) != EXPECTED_SOURCES:
        raise RuntimeError(f"state has {len(state)} sources")
    pages = {normalized_page(row["source_page"]) for row in state.values()}
    downloads = {row["download_url"] for row in state.values()}
    if len(pages) != EXPECTED_SOURCES or len(downloads) != EXPECTED_SOURCES:
        raise RuntimeError("source pages or media URLs are not unique")
    if pages & existing_pages():
        raise RuntimeError("new song pool overlaps the existing repository inventory")
    if any(not no_credit(row["license"]) for row in state.values()):
        raise RuntimeError("source policy mismatch")
    for song in all_songs():
        song_rows(song, state)
    print(
        f"check passed: {len(defined)} sources, {len(all_songs())} songs, "
        "48 inputs per song"
    )


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least one")
    return parsed


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--state",
        type=Path,
        default=PROJECT_DIR / ".scratch" / "song_sources.json",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("discover")
    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument(
        "--media-dir",
        type=Path,
        default=PROJECT_DIR / "samples" / "song_pool",
    )
    validate_parser.add_argument("--jobs", type=positive_integer, default=8)
    subparsers.add_parser("write")
    seed_parser = subparsers.add_parser("seed")
    seed_parser.add_argument(
        "--scratch-dir",
        type=Path,
        default=PROJECT_DIR / ".scratch",
    )
    subparsers.add_parser("check")
    redo_parser = subparsers.add_parser("redo")
    redo_parser.add_argument("keys", nargs="+")
    args = parser.parse_args()
    state_path = args.state.resolve()
    if args.command == "discover":
        discover(state_path)
    elif args.command == "validate":
        validate(state_path, args.media_dir.resolve(), args.jobs)
    elif args.command == "write":
        write_configs(state_path)
    elif args.command == "seed":
        seed(state_path, args.scratch_dir.resolve())
    elif args.command == "redo":
        redo(state_path, args.keys)
    else:
        check(state_path)


if __name__ == "__main__":
    main()
