# dmabuf format tester — see README.md
#
# The wayland-scanner output is checked in, so `make` works without
# wayland-protocols installed. `make regen` refreshes it from the system XML.

CFLAGS   ?= -O2 -Wall -Wno-comment
PROTO    ?= /usr/share/wayland-protocols
SCANNER  ?= wayland-scanner
LIBS_WL  := -lwayland-client
LIBS_GBM := -lgbm
LIBS_VK  := -lvulkan

DMABUF_GEN := linux-dmabuf-v1-protocol.c linux-dmabuf-v1-client-protocol.h
XDG_GEN    := xdg-shell-protocol.c xdg-shell-client-protocol.h

TOOLS := dmabuf-probe dmabuf-format-test dmabuf-draw vkfmt

all: color-stress $(TOOLS)

dmabuf-probe: dmabuf-probe.c $(DMABUF_GEN)
	$(CC) $(CFLAGS) $< linux-dmabuf-v1-protocol.c $(LIBS_WL) -o $@

dmabuf-format-test: dmabuf-format-test.c $(DMABUF_GEN)
	$(CC) $(CFLAGS) $< linux-dmabuf-v1-protocol.c $(LIBS_WL) $(LIBS_GBM) -o $@

dmabuf-draw: dmabuf-draw.c $(DMABUF_GEN) $(XDG_GEN)
	$(CC) $(CFLAGS) $< linux-dmabuf-v1-protocol.c xdg-shell-protocol.c \
		$(LIBS_WL) $(LIBS_GBM) -lEGL -lGLESv2 -lm -o $@

# Device capability, no wayland and no surface — safe against a live session,
# unlike the tools above. Answers what query::renderable / ::sampleable and
# modifier::modifiers_with see.
vkfmt: vkfmt.c
	$(CC) $(CFLAGS) $< $(LIBS_VK) -o $@

# Throwaway probes kept because they answered "why does GBM refuse this": on the
# NVIDIA backend, RENDERING|LINEAR fails while USE_WRITE yields a LINEAR bo.
gbmtest: gbmtest.c
	$(CC) $(CFLAGS) $< $(LIBS_GBM) -o $@
gbmtry: gbmtry.c
	$(CC) $(CFLAGS) $< $(LIBS_GBM) -o $@

regen:
	$(SCANNER) client-header $(PROTO)/stable/linux-dmabuf/linux-dmabuf-v1.xml linux-dmabuf-v1-client-protocol.h
	$(SCANNER) private-code  $(PROTO)/stable/linux-dmabuf/linux-dmabuf-v1.xml linux-dmabuf-v1-protocol.c
	$(SCANNER) client-header $(PROTO)/stable/xdg-shell/xdg-shell.xml xdg-shell-client-protocol.h
	$(SCANNER) private-code  $(PROTO)/stable/xdg-shell/xdg-shell.xml xdg-shell-protocol.c

clean:
	rm -f $(TOOLS) color-stress gbmtest gbmtry

.PHONY: all regen clean

color-stress: color-stress.c $(DMABUF_GEN)
	$(CC) $(CFLAGS) $< linux-dmabuf-v1-protocol.c -lwayland-client -lgbm -o $@
