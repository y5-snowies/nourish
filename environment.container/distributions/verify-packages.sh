#!/usr/bin/env bash
# Pre-CI gate: verify the installer's per-distro runtime package NAMES resolve against each
# distro's real repos — WITHOUT the heavy native build (no cargo, no Tauri; seconds, not hours).
#
# The multiarch CI builds every component natively per distro but NEVER runs the installer, so a
# wrong package name (a t64 rename, a soversion suffix, an Arch vendor split) wouldn't surface
# there. This script closes that gap cheaply: it reuses each Containerfile's pinned base image
# (its `FROM` line — nothing is built) and, inside it, refreshes the package index and checks
# that every name the installer would install actually exists in that distro's repos.
#
# The name list comes from the installer itself (`y5-install --emit-packages=<mgr>[:<rel>]`), so
# this validates the SAME table the real install uses — single source of truth.
#
# Usage: ./verify-packages.sh [distro ...]     (default: the multiarch-published set)
#   Needs: podman (same prerequisite as the rest of distributions/). Builds y5-install once
#   (tiny, std-only) unless Y5_INSTALL_BIN points at a prebuilt one.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
DISTROS=("${@:-}")
[ -n "${DISTROS[0]:-}" ] || DISTROS=(fedora-44 debian-12 debian-13 ubuntu-24.04 ubuntu-26.04 arch nixos)

command -v podman >/dev/null || { echo "verify-packages: needs podman" >&2; exit 1; }

# Build (once) the installer that emits the package names — cheap: pure-std crates, no vendor graph.
INSTALL_BIN="${Y5_INSTALL_BIN:-}"
if [ -z "$INSTALL_BIN" ]; then
    echo ">> building y5-install (for --emit-packages) ..." >&2
    ( cd "$REPO_ROOT/compositor.installer/installer.process" && cargo build -q )
    INSTALL_BIN="$REPO_ROOT/compositor.installer/installer.process/target/debug/y5-install"
fi
[ -x "$INSTALL_BIN" ] || { echo "verify-packages: no installer binary at $INSTALL_BIN" >&2; exit 1; }

# distro dir -> package manager + emit spec (mgr[:release]).
emit_spec() {
    case "$1" in
        fedora-*)  echo "dnf" ;;
        debian-11) echo "apt:11" ;;
        debian-12) echo "apt:12" ;;
        debian-13) echo "apt:13" ;;
        ubuntu-*)  echo "apt:${1#ubuntu-}" ;;
        arch)      echo "pacman" ;;
        nixos|nix) echo "nix" ;;
        *) echo "" ;;
    esac
}

# NixOS is not a build target here (nix-ld runs a glibc bundle), so there's no Containerfile
# base image. Instead we check that every nixpkgs ATTRIBUTE the installer would emit into the
# nix-ld profile actually exists — one evaluate-only `nix eval` (NO build) inside the nixos/nix
# image, against nixpkgs-unstable. `builtins.hasAttr` doesn't force the derivation, so unfree /
# broken packages don't false-negative. Returns nonzero if any attr is missing.
check_nix() {
    local bin="$1" img="${Y5_NIX_IMAGE:-docker.io/nixos/nix:latest}"
    local names expr missing
    names="$("$bin" --emit-packages=nix)"
    echo
    echo "==================================================================="
    echo ">> nixos    image=$img   emit=nix   ($(echo "$names" | grep -c .) attrs, nixpkgs-unstable)"
    echo "==================================================================="
    # Build one expression: filter the attr list down to the ones NOT present in nixpkgs.
    expr='let p = (builtins.getFlake "github:NixOS/nixpkgs/nixpkgs-unstable").legacyPackages.${builtins.currentSystem}; in builtins.filter (a: ! (builtins.hasAttr a p)) ['
    while read -r n; do [ -n "$n" ] && expr+=" \"$n\""; done <<<"$names"
    expr+=' ]'
    if missing="$(podman run --rm "$img" \
            nix eval --impure --json --extra-experimental-features 'nix-command flakes' \
                --expr "$expr")"; then
        if [ "$missing" = "[]" ]; then echo "  ALL nixpkgs ATTRS RESOLVE"; return 0
        else echo "  MISSING attrs: $missing"; return 1; fi
    fi
    echo "!! nixos: nix eval failed (see errors above)" >&2
    return 1
}

# The in-container checker: reads package names on stdin (one per line) and reports any that the
# distro's repos don't carry. $1 = manager, $2 = distro (for the debian-12 backports special-case).
# Runs as the base image's default (root) user; only reads metadata, installs nothing.
container_check() {
    cat <<'CHECK'
set -eu
mgr="$1"; distro="${2:-}"
case "$mgr" in
  apt)
    export DEBIAN_FRONTEND=noninteractive
    # libdisplay-info2 lives in bookworm-backports on Debian 12 (the installer enables it) — mirror that.
    if [ "$distro" = debian-12 ]; then
      echo 'deb http://deb.debian.org/debian bookworm-backports main' > /etc/apt/sources.list.d/backports.list
    fi
    apt-get update -qq
    check() { apt-cache show "$1" >/dev/null 2>&1; } ;;
  pacman)
    pacman -Sy --noconfirm >/dev/null 2>&1
    check() { pacman -Si "$1" >/dev/null 2>&1; } ;;
  dnf)
    dnf -q makecache >/dev/null 2>&1 || true
    check() { dnf -q info "$1" >/dev/null 2>&1; } ;;
esac
missing=0
while read -r pkg; do
  [ -n "$pkg" ] || continue
  if check "$pkg"; then printf '  ok   %s\n' "$pkg"
  else printf '  MISS %s\n' "$pkg"; missing=$((missing+1)); fi
done
echo "---"
[ "$missing" -eq 0 ] && echo "ALL PACKAGES RESOLVE" || { echo "$missing PACKAGE(S) MISSING"; exit 1; }
CHECK
}

overall=0
for distro in "${DISTROS[@]}"; do
    # NixOS has no Containerfile — it's checked by nixpkgs-attr evaluation, not a base image.
    if [ "$distro" = nixos ] || [ "$distro" = nix ]; then
        check_nix "$INSTALL_BIN" || { echo "!! nixos: verification FAILED" >&2; overall=1; }
        continue
    fi
    cf="$HERE/$distro/Containerfile"
    [ -f "$cf" ] || { echo "!! $distro: no Containerfile — skipping" >&2; overall=1; continue; }
    spec="$(emit_spec "$distro")"; mgr="${spec%%:*}"
    base="$(awk '/^FROM/ {print $2; exit}' "$cf")"
    [ -n "$spec" ] && [ -n "$base" ] || { echo "!! $distro: cannot map spec/base image" >&2; overall=1; continue; }

    echo
    echo "==================================================================="
    echo ">> $distro   base=$base   emit=$spec"
    echo "==================================================================="
    # The package names flow in on the container's stdin (the pipe); the checker script is
    # passed as a `bash -c` argument so it doesn't consume that stdin (`_` fills $0, then $1/$2).
    if "$INSTALL_BIN" --emit-packages="$spec" \
        | podman run --rm -i "$base" bash -c "$(container_check)" _ "$mgr" "$distro"; then
        :
    else
        # Distinguish a real missing-package failure from podman/pull errors, both surfaced above.
        echo "!! $distro: verification FAILED" >&2
        overall=1
    fi
done

echo
[ "$overall" -eq 0 ] && echo "✔ all distros: package names resolve" \
    || { echo "✘ some distros have unresolved package names (see above)"; }
exit "$overall"
