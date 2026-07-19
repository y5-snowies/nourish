#!/usr/bin/env python3
"""Notification daemon with history, as overlay layer surfaces.

Owns `org.freedesktop.Notifications` on the session bus (replaces mako/swaync,
which aren't installable here) and renders each notification as a toast stacked
top-right on the OVERLAY layer. Keeps a scrollback; SIGUSR1 toggles a history
panel (Escape to close). Built purely to exercise y5's layer-shell overlay +
dynamic-sizing + keyboard paths.

    ./notify.py                       # run the daemon
    kill -USR1 $(pgrep -f notify.py)  # toggle history
    notify-send "hi" "body text"      # emit a test notification
"""
import os
import signal
import sys
import time

# gtk4-layer-shell must intercept libwayland before GTK connects; via Python GI it
# loads too late and toasts fall back to normal xdg_toplevels. Re-exec once with the
# real gtk4-layer-shell lib so its hook runs first and the GI setters apply.
_PRELOAD = "libgtk4-layer-shell.so.0"
if _PRELOAD not in os.environ.get("LD_PRELOAD", ""):
    os.environ["LD_PRELOAD"] = ":".join(
        p for p in (_PRELOAD, os.environ.get("LD_PRELOAD", "")) if p)
    os.execv(sys.executable, [sys.executable, *sys.argv])

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Gtk4LayerShell", "1.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gtk, Gtk4LayerShell as LS, Gdk, GLib, Gio  # noqa: E402

FDN_XML = """
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="Notify">
      <arg type="s" name="app_name" direction="in"/>
      <arg type="u" name="replaces_id" direction="in"/>
      <arg type="s" name="app_icon" direction="in"/>
      <arg type="s" name="summary" direction="in"/>
      <arg type="s" name="body" direction="in"/>
      <arg type="as" name="actions" direction="in"/>
      <arg type="a{sv}" name="hints" direction="in"/>
      <arg type="i" name="expire_timeout" direction="in"/>
      <arg type="u" name="id" direction="out"/>
    </method>
    <method name="CloseNotification">
      <arg type="u" name="id" direction="in"/>
    </method>
    <method name="GetCapabilities">
      <arg type="as" name="caps" direction="out"/>
    </method>
    <method name="GetServerInformation">
      <arg type="s" name="name" direction="out"/>
      <arg type="s" name="vendor" direction="out"/>
      <arg type="s" name="version" direction="out"/>
      <arg type="s" name="spec_version" direction="out"/>
    </method>
    <signal name="NotificationClosed">
      <arg type="u" name="id"/>
      <arg type="u" name="reason"/>
    </signal>
    <signal name="ActionInvoked">
      <arg type="u" name="id"/>
      <arg type="s" name="action_key"/>
    </signal>
  </interface>
</node>
"""

CLOSE_EXPIRED, CLOSE_DISMISSED, CLOSE_REQUEST = 1, 2, 3
DEFAULT_TIMEOUT_MS = 5000

CSS = b"""
.toasts { }
.toast { background:rgba(30,30,46,0.95); color:#cdd6f4; border-radius:12px;
         border:1px solid rgba(180,190,254,0.25); }
.toast.crit { border-color:#f38ba8; }
.toast .app { color:#89b4fa; font-size:10px; }
.toast .sum { font-weight:bold; }
.toast .body { color:#bac2de; font-size:11px; }
.hist { background:rgba(24,24,37,0.97); color:#cdd6f4; border-radius:12px; }
.hist .head { font-weight:bold; font-size:14px; color:#89b4fa; }
.hist .row { border-bottom:1px solid rgba(88,91,112,0.4); }
.hist .when { color:#6c7086; font-size:10px; }
"""


