#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
jobs=${CONV_JOBS:-$(getconf _NPROCESSORS_ONLN)}
prepare_jobs=${DOWNLOAD_JOBS:-$jobs}

if (($# == 0)); then
    set -- \
        "$project_dir/lists/fieldatlas.txt" \
        "$project_dir/lists/melodyworks.txt" \
        "$project_dir/lists/drift.txt" \
        "$project_dir/lists/menagerie.txt" \
        "$project_dir/lists/passage.txt" \
        "$project_dir/lists/foundry.txt" \
        "$project_dir/lists/commons.txt" \
        "$project_dir/lists/sonora.txt" \
        "$project_dir/lists/signals.txt" \
        "$project_dir/lists/tempest.txt" \
        "$project_dir/lists/wildwire.txt" \
        "$project_dir/lists/tideforge.txt" \
        "$project_dir/lists/stormfolk.txt" \
        "$project_dir/lists/railchime.txt"
fi

exec "$project_dir/scripts/batch.py" \
    --jobs "$jobs" \
    --prepare-jobs "$prepare_jobs" \
    "$@"
