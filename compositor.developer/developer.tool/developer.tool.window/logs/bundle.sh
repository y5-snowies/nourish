#!/usr/bin/env bash
# Bundle the y5 log viewer (Tauri 2 app) into a distributable executable.
#
# Usage: ./bundle.sh [appimage|rpm|deb|all|none]
#   appimage  (default) -> single self-contained portable executable (*.AppImage)
#   rpm                 -> Fedora/RHEL package
#   deb                 -> Debian/Ubuntu package
#   all                 -> every Linux bundle target Tauri supports here
#   none                -> just the bare release binary, no installer/bundle
#
# Bundling needs the system GUI libs from ./setup.sh and, for appimage, network
# access (Tauri downloads the AppImage runtime + linuxdeploy on first run).
#
# Output:
#   bare binary -> src-tauri/target/release/compositor-developer-tool
#   bundles     -> src-tauri/target/release/bundle/<format>/
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

TARGET="${1:-rpm}"

# Frontend deps (esbuild bundle is produced by `npm run build`, which tauri's
# beforeBuildCommand invokes).
[ -d node_modules ] || npm install

case "$TARGET" in
    none)  npm run tauri build -- --no-bundle ;;
    all)   npm run tauri build -- --bundles rpm ;;
    appimage|rpm|deb)
           npm run tauri build -- --bundles "$TARGET" ;;
    *) echo "unknown target '$TARGET' (expected appimage|rpm|deb|all|none)" >&2; exit 1 ;;
esac

# Where cargo (and so tauri) actually wrote it: `.cargo/config.toml` pins one target
# dir for the whole repo, so this is NOT src-tauri/target/ any more.
OUT="$( cd src-tauri && cargo metadata --no-deps --format-version 1 \
          | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' )/release"

echo
echo "Done. Artifacts under $OUT"
if [ "$TARGET" = "none" ]; then
    echo "  bare binary: $OUT/compositor-developer-tool"
else
    find "$OUT/bundle" -maxdepth 2 -type f \
        \( -name '*.AppImage' -o -name '*.rpm' -o -name '*.deb' \) 2>/dev/null \
        | sed 's/^/  /' || true
fi
