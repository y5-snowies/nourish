// present-probe — does this compositor actually answer wp_presentation?
//
// Maps a small toplevel, then commits a new frame every time the previous one
// gets its frame callback, requesting `wp_presentation.feedback` on each commit.
// Reports, per feedback: presented vs discarded, and whether the MSC sequence and
// the presentation timestamp ADVANCE between presentations.
//
// The distinction this exists to draw: `wl_surface.frame` callbacks and
// `wp_presentation_feedback` are different protocols with different guarantees,
// and a compositor can honour the first while never answering the second. A
// client that paces on presentation feedback then sees every frame reported as
// `discarded` — "your content never reached the screen" — which is a lie if it
// did. Counting both side by side is the only way to tell them apart from
// outside.
//
// Read-only with respect to the compositor: it creates one surface and destroys
// it. Safe to run against a live session.

#define _GNU_SOURCE
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#include <wayland-client.h>

#include "presentation-time-client-protocol.h"
#include "xdg-shell-client-protocol.h"

static struct wl_compositor *g_compositor;
static struct xdg_wm_base *g_wm_base;
static struct wl_shm *g_shm;
static struct wp_presentation *g_presentation;
static struct wl_surface *g_surface;
static struct xdg_surface *g_xdg_surface;
static struct xdg_toplevel *g_toplevel;

static int g_width = 320, g_height = 240;
static int g_configured;
static int g_running = 1;
static int g_verbose;

// Counters.
static unsigned g_committed, g_frame_cb, g_presented, g_discarded;
// Sequence / timestamp advance tracking.
static uint64_t g_last_seq, g_last_ns;
static int g_have_seq, g_have_ns;
static unsigned g_seq_advanced, g_seq_stalled, g_seq_went_back;
static unsigned g_time_advanced, g_time_stalled, g_time_went_back;
static uint32_t g_last_refresh;
static uint32_t g_flags_or;

// Two shm buffers, alternated, so a commit never reuses one the compositor
// still holds.
struct slot {
    struct wl_buffer *buffer;
    void *data;
    int busy;
};
static struct slot g_slots[2];
static size_t g_slot_bytes;

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

// ---------------------------------------------------------------- buffers

static void buffer_release(void *data, struct wl_buffer *buffer) {
    (void)buffer;
    ((struct slot *)data)->busy = 0;
}
static const struct wl_buffer_listener buffer_listener = { buffer_release };

static int make_slots(void) {
    int stride = g_width * 4;
    g_slot_bytes = (size_t)stride * g_height;
    size_t total = g_slot_bytes * 2;

    int fd = memfd_create("present-probe", MFD_CLOEXEC);
    if (fd < 0) { perror("memfd_create"); return -1; }
    if (ftruncate(fd, total) < 0) { perror("ftruncate"); close(fd); return -1; }

    void *base = mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (base == MAP_FAILED) { perror("mmap"); close(fd); return -1; }

    struct wl_shm_pool *pool = wl_shm_create_pool(g_shm, fd, (int32_t)total);
    for (int i = 0; i < 2; i++) {
        g_slots[i].data = (char *)base + g_slot_bytes * i;
        g_slots[i].buffer = wl_shm_pool_create_buffer(
            pool, (int32_t)(g_slot_bytes * i), g_width, g_height, stride, WL_SHM_FORMAT_XRGB8888);
        wl_buffer_add_listener(g_slots[i].buffer, &buffer_listener, &g_slots[i]);
        g_slots[i].busy = 0;
    }
    wl_shm_pool_destroy(pool);
    close(fd);
    return 0;
}

// Fill with a colour that changes per frame, so the content genuinely differs
// and the compositor has real damage to present rather than a no-op commit.
static struct slot *next_slot(unsigned frame) {
    for (int i = 0; i < 2; i++) {
        if (!g_slots[i].busy) {
            uint32_t c = 0xff000000u | ((frame * 7) & 0xff) << 16 | ((frame * 13) & 0xff) << 8
                         | ((frame * 3) & 0xff);
            uint32_t *px = g_slots[i].data;
            for (size_t p = 0; p < g_slot_bytes / 4; p++) px[p] = c;
            return &g_slots[i];
        }
    }
    return NULL;
}

