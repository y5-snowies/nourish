Remaining
- Activation travel from WLR explictly
- Check world and output changes
- Make sure preference gate hot reloads


# Layer shell applications

* All layer-shell anchors:

  * Top
  * Bottom
  * Left
  * Right
  * Top+Left
  * Top+Right
  * Bottom+Left
  * Bottom+Right
  * All edges (fullscreen panel/background)
  * No anchors (floating)
* Exclusive zone:

  * `-1`
  * `0`
  * Positive values
* Margins
* Keyboard interactivity
* Different layer levels:

  * Background
  * Bottom
  * Top
  * Overlay
* Dynamic reconfiguration
* Popup creation from a layer surface
* Input regions
* Empty input regions (click-through)
* Opaque regions
* Interactive wallpapers
* Multiple outputs
* Fractional scaling
* HiDPI

## 1. gtk4-layer-shell demo (best starting point)

Install:

```bash
sudo dnf install gtk4-layer-shell
```

Depending on the package version, there may be demo binaries. If not, the source repository includes examples that expose most layer-shell options.

---

## 2. wayland-protocols layer-shell examples

Several compositors include protocol examples.

Worth building:

* wlroots examples
* wayfire examples
* labwc examples

They often let you specify:

```text
--layer overlay
--anchor top,left
--exclusive-zone 40
--margin 5
```

---

## 3. wlr-randr

Useful for changing outputs while layer surfaces are alive.

```bash
sudo dnf install wlr-randr
```

---

## 4. swaybg

Exercises background layer.

```bash
sudo dnf install swaybg
```

---

## 5. swayidle

Creates invisible/background layer surfaces.

---

## 6. swaync

Notification daemon.

Exercises:

* overlay layer
* popups
* animations
* dynamic sizing

---

## 7. waybar

Probably the single best real-world test.

```bash
sudo dnf install waybar
```

Can configure:

* top
* bottom
* left
* right

and change exclusive zones.

---

## 8. nwg-panel

Even better because it supports:

* top
* bottom
* left
* right
* floating
* autohide
* margins

---

## 9. Hyprpaper

Interactive-ish wallpaper/background.

Exercises background layer.

---

## 10. mpvpaper

Wallpaper using mpv.

Useful for:

* video wallpaper
* continuous frame updates

---

## 11. swww

Animated wallpaper.

Useful because it continuously updates background surfaces.

---

## 12. wpaperd

Another wallpaper daemon with transitions.

---

## 13. eww

Excellent stress test.

Can create arbitrary layer-shell windows:

* overlay
* top
* bottom
* background

and move them live.

---

## 14. wlogout

Overlay with buttons.

Exercises:

* overlay
* keyboard
* pointer
* exclusive zone

---

## 15. bemenu / wofi
While not layer-shell everywhere, they exercise popup/input interactions.

---

# Interactive wallpaper


An interactive wallpaper is typically:

* background layer
* **keyboard_interactivity = exclusive or on-demand**
* input region enabled

or

* background layer
* transparent regions
* custom hit testing

Applications like:

* mpvpaper
* custom GTK layer-shell apps
* SDL layer-shell clients


---
```text
--layer background|bottom|top|overlay

--anchor top
--anchor bottom
--anchor left
--anchor right

--exclusive-zone -1|0|30

--margin-top 10

--margin-left 20

--keyboard none|exclusive|ondemand

--width 300
--height 200

--input full
--input none
--input circle
--input holes

--opaque full
--opaque none

--popup

--transparent

--animated

--interactive
```

