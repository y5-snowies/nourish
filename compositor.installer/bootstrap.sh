#!/usr/bin/env bash
# Universal, distro-aware bootstrap for the y5 Wayland compositor.
#
# Unlike get.sh (which always pulls the Fedora bundle), this detects your distro + CPU arch,
# downloads the MATCHING per-distro bundle from the multiarch release, verifies it, and runs
# its installer. On NixOS it doesn't install imperatively — it fetches a glibc bundle and runs
# its nixos-setup.sh, which prints the nix-ld module/flake to add.
#
# Host it at a stable URL so users can run:
#     curl -fsSL https://nourish.snowies.com/install | bash
#
# The bundles live as GitHub Release assets: package-<distro>-<arch>.tar.gz next to a single
# SHA256SUMS, under a release tag. Verification against SHA256SUMS is MANDATORY (fail-closed) —
# there is no skip switch: a bundle whose checksum can't be confirmed is never installed.
#
# Overridable via env:
#   Y5_RELEASE_REPO  GitHub owner/repo         (default: y5-snowies/nourish)
#   Y5_RELEASE_TAG   release tag               (default: bundles-rolling; pin e.g. bundles-v1.4.1-rc.2)
#   Y5_RELEASE_BASE  full asset base URL       (default: https://github.com/<repo>/releases/download/<tag>)
#   Y5_DISTRO        force the distro dir       (e.g. debian-13; skips detection)
#   Y5_ARCH          force the arch            (x86_64 | aarch64; skips uname)
#   Y5_NIX_BUNDLE    glibc bundle to use on NixOS (default: fedora-44)
#   Y5_INSTALL_ARGS  extra args to y5-install  (e.g. --dry-run)
#   --list           print the available <distro> names and exit
set -euo pipefail

REPO="${Y5_RELEASE_REPO:-y5-snowies/nourish}"
TAG="${Y5_RELEASE_TAG:-bundles-rolling}"
BASE="${Y5_RELEASE_BASE:-https://github.com/$REPO/releases/download/$TAG}"

# The distro dirs the multiarch pipeline builds (must match .github/workflows/multiarch-publish.yml).
KNOWN="fedora-43 fedora-44 debian-12 debian-13 ubuntu-24.04 ubuntu-26.04 arch"

say()  { printf '\033[1;36m::\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }

case "${1:-}" in
    --list) printf '%s\n' $KNOWN; exit 0 ;;
esac

need curl; need tar; need sha256sum; need uname

# uname -m -> the arch token used in the asset name.
detect_arch() {
    case "$(uname -m)" in
        x86_64 | amd64)  echo x86_64 ;;
        aarch64 | arm64) echo aarch64 ;;
        *) echo "" ;;
    esac
}

# /etc/os-release ID(+VERSION_ID) -> a <distro> dir from KNOWN. Exact match preferred; an
# unrecognized version of a known family falls back to that family's NEWEST bundle (host glibc
# is typically >= the bundle's, so forward-compat holds) with a warning. `nixos` is returned
# verbatim for the special path. Empty when nothing matches (caller asks for Y5_DISTRO).
detect_distro() {
    [ -r /etc/os-release ] || { echo ""; return; }
    . /etc/os-release
    local id="${ID:-}" ver="${VERSION_ID:-}" like="${ID_LIKE:-}"
    # Try the exact <id>-<version> first for the versioned families.
    case "$id" in
        fedora|debian|ubuntu)
            if in_known "$id-$ver"; then echo "$id-$ver"; return; fi
            ;;
    esac
    case "$id" in
        fedora)  warn "no fedora-$ver bundle; using the newest (fedora-44)"; echo fedora-44 ;;
        debian)  warn "no debian-$ver bundle; using the newest (debian-13)"; echo debian-13 ;;
        ubuntu)  warn "no ubuntu-$ver bundle; using the newest (ubuntu-26.04)"; echo ubuntu-26.04 ;;
        arch|archarm|manjaro|endeavouros) echo arch ;;
        nixos)   echo nixos ;;
        *)
            case " $like " in
                *" debian "*|*" ubuntu "*) warn "unknown distro '$id'; using debian-13 via ID_LIKE"; echo debian-13 ;;
                *" fedora "*|*" rhel "*)   warn "unknown distro '$id'; using fedora-44 via ID_LIKE";  echo fedora-44 ;;
                *" arch "*)                warn "unknown distro '$id'; using arch via ID_LIKE";       echo arch ;;
                *) echo "" ;;
            esac
            ;;
    esac
}

