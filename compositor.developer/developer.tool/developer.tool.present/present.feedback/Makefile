# present-probe — see README.md
#
# The wayland-scanner output is checked in, so `make` works without
# wayland-protocols installed. `make regen` refreshes it from the system XML.

CFLAGS  ?= -O2 -Wall -Wno-comment
PROTO   ?= /usr/share/wayland-protocols
SCANNER ?= wayland-scanner
LIBS_WL := -lwayland-client

XDG_GEN     := xdg-shell-protocol.c xdg-shell-client-protocol.h
PRESENT_GEN := presentation-time-protocol.c presentation-time-client-protocol.h

all: present-probe

present-probe: present-probe.c $(XDG_GEN) $(PRESENT_GEN)
	$(CC) $(CFLAGS) $< xdg-shell-protocol.c presentation-time-protocol.c $(LIBS_WL) -o $@

xdg-shell-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/xdg-shell/xdg-shell.xml $@
xdg-shell-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/xdg-shell/xdg-shell.xml $@
presentation-time-protocol.c:
	$(SCANNER) private-code $(PROTO)/stable/presentation-time/presentation-time.xml $@
presentation-time-client-protocol.h:
	$(SCANNER) client-header $(PROTO)/stable/presentation-time/presentation-time.xml $@

regen:
	rm -f $(XDG_GEN) $(PRESENT_GEN)
	$(MAKE) $(XDG_GEN) $(PRESENT_GEN)

clean:
	rm -f present-probe

distclean: clean
	rm -f $(XDG_GEN) $(PRESENT_GEN)

.PHONY: all regen clean distclean