// ------------------------------------------------------- presentation feedback

static void fb_sync_output(void *d, struct wp_presentation_feedback *f, struct wl_output *o) {
    (void)d; (void)f; (void)o;
}

static void fb_presented(void *data, struct wp_presentation_feedback *fb, uint32_t tv_sec_hi,
                         uint32_t tv_sec_lo, uint32_t tv_nsec, uint32_t refresh,
                         uint32_t seq_hi, uint32_t seq_lo, uint32_t flags) {
    (void)data;
    g_presented++;
    g_last_refresh = refresh;
    g_flags_or |= flags;

    uint64_t seq = ((uint64_t)seq_hi << 32) | seq_lo;
    uint64_t ns = (((uint64_t)tv_sec_hi << 32) | tv_sec_lo) * 1000000000ull + tv_nsec;

    if (g_have_seq) {
        if (seq > g_last_seq) g_seq_advanced++;
        else if (seq == g_last_seq) g_seq_stalled++;
        else g_seq_went_back++;
    }
    if (g_have_ns) {
        if (ns > g_last_ns) g_time_advanced++;
        else if (ns == g_last_ns) g_time_stalled++;
        else g_time_went_back++;
    }

    if (g_verbose || g_presented <= 5) {
        printf("  presented #%u  seq=%llu%s  t=%llu.%09llu%s  refresh=%uns flags=0x%x\n",
               g_presented, (unsigned long long)seq,
               g_have_seq ? (seq > g_last_seq ? " (+)" : seq == g_last_seq ? " (SAME)" : " (BACK)") : "",
               (unsigned long long)(ns / 1000000000ull), (unsigned long long)(ns % 1000000000ull),
               g_have_ns ? (ns > g_last_ns ? " (+)" : ns == g_last_ns ? " (SAME)" : " (BACK)") : "",
               refresh, flags);
    }
    g_last_seq = seq; g_have_seq = 1;
    g_last_ns = ns;   g_have_ns = 1;
    wp_presentation_feedback_destroy(fb);
}

static void fb_discarded(void *data, struct wp_presentation_feedback *fb) {
    (void)data;
    g_discarded++;
    if (g_verbose || g_discarded <= 5)
        printf("  discarded #%u\n", g_discarded);
    wp_presentation_feedback_destroy(fb);
}

static const struct wp_presentation_feedback_listener fb_listener = {
    fb_sync_output, fb_presented, fb_discarded,
};

// ------------------------------------------------------------------ frame

static void commit_frame(void);

static void frame_done(void *data, struct wl_callback *cb, uint32_t time) {
    (void)data; (void)time;
    wl_callback_destroy(cb);
    g_frame_cb++;
    commit_frame();
}
static const struct wl_callback_listener frame_listener = { frame_done };

static void commit_frame(void) {
    if (!g_configured || !g_running) return;
    struct slot *s = next_slot(g_committed);
    if (!s) return;  // both held; the next release re-arms us via frame_done

    struct wl_callback *cb = wl_surface_frame(g_surface);
    wl_callback_add_listener(cb, &frame_listener, NULL);

    if (g_presentation) {
        struct wp_presentation_feedback *fb = wp_presentation_feedback(g_presentation, g_surface);
        wp_presentation_feedback_add_listener(fb, &fb_listener, NULL);
    }

    s->busy = 1;
    wl_surface_attach(g_surface, s->buffer, 0, 0);
    wl_surface_damage_buffer(g_surface, 0, 0, g_width, g_height);
    wl_surface_commit(g_surface);
    g_committed++;
}

// ------------------------------------------------------------------ shell

