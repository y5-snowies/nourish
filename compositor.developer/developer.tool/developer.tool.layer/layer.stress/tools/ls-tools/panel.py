#!/usr/bin/env python3
"""Configurable gtk4-layer-shell test client.

Fedora's gtk4-layer-shell package ships no `gtk4-layer-demo` binary, so this
stands in for it and drives every knob in LS.md: layer, anchors, exclusive
zone, margins, keyboard interactivity, size, popups and click-through.

Each invocation is ONE layer surface; LS.sh launches several with different
flags to cover the matrix. Run standalone, e.g.:

    ./panel.py --layer top --anchor top,left,right --exclusive-zone auto \
               --height 34 --label "top bar" --keyboard none
"""
import os
import sys

# gtk4-layer-shell intercepts libwayland and MUST be loaded before GTK opens its
# Wayland connection. Pulled in lazily through Python GI it loads too late, so the
# surface silently falls back to a normal resizable xdg_toplevel. Re-exec once with
# the real library in LD_PRELOAD so its constructor installs the hook first AND the
# GI setters (set_anchor/…) drive the same instance.
# NB: preload libgtk4-layer-shell itself, NOT liblayer-shell-preload.so — that shim
# is a standalone env-var "layerify any app" tool that ignores the API and would
# create a default (anchor 0, centred) surface.
_PRELOAD = "libgtk4-layer-shell.so.0"
if _PRELOAD not in os.environ.get("LD_PRELOAD", ""):
    os.environ["LD_PRELOAD"] = ":".join(
        p for p in (_PRELOAD, os.environ.get("LD_PRELOAD", "")) if p)
    os.execv(sys.executable, [sys.executable, *sys.argv])

import argparse

import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Gtk4LayerShell", "1.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gtk, Gtk4LayerShell as LS, Gdk, GLib, Gio  # noqa: E402

EDGES = {
    "top": LS.Edge.TOP,
    "bottom": LS.Edge.BOTTOM,
    "left": LS.Edge.LEFT,
    "right": LS.Edge.RIGHT,
}
LAYERS = {
    "background": LS.Layer.BACKGROUND,
    "bottom": LS.Layer.BOTTOM,
    "top": LS.Layer.TOP,
    "overlay": LS.Layer.OVERLAY,
}
KBD = {
    "none": LS.KeyboardMode.NONE,
    "exclusive": LS.KeyboardMode.EXCLUSIVE,
    "ondemand": LS.KeyboardMode.ON_DEMAND,
}


def parse_args():
    p = argparse.ArgumentParser(description="gtk4-layer-shell test client")
    p.add_argument("--layer", choices=LAYERS, default="top")
    p.add_argument("--anchor", default="",
                   help="comma list of top,bottom,left,right (empty = floating)")
    p.add_argument("--exclusive-zone", default="0",
                   help="int, or 'auto' to reserve exactly the window size")
    p.add_argument("--margin-top", type=int, default=0)
    p.add_argument("--margin-bottom", type=int, default=0)
    p.add_argument("--margin-left", type=int, default=0)
    p.add_argument("--margin-right", type=int, default=0)
    p.add_argument("--keyboard", choices=KBD, default="none")
    p.add_argument("--width", type=int, default=0, help="0 = let anchors decide")
    p.add_argument("--height", type=int, default=0)
    p.add_argument("--label", default="layer surface")
    p.add_argument("--color", default="#1e1e2e", help="background CSS color")
    p.add_argument("--namespace", default="ls-panel")
    p.add_argument("--popup", action="store_true",
                   help="add a button that opens an xdg_popup off the surface")
    p.add_argument("--auto-popup", action="store_true",
                   help="open the popup on its own ~1s after mapping and re-open every "
                        "4s (implies --popup) — exercises the layer new_popup path hands-free")
    p.add_argument("--click-through", action="store_true",
                   help="best-effort empty input region (background click-through)")
    return p.parse_args()


