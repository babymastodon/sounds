#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$project_dir/sources.tsv"
raw_dir="$project_dir/samples/raw"
prepared_dir="$project_dir/samples/prepared"
download_jobs=${DOWNLOAD_JOBS:-2}

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
    local prepared_path="$prepared_dir/$id.wav"
    local temporary="$prepared_path.part.wav"
    local expected_frames

    expected_frames=$(awk -v seconds="$seconds" 'BEGIN { printf "%.0f", seconds*48000 }')

    if [[ ! -s "$raw_path" ]]; then
        echo "download $id" >&2
        local downloaded=0
        local attempt
        for attempt in 1 2 3 4 5; do
            if curl --fail --location --silent --show-error \
                --user-agent 'conv3-audio-research/0.1 (license-tracked offline DSP project)' \
                --output "$raw_path.part" "$download_url"; then
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
        mv "$raw_path.part" "$raw_path"
    fi

    local needs_prepare=0
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

    if ((needs_prepare)); then
        local fade_out
        fade_out=$(awk -v seconds="$seconds" 'BEGIN { value=seconds-0.02; if (value<0) value=0; printf "%.6f", value }')
        echo "prepare $id (${seconds}s)" >&2
        ffmpeg -nostdin -hide_banner -loglevel error -y \
            -ss "$trim_start" -i "$raw_path" -t "$seconds" -vn \
            -af "highpass=f=15,lowpass=f=21000,afade=t=in:st=0:d=0.02,afade=t=out:st=$fade_out:d=0.02" \
            -ar 48000 -ac 1 -c:a pcm_f32le "$temporary"
        mv "$temporary" "$prepared_path"
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

manifest_count=0
while IFS=$'\t' read -r id _category _domain seconds trim_start _provider _creator _license _license_url _source_page download_url; do
    [[ "$id" == "id" || -z "$id" ]] && continue
    manifest_count=$((manifest_count + 1))
    prepare_one "$id" "$seconds" "$trim_start" "$download_url" &
    active_pids+=("$!")
    if ((${#active_pids[@]} >= download_jobs)); then
        wait "${active_pids[0]}"
        active_pids=("${active_pids[@]:1}")
    fi
done < "$manifest"

for pid in "${active_pids[@]}"; do
    wait "$pid"
done
active_pids=()
trap - EXIT INT TERM

cp "$manifest" "$prepared_dir/SOURCES.tsv"
find "$raw_dir" -type f -name '*.media' -print0 \
    | sort -z \
    | xargs -0 sha256sum > "$raw_dir/SHA256SUMS"

echo "prepared $manifest_count manifest clips in $prepared_dir" >&2
