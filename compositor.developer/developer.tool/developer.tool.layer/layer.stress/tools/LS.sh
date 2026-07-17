#!/usr/bin/env bash
# LS.sh — interactive layer-shell test harness for the y5 compositor.
#
# Toggle each layer-shell client on/off individually to exercise the matrix in
# LS.md (anchors, layers, exclusive zones, margins, keyboard, popups, overlays).
# Everything starts DE-ACTIVATED: nothing launches until you pick it.
#
# Run this INSIDE a running y5 session (it needs WAYLAND_DISPLAY). Notifications
# additionally need a session bus (DBUS_SESSION_BUS_ADDRESS); if none is present
# the script offers to re-exec itself under dbus-run-session.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SELF="$HERE/$(basename "${BASH_SOURCE[0]}")"
TOOLS="$HERE/ls-tools"
TONE="/tmp/ls-tone.mp3"

# ── entry registry ──────────────────────────────────────────────────────────
# Parallel arrays. KIND: daemon = long-lived toggle; oneshot = fire-and-forget
# (menus/dialogs that exit themselves); action = a shell function.
declare -a KEY LABEL KIND CMD
declare -A PID          # key -> process-group leader pid (daemons only)
add() { KEY+=("$1"); LABEL+=("$2"); KIND+=("$3"); CMD+=("$4"); }

# Real bars / panels
add waybar   "waybar — real bar (clock calendar + ☰ menu + tooltip popups)" daemon "env XDG_CONFIG_HOME='$TOOLS' waybar -c '$TOOLS/waybar/config.jsonc' -s '$TOOLS/waybar/style.css'"
add nwgpanel "nwg-panel — configurable bar"                   daemon "nwg-panel"
add swaybg   "swaybg — background layer (solid colour)"       daemon "swaybg -c 285577"

# Panel presets (my gtk4-layer-shell client — covers the LS.md matrix)
add p_top    "panel: TOP bar, anchor T+L+R, exclusive=auto"   daemon "python3 $TOOLS/panel.py --layer top --anchor top,left,right --exclusive-zone auto --height 34 --label 'top bar' --color '#313244'"
add p_bot    "panel: BOTTOM bar, anchor B+L+R, exclusive=auto" daemon "python3 $TOOLS/panel.py --layer top --anchor bottom,left,right --exclusive-zone auto --height 30 --label 'bottom bar' --color '#45475a'"
add p_left   "panel: LEFT bar, anchor L+T+B, exclusive=auto"  daemon "python3 $TOOLS/panel.py --layer top --anchor left,top,bottom --exclusive-zone auto --width 56 --label L --color '#585b70'"
add p_right  "panel: RIGHT bar, anchor R+T+B, exclusive=auto" daemon "python3 $TOOLS/panel.py --layer top --anchor right,top,bottom --exclusive-zone auto --width 56 --label R --color '#585b70'"
add p_tl     "panel: TOP-LEFT corner, margins, no exclusive"  daemon "python3 $TOOLS/panel.py --layer overlay --anchor top,left --margin-top 12 --margin-left 12 --label 'top-left' --color '#89b4fa'"
add p_tr     "panel: TOP-RIGHT corner (with popup button)"    daemon "python3 $TOOLS/panel.py --layer overlay --anchor top,right --margin-top 12 --margin-right 12 --label 'top-right' --popup --color '#a6e3a1'"
add p_bl     "panel: BOTTOM-LEFT corner"                      daemon "python3 $TOOLS/panel.py --layer overlay --anchor bottom,left --margin-bottom 12 --margin-left 12 --label 'bottom-left' --color '#f9e2af'"
add p_br     "panel: BOTTOM-RIGHT corner"                     daemon "python3 $TOOLS/panel.py --layer overlay --anchor bottom,right --margin-bottom 12 --margin-right 12 --label 'bottom-right' --color '#fab387'"
add p_float  "panel: FLOATING (no anchor), overlay, popup"    daemon "python3 $TOOLS/panel.py --layer overlay --anchor '' --width 320 --height 90 --label floating --popup --color '#cba6f7'"
add p_full   "panel: FULLSCREEN background (anchor all, z=-1)" daemon "python3 $TOOLS/panel.py --layer background --anchor top,bottom,left,right --exclusive-zone -1 --label 'wallpaper layer' --color '#11111b'"
add p_kbd    "panel: OVERLAY, keyboard=EXCLUSIVE (grabs kbd)" daemon "python3 $TOOLS/panel.py --layer overlay --anchor top --exclusive-zone 0 --height 40 --keyboard exclusive --label 'kbd-exclusive (Esc closes)' --color '#f38ba8'"
add p_click  "panel: BACKGROUND, empty input (click-through)" daemon "python3 $TOOLS/panel.py --layer background --anchor top,left --width 260 --height 80 --click-through --label 'click-through' --color '#94e2d5'"
add p_zone   "panel: exclusive-zone=100 stress (top)"         daemon "python3 $TOOLS/panel.py --layer top --anchor top,left,right --exclusive-zone 100 --height 100 --label 'exclusive-zone=100' --color '#74c7ec'"
add p_popup  "panel: POPUP auto-opens (layer→xdg_popup, re-fires 4s)" daemon "python3 $TOOLS/panel.py --layer top --anchor top,left --margin-top 8 --margin-left 8 --auto-popup --label 'popup test' --color '#b4befe'"

