// Ask the compositor to accept a dmabuf of a given fourcc, and report whether it
// did. One row per format: what was allocated, what was declared, and whether
// zwp_linux_buffer_params.created or .failed came back.
//
// The compositor never inspects pixel content, so a format GBM cannot allocate
// natively is still testable: allocate a byte-compatible LINEAR carrier (XR24)
// and DECLARE the format under test. Stride is declared >= width*bpp/8, which is
// all the protocol requires.
//
//   cc dmabuf-format-test.c linux-dmabuf-v1-protocol.c -lwayland-client -lgbm -o dmabuf-format-test
//   ./dmabuf-format-test [--socket=wayland-N] [--device=/dev/dri/renderD128] [FOURCC ...]
//
// With no FOURCC arguments it runs the built-in list (controls + everything the
// exhaustive table refuses + the newly mapped ones).

#include <fcntl.h>
#include <gbm.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>
#include "linux-dmabuf-v1-client-protocol.h"

#define FOURCC(a, b, c, d) ((uint32_t)(a) | ((uint32_t)(b) << 8) | ((uint32_t)(c) << 16) | ((uint32_t)(d) << 24))
#define MOD_LINEAR 0ULL
#define MOD_INVALID 0x00ffffffffffffffULL

struct probe { const char *code; uint32_t fourcc; int bpp; const char *expect; };

// `expect` is what the NEW policy should say; it is printed next to the result so
// a mismatch is obvious without cross-referencing the table by hand.
static const struct probe LIST[] = {
    // controls — mapped long before this work, must always be accepted
    { "XR24", FOURCC('X','R','2','4'), 32, "accept (control)" },
    { "AB24", FOURCC('A','B','2','4'), 32, "accept (control)" },
    // newly mapped by format.table
    { "RG16", FOURCC('R','G','1','6'), 16, "accept (new: Rgb565)" },
    { "XR15", FOURCC('X','R','1','5'), 16, "accept (new: Xrgb1555)" },
    { "BA12", FOURCC('B','A','1','2'), 16, "accept (new: Bgra4444)" },
    { "RG24", FOURCC('R','G','2','4'), 24, "accept (new: Rgb888)" },
    { "R8  ", FOURCC('R','8',' ',' '),  8, "accept (new: R8)" },
    { "R16 ", FOURCC('R','1','6',' '), 16, "accept (new: R16)" },
    // fp16: importable, refused by COLOUR POLICY while the composite is SDR
    { "XB4H", FOURCC('X','B','4','H'), 64, "refuse (fp16 policy)" },
    { "AB4H", FOURCC('A','B','4','H'), 64, "refuse (fp16 policy)" },
    // no VkFormat for the layout — refused as a capability
    { "BA24", FOURCC('B','A','2','4'), 32, "refuse (Bgra8888)" },
    { "RA24", FOURCC('R','A','2','4'), 32, "refuse (Rgba8888)" },
    { "BX24", FOURCC('B','X','2','4'), 32, "refuse (Bgrx8888)" },
    { "RX24", FOURCC('R','X','2','4'), 32, "refuse (Rgbx8888)" },
    { "BA30", FOURCC('B','A','3','0'), 32, "refuse (Bgra1010102)" },
    { "RA30", FOURCC('R','A','3','0'), 32, "refuse (Rgba1010102)" },
    { "BX30", FOURCC('B','X','3','0'), 32, "refuse (Bgrx1010102)" },
    { "RX30", FOURCC('R','X','3','0'), 32, "refuse (Rgbx1010102)" },
    { "RG32", FOURCC('R','G','3','2'), 32, "refuse (Rg1616)" },
    { "AB12", FOURCC('A','B','1','2'), 16, "refuse (Abgr4444)" },
    { "AR12", FOURCC('A','R','1','2'), 16, "refuse (Argb4444)" },
    { "AB15", FOURCC('A','B','1','5'), 16, "refuse (Abgr1555)" },
    { "XB15", FOURCC('X','B','1','5'), 16, "refuse (Xbgr1555)" },
    { "RG88", FOURCC('R','G','8','8'), 16, "refuse (Rg88)" },
    { "RGB8", FOURCC('R','G','B','8'),  8, "refuse (Rgb332)" },
    // planar — nothing is allocated per-plane here, so this only tests whether the
    // fourcc itself is refused before plane count matters
    { "NV12", FOURCC('N','V','1','2'),  8, "refuse (Y/Cb/Cr)" },
};

