// damage-probe — isolate wl_surface.damage (surface-local) from
// wl_surface.damage_buffer (buffer-local), under viewport and fractional scale.
//
// See README.md for the method. In short: commit a DARK buffer, then commit a
// BRIGHT buffer reporting damage for ONE rect. A correct compositor repaints only
// that rect, so the bright region that appears on screen IS the compositor's
// answer to "where did that damage land". Everything else stays dark.
//
// The two damage entry points take rects in DIFFERENT spaces, which is the whole
// point of this tool:
//   wl_surface.damage        — surface-local coordinates
//   wl_surface.damage_buffer — buffer coordinates
// With no viewport and buffer_scale 1 they coincide, which is why the difference
// hides until a viewport or a fractional scale is in play. Under wp_viewporter
// the surface size is the viewport DST, while the buffer keeps its own size and
// the SRC rect selects a sub-region of it — so the same numbers name different
// pixels through each entry point.
//
// Deliberately wl_shm, not dmabuf: damage semantics are a property of the
// protocol, not of how the buffer was allocated, and shm keeps this tool free of
// GBM/EGL/driver variables that have nothing to do with the question.

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>

#include "xdg-shell-client-protocol.h"
#include "viewporter-client-protocol.h"
#include "fractional-scale-v1-client-protocol.h"

// ── globals ──────────────────────────────────────────────────────────────────
static struct wl_display *g_display;
static struct wl_compositor *g_compositor;
static struct wl_shm *g_shm;
static struct xdg_wm_base *g_wm_base;
static struct wp_viewporter *g_viewporter;
static struct wp_fractional_scale_manager_v1 *g_frac_mgr;

static struct wl_surface *g_surface;
static struct xdg_surface *g_xdg_surface;
static struct xdg_toplevel *g_toplevel;
static struct wp_viewport *g_viewport;

static bool g_configured;
static bool g_running = true;
static uint32_t g_preferred_scale_120;  // from wp_fractional_scale_v1, 120ths

// ── configuration ────────────────────────────────────────────────────────────
static int g_buf_w = 400, g_buf_h = 300;   // buffer pixels
static int g_buffer_scale = 1;             // wl_surface.set_buffer_scale
static int g_src_x = -1, g_src_y, g_src_w, g_src_h;   // viewport src (-1 = unset)
static int g_dst_w = -1, g_dst_h;                     // viewport dst (-1 = unset)
static int g_rect_x = 100, g_rect_y = 80, g_rect_w = 120, g_rect_h = 60;
static bool g_use_buffer_damage;           // damage_buffer vs damage
static int g_hold_ms = 4000;

// ── shm helpers ──────────────────────────────────────────────────────────────
static int anon_fd(size_t size) {
    int fd = memfd_create("damage-probe", MFD_CLOEXEC);
    if (fd < 0) { perror("memfd_create"); return -1; }
    if (ftruncate(fd, size) < 0) { perror("ftruncate"); close(fd); return -1; }
    return fd;
}

// A solid-fill buffer. `argb` is premultiplied ARGB8888, as WL_SHM_FORMAT_ARGB8888
// requires — for the opaque colours used here that is just 0xFFrrggbb.
static struct wl_buffer *solid_buffer(int w, int h, uint32_t argb) {
    size_t stride = (size_t)w * 4, size = stride * (size_t)h;
    int fd = anon_fd(size);
    if (fd < 0) return NULL;
    uint32_t *px = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED) { perror("mmap"); close(fd); return NULL; }
    for (size_t i = 0; i < size / 4; i++) px[i] = argb;
    munmap(px, size);

    struct wl_shm_pool *pool = wl_shm_create_pool(g_shm, fd, (int32_t)size);
    struct wl_buffer *buf =
        wl_shm_pool_create_buffer(pool, 0, w, h, (int32_t)stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    return buf;
}

