#!/usr/bin/env python3
"""Playback OSD as an overlay layer surface.

Anchored bottom-centre on the OVERLAY layer, shows the active MPRIS player's
art / title / artist plus ⏮ ⏯ ⏭ controls. Tracks players live via Playerctl
(any MPRIS source: mpv, Firefox, Spotify, playerctld ...). Purely a layer-shell
test client for y5 — it reserves no exclusive zone and takes keyboard on-demand.

    ./osd.py            # follows whichever player is active
"""
import os
import sys

# gtk4-layer-shell must intercept libwayland before GTK connects; via Python GI it
# loads too late and the surface falls back to a normal xdg_toplevel. Re-exec once
# with the real gtk4-layer-shell lib so its hook runs first and the GI setters apply.
_PRELOAD = "libgtk4-layer-shell.so.0"
if _PRELOAD not in os.environ.get("LD_PRELOAD", ""):
    os.environ["LD_PRELOAD"] = ":".join(
        p for p in (_PRELOAD, os.environ.get("LD_PRELOAD", "")) if p)
    os.execv(sys.executable, [sys.executable, *sys.argv])

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Gtk4LayerShell", "1.0")
gi.require_version("Gdk", "4.0")
gi.require_version("Playerctl", "2.0")
from gi.repository import Gtk, Gtk4LayerShell as LS, Gdk, GdkPixbuf, GLib, Gio, Playerctl  # noqa: E402


class OSD:
    def __init__(self):
        self.win = None
        self.player = None
        self.manager = Playerctl.PlayerManager()
        self.manager.connect("name-appeared", self.on_name_appeared)
        self.manager.connect("player-vanished", self.on_player_vanished)
        for name in self.manager.props.player_names:
            self.manage(name)
        self.build()
        self.refresh()

    # ── player wiring ────────────────────────────────────────────────────
    def manage(self, name):
        player = Playerctl.Player.new_from_name(name)
        player.connect("metadata", lambda *_: self.refresh())
        player.connect("playback-status", lambda *_: self.refresh())
        self.manager.manage_player(player)
        self.player = player  # newest player wins focus

    def on_name_appeared(self, _mgr, name):
        self.manage(name)
        self.refresh()

    def on_player_vanished(self, _mgr, player):
        if player is self.player:
            self.player = None
            players = self.manager.props.players
            self.player = players[0] if players else None
        self.refresh()

    # ── ui ───────────────────────────────────────────────────────────────
    def build(self):
        css = Gtk.CssProvider()
        css.load_from_data(
            b".osd { background:rgba(24,24,37,0.92); color:#cdd6f4; border-radius:14px; }"
            b".osd button { min-width:34px; min-height:34px; border-radius:8px; }"
            b".osd .title { font-weight:bold; font-size:13px; }"
            b".osd .artist { color:#a6adc8; font-size:11px; }"
        )
        Gtk.StyleContext.add_provider_for_display(
            Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        )

        self.win = Gtk.Window()
        self.win.set_decorated(False)
        LS.init_for_window(self.win)
        LS.set_namespace(self.win, "ls-osd")
        LS.set_layer(self.win, LS.Layer.OVERLAY)
        LS.set_anchor(self.win, LS.Edge.BOTTOM, True)
        LS.set_margin(self.win, LS.Edge.BOTTOM, 48)
        LS.set_keyboard_mode(self.win, LS.KeyboardMode.ON_DEMAND)

        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        row.add_css_class("osd")
        row.set_margin_top(10)
        row.set_margin_bottom(10)
        row.set_margin_start(14)
        row.set_margin_end(14)

        self.art = Gtk.Image()
        self.art.set_pixel_size(48)
        row.append(self.art)

        meta = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1)
        meta.set_valign(Gtk.Align.CENTER)
        self.title = Gtk.Label(label="No media", xalign=0.0)
        self.title.add_css_class("title")
        self.title.set_ellipsize(3)  # PANGO_ELLIPSIZE_END
        self.title.set_max_width_chars(28)
        self.artist = Gtk.Label(label="", xalign=0.0)
        self.artist.add_css_class("artist")
        self.artist.set_ellipsize(3)
        self.artist.set_max_width_chars(28)
        meta.append(self.title)
        meta.append(self.artist)
        row.append(meta)

        controls = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        controls.set_valign(Gtk.Align.CENTER)
        for glyph, action in (("⏮", "previous"),
                              ("⏯", "play_pause"),
                              ("⏭", "next")):
            b = Gtk.Button(label=glyph)
            b.connect("clicked", self.on_control, action)
            controls.append(b)
        self.playpause = controls
        row.append(controls)

        self.win.set_child(row)
        self.win.present()

        esc = Gtk.EventControllerKey()
        esc.connect("key-pressed", self.on_key)
        self.win.add_controller(esc)

    def on_key(self, _c, keyval, _code, _state):
        if Gdk.keyval_name(keyval) == "Escape":
            self.loop.quit()
        return False

    def on_control(self, _btn, action):
        if not self.player:
            return
        try:
            getattr(self.player, action)()
        except GLib.Error as e:  # noqa: BLE001
            print(f"osd: {action} failed: {e}", file=sys.stderr)

    # ── render ───────────────────────────────────────────────────────────
    def refresh(self):
        if not self.player:
            self.title.set_text("No media playing")
            self.artist.set_text("")
            self.art.set_from_icon_name("audio-x-generic-symbolic")
            return
        try:
            meta = self.player.props.metadata
        except Exception:  # noqa: BLE001
            meta = None
        title = self._m(meta, "xesam:title") or "Unknown title"
        artists = self._m(meta, "xesam:artist")
        if isinstance(artists, (list, tuple)):
            artists = ", ".join(artists)
        status = self.player.props.playback_status.value_nick if self.player else ""
        self.title.set_text(title)
        self.artist.set_text(f"{artists or ''}   • {status}".strip())
        self._set_art(self._m(meta, "mpris:artUrl"))

    @staticmethod
    def _m(meta, key):
        if meta is None:
            return None
        try:
            if key in meta.keys():
                return meta[key]
        except Exception:  # noqa: BLE001
            pass
        return None

    def _set_art(self, url):
        if url and url.startswith("file://"):
            path = GLib.filename_from_uri(url)[0]
            try:
                pb = GdkPixbuf.Pixbuf.new_from_file_at_scale(path, 48, 48, True)
                self.art.set_from_pixbuf(pb)
                return
            except Exception:  # noqa: BLE001
                pass
        self.art.set_from_icon_name("audio-x-generic-symbolic")

    def run(self):
        self.loop = GLib.MainLoop()
        self.win.connect("close-request", lambda *_: (self.loop.quit(), False)[1])
        self.loop.run()


if __name__ == "__main__":
    if not Gtk.init_check():
        print("osd: could not initialise GTK (need a running Wayland session)",
              file=sys.stderr)
        raise SystemExit(1)
    OSD().run()
