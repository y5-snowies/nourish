#!/usr/bin/env bash
# Installation preparation: build every shipped binary (compositor, dev-tool
# window, the interactive installer, polkit agent, MX daemon), stage them with the
# config templates, and bundle everything into a single uploadable tarball.
#
# Run on a dev/CI host with the build toolchain. The produced tarball contains the
# prebuilt binaries + templates + the `y5-install` interactive installer; an end
# user fetches it and runs ./install.sh (or ./y5-install) on the target — which
# installs only the runtime libs (not the toolchain), because the binaries are
# already compiled here.
#
# The artifact is what the one-line installer (compositor.installer/get.sh) pulls.
# Upload <out>/package.tar.gz + <out>/SHA256SUMS to the host path the guide uses:
#   https://nourish.snowies.com/release/latest/fedora44/package.tar.gz
#
# Usage: ./prepare.sh [options]
#   --debug            build the installer in debug (faster) instead of release
#   --skip=a,b,...     skip components: compositor,devtool,settings,installer,polkit,mx,xwayland
#   --out=DIR          output dir (default: compositor.installer/dist)
#   -h, --help         this help
#
# Output:
#   <out>/stage/         the assembled tree (also usable in place via Y5_INSTALL_STAGE)
#   <out>/package.tar.gz the artifact to upload (extracts to ./y5-install/)
#   <out>/SHA256SUMS     checksum of the artifact
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/.." && pwd)"

PROFILE=release
SKIP=""
OUT="$HERE/dist"
for arg in "$@"; do
    case "$arg" in
        --debug) PROFILE=debug ;;
        --skip=*) SKIP="${arg#--skip=}" ;;
        --out=*) OUT="${arg#--out=}" ;;
        -h | --help) sed -n '2,27p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "prepare.sh: unknown arg '$arg' (see --help)" >&2; exit 1 ;;
    esac
done