static struct wl_compositor *comp;
static struct zwp_linux_dmabuf_v1 *dmabuf;
static uint32_t dmabuf_version;
static int result; // 0 pending, 1 created, 2 failed
static int protocol_error;

static void params_created(void *u, struct zwp_linux_buffer_params_v1 *p, struct wl_buffer *b) {
    (void)u; (void)p;
    result = 1;
    wl_buffer_destroy(b);
}
static void params_failed(void *u, struct zwp_linux_buffer_params_v1 *p) { (void)u; (void)p; result = 2; }
static const struct zwp_linux_buffer_params_v1_listener params_listener = { params_created, params_failed };

static void global(void *u, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
    (void)u;
    if (!strcmp(iface, "wl_compositor"))
        comp = wl_registry_bind(r, name, &wl_compositor_interface, 1);
    else if (!strcmp(iface, "zwp_linux_dmabuf_v1")) {
        dmabuf_version = ver < 4 ? ver : 4;
        dmabuf = wl_registry_bind(r, name, &zwp_linux_dmabuf_v1_interface, dmabuf_version);
    }
}
static void global_remove(void *u, struct wl_registry *r, uint32_t n) { (void)u; (void)r; (void)n; }
static const struct wl_registry_listener reg_listener = { global, global_remove };

// A fresh connection per format: asking for an unadvertised fourcc is a PROTOCOL
// ERROR (smithay posts InvalidFormat and the client dies), so one row must not
// take the rest of the sweep with it.
static struct wl_display *connect_bind(const char *sock) {
    comp = NULL; dmabuf = NULL; dmabuf_version = 0;
    struct wl_display *d = wl_display_connect(sock);
    if (!d) { fprintf(stderr, "cannot connect to wayland display\n"); return NULL; }
    struct wl_registry *r = wl_display_get_registry(d);
    wl_registry_add_listener(r, &reg_listener, NULL);
    wl_display_roundtrip(d);
    if (!dmabuf) { fprintf(stderr, "no zwp_linux_dmabuf_v1\n"); wl_display_disconnect(d); return NULL; }
    return d;
}

// Name the protocol error the compositor killed us with, so "not advertised"
// (InvalidFormat) reads differently from a soft import failure.
static const char *proto_error(struct wl_display *d) {
    if (wl_display_get_error(d) == 0) return "display closed";
    const struct wl_interface *iface = NULL; uint32_t id = 0;
    uint32_t code = wl_display_get_protocol_error(d, &iface, &id);
    if (!iface) return "no protocol error (EPIPE?)";
    if (!strcmp(iface->name, "zwp_linux_buffer_params_v1")) {
        switch (code) {          // enum values from linux-dmabuf-v1-client-protocol.h
            case 0: return "AlreadyUsed";
            case 1: return "PlaneIdx";
            case 2: return "PlaneSet";
            case 3: return "Incomplete";
            case 4: return "InvalidFormat -> NOT ADVERTISED";
            case 5: return "InvalidDimensions";
            case 6: return "OutOfBounds";
            case 7: return "InvalidWlBuffer";
            case 8: return "InvalidDevTSize";
            default: break;
        }
    }
    return iface->name;
}