// ── listeners ────────────────────────────────────────────────────────────────
static void wm_base_ping(void *d, struct xdg_wm_base *b, uint32_t serial) {
    (void)d; xdg_wm_base_pong(b, serial);
}
static const struct xdg_wm_base_listener wm_base_listener = { .ping = wm_base_ping };

static void xdg_surface_configure(void *d, struct xdg_surface *s, uint32_t serial) {
    (void)d;
    xdg_surface_ack_configure(s, serial);
    g_configured = true;
}
static const struct xdg_surface_listener xdg_surface_listener = {
    .configure = xdg_surface_configure,
};

static void toplevel_configure(void *d, struct xdg_toplevel *t, int32_t w, int32_t h,
                               struct wl_array *states) {
    (void)d; (void)t; (void)w; (void)h; (void)states;
}
static void toplevel_close(void *d, struct xdg_toplevel *t) {
    (void)d; (void)t; g_running = false;
}
static const struct xdg_toplevel_listener toplevel_listener = {
    .configure = toplevel_configure,
    .close = toplevel_close,
};

static void frac_scale(void *d, struct wp_fractional_scale_v1 *s, uint32_t scale_120) {
    (void)d; (void)s;
    g_preferred_scale_120 = scale_120;
    printf("  <- wp_fractional_scale_v1.preferred_scale = %u/120 (%.3fx)\n",
           scale_120, scale_120 / 120.0);
}
static const struct wp_fractional_scale_v1_listener frac_listener = {
    .preferred_scale = frac_scale,
};

static void registry_global(void *d, struct wl_registry *r, uint32_t name,
                            const char *iface, uint32_t ver) {
    (void)d;
    if (!strcmp(iface, wl_compositor_interface.name))
        g_compositor = wl_registry_bind(r, name, &wl_compositor_interface, ver < 4 ? ver : 4);
    else if (!strcmp(iface, wl_shm_interface.name))
        g_shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
    else if (!strcmp(iface, xdg_wm_base_interface.name))
        g_wm_base = wl_registry_bind(r, name, &xdg_wm_base_interface, 1);
    else if (!strcmp(iface, wp_viewporter_interface.name))
        g_viewporter = wl_registry_bind(r, name, &wp_viewporter_interface, 1);
    else if (!strcmp(iface, wp_fractional_scale_manager_v1_interface.name))
        g_frac_mgr = wl_registry_bind(r, name, &wp_fractional_scale_manager_v1_interface, 1);
}
static void registry_remove(void *d, struct wl_registry *r, uint32_t name) {
    (void)d; (void)r; (void)name;
}
static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_remove,
};