skipped() { case ",$SKIP," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

# Note: the compositor's version is baked in at COMPILE time from the repo-root VERSION
# file (include_str! in the loader bin), NOT injected here — so nothing in the install
# path can make the embedded number disagree with the build. CI sets the release number
# by writing that file before this runs; see ci/scripts/version.sh.

STAGE="$OUT/stage"
BIN="$STAGE/binaries"
TPL="$STAGE/templates"
rm -rf "$STAGE"
mkdir -p "$BIN" "$TPL/pam" "$TPL/mx" "$TPL/xwayland"

log() { printf '\n>> %s\n' "$1" >&2; }

# 1) Compositor (udev release) — installed as both the system and dev binaries.
if skipped compositor; then
    log "skip: compositor"
else
    log "building compositor (udev release)"
    COMPOSITOR_BIN="$("$REPO_ROOT/environment/build.sh" udev release)"
    install -m755 "$COMPOSITOR_BIN" "$BIN/y5.compositor"
    install -m755 "$COMPOSITOR_BIN" "$BIN/y5.compositor.dev"
fi

# 2) Developer tool window (bare binary).
if skipped devtool; then
    log "skip: devtool"
else
    log "building developer tool window"
    "$REPO_ROOT/compositor.developer/developer.tool/developer.tool.window/logs/bundle.sh" none
    install -m755 \
        "$REPO_ROOT/compositor.developer/developer.tool/developer.tool.window/logs/src-tauri/target/release/compositor-developer-tool" \
        "$BIN/compositor-developer-tool"
fi

# 3) Polkit agent.
if skipped polkit; then
    log "skip: polkit"
else
    log "building polkit agent"
    ( cd "$HERE/component/pollkit-agent" && cargo build --release )
    install -m755 "$HERE/component/pollkit-agent/target/release/iced_polkit_agent" "$BIN/y5-polkit-agent"
fi

# 4) MX gesture daemon (+ its templates).
if skipped mx; then
    log "skip: mx"
else
    log "building MX gesture daemon"
    ( cd "$HERE/component/mx-gesture-daemon" && cargo build --release )
    install -m755 "$HERE/component/mx-gesture-daemon/target/release/mx-gesture-daemon" "$BIN/mx-gesture-daemon"
    install -m644 "$HERE/component/mx-gesture-daemon/42-logitech-hidpp.rules" "$TPL/mx/42-logitech-hidpp.rules"
    install -m644 "$HERE/component/mx-gesture-daemon/config.example.toml" "$TPL/mx/config.example.toml"
    install -m644 "$HERE/component/mx-gesture-daemon/mx-gesture-daemon.service" "$TPL/mx/mx-gesture-daemon.service"
fi

# 5) Patched xwayland-satellite (X11-app compatibility) + its user service.
if skipped xwayland; then
    log "skip: xwayland"
else
    log "building xwayland-satellite"
    ( cd "$HERE/component/xwayland-satellite/xwayland-fixes" && cargo build --release )
    install -m755 "$HERE/component/xwayland-satellite/xwayland-fixes/target/release/xwayland-satellite" "$BIN/xwayland-satellite"
    install -m644 "$HERE/component/xwayland-satellite/xwayland-fixes/xwayland.service" "$TPL/xwayland/xwayland.service"
fi

# 5b) Settings tool — installed to /usr/bin/y5.compositor.settings, the only supported
#     way to author ~/.config/y5.compositor/settings.json (the session wrappers no longer
#     write it). Cargo target names can't contain '.', so the built binary is
#     `y5-compositor-settings`; we stage it under the dotted command name.
if skipped settings; then
    log "skip: settings tool"
else
    log "building settings tool (y5.compositor.settings)"
    ( cd "$HERE/component/settings-editor" && cargo build --release )
    install -m755 "$HERE/component/settings-editor/target/release/y5-compositor-settings" "$BIN/y5.compositor.settings"
fi

# 6) The interactive installer itself.
if skipped installer; then
    log "skip: installer"
else
    log "building interactive installer ($PROFILE)"
    if [ "$PROFILE" = release ]; then
        ( cd "$HERE/installer.process" && cargo build --release )
        install -m755 "$HERE/installer.process/target/release/y5-install" "$STAGE/y5-install"
    else
        ( cd "$HERE/installer.process" && cargo build )
        install -m755 "$HERE/installer.process/target/debug/y5-install" "$STAGE/y5-install"
    fi
fi

# 6b) NixOS: pre-generate the nix-ld module + a native setup entry point. NixOS is
#     declarative + non-FHS, so the interactive installer can't run there — instead the
#     bundle ships a ready configuration.nix module (generated HERE, where y5-install runs
#     natively) plus a pure-bash `nixos-setup.sh` the user runs on NixOS to print it.
if [ -x "$STAGE/y5-install" ]; then
    log "generating NixOS module + flake (nix-ld)"
    mkdir -p "$STAGE/nixos"
    "$STAGE/y5-install" --emit-nixos > "$STAGE/nixos/configuration-y5.nix"
    "$STAGE/y5-install" --emit-nixos-flake > "$STAGE/nixos/flake.nix"
    cat > "$STAGE/nixos-setup.sh" <<'EOF'
#!/usr/bin/env bash
# NixOS setup for y5. NixOS is declarative + non-FHS: nothing is installed imperatively.
# This prints the nix-ld module to add to your system config + how to apply it, and the path
# to the prebuilt binaries. It changes NOTHING. Safe to run as your normal user.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOD="$HERE/nixos/configuration-y5.nix"
[ -f "$MOD" ] || { echo "nixos-setup: missing $MOD" >&2; exit 1; }
cat >&2 <<TXT
y5 on NixOS
===========
The y5 binaries are prebuilt (FHS/glibc); on NixOS they run via nix-ld. Add the module
printed below to your NixOS configuration:

  1a) Plain module — sudo cp "$MOD" /etc/nixos/y5.nix
      and add it to your imports in /etc/nixos/configuration.nix:
        imports = [ ./y5.nix ];
  1b) Or flakes — point an input at this dir and import its module:
        inputs.y5.url = "path:$HERE/nixos";
        imports = [ inputs.y5.nixosModules.default ];
      (flake: $HERE/nixos/flake.nix)
  2) sudo nixos-rebuild switch
  3) The prebuilt binaries live in:  $HERE/binaries/
     Launch the compositor as  $HERE/binaries/y5.compositor  (nix-ld resolves its libs),
     or wire a wayland-session that execs it. If launching reports a missing library, add
     that library's nixpkgs package to programs.nix-ld.libraries in y5.nix and rebuild.

