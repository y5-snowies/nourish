// color-stress — the interactive front end for this directory.
//
// Three questions, one prompt, and the ability to act on the answer:
//
//   1. what does the COMPOSITOR advertise?     zwp_linux_dmabuf_v1 feedback
//   2. what does the DEVICE actually support?  GBM on the DRM node, no compositor
//   3. where do those two disagree?            the diff is the interesting part
//
// then pick a `(fourcc, modifier)` pair off either list and render with exactly it.
//
// # Why the device probe is here and not just `vkfmt`
//
// `vkfmt` asks Vulkan what a VkFormat can do. That is the right question for the
// COMPOSITE side, and it is what y5's own gates read. It is the wrong question for
// "can I, a client, allocate this" — that goes through GBM, on the node the client
// opens, and GBM refuses combinations Vulkan reports as fine (`RENDERING|LINEAR` on
// NVIDIA is the standing example). So this probes allocation directly: for every
// advertised pair, try to create a BO with that exact modifier and report what came
// back. It is the only answer a client can act on.
//
// # Rendering
//
// The renderers already exist and are proven, so this execs `dmabuf-draw` rather than
// reimplementing them. That keeps one code path for what reaches the screen — a second
// copy would drift, and the drift would look like a format bug.
//
//   cc color-stress.c linux-dmabuf-v1-protocol.c -lwayland-client -lgbm -o color-stress

#define _GNU_SOURCE
#include <fcntl.h>
#include <gbm.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <unistd.h>
#include <wayland-client.h>
#include "linux-dmabuf-v1-client-protocol.h"

#define MAX_PAIRS 4096

struct pair {
    uint32_t fourcc;
    uint64_t modifier;
    int scanout;       // came from a tranche flagged SCANOUT
    // Two verdicts, not one. A modifier is not "allocatable" in the abstract: it is
    // allocatable FOR A USE, and the two uses a client actually has disagree on this
    // hardware. -1 not probed, 0 refused, 1 allocated.
    int cpu_ok;        // GBM_BO_USE_WRITE     — a client that mmaps and writes pixels
    int gpu_ok;        // GBM_BO_USE_RENDERING — a client that renders with GL/Vulkan
    uint64_t got;      // what gbm actually handed back (from whichever path took it)
};

// Allocatable at all, by anyone.
static int pair_ok(const struct pair *p) {
    return p->cpu_ok == 1 || p->gpu_ok == 1;
}

static struct pair pairs[MAX_PAIRS];
static int npairs;

static struct zwp_linux_dmabuf_v1 *dmabuf;
static uint32_t dmabuf_version;
struct entry { uint32_t format; uint32_t pad; uint64_t modifier; };
static struct entry *table;
static size_t table_len;
static int fb_done_flag, tranche_flags_cur;
static char main_dev[32] = "?";

static void fourcc_str(uint32_t f, char out[5]) {
    out[0] = f & 0xff; out[1] = (f >> 8) & 0xff; out[2] = (f >> 16) & 0xff;
    out[3] = (f >> 24) & 0xff; out[4] = 0;
    for (int i = 0; i < 4; i++) if (out[i] < 32 || out[i] > 126) out[i] = '?';
}

static const char *mod_name(uint64_t m) {
    if (m == 0) return "LINEAR";
    if (m == 0x00ffffffffffffffULL) return "INVALID";
    switch (m >> 56) {
        case 1: return "INTEL";  case 2: return "AMD";     case 3: return "NVIDIA";
        case 4: return "SAMSUNG"; case 7: return "ARM";    case 9: return "BROADCOM";
        default: return "vendor?";
    }
}

static void add_pair(uint32_t fourcc, uint64_t mod, int scanout) {
    for (int i = 0; i < npairs; i++)
        if (pairs[i].fourcc == fourcc && pairs[i].modifier == mod) {
            pairs[i].scanout |= scanout;
            return;
        }
    if (npairs >= MAX_PAIRS) return;
    pairs[npairs++] = (struct pair){ fourcc, mod, scanout, -1, -1, 0 };
}

