#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(cd -- "$script_dir/.." && pwd)
manifest=${1:-"$project_dir/sources.tsv"}
download_jobs=${DOWNLOAD_JOBS:-4}
raw_dir="$project_dir/samples/raw"
prepared_dir="$project_dir/samples/prepared"

if ! awk -F '\t' '
    NR == 1 { next }
    $8 != "CC0 1.0" ||
        $9 != "https://creativecommons.org/publicdomain/zero/1.0/" {
        printf "%s: only CC0 1.0 sources are allowed (found %s)\\n", $1, $8 > "/dev/stderr"
        invalid = 1
    }
    END { exit invalid }
' "$manifest"; then
    echo "refusing to download a manifest containing non-CC0 sources" >&2
    exit 1
fi

mkdir -p "$raw_dir" "$prepared_dir"

synthetic_generator=
synthetic_revision=
if awk -F '\t' 'NR > 1 && $11 ~ /^synthetic:\/\// { found = 1 } END { exit !found }' "$manifest"; then
    echo "build deterministic synthetic-source generator" >&2
    cargo build --release --offline --manifest-path "$project_dir/Cargo.toml" \
        --bin generate_synthetic
    synthetic_generator="$project_dir/target/release/generate_synthetic"
    synthetic_revision=$(
        find "$project_dir/src/synthetic.rs" "$project_dir/src/synthetic" \
            -type f -name '*.rs' -print0 |
            sort -z |
            xargs -0 sha256sum |
            sha256sum |
            awk '{ print $1 }'
    )
fi

# Migrate prepared files made before per-source recipes were introduced. A completed
# SOURCES.tsv is only copied after every source validates, so it is authoritative
# for the WAVs beside it.
if [[ -s "$prepared_dir/SOURCES.tsv" ]]; then
    while IFS=$'\t' read -r old_id _category _kind old_seconds old_trim_start \
        _provider _creator _license _license_url _source_page old_download_url _cache_source; do
        [[ "$old_id" == "id" || -z "$old_id" ]] && continue
        old_prepared="$prepared_dir/$old_id.wav"
        old_recipe="$prepared_dir/$old_id.recipe"
        old_raw="$raw_dir/$old_id.media"
        old_raw_recipe="$raw_dir/$old_id.source"
        if [[ -s "$old_prepared" && ! -e "$old_recipe" ]]; then
            printf '%s\t%s\t%s\n' "$old_download_url" "$old_trim_start" "$old_seconds" > "$old_recipe"
        fi
        if [[ -s "$old_raw" && ! -e "$old_raw_recipe" ]]; then
            printf '%s\t%s\n' "$old_download_url" "$_cache_source" > "$old_raw_recipe"
        fi
    done < "$prepared_dir/SOURCES.tsv"
fi