static void xdg_surface_configure(void *data, struct xdg_surface *xs, uint32_t serial) {
    (void)data;
    xdg_surface_ack_configure(xs, serial);
    if (!g_configured) {
        g_configured = 1;
        commit_frame();
    }
}
static const struct xdg_surface_listener xdg_surface_listener = { xdg_surface_configure };

static void toplevel_configure(void *d, struct xdg_toplevel *t, int32_t w, int32_t h,
                               struct wl_array *states) {
    (void)d; (void)t; (void)w; (void)h; (void)states;
}
static void toplevel_close(void *d, struct xdg_toplevel *t) {
    (void)d; (void)t;
    g_running = 0;
}
static const struct xdg_toplevel_listener toplevel_listener = { toplevel_configure, toplevel_close };

static void wm_base_ping(void *d, struct xdg_wm_base *b, uint32_t serial) {
    (void)d;
    xdg_wm_base_pong(b, serial);
}
static const struct xdg_wm_base_listener wm_base_listener = { wm_base_ping };

static void presentation_clock_id(void *d, struct wp_presentation *p, uint32_t clk_id) {
    (void)d; (void)p;
    printf("wp_presentation: clock id %u (%s)\n", clk_id,
           clk_id == 1 ? "CLOCK_MONOTONIC" : "see <time.h>");
}
static const struct wp_presentation_listener presentation_listener = { presentation_clock_id };

// --------------------------------------------------------------- registry

static void registry_global(void *data, struct wl_registry *reg, uint32_t name,
                            const char *iface, uint32_t version) {
    (void)data;
    if (!strcmp(iface, wl_compositor_interface.name))
        g_compositor = wl_registry_bind(reg, name, &wl_compositor_interface, version < 4 ? version : 4);
    else if (!strcmp(iface, xdg_wm_base_interface.name))
        g_wm_base = wl_registry_bind(reg, name, &xdg_wm_base_interface, 1);
    else if (!strcmp(iface, wl_shm_interface.name))
        g_shm = wl_registry_bind(reg, name, &wl_shm_interface, 1);
    else if (!strcmp(iface, wp_presentation_interface.name))
        g_presentation = wl_registry_bind(reg, name, &wp_presentation_interface, 1);
}
static void registry_global_remove(void *d, struct wl_registry *r, uint32_t name) {
    (void)d; (void)r; (void)name;
}
static const struct wl_registry_listener registry_listener = { registry_global, registry_global_remove };

// ------------------------------------------------------------------- main

static void usage(const char *me) {
    fprintf(stderr,
            "usage: %s [--socket=NAME] [--seconds=N] [--size=WxH] [-v]\n"
            "  --socket   wayland socket to connect to (default: $WAYLAND_DISPLAY)\n"
            "  --seconds  how long to run (default 5)\n"
            "  -v         print every feedback, not just the first five\n",
            me);
}