--- module ($MOD) ---
TXT
cat "$MOD"
EOF
    chmod +x "$STAGE/nixos-setup.sh"
fi

# 7) PAM lock template (always staged — tiny, no build).
install -m644 "$HERE/installation-y5-lock" "$TPL/pam/installation-y5-lock"

# 8) In-artifact README so the unpacked tree is self-documenting.
cat > "$STAGE/README.txt" <<'EOF'
y5 compositor — install bundle
==============================

Prebuilt binaries + the interactive installer. Installing pulls only the runtime
shared libraries (no Rust toolchain, no -devel headers) because everything here is
already compiled. The installer is distro-aware: it detects your package manager
(dnf / apt-get / pacman) from /etc/os-release and installs that distro's runtime
package names. On NixOS the installer doesn't apply (declarative + non-FHS) — run
./nixos-setup.sh instead to get the nix-ld module + how to apply it.

NOTE: each bundle's binaries are dynamically linked to ONE distro's system libraries
(the distro it was built on) — use the bundle that matches your distro. The Fedora
one-command install:

    curl -fsSL https://nourish.snowies.com/release/latest/fedora44/package.tar.gz \
        | tar -xz && y5-install/install.sh

(Debian/Ubuntu/Arch: download package-<distro>-<arch>.tar.gz from the multiarch
release instead.) Already have this tree unpacked? Just run:

    ./install.sh                 # interactive install (uses sudo for system steps)
    ./install.sh --dry-run       # preview without changing anything

It is safe to re-run; it overwrites the previous install. Afterwards, log out and
pick a "Y5…" session in your display manager.

Contents
--------
  y5-install                         the interactive installer
  install.sh                         launcher (sets Y5_INSTALL_STAGE, runs y5-install)
  nixos-setup.sh                     NixOS entry point: print the nix-ld module + how to apply
  nixos/configuration-y5.nix         the ready NixOS module (nix-ld + runtime libs)
  nixos/flake.nix                    the same as a flake (nixosModules.default)
  binaries/y5.compositor             the compositor (system session)
  binaries/y5.compositor.dev         the compositor (dev / experimental sessions)
  binaries/y5.compositor.settings    the settings tool (installed to /usr/bin)
  binaries/compositor-developer-tool the developer log viewer (installed as
                                     y5.compositor.monitor, + app-launcher entry)
  binaries/y5-polkit-agent           polkit authentication agent
  binaries/mx-gesture-daemon         MX Master gesture daemon
  binaries/xwayland-satellite        patched Xwayland satellite (X11 app support)
  templates/                         PAM, udev and config templates

Note: the compositor reads all of its configuration from a single settings file,
~/.config/y5.compositor/settings.json. The installer seeds it once from your answers;
after that the session wrappers NEVER overwrite it, so your edits stick. Re-author it
anytime with the `y5.compositor.settings` tool (installed to /usr/bin); a session
refuses to start if the file is missing. Run the installer as your normal user (not
sudo) so the file lands in your home — it uses sudo itself for the system steps.
EOF

# 9) Convenience launcher inside the artifact.
cat > "$STAGE/install.sh" <<'EOF'
#!/usr/bin/env bash
# Run the interactive y5 installer from this unzipped artifact.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export Y5_INSTALL_STAGE="$HERE"
exec "$HERE/y5-install" "$@"
EOF
chmod +x "$STAGE/install.sh"

# 10) Bundle. Pack so the tarball extracts to a single top-level ./y5-install/ dir,
#    which is what get.sh / the one-line install command expect.
log "bundling artifact"
PKG="$OUT/package.tar.gz"
SUMS="$OUT/SHA256SUMS"
rm -f "$PKG" "$SUMS"
tar -czf "$PKG" -C "$OUT" --transform 's,^stage,y5-install,' stage
( cd "$OUT" && sha256sum "$(basename "$PKG")" > "$SUMS" )

echo
echo "Staged tree: $STAGE"
echo "Artifact:    $PKG"
echo "Checksum:    $SUMS"
echo "  $(cat "$SUMS")"
( cd "$STAGE" && find . -type f | sort | sed 's/^/  /' )
echo
echo "Upload both files to: https://nourish.snowies.com/release/latest/fedora44/"