// ---- compositor feedback ----------------------------------------------------
static void fb_done(void *u, struct zwp_linux_dmabuf_feedback_v1 *f) { (void)u; (void)f; fb_done_flag = 1; }

static void fb_format_table(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, int32_t fd, uint32_t size) {
    (void)u; (void)f;
    void *m = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (m == MAP_FAILED) return;
    table = m;
    table_len = size / sizeof(struct entry);
}

static void fb_main_device(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, struct wl_array *a) {
    (void)u; (void)f;
    if (a->size < sizeof(dev_t)) return;
    dev_t d; memcpy(&d, a->data, sizeof d);
    snprintf(main_dev, sizeof main_dev, "%u:%u", major(d), minor(d));
}

static void fb_tranche_target(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, struct wl_array *a) {
    (void)u; (void)f; (void)a;
}
static void fb_tranche_flags(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, uint32_t flags) {
    (void)u; (void)f; tranche_flags_cur = (int)flags;
}
static void fb_tranche_formats(void *u, struct zwp_linux_dmabuf_feedback_v1 *f, struct wl_array *a) {
    (void)u; (void)f;
    if (!table) return;
    uint16_t *idx = a->data;
    size_t n = a->size / sizeof(uint16_t);
    for (size_t i = 0; i < n; i++)
        if (idx[i] < table_len)
            add_pair(table[idx[i]].format, table[idx[i]].modifier, tranche_flags_cur & 1);
}
static void fb_tranche_done(void *u, struct zwp_linux_dmabuf_feedback_v1 *f) {
    (void)u; (void)f; tranche_flags_cur = 0;
}
static const struct zwp_linux_dmabuf_feedback_v1_listener fb_listener = {
    .done = fb_done, .format_table = fb_format_table, .main_device = fb_main_device,
    .tranche_done = fb_tranche_done, .tranche_target_device = fb_tranche_target,
    .tranche_formats = fb_tranche_formats, .tranche_flags = fb_tranche_flags,
};

// v3 fallback: no feedback, just format/modifier events.
static void v3_format(void *u, struct zwp_linux_dmabuf_v1 *d, uint32_t fmt) {
    (void)u; (void)d; add_pair(fmt, 0x00ffffffffffffffULL, 0);
}
static void v3_modifier(void *u, struct zwp_linux_dmabuf_v1 *d, uint32_t fmt, uint32_t hi, uint32_t lo) {
    (void)u; (void)d; add_pair(fmt, ((uint64_t)hi << 32) | lo, 0);
}
static const struct zwp_linux_dmabuf_v1_listener v3_listener = { v3_format, v3_modifier };

static void global(void *u, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
    (void)u;
    if (!strcmp(iface, "zwp_linux_dmabuf_v1")) {
        dmabuf_version = ver < 5 ? ver : 5;
        dmabuf = wl_registry_bind(r, name, &zwp_linux_dmabuf_v1_interface, dmabuf_version);
    }
}
static void global_remove(void *u, struct wl_registry *r, uint32_t n) { (void)u; (void)r; (void)n; }
static const struct wl_registry_listener reg_listener = { global, global_remove };

static int query_compositor(const char *sock) {
    struct wl_display *dpy = wl_display_connect(sock);
    if (!dpy) { printf("  cannot connect to %s\n", sock ? sock : "$WAYLAND_DISPLAY"); return -1; }
    struct wl_registry *reg = wl_display_get_registry(dpy);
    wl_registry_add_listener(reg, &reg_listener, NULL);
    wl_display_roundtrip(dpy);
    if (!dmabuf) { printf("  no zwp_linux_dmabuf_v1 global\n"); wl_display_disconnect(dpy); return -1; }
    if (dmabuf_version >= 4) {
        struct zwp_linux_dmabuf_feedback_v1 *fb = zwp_linux_dmabuf_v1_get_default_feedback(dmabuf);
        zwp_linux_dmabuf_feedback_v1_add_listener(fb, &fb_listener, NULL);
        while (!fb_done_flag && wl_display_dispatch(dpy) != -1) {}
    } else {
        zwp_linux_dmabuf_v1_add_listener(dmabuf, &v3_listener, NULL);
        wl_display_roundtrip(dpy);
        wl_display_roundtrip(dpy);
    }
    wl_display_disconnect(dpy);
    return 0;
}

