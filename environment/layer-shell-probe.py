#!/usr/bin/env python3
"""
layer-shell-probe — exercise every wlr-layer-shell layer and anchor against y5.

gtk-layer-shell is installed on this box, so this drives it through GObject
introspection: one interactive window you can place on ANY layer, anchored to
ANY combination of edges, with selectable keyboard interactivity. That covers
the whole surface API a real dock/bar/wallpaper uses — including an
"interactive wallpaper" (--layer background --keyboard on-demand).

Usage:
    ./layer-shell-probe.py [--layer background|bottom|top|overlay]
                           [--anchor top,bottom,left,right]   (comma list, any subset)
                           [--margin N] [--exclusive N]
                           [--keyboard none|on-demand|exclusive]

Examples:
    # macOS-style bottom dock (Top layer, hugs bottom edge):
    ./layer-shell-probe.py --layer top --anchor bottom --margin 12

    # full-width bar at the top:
    ./layer-shell-probe.py --layer top --anchor top,left,right --exclusive 40

    # left rail:
    ./layer-shell-probe.py --layer overlay --anchor top,bottom,left

    # INTERACTIVE WALLPAPER — fills the screen behind windows, still clickable:
    ./layer-shell-probe.py --layer background --anchor top,bottom,left,right --keyboard on-demand

The window shows its live layer/anchor config and a button + text entry so you
can confirm pointer AND keyboard input actually reach the surface at that layer.
"""
import argparse
import gi

gi.require_version("Gtk", "3.0")
gi.require_version("GtkLayerShell", "0.1")
from gi.repository import Gtk, GtkLayerShell  # noqa: E402

LAYERS = {
    "background": GtkLayerShell.Layer.BACKGROUND,
    "bottom": GtkLayerShell.Layer.BOTTOM,
    "top": GtkLayerShell.Layer.TOP,
    "overlay": GtkLayerShell.Layer.OVERLAY,
}
EDGES = {
    "top": GtkLayerShell.Edge.TOP,
    "bottom": GtkLayerShell.Edge.BOTTOM,
    "left": GtkLayerShell.Edge.LEFT,
    "right": GtkLayerShell.Edge.RIGHT,
}
KEYBOARD = {
    "none": GtkLayerShell.KeyboardMode.NONE,
    "on-demand": GtkLayerShell.KeyboardMode.ON_DEMAND,
    "exclusive": GtkLayerShell.KeyboardMode.EXCLUSIVE,
}


def main() -> None:
    ap = argparse.ArgumentParser(description="wlr-layer-shell probe for y5")
    ap.add_argument("--layer", choices=LAYERS, default="top")
    ap.add_argument("--anchor", default="bottom",
                    help="comma list of edges: top,bottom,left,right (any subset)")
    ap.add_argument("--margin", type=int, default=0)
    ap.add_argument("--exclusive", type=int, default=0,
                    help="exclusive zone in px (reserve space so windows don't overlap)")
    ap.add_argument("--keyboard", choices=KEYBOARD, default="on-demand")
    args = ap.parse_args()

    anchors = [e.strip() for e in args.anchor.split(",") if e.strip()]
    for a in anchors:
        if a not in EDGES:
            ap.error(f"unknown anchor edge {a!r}; pick from {list(EDGES)}")

    win = Gtk.Window()
    GtkLayerShell.init_for_window(win)
    GtkLayerShell.set_layer(win, LAYERS[args.layer])
    GtkLayerShell.set_namespace(win, "layer-shell-probe")
    GtkLayerShell.set_keyboard_mode(win, KEYBOARD[args.keyboard])

    for name, edge in EDGES.items():
        on = name in anchors
        GtkLayerShell.set_anchor(win, edge, on)
        if on and args.margin:
            GtkLayerShell.set_margin(win, edge, args.margin)
    if args.exclusive:
        GtkLayerShell.set_exclusive_zone(win, args.exclusive)

    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
    box.set_margin_top(16); box.set_margin_bottom(16)
    box.set_margin_start(16); box.set_margin_end(16)
    box.add(Gtk.Label(label=f"layer={args.layer}  anchor={','.join(anchors) or 'none'}"))
    box.add(Gtk.Label(label=f"keyboard={args.keyboard}  margin={args.margin}  exclusive={args.exclusive}"))

    clicks = {"n": 0}
    btn = Gtk.Button(label="click me (pointer test)")
    def on_click(_b):
        clicks["n"] += 1
        btn.set_label(f"clicked {clicks['n']}x — pointer reaches this layer")
    btn.connect("clicked", on_click)
    box.add(btn)

    entry = Gtk.Entry()
    entry.set_placeholder_text("type here (keyboard test)")
    box.add(entry)

    win.add(box)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()


if __name__ == "__main__":
    main()