// ── the report ───────────────────────────────────────────────────────────────
//
// Printed BEFORE the damaged commit, so the compositor's own log (if it is
// tracing) interleaves after it and the pairing is unambiguous.
static void report_intent(void) {
    double scale = g_preferred_scale_120 ? g_preferred_scale_120 / 120.0 : 1.0;
    // Surface size per wp_viewporter: dst if set, else src size, else buffer size
    // divided by buffer_scale.
    double surf_w, surf_h;
    if (g_dst_w > 0)      { surf_w = g_dst_w;  surf_h = g_dst_h; }
    else if (g_src_x >= 0){ surf_w = g_src_w;  surf_h = g_src_h; }
    else                  { surf_w = (double)g_buf_w / g_buffer_scale;
                            surf_h = (double)g_buf_h / g_buffer_scale; }

    printf("\n=== DAMAGE-PROBE ===\n");
    printf("  buffer        %dx%d px, buffer_scale=%d\n", g_buf_w, g_buf_h, g_buffer_scale);
    if (g_src_x >= 0) printf("  viewport src  %d,%d %dx%d (buffer coords)\n",
                             g_src_x, g_src_y, g_src_w, g_src_h);
    else              printf("  viewport src  unset\n");
    if (g_dst_w > 0)  printf("  viewport dst  %dx%d (surface coords)\n", g_dst_w, g_dst_h);
    else              printf("  viewport dst  unset\n");
    printf("  surface size  %.2fx%.2f (surface-local)\n", surf_w, surf_h);
    printf("  output scale  %.3fx%s\n", scale,
           g_preferred_scale_120 ? "" : "  (no fractional-scale event)");
    printf("  entry point   %s\n",
           g_use_buffer_damage ? "wl_surface.damage_buffer (BUFFER coords)"
                               : "wl_surface.damage (SURFACE coords)");
    printf("  damage rect   %d,%d %dx%d\n", g_rect_x, g_rect_y, g_rect_w, g_rect_h);

    // What the rect SHOULD cover once mapped, so a mismatch is visible without
    // recomputing it by hand. Only the un-cropped cases are predicted here; a
    // src-cropped viewport is exactly the case this tool exists to observe rather
    // than assume, so it is left blank on purpose.
    if (g_src_x < 0 && g_dst_w < 0) {
        double f = g_use_buffer_damage ? (1.0 / g_buffer_scale) * scale : scale;
        printf("  EXPECT on screen: %.1f,%.1f %.1fx%.1f physical px\n",
               g_rect_x * f, g_rect_y * f, g_rect_w * f, g_rect_h * f);
    } else {
        printf("  EXPECT on screen: (compute from the numbers above — this is the case\n"
               "                     the probe exists to OBSERVE, not to predict)\n");
    }
    printf("  The BRIGHT region is where the damage landed. Dark = untouched.\n\n");
    fflush(stdout);
}

static void usage(const char *argv0) {
    fprintf(stderr,
        "usage: %s [options]\n"
        "  --buffer=WxH          buffer size in pixels (default 400x300)\n"
        "  --buffer-scale=N      wl_surface.set_buffer_scale (default 1)\n"
        "  --src=X,Y,W,H         wp_viewport.set_source, BUFFER coords\n"
        "  --dst=WxH             wp_viewport.set_destination, SURFACE coords\n"
        "  --rect=X,Y,W,H        the damage rect (default 100,80,120x60)\n"
        "  --damage=surface|buffer   which entry point (default surface)\n"
        "  --hold=MS             ms to stay up after the damaged commit (default 4000)\n",
        argv0);
}

