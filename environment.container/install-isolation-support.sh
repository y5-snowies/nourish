#!/usr/bin/env bash
# Install everything a y5 session needs in order to stand on its own: a desktop
# portal with a backend, the session services GNOME apps expect, a systemd user
# manager, and PAM for the lock screen.
#
# This is SETUP, not runtime. It installs packages and writes configuration; it
# starts nothing and isolates nothing. The runtime side (a private session bus
# and runtime dir per compositor instance) is separate and not driven from here.
#
# Run it from anywhere; it locates the repo from its own path.
#
#   environment.container/install-isolation-support.sh              configure + report
#   environment.container/install-isolation-support.sh --packages   install missing packages
#   environment.container/install-isolation-support.sh --password   set a password for PAM
#   environment.container/install-isolation-support.sh --desktop N  override desktop identity
#
# Idempotent, and safe to run on a machine that is already set up. Everything it
# cannot do safely it reports instead of forcing — see the keyring section.
#
# Verify the result with:
#   compositor.developer/developer.tool/developer.tool.portal/portal.stress
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PAM_TEMPLATE="$REPO_ROOT/compositor.installer/installation-y5-lock"
PAM_SERVICE=/etc/pam.d/y5-lock
USER_NAME="${SUDO_USER:-$(id -un)}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"

INSTALL_PACKAGES=0
SET_PASSWORD=0
DESKTOP=""

while [ $# -gt 0 ]; do
    case "$1" in
        --packages) INSTALL_PACKAGES=1 ;;
        --password) SET_PASSWORD=1 ;;
        --desktop)  DESKTOP="${2:?--desktop needs a name}"; shift ;;
        -h|--help)  sed -n '2,21p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

say()     { printf '  %-6s %s\n' "$1" "$2"; }
section() { printf '\n== %s ==\n' "$1"; }
have()    { command -v "$1" >/dev/null 2>&1; }

# ── identity ─────────────────────────────────────────────────────────────────
# The desktop name is what xdg-desktop-portal matches a backend on. It has to be
# the same string the compositor exports as XDG_CURRENT_DESKTOP, which comes
# from settings.json — so read it from there rather than hardcoding a guess.
section "identity"
if [ -z "$DESKTOP" ]; then
    SETTINGS="$CONFIG_HOME/y5.compositor/settings.json"
    if [ -r "$SETTINGS" ]; then
        DESKTOP="$(sed -n 's/.*"desktop_name":"\([^"]*\)".*/\1/p' "$SETTINGS")"
    fi
    DESKTOP="${DESKTOP:-${COMPOSITOR_DESKTOP_NAME:-Y5Compositor}}"
fi
DESKTOP_LOWER="$(printf '%s' "$DESKTOP" | tr '[:upper:]' '[:lower:]')"
say OK "desktop identity: $DESKTOP"

DISTRO="$(. /etc/os-release 2>/dev/null && echo "${ID_LIKE:-$ID}" || echo unknown)"
say OK "distro family: $DISTRO"
say "" "user: $USER_NAME"

# ── packages ─────────────────────────────────────────────────────────────────
section "packages"
case "$DISTRO" in
    *fedora*|*rhel*)
        PKGS="dbus-daemon xdg-desktop-portal xdg-desktop-portal-gtk gvfs dconf gnome-keyring pam"
        INSTALL="sudo dnf install -y" ;;
    *debian*|*ubuntu*)
        PKGS="dbus xdg-desktop-portal xdg-desktop-portal-gtk gvfs-daemons dconf-service gnome-keyring libpam-modules"
        INSTALL="sudo apt-get install -y" ;;
    *arch*)
        PKGS="dbus xdg-desktop-portal xdg-desktop-portal-gtk gvfs dconf gnome-keyring pam"
        INSTALL="sudo pacman -S --needed --noconfirm" ;;
    *)
        PKGS=""; INSTALL="" ;;
esac

if [ -z "$INSTALL" ]; then
    say WARN "unrecognised distro — install the portal/gvfs/dconf/keyring packages yourself"
elif [ "$INSTALL_PACKAGES" = 1 ]; then
    say "" "$INSTALL $PKGS"
    $INSTALL $PKGS
    say OK "packages installed"