// ---- device probe: can GBM actually allocate this pair? ---------------------
static struct gbm_device *gbm;
static const char *gbm_path;

static int open_gbm(const char *dev) {
    // Fixed-length, NOT NULL-terminated: `dev` is NULL when --device was not given,
    // and a NULL-terminated walk would stop on it before trying anything.
    const char *cands[] = { dev, "/dev/dri/renderD128", "/dev/dri/card1", "/dev/dri/card0" };
    for (size_t i = 0; i < sizeof cands / sizeof *cands; i++) {
        if (!cands[i]) continue;
        int fd = open(cands[i], O_RDWR | O_CLOEXEC);
        if (fd < 0) continue;
        gbm = gbm_create_device(fd);
        if (gbm) { gbm_path = cands[i]; return 0; }
        close(fd);
    }
    return -1;
}

// One allocation attempt per pair, with EXACTLY the fourcc and modifier named.
//
// Two things this gets wrong if you simplify it, both of which produced a false
// "we no longer support tiled at all" reading:
//
//  1. It must allocate `p->fourcc`, not a fixed XR24. Probing one fourcc and
//     printing the answer on all 48 rows makes every row agree by construction,
//     which looks exactly like a compositor that lost a modifier family.
//  2. It must probe BOTH usages. `GBM_BO_USE_WRITE` asks for a CPU-mappable buffer,
//     and on NVIDIA that refuses every explicit tiled modifier — the buffer has to
//     be linear for the CPU to write it. A GPU client asking for
//     `GBM_BO_USE_RENDERING` gets the tiled layouts. Reporting only the WRITE
//     answer therefore reports "linear only" on a device that supports tiling fine.
static void probe_one(struct pair *p) {
    uint64_t m = p->modifier;
    int implicit = (m == 0x00ffffffffffffffULL); // INVALID: no modifier list, driver picks
    const uint32_t use[2] = { GBM_BO_USE_WRITE, GBM_BO_USE_RENDERING };
    int *slot[2] = { &p->cpu_ok, &p->gpu_ok };
    for (int u = 0; u < 2; u++) {
        struct gbm_bo *bo = implicit
            ? gbm_bo_create(gbm, 64, 64, p->fourcc, use[u])
            : gbm_bo_create_with_modifiers2(gbm, 64, 64, p->fourcc, &m, 1, use[u]);
        *slot[u] = bo ? 1 : 0;
        if (bo) { p->got = gbm_bo_get_modifier(bo); gbm_bo_destroy(bo); }
    }
}

static void probe_device(void) {
    if (!gbm) { printf("  no gbm device open\n"); return; }
    for (int i = 0; i < npairs; i++) probe_one(&pairs[i]);
}