# Custom overlays
add osd      "playback OSD — MPRIS media + controls (overlay)" daemon "python3 $TOOLS/osd.py"
add notify   "notification daemon + history (org.fd.Notifications)" daemon "python3 $TOOLS/notify.py"
add mako     "mako — reference notification daemon (history)" daemon "mako"

# Modal / one-shot clients
add wlogout  "wlogout — overlay logout buttons (Esc to close)" oneshot "wlogout"
add wofi     "wofi — app launcher popup"                       oneshot "wofi --show drun"
add fuzzel   "fuzzel — launcher popup"                         oneshot "fuzzel"
add bemenu   "bemenu — menu from stdin"                        oneshot "printf 'alpha\\nbeta\\ngamma\\ndelta\\n' | bemenu -p LS"

# Actions
add media    "▶ start/stop test media (mpv tone → feeds OSD)"  action  act_media
add sendnote "✉ send a test notification"                      action  act_sendnote
add history  "🕘 toggle notification history (SIGUSR1)"         action  act_history
add outputs  "🖥 list outputs (wlr-randr)"                      action  act_outputs

# ── helpers ─────────────────────────────────────────────────────────────────
have() { command -v "${1%% *}" >/dev/null 2>&1; }

bin_of() { # first token of a command / after python3
  local c="$1"
  [[ "$c" == python3\ * ]] && { echo python3; return; }
  echo "${c%% *}"
}

is_running() { local k="$1"; [[ -n "${PID[$k]:-}" ]] && kill -0 "${PID[$k]}" 2>/dev/null; }

start() {
  local k="$1" cmd="$2"
  setsid bash -c "$cmd" >/dev/null 2>&1 &
  PID[$k]=$!
}

stop() {
  local k="$1" p="${PID[$k]:-}"
  [[ -z "$p" ]] && return
  kill -TERM "-$p" 2>/dev/null || kill -TERM "$p" 2>/dev/null
  unset "PID[$k]"
}

toggle_daemon() {
  local k="$1" cmd="$2"
  if is_running "$k"; then
    stop "$k"; echo "  ↳ stopped $k"
  else
    # notification daemons are mutually exclusive (one owner of the FDN name)
    if [[ "$k" == notify || "$k" == mako ]]; then
      is_running notify && stop notify
      is_running mako && stop mako
    fi
    start "$k" "$cmd"; echo "  ↳ started $k (pid ${PID[$k]})"
  fi
}

# ── action functions ────────────────────────────────────────────────────────
act_media() {
  if is_running media; then stop media; echo "  ↳ media stopped"; return; fi
  if [[ ! -f "$TONE" ]]; then
    have ffmpeg && ffmpeg -nostdin -hide_banner -loglevel error -f lavfi \
      -i "sine=frequency=440:duration=30" -metadata title="LS Test Track" \
      -metadata artist="y5 OSD" "$TONE" -y 2>/dev/null
  fi
  if [[ -f "$TONE" ]]; then
    start media "mpv --ao=null --vo=null --loop --really-quiet '$TONE'"
    echo "  ↳ media started (mpv MPRIS) — turn on the OSD to see it"
  else
    echo "  ! could not create test tone (ffmpeg missing); play any MPRIS source instead"
  fi
}

