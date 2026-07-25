#!/usr/bin/env bash
set -euo pipefail

app_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tauri_dir="$app_dir/src-tauri"
fallback_sysroot="${CONV9_TAURI_SYSROOT:-/tmp/conv9-tauri-devel}"

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
