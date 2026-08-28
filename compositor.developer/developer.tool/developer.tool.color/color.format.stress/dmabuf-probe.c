// What does the compositor on $WAYLAND_DISPLAY advertise over linux-dmabuf?
// Prints the v4/v5 default-feedback tranches (main device, per-tranche device,
// flags, and every fourcc x modifier), falling back to the v3 format/modifier
// events when the compositor offers no feedback.
//
//   cc dmabuf-probe.c linux-dmabuf-v1-protocol.c -lwayland-client -o dmabuf-probe
//   ./dmabuf-probe [wayland-N]

#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>
#include <wayland-client.h>
#include "linux-dmabuf-v1-client-protocol.h"

struct entry { uint32_t format; uint32_t pad; uint64_t modifier; };

static struct zwp_linux_dmabuf_v1 *dmabuf;
static uint32_t dmabuf_version;
static struct entry *table;
static size_t table_len;
static int done;
static int tranche_no;
static int v3_count;

static void fourcc_str(uint32_t f, char out[5]) {
    out[0] = f & 0xff; out[1] = (f >> 8) & 0xff; out[2] = (f >> 16) & 0xff;
    out[3] = (f >> 24) & 0xff; out[4] = 0;
    for (int i = 0; i < 4; i++) if (out[i] < 32 || out[i] > 126) out[i] = '?';
}

static const char *mod_name(uint64_t m) {
    if (m == 0) return "LINEAR";
    if (m == 0x00ffffffffffffffULL) return "INVALID";
    switch (m >> 56) {
        case 1: return "INTEL";
        case 2: return "AMD";
        case 3: return "NVIDIA";
        case 4: return "SAMSUNG";
        case 7: return "ARM";
        case 9: return "BROADCOM";
        default: return "vendor?";
    }
}

static void dev_str(struct wl_array *a, char *out, size_t n) {
    if (a->size < sizeof(dev_t)) { snprintf(out, n, "?"); return; }
    dev_t d; memcpy(&d, a->data, sizeof d);
    snprintf(out, n, "%u:%u", major(d), minor(d));
}

// ---- v4/v5 feedback ----
static void fb_done(void *u, struct zwp_linux_dmabuf_feedback_v1 *f) { (void)u; (void)f; done = 1; }

static void fb_format_table(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, int32_t fd, uint32_t size) {
    (void)u; (void)f;
    void *p = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (p == MAP_FAILED) { perror("mmap format table"); return; }
    table = p;
    table_len = size / sizeof(struct entry);
    printf("format table: %zu (format x modifier) pair(s)\n", table_len);
}

static void fb_main_device(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, struct wl_array *a) {
    (void)u; (void)f;
    char b[32]; dev_str(a, b, sizeof b);
    printf("main device: %s\n", b);
}

static void fb_tranche_target(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, struct wl_array *a) {
    (void)u; (void)f;
    char b[32]; dev_str(a, b, sizeof b);
    printf("\n-- tranche %d (device %s)\n", tranche_no, b);
}

static void fb_tranche_flags(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, uint32_t flags) {
    (void)u; (void)f;
    printf("   flags: 0x%x%s\n", flags, (flags & 1) ? " (scanout)" : "");
}

static void fb_tranche_formats(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, struct wl_array *a) {
    (void)u; (void)f;
    uint16_t *idx = a->data;
    size_t n = a->size / sizeof(uint16_t);
    printf("   %zu index/indices\n", n);
    // group by fourcc, in first-seen order
    uint32_t seen[512]; int mods[512]; int nseen = 0;
    for (size_t i = 0; i < n; i++) {
        if (idx[i] >= table_len) continue;
        uint32_t code = table[idx[i]].format;
        int at = -1;
        for (int j = 0; j < nseen; j++) if (seen[j] == code) { at = j; break; }
        if (at < 0 && nseen < 512) { seen[nseen] = code; mods[nseen] = 0; at = nseen++; }
        if (at >= 0) mods[at]++;
    }
    for (int j = 0; j < nseen; j++) {
        char s[5]; fourcc_str(seen[j], s);
        printf("     %-4s  %2d modifier(s):", s, mods[j]);
        for (size_t i = 0; i < n; i++) {
            if (idx[i] >= table_len || table[idx[i]].format != seen[j]) continue;
            uint64_t m = table[idx[i]].modifier;
            printf(" %s(0x%016llx)", mod_name(m), (unsigned long long)m);
        }
        printf("\n");
    }
}