int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
        if (!strncmp(a, "--buffer=", 9))            sscanf(a + 9, "%dx%d", &g_buf_w, &g_buf_h);
        else if (!strncmp(a, "--buffer-scale=", 15)) g_buffer_scale = atoi(a + 15);
        else if (!strncmp(a, "--src=", 6))
            sscanf(a + 6, "%d,%d,%d,%d", &g_src_x, &g_src_y, &g_src_w, &g_src_h);
        else if (!strncmp(a, "--dst=", 6))          sscanf(a + 6, "%dx%d", &g_dst_w, &g_dst_h);
        else if (!strncmp(a, "--rect=", 7))
            sscanf(a + 7, "%d,%d,%d,%d", &g_rect_x, &g_rect_y, &g_rect_w, &g_rect_h);
        else if (!strncmp(a, "--damage=", 9))       g_use_buffer_damage = !strcmp(a + 9, "buffer");
        else if (!strncmp(a, "--hold=", 7))         g_hold_ms = atoi(a + 7);
        else { usage(argv[0]); return 2; }
    }

    g_display = wl_display_connect(NULL);
    if (!g_display) { fprintf(stderr, "no WAYLAND_DISPLAY\n"); return 1; }
    struct wl_registry *reg = wl_display_get_registry(g_display);
    wl_registry_add_listener(reg, &registry_listener, NULL);
    wl_display_roundtrip(g_display);

    if (!g_compositor || !g_shm || !g_wm_base) {
        fprintf(stderr, "missing wl_compositor / wl_shm / xdg_wm_base\n");
        return 1;
    }
    printf("globals: viewporter=%s fractional_scale=%s\n",
           g_viewporter ? "yes" : "NO", g_frac_mgr ? "yes" : "NO");
    if ((g_src_x >= 0 || g_dst_w > 0) && !g_viewporter) {
        fprintf(stderr, "viewport requested but wp_viewporter is not advertised\n");
        return 1;
    }
    xdg_wm_base_add_listener(g_wm_base, &wm_base_listener, NULL);

    g_surface = wl_compositor_create_surface(g_compositor);
    if (g_frac_mgr) {
        struct wp_fractional_scale_v1 *fs =
            wp_fractional_scale_manager_v1_get_fractional_scale(g_frac_mgr, g_surface);
        wp_fractional_scale_v1_add_listener(fs, &frac_listener, NULL);
    }
    g_xdg_surface = xdg_wm_base_get_xdg_surface(g_wm_base, g_surface);
    xdg_surface_add_listener(g_xdg_surface, &xdg_surface_listener, NULL);
    g_toplevel = xdg_surface_get_toplevel(g_xdg_surface);
    xdg_toplevel_add_listener(g_toplevel, &toplevel_listener, NULL);
    xdg_toplevel_set_title(g_toplevel, "damage-probe");
    wl_surface_commit(g_surface);

    while (g_running && !g_configured) {
        if (wl_display_dispatch(g_display) < 0) { fprintf(stderr, "dispatch failed\n"); return 1; }
    }

    if (g_viewporter) g_viewport = wp_viewporter_get_viewport(g_viewporter, g_surface);
    if (g_buffer_scale != 1) wl_surface_set_buffer_scale(g_surface, g_buffer_scale);
    if (g_viewport && g_src_x >= 0)
        wp_viewport_set_source(g_viewport, wl_fixed_from_int(g_src_x), wl_fixed_from_int(g_src_y),
                               wl_fixed_from_int(g_src_w), wl_fixed_from_int(g_src_h));
    if (g_viewport && g_dst_w > 0) wp_viewport_set_destination(g_viewport, g_dst_w, g_dst_h);

    // ---- Frame 1: DARK everywhere, full damage. This is the baseline the second
    // commit is read against, so it must be unambiguously complete.
    struct wl_buffer *dark = solid_buffer(g_buf_w, g_buf_h, 0xFF101018);
    struct wl_buffer *bright = solid_buffer(g_buf_w, g_buf_h, 0xFF30F0C0);
    if (!dark || !bright) return 1;

    wl_surface_attach(g_surface, dark, 0, 0);
    wl_surface_damage_buffer(g_surface, 0, 0, g_buf_w, g_buf_h);
    wl_surface_commit(g_surface);
    wl_display_roundtrip(g_display);
    // Let the dark frame actually reach the screen before the damaged one lands;
    // committing both inside one compositor frame would collapse them and the
    // whole surface would repaint, which is precisely the observation being made.
    struct timespec settle = { .tv_sec = 0, .tv_nsec = 300L * 1000000L };
    nanosleep(&settle, NULL);
    // Drain the fractional-scale event, so the report prints the real scale.
    wl_display_roundtrip(g_display);

    // ---- Frame 2: BRIGHT everywhere, damage ONE rect through the chosen entry
    // point. Only that rect should repaint.
    report_intent();
    wl_surface_attach(g_surface, bright, 0, 0);
    if (g_use_buffer_damage)
        wl_surface_damage_buffer(g_surface, g_rect_x, g_rect_y, g_rect_w, g_rect_h);
    else
        wl_surface_damage(g_surface, g_rect_x, g_rect_y, g_rect_w, g_rect_h);
    wl_surface_commit(g_surface);
    wl_display_roundtrip(g_display);

    struct timespec hold = { .tv_sec = g_hold_ms / 1000,
                             .tv_nsec = (long)(g_hold_ms % 1000) * 1000000L };
    nanosleep(&hold, NULL);
    printf("=== done ===\n");
    return 0;
}
