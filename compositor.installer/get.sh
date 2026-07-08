#!/usr/bin/env bash
# One-line bootstrap for the y5 Wayland compositor (Fedora 44).
#
# This is the hostable script behind the single install command. Host it at a
# stable URL (e.g. https://nourish.snowies.com/install) so users can run:
#
#     curl -fsSL https://nourish.snowies.com/install | bash
#
# It downloads the prebuilt release tarball, verifies its checksum, unpacks it to
# a temp dir, and launches the interactive installer. Because the tarball ships
# compiled binaries, the installer only pulls the runtime shared libraries — no
# Rust toolchain, no -devel headers.
#
# Overridable via env:
#   Y5_RELEASE_URL  full URL to package.tar.gz
#                   (default: the Fedora 44 "latest" release on nourish.snowies.com)
#   Y5_SUMS_URL     full URL to SHA256SUMS (default: alongside the tarball)
#   Y5_INSTALL_ARGS extra args forwarded to y5-install (e.g. --dry-run)
#   Y5_INSECURE_SKIP_CHECKSUM=1  install even if SHA256SUMS can't be fetched (NOT recommended)
set -euo pipefail

BASE_URL="${Y5_RELEASE_BASE:-https://nourish.snowies.com/release/latest/fedora44}"
URL="${Y5_RELEASE_URL:-$BASE_URL/package.tar.gz}"
SUMS_URL="${Y5_SUMS_URL:-$BASE_URL/SHA256SUMS}"

say()  { printf '\033[1;36m::\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }

need curl
need tar
need sha256sum

# Friendly (non-fatal) distro check. This bootstrap fetches the FEDORA bundle: its binaries
# are dynamically linked to Fedora's system libraries and won't reliably run elsewhere. The
# interactive installer is distro-aware (it detects apt/pacman/dnf and, on NixOS, prints a
# nix-ld profile), but that only helps once you have a bundle built for YOUR distro. Debian/
# Ubuntu/Arch users should grab `package-<distro>-<arch>.tar.gz` from the `bundles-rolling`
# release instead of this Fedora one.
if [ -r /etc/os-release ]; then
    . /etc/os-release
    case "${ID:-}" in
        fedora) : ;;
        *) say "warning: this bootstrap fetches the Fedora bundle, but detected '${ID:-unknown}'." \
                "For a matching build, download package-<distro>-<arch>.tar.gz from the" \
                "multiarch release and run its y5-install/install.sh. Continuing anyway." ;;
    esac
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/y5-install.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

say "downloading $URL"
curl -fSL --proto '=https' "$URL" -o "$WORK/package.tar.gz" \
    || die "download failed: $URL"

# Verify the tarball against the published SHA256SUMS. The release pipeline ALWAYS ships
# SHA256SUMS next to package.tar.gz, so verification is mandatory: a missing/unreachable sums
# file is a hard failure (fail closed), not a silent skip — otherwise anyone who can tamper with
# the tarball can also strip the sums response and bypass the check entirely. Set
# Y5_INSECURE_SKIP_CHECKSUM=1 to deliberately override (NOT recommended).
say "verifying checksum"
if ! curl -fsSL --proto '=https' "$SUMS_URL" -o "$WORK/SHA256SUMS"; then
    [ "${Y5_INSECURE_SKIP_CHECKSUM:-0}" = 1 ] \
        || die "could not fetch checksums from $SUMS_URL — refusing to install unverified" \
               "(set Y5_INSECURE_SKIP_CHECKSUM=1 to override, or check Y5_SUMS_URL)."
    say "warning: Y5_INSECURE_SKIP_CHECKSUM=1 set — installing UNVERIFIED tarball"
else
    # SHA256SUMS may list several files; select the line for OUR tarball by name (sha256sum
    # writes 'hash  name' in text mode or 'hash *name' in binary mode), never just the first line.
    want="$(awk '$2 ~ /^\*?package\.tar\.gz$/ {print $1; exit}' "$WORK/SHA256SUMS")"
    [ -n "$want" ] || die "SHA256SUMS from $SUMS_URL has no entry for package.tar.gz"
    have="$(sha256sum "$WORK/package.tar.gz" | awk '{print $1}')"
    [ "$want" = "$have" ] || die "checksum mismatch for package.tar.gz (want $want, have $have)"
    say "checksum OK"
fi

say "unpacking"
tar -xzf "$WORK/package.tar.gz" -C "$WORK"
STAGE="$WORK/y5-install"
[ -x "$STAGE/y5-install" ] || die "artifact missing the installer (expected $STAGE/y5-install)"

# The installer is interactive. Under `curl ... | bash` this script's stdin is the
# pipe, so feed the installer the real terminal when one exists.
say "launching installer"
export Y5_INSTALL_STAGE="$STAGE"
# shellcheck disable=SC2086 # word-splitting of optional args is intended
if [ -e /dev/tty ]; then
    exec "$STAGE/y5-install" ${Y5_INSTALL_ARGS:-} < /dev/tty
else
    exec "$STAGE/y5-install" ${Y5_INSTALL_ARGS:-}
fi