// ---- listing ---------------------------------------------------------------
static void list_pairs(int only_allocatable) {
    char s[5];
    printf("\n  %-4s  %-8s %-22s %-8s %-9s %s\n",
           "#", "fourcc", "modifier", "scanout", "cpu-write", "gpu-render");
    int shown = 0;
    for (int i = 0; i < npairs; i++) {
        struct pair *p = &pairs[i];
        if (only_allocatable && !pair_ok(p)) continue;
        fourcc_str(p->fourcc, s);
        char mod[40];
        snprintf(mod, sizeof mod, "%s(0x%llx)", mod_name(p->modifier),
                 (unsigned long long)p->modifier);
        const char *c = p->cpu_ok < 0 ? "-" : p->cpu_ok ? "alloc" : "REFUSED";
        const char *g = p->gpu_ok < 0 ? "-" : p->gpu_ok ? "alloc" : "REFUSED";
        printf("  %-4d  %-8s %-22s %-8s %-9s %s", i, s, mod,
               p->scanout ? "yes" : "", c, g);
        if (pair_ok(p) && p->got != p->modifier
            && p->modifier != 0x00ffffffffffffffULL)
            printf("  (driver returned 0x%llx)", (unsigned long long)p->got);
        printf("\n");
        shown++;
    }
    printf("\n  %d pair(s)%s. main device %s, gbm node %s\n",
           shown, only_allocatable ? " a client can allocate (either usage)" : " advertised",
           main_dev, gbm_path ? gbm_path : "(none)");
}

// The disagreement, which is the whole reason both halves are here: a pair the
// compositor advertises that this client cannot allocate AT ALL is a promise it
// cannot use. A pair only one usage can take is not a fault — see `probe_one`.
static void list_diff(void) {
    char s[5];
    int bad = 0, cpu_only = 0, gpu_only = 0;
    printf("\n  advertised but NOT allocatable in EITHER usage on %s:\n",
           gbm_path ? gbm_path : "?");
    for (int i = 0; i < npairs; i++) {
        struct pair *p = &pairs[i];
        if (p->cpu_ok == 1 && p->gpu_ok != 1) cpu_only++;
        if (p->gpu_ok == 1 && p->cpu_ok != 1) gpu_only++;
        if (pair_ok(p)) continue;
        fourcc_str(p->fourcc, s);
        printf("    %-4s  %s(0x%llx)%s\n", s, mod_name(p->modifier),
               (unsigned long long)p->modifier, p->scanout ? "  [scanout tranche]" : "");
        bad++;
    }
    if (!bad) printf("    (none — every advertised pair allocates for at least one usage)\n");
    printf("\n  %d pair(s) CPU-write only, %d GPU-render only.\n", cpu_only, gpu_only);
    printf("  A pair that only GPU-render can take is normal, not a bug: GBM_BO_USE_WRITE\n"
           "  asks for a CPU-mappable buffer, and a tiled layout cannot be one. If the\n"
           "  gpu-render column is empty across the board, THAT is the tiling regression\n"
           "  this tool is looking for.\n");
}

// ---- run a renderer --------------------------------------------------------
static void run_draw(const char *sock, const char *const *extra, int nextra,
                     uint32_t fourcc, uint64_t mod, int have_mod) {
    char s[5]; fourcc_str(fourcc, s);
    char sockarg[64], modarg[64];
    const char *argv[16]; int n = 0;
    argv[n++] = "./dmabuf-draw";
    if (sock) { snprintf(sockarg, sizeof sockarg, "--socket=%s", sock); argv[n++] = sockarg; }
    if (have_mod) {
        snprintf(modarg, sizeof modarg, "--modifier=0x%llx", (unsigned long long)mod);
        argv[n++] = modarg;
    }
    for (int i = 0; i < nextra && n < 14; i++) argv[n++] = extra[i];
    if (fourcc) argv[n++] = s;
    argv[n] = NULL;

    printf("\n  exec:");
    for (int i = 0; i < n; i++) printf(" %s", argv[i]);
    printf("\n\n");
    fflush(stdout);

    pid_t pid = fork();
    if (pid == 0) { execv(argv[0], (char *const *)argv); _exit(127); }
    if (pid < 0) { printf("  fork failed\n"); return; }
    int st = 0;
    waitpid(pid, &st, 0);
    // A protocol kill is a RESULT here, not a crash: the compositor refusing a
    // fourcc it never advertised is exactly what `NOTADV` means in the grid.
    if (WIFEXITED(st) && WEXITSTATUS(st) == 127)
        printf("  could not exec ./dmabuf-draw — run `make` first\n");
    else
        printf("\n  dmabuf-draw exited (status %d)\n", WIFEXITED(st) ? WEXITSTATUS(st) : -1);
}

