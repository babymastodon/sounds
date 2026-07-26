#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
jobs=${CONV_JOBS:-$(getconf _NPROCESSORS_ONLN)}

cd "$project_dir"
./scripts/download_samples.sh
cargo run --release -- render --jobs "$jobs"
cargo run --release -- verify --jobs "$jobs"
cargo run --release -- concat
