#!/usr/bin/env bash
set -euo pipefail

app_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd -- "$app_dir/.." && pwd)"
tauri_dir="$app_dir/src-tauri"
fallback_sysroot="${CONV9_TAURI_SYSROOT:-/tmp/conv9-tauri-devel}"
manifest="${CONV9_MANIFEST:-$project_dir/sources.tsv}"
input_dir="${CONV9_INPUT_DIR:-$project_dir/samples/prepared}"

if [[ ! -f "$manifest" ]]; then
  echo "conv9 source manifest is missing: $manifest" >&2
  exit 1
fi

missing_count=0
first_missing=
while IFS=$'\t' read -r id _rest; do
  [[ "$id" == "id" || -z "$id" ]] && continue
  if [[ ! -s "$input_dir/$id.wav" ]]; then
    ((missing_count += 1))
    [[ -n "$first_missing" ]] || first_missing="$input_dir/$id.wav"
  fi
done < "$manifest"

if ((missing_count > 0)); then
  cat >&2 <<EOF
conv9 cannot start: $missing_count prepared source WAVs are missing.
First missing file: $first_missing

Prepare the CC0 source library, then launch again:
  cd "$project_dir"
  ./scripts/download_samples.sh
EOF
  exit 1
fi

if ! pkg-config --exists glib-2.0 gtk+-3.0 webkit2gtk-4.1 2>/dev/null; then
  if [[ -f "$fallback_sysroot/usr/lib64/pkgconfig/webkit2gtk-4.1.pc" ]]; then
    export PKG_CONFIG_PATH="$fallback_sysroot/usr/lib64/pkgconfig:$fallback_sysroot/usr/share/pkgconfig"
    export PKG_CONFIG_SYSROOT_DIR="$fallback_sysroot"
    export LIBRARY_PATH="$fallback_sysroot/usr/lib64${LIBRARY_PATH:+:$LIBRARY_PATH}"
    export LD_LIBRARY_PATH="$fallback_sysroot/usr/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  else
    cat >&2 <<'EOF'
conv9 needs the Tauri Linux development libraries.

Fedora:
  sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file \
    libappindicator-gtk3-devel librsvg2-devel libxdo-devel
  sudo dnf group install "c-development"

Then run this script again. See https://v2.tauri.app/start/prerequisites/
EOF
    exit 1
  fi
fi

cd "$tauri_dir"
exec cargo run "$@"
