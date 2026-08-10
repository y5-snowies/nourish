#!/usr/bin/env bash
# Install the shipped example shader bundles into the compositor's data dir.
#
# Usage: ./install-shaders.sh [--dry-run] [--prune] [BUNDLE ...]
#   (no args)  install every bundle in THIS folder
#   BUNDLE ... install only the named bundles (folder names, e.g. tb-crt-spin)
#   --dry-run  list what would change and exit non-destructively
#   --prune    also DELETE installed bundles that no longer exist in the tree
#
# WHY THIS EXISTS
# ---------------
# The compositor loads bundles from `<data>/background/shader/<name>/` — a plain
# directory, not the repo — so a bundle edited in the tree has NO effect until it
# is copied over. That is fine until the engine and the bundles share an ABI, at
# which point a stale copy stops being "an old shader" and becomes a silently
# WRONG one: `struct Windows` restates the engine's array length, so a bundle
# built against a smaller one reads `srcs` at the wrong byte offset, finds rect
# data there, and paints every window with garbage crop coordinates. Nothing
# errors. It looks like the shader broke.
#
# Copying by hand is what let that happen once already, so it gets a script.
#
# NOT the `mp-*` set. Those ship compiled into the binary (see the `shader.embed`
# crate) and are selected as `builtin:mp-<name>`, so they are already current by
# construction and nothing here needs to touch them. Installing a copy would in
# fact be worse than useless: it would appear in the picker a SECOND time, under
# its own heading, as a folder bundle that can then go stale.
#
# The destination mirrors `system.persist/path.base::data_dir()` — XDG_DATA_HOME,
# else ~/.local/share, else /tmp — plus `y5`. Keep the two in step; this is the
# only other place that rule is written down.
set -euo pipefail

# The bundles are the script's own siblings — it lives in the tree it installs,
# so there is no path between the two to keep correct.
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${XDG_DATA_HOME:-${HOME:-/tmp}/.local/share}/y5/background/shader"

DRY=0
PRUNE=0
NAMES=()
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY=1 ;;
    --prune)   PRUNE=1 ;;
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*)        echo "install-shaders: unknown flag $arg" >&2; exit 2 ;;
    *)         NAMES+=("$arg") ;;
  esac
done

[ -d "$SRC" ] || { echo "install-shaders: no bundles at $SRC" >&2; exit 1; }

# Nothing named -> every directory holding a bundle. A bundle is a directory; the
# loose MATRIX.md / README.md at the root are documentation and are copied too,
# because the installed tree is what someone reads when a bundle misbehaves.
if [ ${#NAMES[@]} -eq 0 ]; then
  while IFS= read -r d; do NAMES+=("$(basename "$d")"); done \
    < <(find "$SRC" -mindepth 1 -maxdepth 1 -type d | sort)
fi

echo "install-shaders: $SRC -> $DEST"
[ "$DRY" -eq 1 ] && echo "  (dry run — nothing will be written)"

changed=0
for name in "${NAMES[@]}"; do
  from="$SRC/$name"
  to="$DEST/$name"
  if [ ! -d "$from" ]; then
    echo "  !! $name: no such bundle in the tree" >&2
    exit 1
  fi
  # `diff -rq` rather than a timestamp check: a hand-edited installed copy is
  # exactly the case worth reporting, and mtimes lie after a checkout.
  if [ -d "$to" ] && diff -rq "$from" "$to" >/dev/null 2>&1; then
    continue
  fi
  status="update"
  [ -d "$to" ] || status="new"
  echo "  $status $name"
  changed=$((changed + 1))
  if [ "$DRY" -eq 0 ]; then
    # Replace rather than merge: a file DELETED from a bundle must not survive in
    # the installed copy, where it would keep being `#import`ed or read.
    rm -rf "$to"
    mkdir -p "$to"
    cp -a "$from/." "$to/"
  fi
done

# Loose top-level docs (MATRIX.md, README.md) — only when installing everything.
# Not this script: it is a sibling of the bundles now, and the data dir is a
# place the compositor READS bundles from, not somewhere to leave a copy of the
# installer that put them there.
if [ ${#NAMES[@]} -gt 1 ] && [ "$DRY" -eq 0 ]; then
  mkdir -p "$DEST"
  find "$SRC" -mindepth 1 -maxdepth 1 -type f \
    ! -name "$(basename "${BASH_SOURCE[0]}")" -exec cp -a {} "$DEST/" \;
fi

if [ "$PRUNE" -eq 1 ]; then
  while IFS= read -r d; do
    name="$(basename "$d")"
    if [ ! -d "$SRC/$name" ]; then
      echo "  prune $name"
      changed=$((changed + 1))
      [ "$DRY" -eq 0 ] && rm -rf "$d"
    fi
  done < <(find "$DEST" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort)
fi

if [ "$changed" -eq 0 ]; then
  echo "  already in sync"
else
  echo "  $changed bundle(s) $([ "$DRY" -eq 1 ] && echo "would change" || echo "installed")"
fi