in_known() { case " $KNOWN " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

DISTRO="${Y5_DISTRO:-$(detect_distro)}"
ARCH="${Y5_ARCH:-$(detect_arch)}"
[ -n "$ARCH" ] || die "could not determine CPU arch from '$(uname -m)' — set Y5_ARCH=x86_64|aarch64."
[ -n "$DISTRO" ] || die "could not identify your distro — set Y5_DISTRO to one of: $KNOWN (see --list)."

WORK="$(mktemp -d "${TMPDIR:-/tmp}/y5-install.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# Download package-<combo>.tar.gz + SHA256SUMS from the release, verify (MANDATORY), unpack into
# $WORK. Echoes the unpacked y5-install dir. $1 = <distro>-<arch> combo.
fetch_bundle() {
    local combo="$1" asset="package-$1.tar.gz"
    say "downloading $asset  ($BASE)"
    curl -fSL --proto '=https' "$BASE/$asset" -o "$WORK/$asset" \
        || die "download failed: $BASE/$asset  (does this distro/arch combo exist in $TAG?)"

    # Verify against SHA256SUMS at the same base. No override, no skip: a bundle we can't verify
    # is never installed. SHA256SUMS lists every combo; pick the line for OUR asset by name.
    say "verifying checksum (SHA256SUMS)"
    curl -fsSL --proto '=https' "$BASE/SHA256SUMS" -o "$WORK/SHA256SUMS" \
        || die "could not fetch SHA256SUMS from $BASE — refusing to install unverified."
    local want have
    want="$(awk -v a="$asset" '$2 ~ ("^\\*?" a "$") {print $1; exit}' "$WORK/SHA256SUMS")"
    [ -n "$want" ] || die "SHA256SUMS has no entry for $asset."
    have="$(sha256sum "$WORK/$asset" | awk '{print $1}')"
    [ "$want" = "$have" ] || die "checksum mismatch for $asset (want $want, have $have)."
    say "checksum OK"

    say "unpacking"
    tar -xzf "$WORK/$asset" -C "$WORK"
    local stage="$WORK/y5-install"
    [ -d "$stage" ] || die "bundle $asset did not unpack to ./y5-install/"
    echo "$stage"
}

# NixOS: don't install imperatively (declarative + non-FHS). Fetch a glibc bundle (its binaries
# run via nix-ld) and run its nixos-setup.sh, which prints the nix-ld module/flake to add.
if [ "$DISTRO" = nixos ] || [ "$DISTRO" = nix ]; then
    NIX_BUNDLE="${Y5_NIX_BUNDLE:-fedora-44}"
    say "NixOS detected — fetching the $NIX_BUNDLE ($ARCH) glibc bundle to run under nix-ld"
    STAGE="$(fetch_bundle "$NIX_BUNDLE-$ARCH")"
    [ -x "$STAGE/nixos-setup.sh" ] || die "bundle is missing nixos-setup.sh (rebuild with a newer prepare.sh)."
    say "running nixos-setup.sh (prints the module/flake — installs nothing)"
    exec "$STAGE/nixos-setup.sh"
fi

# Non-Nix: validate the combo, fetch, and run the interactive installer.
in_known "$DISTRO" || die "unknown distro '$DISTRO' — one of: $KNOWN (see --list)."
if [ "$DISTRO" = arch ] && [ "$ARCH" != x86_64 ]; then
    die "the arch bundle is x86_64-only (the official archlinux image has no arm64 build)."
fi

say "target: $DISTRO ($ARCH), release tag $TAG"
STAGE="$(fetch_bundle "$DISTRO-$ARCH")"
[ -x "$STAGE/y5-install" ] || die "bundle is missing the installer ($STAGE/y5-install)."

# The installer is interactive. Under `curl ... | bash` this script's stdin is the pipe, so feed
# the installer the real terminal when one exists.
say "launching installer"
export Y5_INSTALL_STAGE="$STAGE"
# shellcheck disable=SC2086 # word-splitting of optional args is intended
if [ -e /dev/tty ]; then
    exec "$STAGE/y5-install" ${Y5_INSTALL_ARGS:-} < /dev/tty
else
    exec "$STAGE/y5-install" ${Y5_INSTALL_ARGS:-}
fi
