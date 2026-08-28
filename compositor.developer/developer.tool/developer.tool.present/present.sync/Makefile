# sync-probe — see README.md
#
# The wayland-scanner output is checked in, so `make` works without
# wayland-protocols installed. `make regen` refreshes it from the system XML.

CFLAGS  ?= -O2 -Wall -Wno-comment
PROTO   ?= /usr/share/wayland-protocols
SCANNER ?= wayland-scanner
LIBS    := -lwayland-client -lvulkan

XDG_GEN     := xdg-shell-protocol.c xdg-shell-client-protocol.h
PRESENT_GEN := presentation-time-protocol.c presentation-time-client-protocol.h
DMABUF_GEN  := linux-dmabuf-v1-protocol.c linux-dmabuf-v1-client-protocol.h
VIEWPORT_GEN := viewporter-protocol.c viewporter-client-protocol.h
GEN         := $(XDG_GEN) $(PRESENT_GEN) $(DMABUF_GEN) $(VIEWPORT_GEN)

all: sync-probe

sync-probe: sync-probe.c $(GEN)
	$(CC) $(CFLAGS) $< xdg-shell-protocol.c presentation-time-protocol.c \
		linux-dmabuf-v1-protocol.c viewporter-protocol.c $(LIBS) -o $@

xdg-shell-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/xdg-shell/xdg-shell.xml $@
xdg-shell-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/xdg-shell/xdg-shell.xml $@
presentation-time-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/presentation-time/presentation-time.xml $@
presentation-time-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/presentation-time/presentation-time.xml $@
linux-dmabuf-v1-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/linux-dmabuf/linux-dmabuf-v1.xml $@
linux-dmabuf-v1-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/linux-dmabuf/linux-dmabuf-v1.xml $@
viewporter-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/viewporter/viewporter.xml $@
viewporter-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/viewporter/viewporter.xml $@

regen:
	rm -f $(GEN)
	$(MAKE) $(GEN)

clean:
	rm -f sync-probe

distclean: clean
	rm -f $(GEN)

.PHONY: all regen clean distclean