static void fb_tranche_done(void *u, struct zwp_linux_dmabuf_feedback_v1 *f) {
    (void)u; (void)f; tranche_no++;
}

static const struct zwp_linux_dmabuf_feedback_v1_listener fb_listener = {
    .done = fb_done,
    .format_table = fb_format_table,
    .main_device = fb_main_device,
    .tranche_done = fb_tranche_done,
    .tranche_target_device = fb_tranche_target,
    .tranche_formats = fb_tranche_formats,
    .tranche_flags = fb_tranche_flags,
};

// ---- v3 fallback ----
static void v3_format(void *u, struct zwp_linux_dmabuf_v1 *d, uint32_t format) {
    (void)u; (void)d;
    char s[5]; fourcc_str(format, s);
    printf("  [v3] %s\n", s); v3_count++;
}
static void v3_modifier(void *u, struct zwp_linux_dmabuf_v1 *d, uint32_t format, uint32_t hi, uint32_t lo) {
    (void)u; (void)d;
    char s[5]; fourcc_str(format, s);
    uint64_t m = ((uint64_t)hi << 32) | lo;
    printf("  [v3] %-4s %s(0x%016llx)\n", s, mod_name(m), (unsigned long long)m); v3_count++;
}
static const struct zwp_linux_dmabuf_v1_listener v3_listener = { v3_format, v3_modifier };

static int nglobals;
static void global(void *u, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
    (void)u;
    nglobals++;
    // Compositor-specific globals identify who is on the other end.
    if (strncmp(iface, "wl_", 3) && strncmp(iface, "zwp_", 4) && strncmp(iface, "xdg_", 4) &&
        strncmp(iface, "zxdg_", 5) && strncmp(iface, "wp_", 3) && strncmp(iface, "xwayland_", 9))
        printf("global: %s (v%u)\n", iface, ver);
    if (strcmp(iface, "zwp_linux_dmabuf_v1") == 0) {
        dmabuf_version = ver < 5 ? ver : 5;
        dmabuf = wl_registry_bind(r, name, &zwp_linux_dmabuf_v1_interface, dmabuf_version);
    }
}
static void global_remove(void *u, struct wl_registry *r, uint32_t n) { (void)u; (void)r; (void)n; }
static const struct wl_registry_listener reg_listener = { global, global_remove };

int main(int argc, char **argv) {
    const char *sock = argc > 1 ? argv[1] : NULL;
    struct wl_display *d = wl_display_connect(sock);
    if (!d) { fprintf(stderr, "cannot connect to %s\n", sock ? sock : getenv("WAYLAND_DISPLAY")); return 1; }
    struct wl_registry *r = wl_display_get_registry(d);
    wl_registry_add_listener(r, &reg_listener, NULL);
    wl_display_roundtrip(d);
    if (!dmabuf) { fprintf(stderr, "compositor advertises no zwp_linux_dmabuf_v1\n"); return 2; }
    printf("zwp_linux_dmabuf_v1 version %u\n", dmabuf_version);

    if (dmabuf_version >= 4) {
        struct zwp_linux_dmabuf_feedback_v1 *fb = zwp_linux_dmabuf_v1_get_default_feedback(dmabuf);
        zwp_linux_dmabuf_feedback_v1_add_listener(fb, &fb_listener, NULL);
        while (!done && wl_display_dispatch(d) != -1) {}
        printf("\n%d tranche(s)\n", tranche_no);
    } else {
        zwp_linux_dmabuf_v1_add_listener(dmabuf, &v3_listener, NULL);
        wl_display_roundtrip(d);
        wl_display_roundtrip(d);
        printf("%d v3 format/modifier event(s)\n", v3_count);
    }
    wl_display_disconnect(d);
    return 0;
}