class Notifier:
    def __init__(self):
        self.next_id = 1
        self.active = {}          # id -> (row_widget, timeout_source_id)
        self.history = []         # list of dicts, newest last
        self.conn = None
        self.hist_win = None
        self._css()
        self._build_toasts()

    def _css(self):
        prov = Gtk.CssProvider()
        prov.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_display(
            Gdk.Display.get_default(), prov, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        )

    # ── toast stack (single top-right overlay window) ────────────────────
    def _build_toasts(self):
        self.toast_win = Gtk.Window()
        self.toast_win.set_decorated(False)
        LS.init_for_window(self.toast_win)
        LS.set_namespace(self.toast_win, "ls-notify")
        LS.set_layer(self.toast_win, LS.Layer.OVERLAY)
        LS.set_anchor(self.toast_win, LS.Edge.TOP, True)
        LS.set_anchor(self.toast_win, LS.Edge.RIGHT, True)
        LS.set_margin(self.toast_win, LS.Edge.TOP, 12)
        LS.set_margin(self.toast_win, LS.Edge.RIGHT, 12)
        LS.set_keyboard_mode(self.toast_win, LS.KeyboardMode.NONE)
        self.toast_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.toast_box.add_css_class("toasts")
        self.toast_win.set_child(self.toast_box)
        # presented lazily; hidden while empty so nothing shows at startup

    def _toast_visibility(self):
        if self.active:
            self.toast_win.present()
        else:
            self.toast_win.set_visible(False)

    def _make_row(self, nid, app, icon, summary, body, urgent):
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        row.add_css_class("toast")
        if urgent:
            row.add_css_class("crit")
        row.set_margin_top(8)
        row.set_margin_bottom(8)
        row.set_margin_start(12)
        row.set_margin_end(8)

        img = Gtk.Image()
        img.set_pixel_size(36)
        img.set_valign(Gtk.Align.START)
        if icon:
            img.set_from_icon_name(icon)
        else:
            img.set_from_icon_name("dialog-information-symbolic")
        row.append(img)

        col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1)
        col.set_hexpand(True)
        if app:
            lab = Gtk.Label(label=app, xalign=0.0)
            lab.add_css_class("app")
            col.append(lab)
        s = Gtk.Label(label=summary, xalign=0.0)
        s.add_css_class("sum")
        s.set_wrap(True)
        s.set_max_width_chars(34)
        col.append(s)
        if body:
            b = Gtk.Label(label=body, xalign=0.0)
            b.add_css_class("body")
            b.set_wrap(True)
            b.set_max_width_chars(34)
            b.set_use_markup(False)
            col.append(b)
        row.append(col)

        x = Gtk.Button(label="✕")
        x.set_valign(Gtk.Align.START)
        x.add_css_class("flat")
        x.connect("clicked", lambda *_: self.close(nid, CLOSE_DISMISSED))
        row.append(x)
        return row

    # ── D-Bus method dispatch ────────────────────────────────────────────
    def on_call(self, _conn, _sender, _path, _iface, method, params, inv):
        if method == "Notify":
            app, replaces, icon, summary, body, actions, hints, timeout = params.unpack()
            nid = self.notify(app, replaces, icon, summary, body, hints, timeout)
            inv.return_value(GLib.Variant("(u)", (nid,)))
        elif method == "CloseNotification":
            (nid,) = params.unpack()
            self.close(nid, CLOSE_REQUEST)
            inv.return_value(None)
        elif method == "GetCapabilities":
            inv.return_value(GLib.Variant("(as)",
                             (["body", "body-markup", "actions", "icon-static", "persistence"],)))
        elif method == "GetServerInformation":
            inv.return_value(GLib.Variant("(ssss)", ("ls-notify", "y5", "1.0", "1.2")))
        else:
            inv.return_value(None)

    def notify(self, app, replaces, icon, summary, body, hints, timeout):
        nid = replaces if replaces else self.next_id
        if not replaces:
            self.next_id += 1
        urgency = 1
        try:
            if "urgency" in hints:
                urgency = int(hints["urgency"])
        except Exception:  # noqa: BLE001
            pass

        if replaces and replaces in self.active:
            self.close(replaces, CLOSE_REQUEST, emit=False)

        row = self._make_row(nid, app, icon, summary, body, urgency >= 2)
        self.toast_box.append(row)

        ms = DEFAULT_TIMEOUT_MS if timeout < 0 else timeout
        src = 0
        if urgency < 2 and ms > 0:  # critical stays until dismissed
            src = GLib.timeout_add(ms, self._expire, nid)
        self.active[nid] = (row, src)

        self.history.append({
            "id": nid, "app": app, "icon": icon, "summary": summary,
            "body": body, "urgency": urgency, "when": time.strftime("%H:%M:%S"),
        })
        self._toast_visibility()
        if self.hist_win and self.hist_win.get_visible():
            self._rebuild_history()
        return nid

    def _expire(self, nid):
        self.close(nid, CLOSE_EXPIRED)
        return GLib.SOURCE_REMOVE

    def close(self, nid, reason, emit=True):
        entry = self.active.pop(nid, None)
        if entry:
            row, src = entry
            if src:
                GLib.source_remove(src)
            self.toast_box.remove(row)
            self._toast_visibility()
        if emit and self.conn:
            self.conn.emit_signal(None, "/org/freedesktop/Notifications",
                                  "org.freedesktop.Notifications",
                                  "NotificationClosed",
                                  GLib.Variant("(uu)", (nid, reason)))

    # ── history panel (SIGUSR1) ──────────────────────────────────────────
    def toggle_history(self):
        if self.hist_win and self.hist_win.get_visible():
            self.hist_win.set_visible(False)
            return GLib.SOURCE_CONTINUE
        if not self.hist_win:
            self._build_history_win()
        self._rebuild_history()
        self.hist_win.present()
        return GLib.SOURCE_CONTINUE

    def _build_history_win(self):
        self.hist_win = Gtk.Window()
        self.hist_win.set_decorated(False)
        self.hist_win.set_default_size(360, 520)
        LS.init_for_window(self.hist_win)
        LS.set_namespace(self.hist_win, "ls-notify-history")
        LS.set_layer(self.hist_win, LS.Layer.OVERLAY)
        LS.set_anchor(self.hist_win, LS.Edge.TOP, True)
        LS.set_anchor(self.hist_win, LS.Edge.RIGHT, True)
        LS.set_anchor(self.hist_win, LS.Edge.BOTTOM, True)
        LS.set_margin(self.hist_win, LS.Edge.TOP, 12)
        LS.set_margin(self.hist_win, LS.Edge.RIGHT, 12)
        LS.set_margin(self.hist_win, LS.Edge.BOTTOM, 12)
        LS.set_keyboard_mode(self.hist_win, LS.KeyboardMode.ON_DEMAND)

        key = Gtk.EventControllerKey()
        key.connect("key-pressed", lambda _c, kv, *_: (
            self.hist_win.set_visible(False) if Gdk.keyval_name(kv) == "Escape" else None, False)[1])
        self.hist_win.add_controller(key)

    def _rebuild_history(self):
        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        outer.add_css_class("hist")
        outer.set_margin_top(12)
        outer.set_margin_bottom(12)
        outer.set_margin_start(12)
        outer.set_margin_end(12)

        head_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        head = Gtk.Label(label=f"Notifications ({len(self.history)})", xalign=0.0)
        head.add_css_class("head")
        head.set_hexpand(True)
        head_row.append(head)
        clear = Gtk.Button(label="Clear")
        clear.connect("clicked", lambda *_: (self.history.clear(), self._rebuild_history()))
        head_row.append(clear)
        outer.append(head_row)

        scroller = Gtk.ScrolledWindow()
        scroller.set_vexpand(True)
        lst = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        for h in reversed(self.history):
            r = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1)
            r.add_css_class("row")
            r.set_margin_bottom(4)
            top = Gtk.Label(xalign=0.0)
            top.set_markup(f"<b>{GLib.markup_escape_text(h['summary'])}</b>")
            top.set_wrap(True)
            r.append(top)
            if h["body"]:
                bl = Gtk.Label(label=h["body"], xalign=0.0)
                bl.set_wrap(True)
                r.append(bl)
            meta = Gtk.Label(label=f"{h['app'] or '?'} · {h['when']}", xalign=0.0)
            meta.add_css_class("when")
            r.append(meta)
            lst.append(r)
        scroller.set_child(lst)
        outer.append(scroller)
        self.hist_win.set_child(outer)

    # ── lifecycle ────────────────────────────────────────────────────────
    def run(self):
        try:
            self.conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        except GLib.Error as e:
            print(f"notify: no session bus ({e}); run inside your session",
                  file=sys.stderr)
            return 1
        node = Gio.DBusNodeInfo.new_for_xml(FDN_XML)
        self.conn.register_object("/org/freedesktop/Notifications",
                                  node.interfaces[0], self.on_call, None, None)
        Gio.bus_own_name_on_connection(
            self.conn, "org.freedesktop.Notifications",
            Gio.BusNameOwnerFlags.REPLACE | Gio.BusNameOwnerFlags.ALLOW_REPLACEMENT,
            lambda *_: print("notify: owning org.freedesktop.Notifications"),
            lambda *_: print("notify: could not own the name (another daemon running?)",
                             file=sys.stderr),
        )
        loop = GLib.MainLoop()
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGUSR1,
                             self.toggle_history)
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT,
                             lambda: (loop.quit(), GLib.SOURCE_REMOVE)[1])
        loop.run()
        return 0


if __name__ == "__main__":
    if not Gtk.init_check():
        print("notify: could not initialise GTK (need a running Wayland session)",
              file=sys.stderr)
        raise SystemExit(1)
    raise SystemExit(Notifier().run())
