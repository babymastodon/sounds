#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
jobs=${CONV_JOBS:-$(getconf _NPROCESSORS_ONLN)}
prepare_jobs=${DOWNLOAD_JOBS:-$jobs}

if (($# == 0)); then
    set -- \
        "$project_dir/lists/conv10.txt" \
        "$project_dir/lists/conv8.txt"
fi

exec "$project_dir/scripts/batch.py" \
    --jobs "$jobs" \
    --prepare-jobs "$prepare_jobs" \
    "$@"
