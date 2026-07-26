#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$project_dir/sources.tsv"
raw_dir="$project_dir/samples/raw"
prepared_dir="$project_dir/samples/prepared"
download_jobs=${DOWNLOAD_JOBS:-2}
preparation_version=conv10-mono-f32-48k-v4

for command in curl ffmpeg ffprobe sha256sum awk; do
    command -v "$command" >/dev/null || {
        echo "missing required command: $command" >&2
        exit 1
    }
done

mkdir -p "$raw_dir" "$prepared_dir"

prepare_one() {
    local id=$1
    local seconds=$2
    local trim_start=$3
    local download_url=$4
    local raw_path="$raw_dir/$id.media"
    local raw_recipe_path="$raw_dir/$id.source"
    local prepared_path="$prepared_dir/$id.wav"
    local raw_temporary="$raw_path.part.$BASHPID"
    local temporary="$prepared_path.part.$BASHPID"
    local recipe_path="$prepared_dir/$id.recipe"
    local expected_raw_recipe=$download_url
    local expected_recipe="$preparation_version	$download_url	$trim_start	$seconds"
    local expected_frames
    local start_frame
    local end_frame

    expected_frames=$(awk -v seconds="$seconds" 'BEGIN { printf "%.0f", seconds*48000 }')
    start_frame=$(awk -v start="$trim_start" 'BEGIN { printf "%.0f", start*48000 }')
    end_frame=$((start_frame + expected_frames))

    local actual_raw_recipe=
    if [[ -s "$raw_recipe_path" ]]; then
        actual_raw_recipe=$(<"$raw_recipe_path")
    fi
    if [[ ! -s "$raw_path" || "$actual_raw_recipe" != "$expected_raw_recipe" ]]; then
        echo "download $id" >&2
        local downloaded=0
        local attempt
        for attempt in 1 2 3 4 5; do
            if curl --fail --location --silent --show-error \
                --user-agent 'conv10-audio-research/0.1 (offline DSP project)' \
                --output "$raw_temporary" "$download_url"; then
                downloaded=1
                break
            fi
            echo "$id: download attempt $attempt failed; retrying shortly" >&2
            sleep $((attempt * 5))
        done
        if ((downloaded == 0)); then
            echo "$id: download failed after 5 attempts" >&2
            return 1
        fi
        mv "$raw_temporary" "$raw_path"
        printf '%s\n' "$expected_raw_recipe" > "$raw_recipe_path.part"
        mv "$raw_recipe_path.part" "$raw_recipe_path"
    fi

    if ! ffmpeg -xerror -nostdin -hide_banner -loglevel error \
        -i "$raw_path" -map 0:a:0 -f null -; then
        echo "$id: downloaded media does not decode end to end" >&2
        return 1
    fi

    local source_duration
    source_duration=$(ffprobe -v error -show_entries format=duration \
        -of default=nw=1:nk=1 "$raw_path")
    if ! awk -v duration="$source_duration" -v start="$trim_start" -v seconds="$seconds" \
        'BEGIN { exit !(duration > 0 && duration + 0.001 >= start + seconds) }'; then
        echo "$id: source is ${source_duration}s and cannot provide ${seconds}s from ${trim_start}s" >&2
        return 1
    fi

    local needs_prepare=0
    local actual_recipe=
    if [[ -s "$recipe_path" ]]; then
        actual_recipe=$(<"$recipe_path")
    fi
    if [[ ! -s "$prepared_path" ]]; then
        needs_prepare=1
    else
        local cached_frames
        cached_frames=$(ffprobe -v error -select_streams a:0 \
            -show_entries stream=duration_ts -of default=nw=1:nk=1 "$prepared_path")
        if [[ "$cached_frames" != "$expected_frames" ]]; then
            echo "rebuild $id: manifest now expects $expected_frames frames" >&2
            needs_prepare=1
        fi
    fi
    if [[ "$actual_recipe" != "$expected_recipe" ]]; then
        needs_prepare=1
    fi

    if ((needs_prepare)); then
        local fade_out
        fade_out=$(awk -v seconds="$seconds" 'BEGIN { value=seconds-0.02; if (value<0) value=0; printf "%.6f", value }')
        echo "prepare $id (${seconds}s)" >&2
        ffmpeg -nostdin -hide_banner -loglevel error -y \
            -i "$raw_path" -vn \
            -af "aresample=48000,asetpts=N/SR/TB,atrim=start_sample=$start_frame:end_sample=$end_frame,asetpts=N/SR/TB,highpass=f=15,lowpass=f=21000,afade=t=in:st=0:d=0.02,afade=t=out:st=$fade_out:d=0.02,apad,atrim=end_sample=$expected_frames,asetpts=N/SR/TB" \
            -ar 48000 -ac 1 -c:a pcm_f32le -f wav "$temporary"
        mv "$temporary" "$prepared_path"
        printf '%s\n' "$expected_recipe" > "$recipe_path.part"
        mv "$recipe_path.part" "$recipe_path"
    fi

    local actual_frames
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
        echo "download or preparation worker $finished_pid failed" >&2
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
while IFS=$'\t' read -r id _category _domain seconds trim_start _provider _creator _source_page download_url; do
    [[ "$id" == "id" || -z "$id" ]] && continue
    manifest_count=$((manifest_count + 1))
    prepare_one "$id" "$seconds" "$trim_start" "$download_url" &
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
find "$raw_dir" -type f -name '*.media' -print0 \
    | sort -z \
    | xargs -0 sha256sum > "$raw_dir/SHA256SUMS"
find "$prepared_dir" -type f -name '*.wav' -print0 \
    | sort -z \
    | xargs -0 sha256sum > "$prepared_dir/SHA256SUMS"

echo "prepared $manifest_count manifest clips in $prepared_dir" >&2
