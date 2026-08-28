# damage-probe — see README.md
#
# The wayland-scanner output is checked in, so `make` works without
# wayland-protocols installed. `make regen` refreshes it from the system XML.

CFLAGS  ?= -O2 -Wall -Wno-comment
PROTO   ?= /usr/share/wayland-protocols
SCANNER ?= wayland-scanner
LIBS    := -lwayland-client -lrt

XDG_GEN      := xdg-shell-protocol.c xdg-shell-client-protocol.h
VIEWPORT_GEN := viewporter-protocol.c viewporter-client-protocol.h
FRAC_GEN     := fractional-scale-v1-protocol.c fractional-scale-v1-client-protocol.h
GEN          := $(XDG_GEN) $(VIEWPORT_GEN) $(FRAC_GEN)

all: damage-probe

damage-probe: damage-probe.c $(GEN)
	$(CC) $(CFLAGS) $< xdg-shell-protocol.c viewporter-protocol.c \
		fractional-scale-v1-protocol.c $(LIBS) -o $@

xdg-shell-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/xdg-shell/xdg-shell.xml $@
xdg-shell-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/xdg-shell/xdg-shell.xml $@
viewporter-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/viewporter/viewporter.xml $@
viewporter-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/viewporter/viewporter.xml $@
fractional-scale-v1-protocol.c:
	$(SCANNER) private-code $(PROTO)/staging/fractional-scale/fractional-scale-v1.xml $@
fractional-scale-v1-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/staging/fractional-scale/fractional-scale-v1.xml $@

regen:
	rm -f $(GEN)
	$(MAKE) $(GEN)

clean:
	rm -f damage-probe

distclean: clean
	rm -f $(GEN)

.PHONY: all regen clean distclean