act_sendnote() {
  local urg=("low" "normal" "critical"); local u=${urg[$((RANDOM % 3))]}
  if have notify-send; then
    notify-send -u "$u" "Test notification ($u)" "Sent $(date +%H:%M:%S) from LS.sh"
    echo "  ↳ sent a '$u' notification (needs a daemon: notify/mako ON)"
  else
    gdbus call --session --dest org.freedesktop.Notifications \
      --object-path /org/freedesktop/Notifications \
      --method org.freedesktop.Notifications.Notify \
      "LS.sh" 0 "" "Test notification" "Sent from LS.sh" "[]" "{}" 5000 >/dev/null \
      && echo "  ↳ sent via gdbus" || echo "  ! send failed (no daemon / no bus)"
  fi
}

act_history() {
  local p="${PID[notify]:-}"
  if [[ -n "$p" ]] && kill -0 "$p" 2>/dev/null; then
    kill -USR1 "$p"; echo "  ↳ toggled history panel (Esc closes it)"
  else
    echo "  ! the custom notify daemon isn't running — turn on 'notify' first"
  fi
}

act_outputs() { have wlr-randr && wlr-randr || echo "  ! wlr-randr unavailable"; }

# ── UI ──────────────────────────────────────────────────────────────────────
cleanup_prompt() {
  local running=()
  for k in "${KEY[@]}"; do is_running "$k" && running+=("$k"); done
  if ((${#running[@]})); then
    read -rp "Kill ${#running[@]} running client(s) before exit? [y/N] " a
    [[ "$a" == [yY]* ]] && for k in "${running[@]}"; do stop "$k"; done
  fi
}

menu() {
  clear
  echo "════════════════════════════════════════════════════════════════"
  echo "  y5 layer-shell test harness   (WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-<unset>})"
  echo "  number = toggle/launch · r = refresh · x = stop all · q = quit"
  echo "════════════════════════════════════════════════════════════════"
  local i
  for i in "${!KEY[@]}"; do
    local k="${KEY[$i]}" kind="${KIND[$i]}" cmd="${CMD[$i]}" tag disp
    if [[ "$kind" == daemon ]]; then
      if is_running "$k"; then tag=$'\e[32m[ ON ]\e[0m'
      elif have "$(bin_of "$cmd")"; then tag="[ off]"
      else tag=$'\e[31m[ n/a]\e[0m'; fi
    elif [[ "$kind" == oneshot ]]; then
      have "$(bin_of "$cmd")" && tag="[ run]" || tag=$'\e[31m[ n/a]\e[0m'
    else tag="[  * ]"; fi
    printf "  %2d  %s  %s\n" "$((i + 1))" "$tag" "${LABEL[$i]}"
  done
  echo "────────────────────────────────────────────────────────────────"
}

# ── entry ───────────────────────────────────────────────────────────────────
[[ -z "${WAYLAND_DISPLAY:-}" ]] && \
  echo "warning: WAYLAND_DISPLAY unset — layer-shell clients need a running y5 session." >&2

if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
  echo "note: no session bus — notifications won't work." >&2
  if have dbus-run-session; then
    read -rp "Re-exec LS.sh under dbus-run-session (gives a bus)? [Y/n] " a
    [[ "$a" != [nN]* ]] && exec dbus-run-session -- "$SELF" "$@"
  fi
fi

trap cleanup_prompt EXIT
while :; do
  menu
  read -rp "> " sel || break
  case "$sel" in
    q|Q) break ;;
    r|R) continue ;;
    x|X) for k in "${KEY[@]}"; do is_running "$k" && stop "$k"; done ;;
    ''|*[!0-9]*) ;;
    *)
      idx=$((sel - 1))
      if ((idx >= 0 && idx < ${#KEY[@]})); then
        k="${KEY[$idx]}" kind="${KIND[$idx]}" cmd="${CMD[$idx]}"
        case "$kind" in
          daemon)  toggle_daemon "$k" "$cmd" ;;
          oneshot) echo "  ↳ launching $k"; setsid bash -c "$cmd" >/dev/null 2>&1 & ;;
          action)  "$cmd" ;;
        esac
        read -rp "  [enter]" _
      fi ;;
  esac
done
