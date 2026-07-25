#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(cd -- "$script_dir/.." && pwd)
manifest=${1:-"$project_dir/sources.tsv"}
download_jobs=${DOWNLOAD_JOBS:-4}
raw_dir="$project_dir/samples/raw"
prepared_dir="$project_dir/samples/prepared"
mkdir -p "$raw_dir" "$prepared_dir"

prepare_one() {
    local id=$1
    local seconds=$2
    local trim_start=$3
    local download_url=$4
    local cache_source=$5
    local raw_path="$raw_dir/$id.media"
    local prepared_path="$prepared_dir/$id.wav"
    local temporary="$prepared_path.part.wav"
    local expected_frames
    expected_frames=$(awk -v seconds="$seconds" 'BEGIN { printf "%.0f", seconds * 48000 }')

    if [[ ! -s "$raw_path" ]]; then
        if [[ -n "$cache_source" && -s "$project_dir/$cache_source" ]]; then
            echo "reuse cached source $id" >&2
            cp --reflink=auto "$project_dir/$cache_source" "$raw_path"
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
            mv "$raw_path.part" "$raw_path"
        fi
    fi

    local actual_frames=
    if [[ -s "$prepared_path" ]]; then
        actual_frames=$(ffprobe -v error -select_streams a:0 \
            -show_entries stream=duration_ts -of default=nw=1:nk=1 "$prepared_path")
    fi
    if [[ "$actual_frames" != "$expected_frames" ]]; then
        local fade_out
        fade_out=$(awk -v seconds="$seconds" 'BEGIN { printf "%.6f", seconds - 0.02 }')
        echo "prepare $id (${seconds}s at 48 kHz mono)" >&2
        ffmpeg -nostdin -hide_banner -loglevel error -y \
            -ss "$trim_start" -i "$raw_path" -t "$seconds" -vn \
            -af "highpass=f=15,lowpass=f=21000,afade=t=in:st=0:d=0.02,afade=t=out:st=$fade_out:d=0.02" \
            -ar 48000 -ac 1 -c:a pcm_f32le "$temporary"
        mv "$temporary" "$prepared_path"
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