static void prompt_and_render(const char *sock) {
    printf("\n  pair number (from the lists above), or blank to cancel: ");
    fflush(stdout);
    char line[64];
    if (!fgets(line, sizeof line, stdin) || line[0] == '\n') return;
    int idx = atoi(line);
    if (idx < 0 || idx >= npairs) { printf("  no such pair\n"); return; }

    printf("  mode: [1] pattern  [2] blend/alpha test  [3] check pattern (no black): ");
    fflush(stdout);
    if (!fgets(line, sizeof line, stdin)) return;
    const char *extra[4]; int nextra = 0;
    extra[nextra++] = "--hold=8000";
    if (line[0] == '2') extra[nextra++] = "--blend";
    else if (line[0] == '3') extra[nextra++] = "--pattern=check";
    run_draw(sock, extra, nextra, pairs[idx].fourcc, pairs[idx].modifier, 1);
}

static void menu(void) {
    printf("\n=========== color-stress ===========\n");
    printf("  1  compositor: what is advertised\n");
    printf("  2  device: what GBM can actually allocate, CPU vs GPU (no compositor)\n");
    printf("  3  diff: advertised but allocatable by nobody\n");
    printf("  4  pick a pair and render it\n");
    printf("  5  grid: every format at once, one window\n");
    printf("  6  blend/alpha test for one fourcc\n");
    printf("  7  re-query the compositor\n");
    printf("  q  quit\n");
    printf("  > ");
    fflush(stdout);
}

int main(int argc, char **argv) {
    const char *sock = NULL, *dev = NULL;
    for (int i = 1; i < argc; i++) {
        if (!strncmp(argv[i], "--socket=", 9)) sock = argv[i] + 9;
        else if (!strncmp(argv[i], "--device=", 9)) dev = argv[i] + 9;
        else {
            printf("usage: %s [--socket=wayland-N] [--device=/dev/dri/renderD128]\n", argv[0]);
            return 2;
        }
    }
    if (open_gbm(dev) != 0)
        printf("warning: no GBM device opened — the device half will be unavailable\n");
    printf("querying the compositor...\n");
    query_compositor(sock);
    printf("  %d advertised pair(s), main device %s\n", npairs, main_dev);
    if (npairs) { printf("probing the device for each...\n"); probe_device(); }

    char line[64];
    for (;;) {
        menu();
        if (!fgets(line, sizeof line, stdin)) break;
        switch (line[0]) {
            case '1': list_pairs(0); break;
            case '2': list_pairs(1); break;
            case '3': list_diff(); break;
            case '4': prompt_and_render(sock); break;
            case '5': {
                const char *extra[] = { "--grid", "--cols=4", "--hold=15000" };
                run_draw(sock, extra, 3, 0, 0, 0);
                break;
            }
            case '6': {
                printf("  fourcc (e.g. AR24, XR24): ");
                fflush(stdout);
                if (!fgets(line, sizeof line, stdin) || strlen(line) < 4) break;
                uint32_t f = (uint32_t)line[0] | ((uint32_t)line[1] << 8)
                           | ((uint32_t)line[2] << 16) | ((uint32_t)line[3] << 24);
                const char *extra[] = { "--blend", "--hold=15000" };
                run_draw(sock, extra, 2, f, 0, 0);
                break;
            }
            case '7':
                npairs = 0; fb_done_flag = 0; table = NULL; table_len = 0; dmabuf = NULL;
                query_compositor(sock);
                if (npairs) probe_device();
                printf("  %d advertised pair(s)\n", npairs);
                break;
            case 'q': case 'Q': return 0;
            default: break;
        }
    }
    return 0;
}