prepare_one() {
    local id=$1
    local seconds=$2
    local trim_start=$3
    local download_url=$4
    local cache_source=$5
    local raw_path="$raw_dir/$id.media"
    local raw_recipe_path="$raw_dir/$id.source"
    local prepared_path="$prepared_dir/$id.wav"
    local temporary="$prepared_path.part.wav"
    local recipe_path="$prepared_dir/$id.recipe"
    local recipe_cache_source=$cache_source
    [[ "$recipe_cache_source" == "-" ]] && recipe_cache_source=
    local expected_raw_recipe="$download_url	$recipe_cache_source"
    if [[ "$download_url" == synthetic://* ]]; then
        expected_raw_recipe+="	$synthetic_revision"
    fi
    local expected_recipe="$download_url	$trim_start	$seconds"
    if [[ "$download_url" == synthetic://* ]]; then
        expected_recipe+="	$synthetic_revision"
    fi
    local expected_frames
    expected_frames=$(awk -v seconds="$seconds" 'BEGIN { printf "%.0f", seconds * 48000 }')

    local actual_raw_recipe=
    if [[ -s "$raw_recipe_path" ]]; then
        actual_raw_recipe=$(<"$raw_recipe_path")
    fi
    if [[ ! -s "$raw_path" || "$actual_raw_recipe" != "$expected_raw_recipe" ]]; then
        if [[ "$download_url" == synthetic://* ]]; then
            echo "generate $id" >&2
            "$synthetic_generator" "$id" "$raw_path.part"
        elif [[ -n "$cache_source" && -s "$project_dir/$cache_source" ]]; then
            echo "reuse cached source $id" >&2
            cp --reflink=auto "$project_dir/$cache_source" "$raw_path.part"
        else
            echo "download $id" >&2
            local downloaded=0
            local attempt
            for attempt in 1 2 3 4 5; do
                if curl --fail --location --silent --show-error \
                    --user-agent 'conv9-windowed-convolution/0.1 (license-tracked offline DSP project)' \
                    --output "$raw_path.part" "$download_url"; then
                    downloaded=1
                    break
                fi
                echo "$id: attempt $attempt failed" >&2
                sleep $((attempt * 5))
            done
            if ((downloaded == 0)); then
                echo "$id: download failed after 5 attempts" >&2
                return 1
            fi
        fi
        mv "$raw_path.part" "$raw_path"
        printf '%s\n' "$expected_raw_recipe" > "$raw_recipe_path.part"
        mv "$raw_recipe_path.part" "$raw_recipe_path"
    fi

    local source_duration
    source_duration=$(ffprobe -v error -show_entries format=duration \
        -of default=nw=1:nk=1 "$raw_path")
    if ! awk -v duration="$source_duration" -v start="$trim_start" -v seconds="$seconds" \
        'BEGIN { exit !(duration > 60 && duration + 0.001 >= start + seconds) }'; then
        echo "$id: source is ${source_duration}s and cannot provide ${seconds}s from ${trim_start}s" >&2
        return 1
    fi

    local actual_frames=
    local actual_recipe=
    if [[ -s "$prepared_path" ]]; then
        actual_frames=$(ffprobe -v error -select_streams a:0 \
            -show_entries stream=duration_ts -of default=nw=1:nk=1 "$prepared_path")
    fi
    if [[ -s "$recipe_path" ]]; then
        actual_recipe=$(<"$recipe_path")
    fi
    if [[ "$actual_frames" != "$expected_frames" || "$actual_recipe" != "$expected_recipe" ]]; then
        local fade_out
        fade_out=$(awk -v seconds="$seconds" 'BEGIN { printf "%.6f", seconds - 0.02 }')
        echo "prepare $id (${seconds}s at 48 kHz mono)" >&2
        ffmpeg -nostdin -hide_banner -loglevel error -y \
            -ss "$trim_start" -t "$seconds" -i "$raw_path" -vn \
            -af "aresample=48000,highpass=f=15,lowpass=f=21000,afade=t=in:st=0:d=0.02,afade=t=out:st=$fade_out:d=0.02,apad,atrim=end_sample=$expected_frames,asetpts=PTS-STARTPTS" \
            -ar 48000 -ac 1 -c:a pcm_s16le "$temporary"
        mv "$temporary" "$prepared_path"
        printf '%s\n' "$expected_recipe" > "$recipe_path.part"
        mv "$recipe_path.part" "$recipe_path"
    fi

    actual_frames=$(ffprobe -v error -select_streams a:0 \
        -show_entries stream=duration_ts -of default=nw=1:nk=1 "$prepared_path")
    if [[ "$actual_frames" != "$expected_frames" ]]; then
        echo "$id: expected $expected_frames frames, found $actual_frames" >&2
        return 1
    fi
}

active_pids=()
cleanup() {
    if ((${#active_pids[@]})); then
        kill "${active_pids[@]}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

wait_for_one() {
    local finished_pid=unknown
    if ! wait -n -p finished_pid "${active_pids[@]}"; then
        echo "sample worker $finished_pid failed" >&2
        return 1
    fi
    local remaining=()
    local pid
    for pid in "${active_pids[@]}"; do
        [[ "$pid" == "$finished_pid" ]] || remaining+=("$pid")
    done
    active_pids=("${remaining[@]}")
}

manifest_count=0
while IFS=$'\t' read -r id _category _kind seconds trim_start _provider _creator _license _license_url _source_page download_url cache_source; do
    [[ "$id" == "id" || -z "$id" ]] && continue
    manifest_count=$((manifest_count + 1))
    prepare_one "$id" "$seconds" "$trim_start" "$download_url" "$cache_source" &
    active_pids+=("$!")
    if ((${#active_pids[@]} >= download_jobs)); then
        wait_for_one
    fi
done < "$manifest"

while ((${#active_pids[@]})); do
    wait_for_one
done
trap - EXIT INT TERM

cp "$manifest" "$prepared_dir/SOURCES.tsv"
find "$raw_dir" -type f -name '*.media' -print0 | sort -z | xargs -0 sha256sum > "$raw_dir/SHA256SUMS"
find "$prepared_dir" -type f -name '*.wav' -print0 | sort -z | xargs -0 sha256sum > "$prepared_dir/SHA256SUMS"
echo "prepared $manifest_count sources in $prepared_dir" >&2