int main(int argc, char **argv) {
    const char *socket_name = NULL;
    double seconds = 5.0;

    for (int i = 1; i < argc; i++) {
        if (!strncmp(argv[i], "--socket=", 9)) socket_name = argv[i] + 9;
        else if (!strncmp(argv[i], "--seconds=", 10)) seconds = atof(argv[i] + 10);
        else if (!strncmp(argv[i], "--size=", 7)) sscanf(argv[i] + 7, "%dx%d", &g_width, &g_height);
        else if (!strcmp(argv[i], "-v")) g_verbose = 1;
        else { usage(argv[0]); return 2; }
    }

    struct wl_display *display = wl_display_connect(socket_name);
    if (!display) {
        fprintf(stderr, "present-probe: cannot connect to %s: %s\n",
                socket_name ? socket_name : (getenv("WAYLAND_DISPLAY") ?: "(unset)"), strerror(errno));
        return 1;
    }
    printf("connected to %s\n", socket_name ? socket_name : (getenv("WAYLAND_DISPLAY") ?: "?"));

    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);

    if (!g_compositor || !g_wm_base || !g_shm) {
        fprintf(stderr, "present-probe: compositor is missing wl_compositor/xdg_wm_base/wl_shm\n");
        return 1;
    }
    printf("wp_presentation advertised: %s\n", g_presentation ? "YES" : "NO");
    if (!g_presentation)
        printf("  (no global — clients cannot ask about presentation at all here)\n");

    xdg_wm_base_add_listener(g_wm_base, &wm_base_listener, NULL);
    if (g_presentation)
        wp_presentation_add_listener(g_presentation, &presentation_listener, NULL);

    if (make_slots() < 0) return 1;

    g_surface = wl_compositor_create_surface(g_compositor);
    g_xdg_surface = xdg_wm_base_get_xdg_surface(g_wm_base, g_surface);
    xdg_surface_add_listener(g_xdg_surface, &xdg_surface_listener, NULL);
    g_toplevel = xdg_surface_get_toplevel(g_xdg_surface);
    xdg_toplevel_add_listener(g_toplevel, &toplevel_listener, NULL);
    xdg_toplevel_set_title(g_toplevel, "present-probe");
    xdg_toplevel_set_app_id(g_toplevel, "present-probe");
    wl_surface_commit(g_surface);
    wl_display_roundtrip(display);

    printf("running %.1fs...\n", seconds);
    uint64_t deadline = now_ns() + (uint64_t)(seconds * 1e9);

    while (g_running && now_ns() < deadline) {
        while (wl_display_prepare_read(display) != 0) wl_display_dispatch_pending(display);
        wl_display_flush(display);
        struct pollfd pfd = { .fd = wl_display_get_fd(display), .events = POLLIN };
        int r = poll(&pfd, 1, 100);
        if (r > 0 && (pfd.revents & POLLIN)) wl_display_read_events(display);
        else wl_display_cancel_read(display);
        if (wl_display_dispatch_pending(display) < 0) break;
    }

    // Let any in-flight feedback land before we tally.
    wl_display_roundtrip(display);

    printf("\n--- summary ---\n");
    printf("commits             %u\n", g_committed);
    printf("frame callbacks     %u\n", g_frame_cb);
    printf("feedback presented  %u\n", g_presented);
    printf("feedback discarded  %u\n", g_discarded);
    if (g_presented) {
        printf("last refresh        %u ns (%.2f Hz)\n", g_last_refresh,
               g_last_refresh ? 1e9 / g_last_refresh : 0.0);
        printf("flags seen (OR)     0x%x\n", g_flags_or);
        printf("msc sequence        %u advanced, %u same, %u went backwards\n",
               g_seq_advanced, g_seq_stalled, g_seq_went_back);
        printf("timestamp           %u advanced, %u same, %u went backwards\n",
               g_time_advanced, g_time_stalled, g_time_went_back);
    }

    printf("\nverdict: ");
    if (!g_presentation)
        printf("wp_presentation NOT ADVERTISED — nothing to answer.\n");
    else if (!g_presented && g_discarded)
        printf("BROKEN — %u feedbacks, every one discarded, none presented.\n", g_discarded);
    else if (!g_presented && !g_discarded)
        printf("SILENT — advertised, but no feedback of either kind came back.\n");
    else if (g_seq_advanced && g_time_advanced)
        printf("WORKING — presentations arrive and both msc and timestamp advance.\n");
    else if (g_presented && !g_seq_advanced && g_seq_stalled)
        printf("PARTIAL — presentations arrive but the msc sequence never advances.\n");
    else
        printf("PARTIAL — %u presented / %u discarded; see the counters above.\n",
               g_presented, g_discarded);

    xdg_toplevel_destroy(g_toplevel);
    xdg_surface_destroy(g_xdg_surface);
    wl_surface_destroy(g_surface);
    wl_display_disconnect(display);
    return 0;
}