else
    say "" "would install: $PKGS"
    say "" "re-run with --packages to do it"
fi

# ── portal backend ───────────────────────────────────────────────────────────
# A portal frontend with no backend is the quiet failure mode: every interface
# answers with a healthy version and the file chooser still does nothing. The
# backend is chosen by matching XDG_CURRENT_DESKTOP, so a session with a private
# desktop name needs a config naming one explicitly.
section "portal backend"
AVAILABLE=""
for portal in /usr/share/xdg-desktop-portal/portals/*.portal; do
    [ -e "$portal" ] || continue
    AVAILABLE="$AVAILABLE $(basename "$portal" .portal)"
done
AVAILABLE="${AVAILABLE# }"

if [ -z "$AVAILABLE" ]; then
    say WARN "no backends installed — the file chooser will never appear"
    BACKEND=""
else
    say OK "backends available: $AVAILABLE"
    BACKEND=""
    for candidate in gtk gnome kde; do
        case " $AVAILABLE " in *" $candidate "*) BACKEND="$candidate"; break ;; esac
    done
    BACKEND="${BACKEND:-${AVAILABLE%% *}}"
fi

if [ -n "$BACKEND" ]; then
    PORTAL_DIR="$CONFIG_HOME/xdg-desktop-portal"
    PORTAL_CONF="$PORTAL_DIR/${DESKTOP_LOWER}-portals.conf"
    BODY="[preferred]
default=$BACKEND"
    mkdir -p "$PORTAL_DIR"
    if [ -e "$PORTAL_CONF" ] && [ "$(cat "$PORTAL_CONF")" = "$BODY" ]; then
        say OK "$PORTAL_CONF already current (default=$BACKEND)"
    else
        printf '%s\n' "$BODY" > "$PORTAL_CONF"
        say OK "$PORTAL_CONF written (default=$BACKEND)"
    fi
fi

# ── systemd user manager ─────────────────────────────────────────────────────
# Apps are adopted into transient y5-app-*.scope units over the SESSION bus, and
# closing a window stops the scope. Both need a user manager on whichever bus
# the session ends up using.
section "systemd user manager"
have systemd || [ -x /usr/lib/systemd/systemd ] \
    && say OK "systemd present" \
    || say WARN "systemd absent — launches fall back to plain reaping (this is handled, not fatal)"
have dbus-daemon \
    && say OK "dbus-daemon present" \
    || say WARN "dbus-daemon absent — a private session bus cannot be started"
[ -d /run/systemd/system ] \
    && say OK "system manager is booted" \
    || say WARN "/run/systemd/system absent — no system manager (normal in a container)"

# ── PAM ──────────────────────────────────────────────────────────────────────
# The lock screen authenticates through the PAM service "y5-lock". Without the
# service file pam_start fails before a password is ever considered, which
# presents as a lock screen that rejects everything.
#
# This does NOT make anything else ask for a password: sudo keeps its existing
# NOPASSWD rule, and `sudo -u $USER` is unaffected.
section "PAM (lock screen)"
if [ ! -r "$PAM_TEMPLATE" ]; then
    say FAIL "template missing: $PAM_TEMPLATE"
    exit 1
fi

# The template ships the RHEL/Fedora stack active with Debian's commented out;
# pick whichever this distro actually provides.
if [ -e /etc/pam.d/system-auth ]; then
    STACK=system-auth
    PAM_BODY="$(cat "$PAM_TEMPLATE")"
elif [ -e /etc/pam.d/common-auth ]; then
    STACK=common-auth
    PAM_BODY="# /etc/pam.d/y5-lock — generated by install-isolation-support.sh (Debian stack)
auth     include    common-auth
account  include    common-account"
else
    say FAIL "neither system-auth nor common-auth exists — unknown PAM layout"
    exit 1
fi

if [ -e "$PAM_SERVICE" ] && [ "$(cat "$PAM_SERVICE")" = "$PAM_BODY" ]; then
    say OK "$PAM_SERVICE already current ($STACK)"
else
    printf '%s\n' "$PAM_BODY" | sudo tee "$PAM_SERVICE" >/dev/null
    sudo chmod 644 "$PAM_SERVICE"
    say OK "$PAM_SERVICE installed ($STACK)"
fi

# pam_unix cannot read /etc/shadow unprivileged; it shells out to unix_chkpwd,
# which must be setuid root. Containers that strip setuid bits break the lock
# screen here and nowhere else, reporting a misleading "authentication failed".
CHKPWD="$(command -v unix_chkpwd || echo /usr/sbin/unix_chkpwd)"
if [ ! -e "$CHKPWD" ]; then
    say FAIL "unix_chkpwd not found — install the pam package"
elif [ -u "$CHKPWD" ]; then
    say OK "$CHKPWD is setuid root"
else
    say WARN "$CHKPWD is NOT setuid — pam_unix will fail as a normal user"
    say "" "fix: sudo chmod u+s $CHKPWD"
fi

STATUS="$(sudo passwd -S "$USER_NAME" 2>/dev/null | awk '{print $2}')"
case "$STATUS" in
    P)  say OK "$USER_NAME has a password — the lock screen can authenticate" ;;
    L)  say WARN "$USER_NAME is locked — PAM will refuse every attempt" ;;
    NP) say WARN "$USER_NAME has no password — pam_unix nullok may accept an empty one" ;;
    *)  say WARN "$USER_NAME: unknown password status '${STATUS:-?}'" ;;
esac

if [ "$SET_PASSWORD" = 1 ]; then
    echo
    echo "Setting a password for '$USER_NAME'. Used ONLY by explicit PAM requests"
    echo "such as the lock screen; sudo keeps its existing NOPASSWD rule."
    sudo passwd "$USER_NAME"
    say OK "password set"
elif [ "$STATUS" != "P" ]; then
    say "" "re-run with --password to set one"
fi

# ── keyring auto-unlock ──────────────────────────────────────────────────────
# REPORT ONLY, deliberately. The login keyring is unlocked by
# pam_gnome_keyring.so capturing the password from the PAM stack that STARTED
# the session — a display manager's service, normally. That is why GNOME never
# prompts and a session started another way does, later and seemingly at random.
#
# Nothing is rewritten here: on Fedora system-auth is generated by authselect
# and hand-edits are overwritten, and getting an auth stack wrong locks you out
# of the machine. The remediation belongs to whichever service starts y5.
section "keyring auto-unlock (report only)"
KEYRING_MODULE=""
for candidate in /usr/lib64/security /usr/lib/x86_64-linux-gnu/security /usr/lib/security /lib/security; do
    [ -e "$candidate/pam_gnome_keyring.so" ] && KEYRING_MODULE="$candidate/pam_gnome_keyring.so" && break
done

if [ -z "$KEYRING_MODULE" ]; then
    say WARN "pam_gnome_keyring.so not installed — nothing can auto-unlock the keyring"
else
    say OK "module: $KEYRING_MODULE"
    WIRED="$(grep -lE '^[^#]*pam_gnome_keyring\.so' /etc/pam.d/* 2>/dev/null || true)"
    if [ -z "$WIRED" ]; then
        say WARN "no /etc/pam.d service references it — expect a prompt on first secret"
    fi
    for service in $WIRED; do
        name="$(basename "$service")"
        auth=no; session=no
        grep -qE '^auth[^#]*pam_gnome_keyring\.so' "$service" && auth=yes
        grep -qE '^session[^#]*pam_gnome_keyring\.so' "$service" && session=yes
        say "" "/etc/pam.d/$name — auth=$auth session=$session"
    done
    say "" "auth= is the half that unlocks. A session started by a service"
    say "" "without it reaches the desktop with the keyring still locked."
fi

# ── done ─────────────────────────────────────────────────────────────────────
section "next"
PROBE="$REPO_ROOT/compositor.developer/developer.tool/developer.tool.portal/portal.stress"
echo "  Verify what this actually produced, against the bus you care about:"
echo "    cd $PROBE"
echo "    cargo build --profile release-fast && ./target/release-fast/portal-stress"
echo
echo "  The keyring section of that report tells you whether the default"
echo "  collection is locked — which is what decides if you get prompted."
