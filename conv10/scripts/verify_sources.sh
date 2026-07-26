#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
manifest=${1:-"$project_dir/sources.tsv"}
jobs=${SOURCE_CHECK_JOBS:-8}

for command in curl rg awk mktemp; do
    command -v "$command" >/dev/null || {
        echo "missing required command: $command" >&2
        exit 1
    }
done

if ! [[ "$jobs" =~ ^[1-9][0-9]*$ ]]; then
    echo "SOURCE_CHECK_JOBS must be a positive integer" >&2
    exit 1
fi

scratch_dir=$(mktemp -d)
cleanup() {
    rm -rf -- "$scratch_dir"
}
trap cleanup EXIT INT TERM

check_one() {
    local id=$1
    local source_page=$2
    local download_url=$3
    local page_path="$scratch_dir/$id.html"

    curl --fail --location --silent --show-error --max-time 45 \
        --retry 3 --retry-delay 2 \
        --user-agent 'conv10-source-check/0.1' \
        --output "$page_path" "$source_page"

    rg --fixed-strings --quiet "$download_url" "$page_path" || {
        echo "$id: source page does not contain the declared media URL" >&2
        return 1
    }
    printf '%s\tpass\n' "$id"
}

export scratch_dir
export -f check_one

tail -n +2 "$manifest" \
    | awk -F '\t' '{print $1 "\t" $8 "\t" $9}' \
    | xargs -d '\n' -P "$jobs" -I '{}' bash -c '
        IFS=$'\''\t'\'' read -r id source_page download_url <<< "$1"
        check_one "$id" "$source_page" "$download_url"
    ' _ '{}'

echo "verified every source page in $manifest" >&2