def build_ui(a):
    root = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
    root.set_margin_top(6)
    root.set_margin_bottom(6)
    root.set_margin_start(10)
    root.set_margin_end(10)
    root.add_css_class("ls-root")

    anchors = a.anchor if a.anchor else "floating"
    caption = Gtk.Label(label=f"{a.label}  [{a.layer}/{anchors}/zone={a.exclusive_zone}]")
    caption.set_hexpand(True)
    caption.set_xalign(0.0)
    root.append(caption)

    keylabel = Gtk.Label(label="")
    keylabel.add_css_class("ls-key")
    root.append(keylabel)

    pop = None
    if a.popup or a.auto_popup:
        btn = Gtk.MenuButton(label="popup ▾")
        pop = Gtk.Popover()
        pbox = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        pbox.set_margin_top(8)
        pbox.set_margin_bottom(8)
        pbox.set_margin_start(12)
        pbox.set_margin_end(12)
        pbox.append(Gtk.Label(label="xdg_popup off a layer surface"))
        pbox.append(Gtk.Button(label="an item"))
        pbox.append(Gtk.Button(label="another item"))
        pop.set_child(pbox)
        btn.set_popover(pop)
        root.append(btn)

    return root, keylabel, pop


def apply_css(color):
    css = Gtk.CssProvider()
    css.load_from_data(
        f".ls-root {{ background:{color}; color:#eee; border-radius:6px; }}"
        ".ls-key { color:#f9e2af; font-family:monospace; }".encode()
    )
    Gtk.StyleContext.add_provider_for_display(
        Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
    )


def on_activate(app, a):
    win = Gtk.ApplicationWindow(application=app)
    win.set_decorated(False)
    if a.width or a.height:
        win.set_default_size(max(a.width, 1), max(a.height, 1))

    LS.init_for_window(win)
    LS.set_namespace(win, a.namespace)
    LS.set_layer(win, LAYERS[a.layer])

    for name, edge in EDGES.items():
        LS.set_anchor(win, edge, name in a.anchor.split(","))

    LS.set_margin(win, LS.Edge.TOP, a.margin_top)
    LS.set_margin(win, LS.Edge.BOTTOM, a.margin_bottom)
    LS.set_margin(win, LS.Edge.LEFT, a.margin_left)
    LS.set_margin(win, LS.Edge.RIGHT, a.margin_right)

    if a.exclusive_zone == "auto":
        LS.auto_exclusive_zone_enable(win)
    else:
        LS.set_exclusive_zone(win, int(a.exclusive_zone))

    LS.set_keyboard_mode(win, KBD[a.keyboard])

    apply_css(a.color)
    root, keylabel, popover = build_ui(a)
    win.set_child(root)

    if a.auto_popup and popover is not None:
        # Open ~1s after map (so the surface is configured first), then re-open every
        # 4s. Each popup() creates a fresh xdg_popup adopted via the layer surface's
        # get_popup — the compositor's `new_popup` path, hands-free.
        GLib.timeout_add(1000, lambda: (popover.popup(), False)[1])
        GLib.timeout_add_seconds(4, lambda: (popover.popup(), True)[1])

    if a.keyboard != "none":
        keyctl = Gtk.EventControllerKey()

        def on_key(_c, keyval, _code, _state):
            name = Gdk.keyval_name(keyval) or "?"
            keylabel.set_text(f"key: {name}")
            if name == "Escape":
                app.quit()
            return False

        keyctl.connect("key-pressed", on_key)
        win.add_controller(keyctl)

    win.present()

    if a.click_through:
        # Best effort: an empty input region makes the whole surface click-through.
        surface = win.get_surface()
        try:
            import cairo
            region = cairo.Region()  # empty
            surface.set_input_region(region)
        except Exception as e:  # noqa: BLE001
            print(f"panel: click-through unavailable on this GTK build: {e}",
                  file=sys.stderr)


def main():
    a = parse_args()
    app = Gtk.Application(application_id=None,
                          flags=Gio.ApplicationFlags.NON_UNIQUE)
    app.connect("activate", on_activate, a)
    return app.run([])


if __name__ == "__main__":
    raise SystemExit(main())
