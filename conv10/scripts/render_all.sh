#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
detected_jobs=$(getconf _NPROCESSORS_ONLN)
if ((detected_jobs < 8)); then
    detected_jobs=8
fi
jobs=${CONV_JOBS:-$detected_jobs}
prepare_jobs=${DOWNLOAD_JOBS:-$jobs}
encode_jobs=${ENCODE_JOBS:-$jobs}
assemble_jobs=${ASSEMBLE_JOBS:-$jobs}
finalize_jobs=${FINALIZE_JOBS:-$jobs}

for job_variable in jobs prepare_jobs encode_jobs assemble_jobs finalize_jobs; do
    if ((${!job_variable} < 8)); then
        printf -v "$job_variable" '%d' 8
    fi
done

if (($# == 0)); then
    set -- \
        "$project_dir/configs/fieldatlas.json" \
        "$project_dir/configs/melodyworks.json" \
        "$project_dir/configs/drift.json" \
        "$project_dir/configs/menagerie.json" \
        "$project_dir/configs/passage.json" \
        "$project_dir/configs/foundry.json" \
        "$project_dir/configs/commons.json" \
        "$project_dir/configs/sonora.json" \
        "$project_dir/configs/signals.json" \
        "$project_dir/configs/tempest.json" \
        "$project_dir/configs/wildwire.json" \
        "$project_dir/configs/tideforge.json" \
        "$project_dir/configs/stormfolk.json" \
        "$project_dir/configs/railchime.json"
fi

exec "$project_dir/scripts/batch.py" \
    --jobs "$jobs" \
    --prepare-jobs "$prepare_jobs" \
    --assemble-jobs "$assemble_jobs" \
    --encode-jobs "$encode_jobs" \
    --finalize-jobs "$finalize_jobs" \
    "$@"
