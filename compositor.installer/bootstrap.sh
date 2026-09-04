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
#   Y5_RELEASE_TAG   release tag to pin        (default: EMPTY = the newest stable release;
#                                              e.g. v1.5.0, or bundles-rolling for the RC channel)
#   Y5_RELEASE_BASE  full asset base URL       (default: derived from repo + tag, see below)
#   Y5_DISTRO        force the distro dir       (e.g. debian-13; skips detection)
#   Y5_ARCH          force the arch            (x86_64 | aarch64; skips uname)
#   Y5_NIX_BUNDLE    glibc bundle to use on NixOS (default: fedora-44)
#   Y5_INSTALL_ARGS  extra args to y5-install  (e.g. --dry-run)
#   --list           print the available <distro> names and exit
set -euo pipefail

REPO="${Y5_RELEASE_REPO:-y5-snowies/nourish}"

# The DEFAULT is the newest STABLE release, expressed as GitHub's `releases/latest/`
# redirect rather than a tag: that pointer follows whatever release is marked "Latest"
# (docs.yml passes `--latest` when it publishes `v<version>`), so it can never go stale and
# needs no tag baked in here.
#
# It must NOT default to a rolling tag. `bundles-rolling` is recreated on every push to
# `candidate` and holds release CANDIDATES — pointing the public one-liner at it would ship
# every visitor an RC. Rolling stays opt-in: `Y5_RELEASE_TAG=bundles-rolling`.
TAG="${Y5_RELEASE_TAG:-}"
if [ -n "$TAG" ]; then
    BASE="${Y5_RELEASE_BASE:-https://github.com/$REPO/releases/download/$TAG}"
    CHANNEL="tag $TAG"
else
    BASE="${Y5_RELEASE_BASE:-https://github.com/$REPO/releases/latest/download}"
    CHANNEL="latest stable"
fi

# The distro dirs the multiarch pipeline builds (must match the `bundles` matrix in
# .github/workflows/docs.yml and the `bundles` matrix in .github/workflows/release-rc.yml).
#
# ubuntu-24.04 and debian-12 are deliberately absent: they ship libinput 1.25.0 / 1.22.x and the
# compositor needs >= 1.26 for the tablet-pad dial symbols, so it does not link there. Hosts on
# those versions land in the fallback below.
KNOWN="fedora-43 fedora-44 debian-13 ubuntu-26.04 arch"

say()  { printf '\033[1;36m::\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }

case "${1:-}" in
    --list) printf '%s\n' $KNOWN; exit 0 ;;
esac

need curl; need tar; need sha256sum; need uname

# Rootless, and checked HERE rather than left to the installer.
#
# `y5-install` refuses uid 0 already, and must: per-action `sudo` covers the steps that
# need privilege, while `$HOME` has to be the real user's or `settings.json` and the user
# systemd units land in /root and `systemctl --user` talks to root's manager instead of
# theirs. But that refusal comes after this script has downloaded, verified and unpacked a
# bundle — as root, into a root-owned temp dir. Checking before the first `curl` turns a
# late failure into an immediate one and leaves nothing behind.
if [ "$(id -u)" -eq 0 ]; then
    die "do not run this as root or with sudo.
Run it as your normal user — the installer invokes sudo itself only for the steps that
need root, so your configuration lands in \$HOME/.config, not /root."
fi

# uname -m -> the arch token used in the asset name.
detect_arch() {
    case "$(uname -m)" in
        x86_64 | amd64)  echo x86_64 ;;
        aarch64 | arm64) echo aarch64 ;;
        *) echo "" ;;
    esac
}

# Newest bundle of family $1 that is NOT NEWER than host version $2, or empty if the host
# predates every bundle we ship for that family.
#
# Direction matters. A bundle is dynamically linked against its own distro's glibc, so a host
# can run an OLDER bundle (its glibc is a superset) but never a NEWER one. Falling back to the
# family's newest unconditionally — which is what this used to do — hands an Ubuntu 24.04 host
# the 26.04 bundle, and every binary in it dies with `GLIBC_2.4x not found`. Now that
# ubuntu-24.04 and debian-12 are no longer built (libinput < 1.26, see KNOWN), that fallback
# is exactly the path those hosts take, so it has to pick downwards or not at all.
family_fallback() {
    local fam="$1" host="$2" best="" k v
    for k in $KNOWN; do
        case "$k" in "$fam"-*) v="${k#"$fam"-}" ;; *) continue ;; esac
        # Skip anything newer than the host: min(v,host) != v means v > host.
        if [ -n "$host" ] && [ "$(printf '%s\n%s\n' "$v" "$host" | sort -V | head -n1)" != "$v" ]; then
            continue
        fi
        if [ -z "$best" ] || [ "$(printf '%s\n%s\n' "$v" "$best" | sort -V | tail -n1)" = "$v" ]; then
            best="$v"
        fi
    done
    [ -n "$best" ] && echo "$fam-$best"
    # Always succeed: the caller assigns this in a command substitution, and under `set -e` a
    # non-zero return would abort the script before it can report the real problem.
    return 0
}

# /etc/os-release ID(+VERSION_ID) -> a <distro> dir from KNOWN. Exact match preferred; an
# unrecognized version of a known family falls back to the newest bundle that is not newer
# than the host (see family_fallback) with a warning, and dies if there is none. `nixos` is
# returned verbatim for the special path. Empty when nothing matches (caller asks for Y5_DISTRO).
detect_distro() {
    [ -r /etc/os-release ] || { echo ""; return; }
    . /etc/os-release
    local id="${ID:-}" ver="${VERSION_ID:-}" like="${ID_LIKE:-}" pick=""
    # Try the exact <id>-<version> first for the versioned families.
    case "$id" in
        fedora|debian|ubuntu)
            if in_known "$id-$ver"; then echo "$id-$ver"; return; fi
            pick="$(family_fallback "$id" "$ver")"
            [ -n "$pick" ] || die "no bundle is compatible with $id $ver — every bundle we build for $id is newer, and its binaries need a newer glibc than your system provides. Upgrade $id, or build from source: https://github.com/$REPO"
            warn "no $id-$ver bundle; using the closest older one ($pick)"
            echo "$pick"
            return
            ;;
    esac
    case "$id" in
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
        || die "download failed: $BASE/$asset  (does this distro/arch combo exist in the $CHANNEL release?)"

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

say "target: $DISTRO ($ARCH), $CHANNEL"
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