int main(int argc, char **argv) {
    const char *sock = NULL, *devpath = "/dev/dri/renderD128";
    const char *only[64]; int nonly = 0;
    for (int i = 1; i < argc; i++) {
        if (!strncmp(argv[i], "--socket=", 9)) sock = argv[i] + 9;
        else if (!strncmp(argv[i], "--device=", 9)) devpath = argv[i] + 9;
        else if (nonly < 64) only[nonly++] = argv[i];
    }

    int fd = open(devpath, O_RDWR | O_CLOEXEC);
    if (fd < 0) { perror(devpath); return 1; }
    struct gbm_device *gbm = gbm_create_device(fd);
    if (!gbm) { fprintf(stderr, "gbm_create_device failed on %s\n", devpath); return 1; }

    setvbuf(stdout, NULL, _IONBF, 0);
    struct wl_display *d = connect_bind(sock);
    if (!d) return 2;
    printf("device %s | zwp_linux_dmabuf_v1 v%u | %d format(s) to test\n\n",
           devpath, dmabuf_version, nonly ? nonly : (int)(sizeof LIST / sizeof *LIST));
    printf("%-5s %-4s %-22s %-9s %s\n", "code", "bpp", "expected (new policy)", "carrier", "RESULT");
    setvbuf(stdout, NULL, _IONBF, 0);

    const int W = 64, H = 64;
    for (size_t i = 0; i < sizeof LIST / sizeof *LIST; i++) {
        const struct probe *p = &LIST[i];
        if (nonly) {
            int want = 0;
            for (int j = 0; j < nonly; j++) if (!strncmp(only[j], p->code, 4)) want = 1;
            if (!want) continue;
        }

        // Carrier: LINEAR XR24, wide enough that its stride covers W * bpp/8.
        uint32_t need_stride = (uint32_t)W * p->bpp / 8;
        uint32_t carrier_w = (need_stride + 3) / 4;
        uint64_t want_linear = MOD_LINEAR;
        const char *carrier = "linear";
        struct gbm_bo *bo = gbm_bo_create_with_modifiers(gbm, carrier_w, H, GBM_FORMAT_XRGB8888,
                                                        &want_linear, 1);
        if (!bo) bo = gbm_bo_create(gbm, carrier_w, H, GBM_FORMAT_XRGB8888, GBM_BO_USE_WRITE);
        if (!bo) {
            // Last resort: whatever the driver gives, with its own modifier. A tiled
            // carrier can be refused for the modifier rather than the fourcc, so the
            // row is marked to keep that ambiguity visible.
            bo = gbm_bo_create(gbm, carrier_w, H, GBM_FORMAT_XRGB8888, GBM_BO_USE_RENDERING);
            carrier = "tiled(!)";
        }
        if (!bo) {
            printf("%-5s %-4d %-22s %-9s ALLOC FAILED\n", p->code, p->bpp, p->expect, "-");
            continue;
        }
        uint64_t mod = gbm_bo_get_modifier(bo);
        int bfd = gbm_bo_get_fd(bo);
        uint32_t stride = gbm_bo_get_stride(bo);
        if (bfd < 0) {
            printf("%-5s %-4d %-22s %-9s EXPORT FAILED\n", p->code, p->bpp, p->expect, carrier);
            gbm_bo_destroy(bo);
            continue;
        }

        result = 0;
        if (protocol_error) {
            wl_display_disconnect(d);
            d = connect_bind(sock);
            if (!d) { printf("%-5s reconnect failed\n", p->code); break; }
            protocol_error = 0;
        }
        struct zwp_linux_buffer_params_v1 *params = zwp_linux_dmabuf_v1_create_params(dmabuf);
        zwp_linux_buffer_params_v1_add_listener(params, &params_listener, NULL);
        zwp_linux_buffer_params_v1_add(params, bfd, 0, 0, stride,
                                       (uint32_t)(mod >> 32), (uint32_t)(mod & 0xffffffff));
        zwp_linux_buffer_params_v1_create(params, W, H, p->fourcc, 0);
        // The compositor answers created/failed; a protocol error kills the display.
        while (!result) {
            if (wl_display_dispatch(d) == -1) { protocol_error = 1; break; }
        }
        printf("%-5s %-4d %-22s %-9s %s\n", p->code, p->bpp, p->expect, carrier,
               protocol_error ? proto_error(d)
                              : result == 1 ? "created  -> ACCEPTED"
                                            : "failed   -> refused (soft)");
        if (!protocol_error) zwp_linux_buffer_params_v1_destroy(params);
        close(bfd);
        gbm_bo_destroy(bo);
    }

    if (d && !protocol_error) wl_display_disconnect(d);
    gbm_device_destroy(gbm);
    close(fd);
    return 0;
}
