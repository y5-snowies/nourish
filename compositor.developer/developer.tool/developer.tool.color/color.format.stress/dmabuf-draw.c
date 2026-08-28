#define _GNU_SOURCE
// Show a spectrum encoded IN each format under test, LABELLED with the fourcc so a
// single screenshot is self-describing. Where dmabuf-format-test answers "did the
// compositor accept it", this answers "did it interpret it correctly": a swapped
// channel order or a mis-read transfer function is visible, not inferred.
//
//   cc dmabuf-draw.c linux-dmabuf-v1-protocol.c xdg-shell-protocol.c \
//      -lwayland-client -lgbm -lm -o dmabuf-draw
//
//   ./dmabuf-draw [--socket=wayland-N] [--size=WxH] [--hold=ms] [--cols=N] MODE
//     MODE = FOURCC...          one window per format, in sequence
//            --all              every format, in sequence
//            --all-concurrent   every format at once, one toplevel each
//            --grid             every format at once, subsurfaces in ONE window
//
// Each tile: the fourcc + bit depth on a black strip, then a hue sweep (a channel
// swap twists it), a greyscale ramp (a mis-read transfer crushes it), then R/G/B/W
// patches (which name the swapped channel outright).
//
// sRGB-encoded formats carry the pattern as authored. FLOAT formats carry it decoded
// to LINEAR light — what such a client actually stores — so a compositor that samples
// it as if encoded shows a visibly darker ramp.
//
// --all-concurrent gives every format its OWN wayland connection on purpose: asking
// for an unadvertised fourcc is a protocol error that kills the connection, and one
// dead format must not take the others with it.

#include <fcntl.h>
#include <gbm.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "linux-dmabuf-v1-client-protocol.h"
#include "xdg-shell-client-protocol.h"

#define FOURCC(a, b, c, d) ((uint32_t)(a) | ((uint32_t)(b) << 8) | ((uint32_t)(c) << 16) | ((uint32_t)(d) << 24))

struct fmt { const char *code; uint32_t fourcc; int bpp; int is_float; const char *note; };

static const struct fmt FORMATS[] = {
    { "XR24", FOURCC('X','R','2','4'), 32, 0, "Xrgb8888 (memory B,G,R,X)" },
    { "AR24", FOURCC('A','R','2','4'), 32, 0, "Argb8888 (memory B,G,R,A)" },
    { "XB24", FOURCC('X','B','2','4'), 32, 0, "Xbgr8888 (memory R,G,B,X)" },
    { "AB24", FOURCC('A','B','2','4'), 32, 0, "Abgr8888 (memory R,G,B,A)" },
    { "XR30", FOURCC('X','R','3','0'), 32, 0, "Xrgb2101010 (packed A2R10G10B10)" },
    { "XB30", FOURCC('X','B','3','0'), 32, 0, "Xbgr2101010 (packed A2B10G10R10)" },
    { "RG16", FOURCC('R','G','1','6'), 16, 0, "Rgb565 (packed R5G6B5)" },
    { "XR15", FOURCC('X','R','1','5'), 16, 0, "Xrgb1555 (packed A1R5G5B5)" },
    { "BA12", FOURCC('B','A','1','2'), 16, 0, "Bgra4444 (packed B4G4R4A4)" },
    { "RG24", FOURCC('R','G','2','4'), 24, 0, "Rgb888 (memory B,G,R)" },
    { "R8",   FOURCC('R','8',' ',' '),  8, 0, "R8 - red only, expect a red ramp" },
    { "R16",  FOURCC('R','1','6',' '), 16, 0, "R16 - red only, 16bpc" },
    { "XB4H", FOURCC('X','B','4','H'), 64, 1, "Xbgr16161616f - LINEAR, halves R,G,B,X" },
    { "AB4H", FOURCC('A','B','4','H'), 64, 1, "Abgr16161616f - LINEAR, halves R,G,B,A" },
};

// ---------------------------------------------------------------- 5x7 font ----
// Rows top->bottom, bit 4 = leftmost column. A-Z then 0-9, then blank for anything
// else, so a label never renders garbage.
static const unsigned char GLYPHS[37][7] = {
    {0x0E,0x11,0x11,0x1F,0x11,0x11,0x11}, // A
    {0x1E,0x11,0x11,0x1E,0x11,0x11,0x1E}, // B
    {0x0E,0x11,0x10,0x10,0x10,0x11,0x0E}, // C
    {0x1E,0x11,0x11,0x11,0x11,0x11,0x1E}, // D
    {0x1F,0x10,0x10,0x1E,0x10,0x10,0x1F}, // E
    {0x1F,0x10,0x10,0x1E,0x10,0x10,0x10}, // F
    {0x0E,0x11,0x10,0x17,0x11,0x11,0x0E}, // G
    {0x11,0x11,0x11,0x1F,0x11,0x11,0x11}, // H
    {0x1F,0x04,0x04,0x04,0x04,0x04,0x1F}, // I
    {0x07,0x02,0x02,0x02,0x02,0x12,0x0C}, // J
    {0x11,0x12,0x14,0x18,0x14,0x12,0x11}, // K
    {0x10,0x10,0x10,0x10,0x10,0x10,0x1F}, // L
    {0x11,0x1B,0x15,0x15,0x11,0x11,0x11}, // M
    {0x11,0x19,0x15,0x13,0x11,0x11,0x11}, // N
    {0x0E,0x11,0x11,0x11,0x11,0x11,0x0E}, // O
    {0x1E,0x11,0x11,0x1E,0x10,0x10,0x10}, // P
    {0x0E,0x11,0x11,0x11,0x15,0x13,0x0F}, // Q
    {0x1E,0x11,0x11,0x1E,0x14,0x12,0x11}, // R
    {0x0F,0x10,0x10,0x0E,0x01,0x01,0x1E}, // S
    {0x1F,0x04,0x04,0x04,0x04,0x04,0x04}, // T
    {0x11,0x11,0x11,0x11,0x11,0x11,0x0E}, // U
    {0x11,0x11,0x11,0x11,0x11,0x0A,0x04}, // V
    {0x11,0x11,0x11,0x15,0x15,0x1B,0x11}, // W
    {0x11,0x11,0x0A,0x04,0x0A,0x11,0x11}, // X
    {0x11,0x11,0x0A,0x04,0x04,0x04,0x04}, // Y
    {0x1F,0x01,0x02,0x04,0x08,0x10,0x1F}, // Z
    {0x0E,0x11,0x13,0x15,0x19,0x11,0x0E}, // 0
    {0x04,0x0C,0x04,0x04,0x04,0x04,0x0E}, // 1
    {0x0E,0x11,0x01,0x02,0x04,0x08,0x1F}, // 2
    {0x1F,0x02,0x04,0x02,0x01,0x11,0x0E}, // 3
    {0x02,0x06,0x0A,0x12,0x1F,0x02,0x02}, // 4
    {0x1F,0x10,0x1E,0x01,0x01,0x11,0x0E}, // 5
    {0x06,0x08,0x10,0x1E,0x11,0x11,0x0E}, // 6
    {0x1F,0x01,0x02,0x04,0x08,0x08,0x08}, // 7
    {0x0E,0x11,0x11,0x0E,0x11,0x11,0x0E}, // 8
    {0x0E,0x11,0x11,0x0F,0x01,0x02,0x0C}, // 9
    {0,0,0,0,0,0,0},                      // space / unknown
};

static const unsigned char *glyph(char c) {
    if (c >= 'a' && c <= 'z') c -= 32;
    if (c >= 'A' && c <= 'Z') return GLYPHS[c - 'A'];
    if (c >= '0' && c <= '9') return GLYPHS[26 + (c - '0')];
    return GLYPHS[36];
}

// ------------------------------------------------------------- test pattern ----
static int label_h;   // strip the label sits in; the pattern starts below it
static int no_black;  // --pattern=check: a buffer containing NO black pixel at all

// --alpha: the pattern VARIES ALPHA, and the caller puts it on a subsurface over an
// opaque high-contrast backdrop.
//
// This is the thing the tool could not answer before. `sample()` hardcoded `*a = 1.0f`,
// so every buffer was fully opaque whatever its fourcc — an `AR24` tile and an `XR24`
// tile were byte-identical in the alpha channel and the compositor had nothing to
// blend. "Alpha works" was never tested; it was assumed.
static int alpha_mode;
static int blend_mode;   // --blend: the two-surface test that actually exercises it
// --straight: write NON-premultiplied alpha, which is wrong on purpose.
//
// Wayland surface content is PREMULTIPLIED alpha, and every compositor blends
// `src + dst*(1-a)` on that assumption (y5's is `ONE` / `ONE_MINUS_SRC_ALPHA`).
// Hand it straight alpha and the result is `colour + dst*(1-a)`: at a=0 you get the
// full colour ADDED to the backdrop rather than the backdrop, so the ramp reads as
// flat, saturated colour and looks exactly like "alpha is being ignored".
//
// That is worth being able to produce on demand, because it is the mistake a client
// actually makes, and telling it apart from a real compositor bug by eye is otherwise
// guesswork: premultiplied fades to the backdrop, straight blows out to white.
static int straight_alpha;
// --gpu: allocate with GBM_BO_USE_RENDERING and draw with GLES into the dmabuf, so the
// buffer is TILED and GPU-written — the combination a real client gets and the CPU path
// can never reach, because GBM_BO_USE_WRITE forces LINEAR on this driver.
static int gpu_render;

// DIAGNOSTIC PATTERN. The spectrum pattern contains black (label strip, the dark end
// of the ramp, two patches), so a black region on screen is ambiguous: it could be
// client content. This one has no black anywhere — magenta/yellow checks inside a
// white border — so ANY black inside the window provably did not come from the
// client, which separates "buffer never got written" from "compositor did not paint
// it" without guessing.
static void check_pattern(int x, int y, int w, int h, float *r, float *g, float *b, float *a) {
    *a = 1.0f;
    int border = 4;
    if (x < border || y < border || x >= w - border || y >= h - border) {
        *r = *g = *b = 1.0f;                       // white frame marks the true extent
        return;
    }
    int cell = 16, on = ((x / cell) + (y / cell)) & 1;
    *r = 1.0f;                                     // magenta / yellow: red always 1
    *g = on ? 1.0f : 0.0f;
    *b = on ? 0.0f : 1.0f;
}

static void hue(float h, float *r, float *g, float *b) {
    float x = fmodf(h * 6.0f, 6.0f), f = fmodf(x, 1.0f);
    switch ((int)x) {
        case 0: *r = 1; *g = f; *b = 0; break;
        case 1: *r = 1 - f; *g = 1; *b = 0; break;
        case 2: *r = 0; *g = 1; *b = f; break;
        case 3: *r = 0; *g = 1 - f; *b = 1; break;
        case 4: *r = f; *g = 0; *b = 1; break;
        default: *r = 1; *g = 0; *b = 1 - f; break;
    }
}

// ALPHA STAIRCASE over a solid colour.
//
// Eight steps of known alpha across the width, so the result is readable as numbers and
// not just "looks translucent": over a black backdrop, step k should composite to
// `colour * k/7`. The top half is a smooth ramp for the eye, the bottom half is the
// staircase for `inspect.py`.
//
// An X-format (no alpha channel) must come out FULLY OPAQUE here — every step the same.
// That is not a failure of the test, it is the control: put an `XR24` window beside an
// `AR24` one and the pair proves the compositor honours the format's alpha semantics
// rather than treating all four bytes the same.
static void alpha_pattern(int x, int y, int w, int h, float *r, float *g, float *b, float *a) {
    *r = 1.0f; *g = 0.25f; *b = 0.0f;                       // one colour, so only A varies
    if (y < label_h) { *r = *g = *b = 0.0f; *a = 1.0f; return; }
    int ph = h - label_h, py = y - label_h;
    float u = (float)x / (float)(w > 1 ? w - 1 : 1);
    if (py < ph / 2) *a = u;                                // smooth ramp
    else { int step = (int)(u * 8.0f); if (step > 7) step = 7; *a = (float)step / 7.0f; }
    // PREMULTIPLY. Wayland surface content is premultiplied alpha — the stored RGB
    // must already be scaled by A — and the compositor blends `src + dst*(1-a)` on
    // that assumption. Writing full-intensity RGB beside a low A is the classic
    // client bug: at a=0 it ADDS the colour to the backdrop instead of showing it.
    if (!straight_alpha) { *r *= *a; *g *= *a; *b *= *a; }
}

static void sample(int x, int y, int w, int h, float *r, float *g, float *b, float *a) {
    *a = 1.0f;
    if (alpha_mode) { alpha_pattern(x, y, w, h, r, g, b, a); return; }
    if (no_black) { check_pattern(x, y, w, h, r, g, b, a); return; }
    if (y < label_h) { *r = *g = *b = 0.0f; return; }        // label strip
    int ph = h - label_h, py = y - label_h;
    float u = (float)x / (float)(w > 1 ? w - 1 : 1);
    if (py < ph * 55 / 100) { hue(u, r, g, b); return; }     // hue sweep
    if (py < ph * 80 / 100) { *r = *g = *b = u; return; }     // grey ramp
    int band = x * 4 / w;                                     // R / G / B / W
    *r = (band == 0 || band == 3);
    *g = (band == 1 || band == 3);
    *b = (band == 2 || band == 3);
}

static float srgb_to_linear(float c) {
    return c <= 0.04045f ? c / 12.92f : powf((c + 0.055f) / 1.055f, 2.4f);
}

static uint16_t f32_to_f16(float f) {
    uint32_t x; memcpy(&x, &f, 4);
    uint32_t sign = (x >> 16) & 0x8000;
    int32_t exp = (int32_t)((x >> 23) & 0xff) - 127 + 15;
    uint32_t man = x & 0x7fffff;
    if (exp <= 0) return (uint16_t)sign;
    if (exp >= 31) return (uint16_t)(sign | 0x7c00);
    return (uint16_t)(sign | (uint32_t)(exp << 10) | (man >> 13));
}

static void put(const struct fmt *f, uint8_t *p, float r, float g, float b, float a) {
    if (f->is_float) {
        uint16_t h[4] = { f32_to_f16(srgb_to_linear(r)), f32_to_f16(srgb_to_linear(g)),
                          f32_to_f16(srgb_to_linear(b)), f32_to_f16(a) };
        memcpy(p, h, 8); // memory order R,G,B,X|A
        return;
    }
    uint8_t R = (uint8_t)lrintf(r * 255), G = (uint8_t)lrintf(g * 255),
            B = (uint8_t)lrintf(b * 255), A = (uint8_t)lrintf(a * 255);
    switch (f->fourcc) {
        case FOURCC('X','R','2','4'): p[0]=B; p[1]=G; p[2]=R; p[3]=0xff; break;
        case FOURCC('A','R','2','4'): p[0]=B; p[1]=G; p[2]=R; p[3]=A;    break;
        case FOURCC('X','B','2','4'): p[0]=R; p[1]=G; p[2]=B; p[3]=0xff; break;
        case FOURCC('A','B','2','4'): p[0]=R; p[1]=G; p[2]=B; p[3]=A;    break;
        case FOURCC('X','R','3','0'): { uint32_t v = (3u << 30)
                | ((uint32_t)lrintf(r*1023) << 20) | ((uint32_t)lrintf(g*1023) << 10)
                | (uint32_t)lrintf(b*1023); memcpy(p, &v, 4); break; }
        case FOURCC('X','B','3','0'): { uint32_t v = (3u << 30)
                | ((uint32_t)lrintf(b*1023) << 20) | ((uint32_t)lrintf(g*1023) << 10)
                | (uint32_t)lrintf(r*1023); memcpy(p, &v, 4); break; }
        case FOURCC('R','G','1','6'): { uint16_t v = (uint16_t)(((R >> 3) << 11) | ((G >> 2) << 5) | (B >> 3));
                memcpy(p, &v, 2); break; }
        case FOURCC('X','R','1','5'): { uint16_t v = (uint16_t)(0x8000 | ((R >> 3) << 10) | ((G >> 3) << 5) | (B >> 3));
                memcpy(p, &v, 2); break; }
        case FOURCC('B','A','1','2'): { uint16_t v = (uint16_t)(((B >> 4) << 12) | ((G >> 4) << 8) | ((R >> 4) << 4) | (A >> 4));
                memcpy(p, &v, 2); break; }
        case FOURCC('R','G','2','4'): p[0]=B; p[1]=G; p[2]=R; break;
        case FOURCC('R','8',' ',' '): p[0]=R; break;
        case FOURCC('R','1','6',' '): { uint16_t v = (uint16_t)(R * 257); memcpy(p, &v, 2); break; }
        default: memset(p, 0x80, (size_t)f->bpp / 8); break;  // no encoder: flat grey
    }
}

static void text(const struct fmt *f, uint8_t *px, uint32_t stride, int W, int H,
                 int x0, int y0, int scale, const char *s) {
    for (int i = 0; s[i]; i++) {
        const unsigned char *g = glyph(s[i]);
        for (int row = 0; row < 7; row++)
            for (int col = 0; col < 5; col++) {
                if (!(g[row] & (1 << (4 - col)))) continue;
                for (int dy = 0; dy < scale; dy++)
                    for (int dx = 0; dx < scale; dx++) {
                        int x = x0 + (i * 6 + col) * scale + dx, y = y0 + row * scale + dy;
                        if (x < 0 || x >= W || y < 0 || y >= H) continue;
                        put(f, px + (size_t)y * stride + (size_t)x * f->bpp / 8, 1, 1, 1, 1);
                    }
            }
    }
}

// Pattern into `px`. With `inline_label`, the fourcc is drawn INTO the tile through
// the same encoder as the pattern — an unreadable label then means the format is
// mis-handled. In grid mode the label lives in a separate known-good header instead,
// so the body is all pattern.
static void fill(const struct fmt *f, uint8_t *px, uint32_t stride, int W, int H, int inline_label) {
    int s1 = H / 64 > 0 ? H / 64 : 1, s2 = H / 128 > 0 ? H / 128 : 1;
    label_h = 0;
    if (inline_label) {
        label_h = 3 + 7 * s1 + 2 + 7 * s2 + 3;
        if (label_h > H / 2) { s1 = s2 = 1; label_h = 3 + 7 + 2 + 7 + 3; }
    }
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) {
            float r, g, b, a; sample(x, y, W, H, &r, &g, &b, &a);
            put(f, px + (size_t)y * stride + (size_t)x * f->bpp / 8, r, g, b, a);
        }
    if (!inline_label) return;
    char line2[32];
    snprintf(line2, sizeof line2, "%dBPP%s", f->bpp, f->is_float ? " LINEAR" : "");
    text(f, px, stride, W, H, 3, 3, s1, f->code);
    text(f, px, stride, W, H, 3, 3 + 7 * s1 + 2, s2, line2);
}

// A header strip: black, with `msg` in white, always in XR24 — the format that has
// worked since long before any of this. So the cell says WHICH format it is even when
// that format renders as garbage, or was refused and has no body at all. (If the
// headers themselves are wrong, XR24 is broken and nothing else matters.)
static void fill_header(uint8_t *px, uint32_t stride, int W, int H, const char *msg) {
    static const struct fmt XR24 = { "XR24", FOURCC('X','R','2','4'), 32, 0, "header" };
    int scale = H >= 26 ? 2 : 1;
    // An empty message means "backdrop": flat mid-grey, so cell gaps are visible and
    // a mis-sized or misplaced tile shows up as a broken grid line.
    memset(px, msg && !*msg ? 0x30 : 0x00, (size_t)stride * H);
    if (msg && *msg) text(&XR24, px, stride, W, H, 4, (H - 7 * scale) / 2, scale, msg);
}

// --------------------------------------------------------------- one client ----
// Surfaces to re-damage. A freshly written carrier can be SAMPLED BEFORE THE WRITE
// IS VISIBLE to the GPU: the tail of the buffer then composites as the BO's initial
// zeros (black), and because nothing damages the surface again, that half-drawn frame
// stays on screen forever. It looked exactly like a per-format import bug — it moved
// between formats run to run, and it hit the plain XR24 parent backdrop too. So every
// surface is re-damaged a few times over the first seconds; content is unchanged, so
// this is idempotent.
struct redraw { struct wl_surface *surface; int w, h; struct gbm_bo *bo; uint32_t stride;
                const struct fmt *f; int header_h; const char *msg; };

struct client {
    struct redraw rd[80];
    int nrd;
    struct wl_display *display;
    struct wl_compositor *comp;
    struct wl_subcompositor *subcomp;
    struct xdg_wm_base *shell;
    struct zwp_linux_dmabuf_v1 *dmabuf;
    struct wl_shm *shm;
    int configured, closed, params_result, dead;
    struct wl_buffer *made;
    struct wl_surface *surface;
    struct xdg_surface *xs;
    struct xdg_toplevel *top;
    struct gbm_bo *bo;
    int bo_fd;
};

static void params_created(void *data, struct zwp_linux_buffer_params_v1 *p, struct wl_buffer *b) {
    (void)p; struct client *c = data; c->made = b; c->params_result = 1;
}
static void params_failed(void *data, struct zwp_linux_buffer_params_v1 *p) {
    (void)p; ((struct client *)data)->params_result = 2;
}
static const struct zwp_linux_buffer_params_v1_listener params_listener = { params_created, params_failed };

static void wm_ping(void *data, struct xdg_wm_base *b, uint32_t s) { (void)data; xdg_wm_base_pong(b, s); }
static const struct xdg_wm_base_listener wm_listener = { wm_ping };

static void surf_configure(void *data, struct xdg_surface *s, uint32_t serial) {
    xdg_surface_ack_configure(s, serial); ((struct client *)data)->configured = 1;
}
static const struct xdg_surface_listener surf_listener = { surf_configure };

static void top_configure(void *d, struct xdg_toplevel *t, int32_t w, int32_t h, struct wl_array *a) {
    (void)d; (void)t; (void)w; (void)h; (void)a;
}
static void top_close(void *data, struct xdg_toplevel *t) { (void)t; ((struct client *)data)->closed = 1; }
static const struct xdg_toplevel_listener top_listener = {
    .configure = top_configure, .close = top_close,
};

static void global(void *data, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
    struct client *c = data;
    if (!strcmp(iface, "wl_compositor")) c->comp = wl_registry_bind(r, name, &wl_compositor_interface, 4);
    else if (!strcmp(iface, "wl_shm")) c->shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
    else if (!strcmp(iface, "wl_subcompositor")) c->subcomp = wl_registry_bind(r, name, &wl_subcompositor_interface, 1);
    else if (!strcmp(iface, "xdg_wm_base")) {
        c->shell = wl_registry_bind(r, name, &xdg_wm_base_interface, 1);
        xdg_wm_base_add_listener(c->shell, &wm_listener, c);
    } else if (!strcmp(iface, "zwp_linux_dmabuf_v1"))
        c->dmabuf = wl_registry_bind(r, name, &zwp_linux_dmabuf_v1_interface, ver < 3 ? ver : 3);
}
static void global_remove(void *d, struct wl_registry *r, uint32_t n) { (void)d; (void)r; (void)n; }
static const struct wl_registry_listener reg_listener = { global, global_remove };

static void track(struct client *c, struct wl_surface *s, int w, int h);
static struct client *blend(struct gbm_device *gbm, const char *sock, const struct fmt *f,
                            int W, int H, const char **why);
static void track_source(struct client *c, struct gbm_bo *bo, uint32_t stride,
                         const struct fmt *f, const char *msg);

static struct client *client_open(const char *sock) {
    struct client *c = calloc(1, sizeof *c);
    c->bo_fd = -1;
    c->display = wl_display_connect(sock);
    if (!c->display) { fprintf(stderr, "cannot connect to wayland display\n"); free(c); return NULL; }
    struct wl_registry *r = wl_display_get_registry(c->display);
    wl_registry_add_listener(r, &reg_listener, c);
    wl_display_roundtrip(c->display);
    if (!c->comp || !c->shell || !c->dmabuf) {
        fprintf(stderr, "missing globals (compositor=%p shell=%p dmabuf=%p)\n",
                (void *)c->comp, (void *)c->shell, (void *)c->dmabuf);
        wl_display_disconnect(c->display); free(c); return NULL;
    }
    return c;
}

static void client_close(struct client *c) {
    if (!c) return;
    if (!c->dead) {
        if (c->top) xdg_toplevel_destroy(c->top);
        if (c->xs) xdg_surface_destroy(c->xs);
        if (c->surface) wl_surface_destroy(c->surface);
        if (c->made) wl_buffer_destroy(c->made);
        wl_display_disconnect(c->display);
    }
    if (c->bo_fd >= 0) close(c->bo_fd);
    if (c->bo) gbm_bo_destroy(c->bo);
    free(c);
}

// Allocate + encode + ask for a wl_buffer. 0 ok, 1 refused, 2 protocol error (dead).
// `header` non-NULL builds a label strip (always XR24) instead of the pattern;
// `inline_label` draws the fourcc into the pattern itself.
static int verify;   // --verify: read the BO back after writing it
static int use_shm;    // --shm: wl_shm instead of dmabuf

static int force_linear; // --linear: force the carrier BO to DRM_FORMAT_MOD_LINEAR
// --modifier=<hex|linear|invalid>: allocate the carrier with EXACTLY this modifier.
// `--linear` is the special case of it that predates it. `have_modifier` is separate
// from the value because 0 IS linear, a legal request, not "unset".
static uint64_t want_modifier;
static int have_modifier;
static int slack_rows;   // --slack=N: allocate N extra carrier rows past the declared H

// Allocate the byte-carrier for one format, honouring --linear, and REPORT what the driver
// actually gave us. The buffer DECLARED to the compositor is W x H of `f`, but the memory
// is a cw x H XRGB8888 bo — so `f->bpp` is what decides the carrier's width. At 32bpp the
// carrier is full width; below that it is narrower, and the driver may pick a different
// block-linear kind (or LINEAR) per width. That is exactly the axis along which the
// black-bar reports split, so print it on every run.
static struct gbm_bo *carrier(struct gbm_device *gbm, const struct fmt *f, int W, int H) {
    uint32_t need = (uint32_t)W * f->bpp / 8;
    uint32_t cw = (need + 3) / 4;
    // --slack: allocate PAST the declared height. We hand the compositor a buffer of
    // exactly W*bpp/8*H bytes, but Vulkan's `requirements.size` for a W x H image of that
     // format can be LARGER (NVIDIA rounds a LINEAR pitch to 256 and may want height
    // alignment on top). If it is, the tail of the image binds memory this client never
    // wrote — which the compositor's own `vulkan import: image needs N byte(s) but the
    // dmabuf holds M` warning predicts as "a black/garbage band at the bottom". Extra rows
    // make the dmabuf comfortably larger without changing what we declare, so if the bars
    // disappear under --slack the cause is buffer SIZE, not the format.
    int ch = H + slack_rows;
    struct gbm_bo *bo;
    if (have_modifier) {
        // ONE modifier offered, so gbm either gives us that layout or fails. Handing
        // it a list would let the driver pick a different one and the run would
        // silently test something other than what was asked for.
        bo = gbm_bo_create_with_modifiers2(gbm, cw, ch, GBM_FORMAT_XRGB8888,
                                           &want_modifier, 1,
                                           gpu_render ? GBM_BO_USE_RENDERING : GBM_BO_USE_WRITE);
        if (!bo) {
            printf("  %-5s carrier REFUSED at modifier 0x%016llx — gbm cannot allocate "
                   "this layout with GBM_BO_USE_WRITE on this device\n",
                   f->code, (unsigned long long)want_modifier);
            return NULL;
        }
    } else if (force_linear) {
        uint64_t lin = 0; // DRM_FORMAT_MOD_LINEAR
        bo = gbm_bo_create_with_modifiers2(gbm, cw, ch, GBM_FORMAT_XRGB8888, &lin, 1,
                                           GBM_BO_USE_WRITE);
    } else {
        bo = gbm_bo_create(gbm, cw, ch, GBM_FORMAT_XRGB8888,
                           gpu_render ? GBM_BO_USE_RENDERING : GBM_BO_USE_WRITE);
    }
    if (!bo) return NULL;
    uint32_t stride = gbm_bo_get_stride(bo);
    uint64_t mod = gbm_bo_get_modifier(bo);
    if (have_modifier && mod != want_modifier) {
        printf("  %-5s carrier ASKED 0x%016llx GOT 0x%016llx — the driver ignored the "
               "request; this run is NOT testing the modifier you named\n",
               f->code, (unsigned long long)want_modifier, (unsigned long long)mod);
    }
    printf("  %-5s carrier %ux%u XR24  stride %u (need %u)  bytes %zu (declared %zu)%s"
           "  modifier 0x%016llx%s\n",
           f->code, cw, (unsigned)ch, stride, need,
           (size_t)stride * ch, (size_t)need * H,
           stride == need ? "" : "  STRIDE>NEED",
           (unsigned long long)mod, mod == 0 ? " (LINEAR)" : " (tiled)");
    return bo;
}

static int compare_bo(struct gbm_bo *bo, const uint8_t *px, uint32_t stride,
                      int W, int H, int bpp, size_t *bad_bytes, int *bad_rows, uint32_t *out_mstride);

static int write_map;  // --write=map: gbm_bo_map/memcpy/unmap instead of gbm_bo_write
static int rewrite;    // --rewrite: re-write the BO on every re-damage tick
static int settle_ms;  // --settle=N: wait N ms after writing, verify, THEN commit
static int force_linear; // --linear: force the carrier BO to DRM_FORMAT_MOD_LINEAR

// THE DECISIVE ORDERING. NVIDIA's gbm CPU-write path is asynchronous: read a BO back
// microseconds after writing it and a few rows can still be stale, which is
// indistinguishable — from the compositor's side — from a client that handed over a
// half-written buffer. So wait, verify the BO byte-for-byte, and only commit once it
// is provably complete. A black band AFTER a clean verify is the compositor's.

// The two ways to get CPU pixels into a BO. gbm_bo_write is one call; map/unmap makes
// the flush explicit. If a black band survives one and not the other, the fault is in
// write VISIBILITY, not in the compositor.
static int put_pixels(struct gbm_bo *bo, const uint8_t *px, uint32_t stride, int H) {
    if (!write_map) return gbm_bo_write(bo, px, (size_t)stride * H);
    uint32_t mstride = 0; void *mdata = NULL;
    void *mp = gbm_bo_map(bo, 0, 0, gbm_bo_get_width(bo), H, GBM_BO_TRANSFER_WRITE, &mstride, &mdata);
    if (!mp) return -1;
    for (int y = 0; y < H; y++)
        memcpy((uint8_t *)mp + (size_t)y * mstride, px + (size_t)y * stride,
               stride < mstride ? stride : mstride);
    gbm_bo_unmap(bo, mp);
    return 0;
}

// ----------------------------------------------------------------- gpu ------
// RENDER INTO THE DMABUF WITH THE GPU, the way a real client does.
//
// Everything else here CPU-writes a carrier and hands it over. That is the right way to
// test what the compositor does with a KNOWN byte pattern, and it is the wrong way to
// answer "does a GPU client work": the buffer is allocated with `GBM_BO_USE_WRITE`,
// which on this driver forces LINEAR, so the tiled modifiers a real client actually
// gets are never exercised at all.
//
// This path allocates with `GBM_BO_USE_RENDERING`, imports the dmabuf back as an
// EGLImage, binds it to an FBO and draws with GLES — so the buffer is tiled, written by
// the GPU, and never touched by the CPU.
//
// The pattern is scissored `glClear`s rather than a shader: exact values, no
// interpolation and no shader compilation to go wrong, which is what a test wants.
// Colours are PREMULTIPLIED, matching the protocol.
static EGLDisplay egl_dpy = EGL_NO_DISPLAY;
static EGLContext egl_ctx = EGL_NO_CONTEXT;

static int gpu_init(struct gbm_device *gbm) {
    if (egl_ctx != EGL_NO_CONTEXT) return 0;
    PFNEGLGETPLATFORMDISPLAYEXTPROC get_dpy =
        (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    egl_dpy = get_dpy ? get_dpy(EGL_PLATFORM_GBM_KHR, gbm, NULL)
                      : eglGetDisplay((EGLNativeDisplayType)gbm);
    if (egl_dpy == EGL_NO_DISPLAY) { printf("  gpu: no EGL display\n"); return -1; }
    if (!eglInitialize(egl_dpy, NULL, NULL)) { printf("  gpu: eglInitialize failed\n"); return -1; }
    if (!eglBindAPI(EGL_OPENGL_ES_API)) { printf("  gpu: eglBindAPI failed\n"); return -1; }
    // Surfaceless: we only ever render to an FBO backed by the imported dmabuf.
    static const EGLint attrs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    egl_ctx = eglCreateContext(egl_dpy, EGL_NO_CONFIG_KHR, EGL_NO_CONTEXT, attrs);
    if (egl_ctx == EGL_NO_CONTEXT) { printf("  gpu: eglCreateContext failed\n"); return -1; }
    if (!eglMakeCurrent(egl_dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, egl_ctx)) {
        printf("  gpu: eglMakeCurrent(surfaceless) failed\n"); return -1;
    }
    printf("  gpu: EGL %s, renderer %s\n", eglQueryString(egl_dpy, EGL_VERSION),
           (const char *)glGetString(GL_RENDERER));
    return 0;
}

// Import `bo` as an EGLImage and draw the alpha staircase into it with GLES.
static int gpu_fill(struct gbm_device *gbm, struct gbm_bo *bo, int W, int H) {
    if (gpu_init(gbm) != 0) return -1;
    int fd = gbm_bo_get_fd(bo);
    if (fd < 0) { printf("  gpu: gbm_bo_get_fd failed\n"); return -1; }
    uint64_t mod = gbm_bo_get_modifier(bo);
    uint32_t stride = gbm_bo_get_stride(bo), off = gbm_bo_get_offset(bo, 0);
    EGLint a[] = {
        EGL_WIDTH, gbm_bo_get_width(bo), EGL_HEIGHT, gbm_bo_get_height(bo),
        EGL_LINUX_DRM_FOURCC_EXT, (EGLint)gbm_bo_get_format(bo),
        EGL_DMA_BUF_PLANE0_FD_EXT, fd,
        EGL_DMA_BUF_PLANE0_OFFSET_EXT, (EGLint)off,
        EGL_DMA_BUF_PLANE0_PITCH_EXT, (EGLint)stride,
        EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, (EGLint)(mod & 0xffffffff),
        EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, (EGLint)(mod >> 32),
        EGL_NONE
    };
    PFNEGLCREATEIMAGEKHRPROC create_img = (void *)eglGetProcAddress("eglCreateImageKHR");
    PFNEGLDESTROYIMAGEKHRPROC destroy_img = (void *)eglGetProcAddress("eglDestroyImageKHR");
    PFNGLEGLIMAGETARGETTEXTURE2DOESPROC img_tex =
        (void *)eglGetProcAddress("glEGLImageTargetTexture2DOES");
    if (!create_img || !img_tex) { printf("  gpu: EGL dmabuf import entry points missing\n"); close(fd); return -1; }
    EGLImageKHR img = create_img(egl_dpy, EGL_NO_CONTEXT, EGL_LINUX_DMA_BUF_EXT, NULL, a);
    close(fd);
    if (img == EGL_NO_IMAGE_KHR) {
        printf("  gpu: eglCreateImageKHR REFUSED this dmabuf (modifier 0x%llx) — the driver "
               "will not import its own buffer for rendering\n", (unsigned long long)mod);
        return -1;
    }
    GLuint tex = 0, fbo = 0;
    glGenTextures(1, &tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    img_tex(GL_TEXTURE_2D, img);
    glGenFramebuffers(1, &fbo);
    glBindFramebuffer(GL_FRAMEBUFFER, fbo);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    GLenum st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
    if (st != GL_FRAMEBUFFER_COMPLETE) {
        printf("  gpu: FBO incomplete (0x%x) — cannot render into this buffer\n", st);
        destroy_img(egl_dpy, img); return -1;
    }
    glViewport(0, 0, W, H);
    glDisable(GL_BLEND);                       // we are WRITING the buffer, not compositing
    glEnable(GL_SCISSOR_TEST);
    // Eight steps of premultiplied orange: rgb scaled by a, exactly as the CPU path.
    for (int i = 0; i < 8; i++) {
        float al = (float)i / 7.0f;
        glScissor(W * i / 8, 0, W / 8 + 1, H);
        glClearColor(1.0f * al, 0.25f * al, 0.0f * al, al);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    glDisable(GL_SCISSOR_TEST);
    glFinish();                                // the compositor may sample immediately
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
    glDeleteFramebuffers(1, &fbo);
    glDeleteTextures(1, &tex);
    destroy_img(egl_dpy, img);
    printf("  gpu: rendered 8 premultiplied alpha steps into the dmabuf (modifier 0x%llx)\n",
           (unsigned long long)mod);
    return 0;
}

static int build(struct client *c, struct gbm_device *gbm, const struct fmt *f,
                 int W, int H, const char *header, int inline_label, const char **why) {
    *why = "";
    struct gbm_bo *bo = carrier(gbm, f, W, H);
    if (!bo) { *why = "carrier alloc failed"; return 1; }
    uint32_t stride = gbm_bo_get_stride(bo);
    uint64_t mod = gbm_bo_get_modifier(bo);

    uint8_t *px = calloc((size_t)stride * H, 1);
    if (gpu_render) {
        // No CPU write at all on this path — the GPU owns the buffer.
        if (gpu_fill(gbm, bo, W, H) != 0) {
            *why = "gpu render failed"; free(px); gbm_bo_destroy(bo); return 1;
        }
    } else {
    if (header) fill_header(px, stride, W, H, header);
    else fill(f, px, stride, W, H, inline_label);
    if (put_pixels(bo, px, stride, H) != 0)
        fprintf(stderr, "%s: pixel write FAILED - window shows stale memory\n", f->code);
    }
    if (settle_ms > 0) {
        usleep(settle_ms * 1000);
        size_t bad = 0; int rows = 0; uint32_t ms = 0;
        int tries = 0;
        while (compare_bo(bo, px, stride, W, H, f->bpp, &bad, &rows, &ms) >= 0 && tries < 20) {
            // Not settled yet: put the pixels in again and wait again, rather than
            // committing something we know is incomplete.
            put_pixels(bo, px, stride, H);
            usleep(settle_ms * 1000);
            tries++;
        }
        printf("  settle %-5s: %s after %d retry/ies\n", f->code,
               tries < 20 ? "BO verified complete before commit" : "STILL incomplete - giving up",
               tries);
    }
    // VERIFY WHAT LANDED. If the tail of the BO reads back as zeros, a black band on
    // screen is the client's own buffer and the compositor is innocent; if the BO is
    // byte-perfect and the screen still shows black, it is not.
    if (verify) {
        uint32_t mstride = 0; void *mdata = NULL; void *mp = NULL;
        mp = gbm_bo_map(bo, 0, 0, gbm_bo_get_width(bo), H, GBM_BO_TRANSFER_READ, &mstride, &mdata);
        if (!mp) {
            printf("  verify %-5s: gbm_bo_map(READ) failed - cannot confirm\n", f->code);
        } else {
            const uint8_t *got = mp;
            int bad_row = -1; size_t bad_bytes = 0;
            for (int y = 0; y < H; y++) {
                size_t n = (size_t)W * f->bpp / 8;
                if (memcmp(got + (size_t)y * mstride, px + (size_t)y * stride, n) != 0) {
                    if (bad_row < 0) bad_row = y;
                    bad_bytes += n;
                }
            }
            if (bad_row < 0)
                printf("  verify %-5s: BO matches what we wrote (%d rows, stride %u/%u) - OK\n",
                       f->code, H, stride, mstride);
            else
                printf("  verify %-5s: MISMATCH from row %d (%zu bytes differ, stride %u/%u)"
                       " -> the write did not fully land\n",
                       f->code, bad_row, bad_bytes, stride, mstride);
            gbm_bo_unmap(bo, mp);
        }
    }
    free(px);

    c->bo = bo;
    c->bo_fd = gbm_bo_get_fd(bo);
    c->params_result = 0; c->made = NULL;
    struct zwp_linux_buffer_params_v1 *params = zwp_linux_dmabuf_v1_create_params(c->dmabuf);
    zwp_linux_buffer_params_v1_add_listener(params, &params_listener, c);
    zwp_linux_buffer_params_v1_add(params, c->bo_fd, 0, 0, stride,
                                   (uint32_t)(mod >> 32), (uint32_t)(mod & 0xffffffff));
    zwp_linux_buffer_params_v1_create(params, W, H, f->fourcc, 0);
    while (!c->params_result)
        if (wl_display_dispatch(c->display) == -1) { c->dead = 1; break; }

    if (c->dead) {
        const struct wl_interface *i = NULL; uint32_t id = 0;
        uint32_t code = wl_display_get_protocol_error(c->display, &i, &id);
        // invalid_format is 4 (see linux-dmabuf-v1-client-protocol.h) — the code
        // that means "this fourcc is not in the advertised set", which smithay
        // raises as a protocol error rather than a soft params.failed.
        *why = (i && !strcmp(i->name, "zwp_linux_buffer_params_v1") && code == 4)
                   ? "InvalidFormat -> NOT ADVERTISED" : "protocol error";
        return 2;
    }
    zwp_linux_buffer_params_v1_destroy(params);
    if (c->params_result == 2) { *why = "failed -> refused (soft)"; return 1; }
    return 0;
}

// THE SAME PATTERN OVER wl_shm. No dmabuf, no modifier, no Vulkan import, no entry in
// format.table — the compositor uploads it itself. If a black band appears here too,
// every dmabuf-side explanation is excluded and what remains is compositing/damage.
static int build_shm(struct client *c, const struct fmt *f, int W, int H, const char **why) {
    *why = "";
    if (!c->shm) { *why = "no wl_shm global"; return 1; }
    uint32_t stride = (uint32_t)W * 4;                 // shm path is always 32bpp
    size_t size = (size_t)stride * H;
    int fd = memfd_create("dmabuf-draw-shm", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, size) != 0) { *why = "memfd/ftruncate failed"; return 1; }
    uint8_t *map = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { *why = "mmap failed"; close(fd); return 1; }

    struct fmt as_xr24 = { f->code, FOURCC('X','R','2','4'), 32, 0, f->note };
    fill(&as_xr24, map, stride, W, H, 1);
    struct wl_shm_pool *pool = wl_shm_create_pool(c->shm, fd, (int32_t)size);
    c->made = wl_shm_pool_create_buffer(pool, 0, W, H, (int32_t)stride, WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    munmap(map, size);                                  // the pool keeps its own mapping
    close(fd);
    *why = c->made ? "" : "create_buffer failed";
    return c->made ? 0 : 1;
}

// A mapped toplevel showing `f`. NULL when the format was refused.
static struct client *window_for(struct gbm_device *gbm, const char *sock, const struct fmt *f,
                                 int W, int H, const char **why) {
    struct client *c = client_open(sock);
    if (!c) { *why = "cannot connect"; return NULL; }
    int rc = use_shm ? build_shm(c, f, W, H, why) : build(c, gbm, f, W, H, NULL, 1, why);
    if (rc != 0) { client_close(c); return NULL; }

    c->surface = wl_compositor_create_surface(c->comp);
    c->xs = xdg_wm_base_get_xdg_surface(c->shell, c->surface);
    xdg_surface_add_listener(c->xs, &surf_listener, c);
    c->top = xdg_surface_get_toplevel(c->xs);
    xdg_toplevel_add_listener(c->top, &top_listener, c);
    char title[64];
    snprintf(title, sizeof title, "dmabuf %s (%dbpp%s)", f->code, f->bpp, f->is_float ? " linear" : "");
    xdg_toplevel_set_title(c->top, title);
    xdg_toplevel_set_app_id(c->top, "dmabuf-draw");
    wl_surface_commit(c->surface);
    while (!c->configured && wl_display_dispatch(c->display) != -1) {}
    wl_surface_attach(c->surface, c->made, 0, 0);
    wl_surface_damage_buffer(c->surface, 0, 0, W, H);
    wl_surface_commit(c->surface);
    wl_display_flush(c->display);
    track(c, c->surface, W, H);
    track_source(c, c->bo, c->bo ? gbm_bo_get_stride(c->bo) : 0, f, NULL);
    *why = "shown";
    return c;
}

// THE BLEND TEST. An opaque backdrop toplevel with a translucent subsurface over it.
//
// Alpha cannot be tested inside one surface: whatever the client draws IS the surface,
// so a translucent pixel has nothing to blend against and the compositor's blend is
// never invoked. The window then looks identical whether the compositor honours alpha
// or ignores it, which is exactly why `AR24` and `XR24` used to look the same here.
//
// Two surfaces make the compositor do the work. The parent is a hard checkerboard in
// `XR24` — a format already proven byte-exact, so anything wrong in the result is the
// child's or the blend's, never the backdrop's. The child carries the alpha staircase
// in the format under test, and NO opaque region is set on it, so the compositor must
// composite it. Over the checkerboard, step k should read as `colour*k/7 + check*(1-k/7)`:
// the left edge is pure checkerboard, the right edge is pure colour.
static struct client *blend(struct gbm_device *gbm, const char *sock, const struct fmt *f,
                            int W, int H, const char **why) {
    struct client *c = client_open(sock);
    if (!c) { *why = "cannot connect"; return NULL; }
    if (!c->subcomp) { *why = "no wl_subcompositor"; client_close(c); return NULL; }

    // Backdrop: same size as the child, offset so a border of pure checkerboard stays
    // visible around it — that border is the reference the blended area is read against.
    const int PAD = 24;
    int BW = W + PAD * 2, BH = H + PAD * 2;
    int saved_alpha = alpha_mode, saved_black = no_black, saved_gpu = gpu_render;
    // The backdrop is ALWAYS CPU-written, even under --gpu. It is the reference the
    // blend is read against, so it has to be a known checkerboard — GPU-rendering it
    // would overwrite it with the same alpha steps as the child and leave nothing to
    // compare. --gpu applies to the surface UNDER TEST, which is the child.
    // ...and it ignores --modifier for the same reason: that flag names the layout the
    // SURFACE UNDER TEST should use, and a CPU-written backdrop cannot take a tiled one.
    int saved_have_mod = have_modifier;
    alpha_mode = 0; no_black = 1; gpu_render = 0; have_modifier = 0;
    struct fmt back = { "BACK", FOURCC('X','R','2','4'), 32, 0, "blend backdrop" };
    if (build(c, gbm, &back, BW, BH, "", 0, why) != 0) {
        printf("blend: the XR24 backdrop could not be built: %s\n", *why);
        alpha_mode = saved_alpha; no_black = saved_black; gpu_render = saved_gpu;
        have_modifier = saved_have_mod; client_close(c); return NULL;
    }
    struct wl_buffer *backbuf = c->made;
    c->surface = wl_compositor_create_surface(c->comp);
    c->xs = xdg_wm_base_get_xdg_surface(c->shell, c->surface);
    xdg_surface_add_listener(c->xs, &surf_listener, c);
    c->top = xdg_surface_get_toplevel(c->xs);
    xdg_toplevel_add_listener(c->top, &top_listener, c);
    char title[80];
    snprintf(title, sizeof title, "blend %s over checkerboard", f->code);
    xdg_toplevel_set_title(c->top, title);
    xdg_toplevel_set_app_id(c->top, "dmabuf-draw");
    wl_surface_commit(c->surface);
    while (!c->configured && wl_display_dispatch(c->display) != -1) {}
    wl_surface_attach(c->surface, backbuf, 0, 0);
    wl_surface_damage_buffer(c->surface, 0, 0, BW, BH);
    track(c, c->surface, BW, BH);

    // Child: the format under test, alpha varying, deliberately WITHOUT an opaque
    // region — declaring one would tell the compositor it may skip the blend, which
    // is the one thing this test must not allow it to do.
    alpha_mode = 1; no_black = saved_black; gpu_render = saved_gpu; have_modifier = saved_have_mod;
    if (build(c, gbm, f, W, H, f->code, 1, why) != 0) {
        alpha_mode = saved_alpha; no_black = saved_black; gpu_render = saved_gpu;
        printf("blend: %s could not be built: %s\n", f->code, *why);
        client_close(c); return NULL;
    }
    alpha_mode = saved_alpha; no_black = saved_black; gpu_render = saved_gpu;
    struct wl_surface *child = wl_compositor_create_surface(c->comp);
    struct wl_subsurface *sub = wl_subcompositor_get_subsurface(c->subcomp, child, c->surface);
    wl_subsurface_set_position(sub, PAD, PAD);
    wl_subsurface_set_desync(sub);
    wl_surface_attach(child, c->made, 0, 0);
    wl_surface_damage_buffer(child, 0, 0, W, H);
    wl_surface_commit(child);
    track(c, child, W, H);
    track_source(c, c->bo, c->bo ? gbm_bo_get_stride(c->bo) : 0, f, NULL);
    wl_surface_commit(c->surface);
    wl_display_flush(c->display);

    printf("\nblend test: %s over an XR24 checkerboard, %dx%d inset by %d\n", f->code, W, H, PAD);
    printf("  expect: left edge = pure checkerboard, right edge = pure orange,\n");
    printf("          eight readable steps across the lower half.\n");
    printf("  an X-format (no alpha channel) must be FULLY OPAQUE throughout — that is\n");
    printf("  the control, not a failure. Compare XR24 against AR24.\n");
    printf("  alpha is %s. Wayland content is premultiplied; --straight writes the\n",
           straight_alpha ? "STRAIGHT (deliberately wrong)" : "premultiplied (correct)");
    printf("  non-premultiplied form a client gets wrong, which blows out to white\n");
    printf("  instead of fading to the backdrop.\n");
    *why = "shown";
    return c;
}

static void track(struct client *c, struct wl_surface *s, int w, int h) {
    if (c->nrd < (int)(sizeof c->rd / sizeof *c->rd)) {
        c->rd[c->nrd] = (struct redraw){ .surface = s, .w = w, .h = h };
        c->nrd++;
    }
}

// Attach the BO + encoder to the last tracked surface so --rewrite can refill it.
static void track_source(struct client *c, struct gbm_bo *bo, uint32_t stride,
                         const struct fmt *f, const char *msg) {
    if (!c->nrd) return;
    struct redraw *r = &c->rd[c->nrd - 1];
    r->bo = bo; r->stride = stride; r->f = f; r->msg = msg;
}

static void redamage(struct client *c) {
    for (int i = 0; i < c->nrd; i++) {
        // --rewrite: put the pixels in again before damaging. If the band heals on a
        // later tick, the first write simply had not become visible; if it survives
        // repeated writes, the imported view of that memory is wrong.
        if (rewrite && c->rd[i].bo && c->rd[i].f) {
            struct redraw *r = &c->rd[i];
            uint8_t *px = calloc((size_t)r->stride * r->h, 1);
            if (r->msg) fill_header(px, r->stride, r->w, r->h, r->msg);
            else fill(r->f, px, r->stride, r->w, r->h, 0);
            put_pixels(r->bo, px, r->stride, r->h);
            free(px);
        }
        wl_surface_damage_buffer(c->rd[i].surface, 0, 0, c->rd[i].w, c->rd[i].h);
        wl_surface_commit(c->rd[i].surface);
    }
    wl_display_flush(c->display);
}

static void pump(struct client **cs, int n, int hold_ms) {
    // Re-damage at these elapsed times (ms), so a write that landed late still gets
    // composited. See `struct redraw`.
    const int REDRAW_AT[] = { 300, 1000, 2500 };
    for (int waited = 0; hold_ms <= 0 || waited < hold_ms; waited += 100) {
        int alive = 0;
        for (int i = 0; i < n; i++) {
            if (!cs[i] || cs[i]->dead || cs[i]->closed) continue;
            alive = 1;
            for (size_t k = 0; k < sizeof REDRAW_AT / sizeof *REDRAW_AT; k++)
                if (waited == REDRAW_AT[k]) redamage(cs[i]);
            wl_display_flush(cs[i]->display);
            if (wl_display_dispatch_pending(cs[i]->display) == -1) cs[i]->dead = 1;
        }
        if (!alive) return;
        usleep(100 * 1000);
    }
}

// ------------------------------------------------------------------- grid ------
// One toplevel, one subsurface per format, positions set by the CLIENT so every tile
// is in a known place in one screenshot. Pre-flighted on separate connections first:
// an unadvertised fourcc would otherwise kill the grid's connection.
static int grid(struct gbm_device *gbm, const char *sock, const struct fmt **sel, int n,
                int TW, int TH, int cols, int gap, int hold_ms) {
    // Pre-flight decides which formats get a BODY. Every format still gets a header,
    // so a refused one leaves a labelled, empty cell rather than vanishing.
    const char *status[64];
    printf("pre-flight:\n");
    for (int i = 0; i < n; i++) {
        struct client *c = client_open(sock);
        if (!c) return -1;
        const char *why;
        int rc = build(c, gbm, sel[i], TW, TH, NULL, 0, &why);
        status[i] = rc == 0 ? NULL : (why && *why ? why : "refused");
        printf("  %-5s %-42s %s\n", sel[i]->code, sel[i]->note, rc == 0 ? "accepted" : why);
        client_close(c);
    }

    struct client *c = client_open(sock);
    if (!c) return -1;
    if (!c->subcomp) { printf("no wl_subcompositor; use --all-concurrent\n"); client_close(c); return -1; }

    // Cell = header strip + body. Gaps let the backdrop show through, so a
    // misplaced or mis-sized tile is visible as a broken grid line.
    const int HDR = TH >= 96 ? 26 : 16;
    int cell_w = TW, cell_h = HDR + TH;
    int pitch_x = cell_w + gap, pitch_y = cell_h + gap;
    int rows = (n + cols - 1) / cols;
    int GW = cols * pitch_x + gap, GH = rows * pitch_y + gap;

    struct fmt backdrop = { "GRID", FOURCC('X','R','2','4'), 32, 0, "backdrop" };
    const char *why;
    if (build(c, gbm, &backdrop, GW, GH, "", 0, &why) != 0) {
        printf("backdrop failed: %s\n", why); client_close(c); return -1;
    }
    c->surface = wl_compositor_create_surface(c->comp);
    c->xs = xdg_wm_base_get_xdg_surface(c->shell, c->surface);
    xdg_surface_add_listener(c->xs, &surf_listener, c);
    c->top = xdg_surface_get_toplevel(c->xs);
    xdg_toplevel_add_listener(c->top, &top_listener, c);
    xdg_toplevel_set_title(c->top, "dmabuf format grid");
    xdg_toplevel_set_app_id(c->top, "dmabuf-draw");
    wl_surface_commit(c->surface);
    while (!c->configured && wl_display_dispatch(c->display) != -1) {}
    wl_surface_attach(c->surface, c->made, 0, 0);
    wl_surface_damage_buffer(c->surface, 0, 0, GW, GH);

    struct gbm_bo *bos[128]; int fds[128], nres = 0;
    printf("\ngrid %d x %d, cells %dx%d (header %d + body %dx%d), gap %d:\n",
           cols, rows, cell_w, cell_h, HDR, TW, TH, gap);
    for (int i = 0; i < n; i++) {
        int cx = gap + (i % cols) * pitch_x, cy = gap + (i / cols) * pitch_y;

        // Header first, and unconditionally: the cell must name itself even when the
        // body below is refused, blank or garbage.
        char msg[48];
        snprintf(msg, sizeof msg, "%s %dB %s", sel[i]->code, sel[i]->bpp,
                 status[i] ? (strstr(status[i], "NOT ADVERTISED") ? "NOTADV" : "REFUSD")
                           : (sel[i]->is_float ? "OK LIN" : "OK"));
        struct client h = *c; h.bo = NULL; h.bo_fd = -1; h.made = NULL;
        if (build(&h, gbm, &backdrop, cell_w, HDR, msg, 0, &why) == 0) {
            struct wl_surface *s = wl_compositor_create_surface(c->comp);
            struct wl_subsurface *sub = wl_subcompositor_get_subsurface(c->subcomp, s, c->surface);
            wl_subsurface_set_position(sub, cx, cy);
            wl_subsurface_set_desync(sub);
            wl_surface_attach(s, h.made, 0, 0);
            wl_surface_damage_buffer(s, 0, 0, cell_w, HDR);
            wl_surface_commit(s);
            track(c, s, cell_w, HDR);
            track_source(c, h.bo, gbm_bo_get_stride(h.bo), &backdrop, msg);
            bos[nres] = h.bo; fds[nres++] = h.bo_fd;
        }

        if (status[i]) {
            printf("  (%4d,%4d) %-5s header only — %s\n", cx, cy, sel[i]->code, status[i]);
            continue;
        }
        struct client tmp = *c; tmp.bo = NULL; tmp.bo_fd = -1; tmp.made = NULL;
        if (build(&tmp, gbm, sel[i], TW, TH, NULL, 0, &why) != 0) {
            printf("  (%4d,%4d) %-5s LOST: %s\n", cx, cy, sel[i]->code, why);
            continue;
        }
        struct wl_surface *s = wl_compositor_create_surface(c->comp);
        struct wl_subsurface *sub = wl_subcompositor_get_subsurface(c->subcomp, s, c->surface);
        wl_subsurface_set_position(sub, cx, cy + HDR);
        wl_subsurface_set_desync(sub);
        wl_surface_attach(s, tmp.made, 0, 0);
        wl_surface_damage_buffer(s, 0, 0, TW, TH);
        wl_surface_commit(s);
        track(c, s, TW, TH);
        track_source(c, tmp.bo, gbm_bo_get_stride(tmp.bo), sel[i], NULL);
        bos[nres] = tmp.bo; fds[nres++] = tmp.bo_fd;
        printf("  (%4d,%4d) %-5s %s\n", cx, cy + HDR, sel[i]->code, sel[i]->note);
    }
    track(c, c->surface, GW, GH);
    track_source(c, c->bo, gbm_bo_get_stride(c->bo), &backdrop, "");
    wl_surface_commit(c->surface);
    wl_display_flush(c->display);
    printf("\nwindow up (%dx%d) - screenshot it with y5's capture (~/Pictures/y5-capture-*.png)\n", GW, GH);

    struct client *one[1] = { c };
    pump(one, 1, hold_ms);
    for (int i = 0; i < nres; i++) { close(fds[i]); gbm_bo_destroy(bos[i]); }
    client_close(c);
    return 0;
}

// Allocate + encode + write + read back, with NO wayland connection at all. Answers
// "did the CPU write fully land in the BO" in isolation, which is the first fork in
// diagnosing a black band: client content, or compositor paint.
static int loops = 1;
static int watch_secs;  // --watch=N: hold one buffer and re-compare for N seconds

// Does the corruption arrive AT WRITE TIME, or does something scribble into the BO
// afterwards? Write once, then re-compare on a timer while doing nothing else. Growing
// damage over time means an external agent is writing into memory this process owns;
// damage that is present immediately and never changes means the write itself lost it.
static int watch_one(struct gbm_device *gbm, const struct fmt *f, int W, int H) {
    struct gbm_bo *bo = carrier(gbm, f, W, H);
    if (!bo) { printf("watch: alloc failed\n"); return 1; }
    uint32_t stride = gbm_bo_get_stride(bo);
    uint8_t *px = calloc((size_t)stride * H, 1);
    fill(f, px, stride, W, H, 0);
    if (put_pixels(bo, px, stride, H) != 0) printf("watch: write failed\n");
    printf("watch %s %dx%d (bo %ux%u stride %u), re-comparing every 250 ms:\n",
           f->code, W, H, gbm_bo_get_width(bo), gbm_bo_get_height(bo), stride);
    int prev_rows = -1, changes = 0;
    for (int t = 0; t <= watch_secs * 4; t++) {
        size_t bad = 0; int rows = 0; uint32_t ms = 0;
        int first = compare_bo(bo, px, stride, W, H, f->bpp, &bad, &rows, &ms);
        if (rows != prev_rows) {
            printf("  t=%5.2fs  %s\n", t * 0.25,
                   first < 0 ? "clean" : ({ static char b[96];
                       snprintf(b, sizeof b, "%d bad row(s), first %d, %zu bytes", rows, first, bad); b; }));
            if (prev_rows >= 0) changes++;
            prev_rows = rows;
        }
        usleep(250 * 1000);
    }
    printf("  -> %s\n", changes ? "DAMAGE CHANGED OVER TIME (something else is writing here)"
                                 : "stable (no external writes observed)");
    free(px); gbm_bo_destroy(bo);
    return 0;
}

// Compare a BO against what we wrote. Returns the first differing row, or -1.
static int compare_bo(struct gbm_bo *bo, const uint8_t *px, uint32_t stride,
                      int W, int H, int bpp, size_t *bad_bytes, int *bad_rows, uint32_t *out_mstride) {
    uint32_t mstride = 0; void *mdata = NULL;
    void *mp = gbm_bo_map(bo, 0, 0, gbm_bo_get_width(bo), H, GBM_BO_TRANSFER_READ, &mstride, &mdata);
    *out_mstride = mstride;
    if (!mp) return -2;
    const uint8_t *got = mp;
    int first = -1; *bad_bytes = 0; *bad_rows = 0;
    for (int y = 0; y < H; y++) {
        size_t nb = (size_t)W * bpp / 8;
        if (memcmp(got + (size_t)y * mstride, px + (size_t)y * stride, nb)) {
            if (first < 0) first = y;
            *bad_bytes += nb; (*bad_rows)++;
        }
    }
    gbm_bo_unmap(bo, mp);
    return first;
}

static int selftest(struct gbm_device *gbm, const struct fmt **sel, int n, int W, int H) {
    int fails = 0, runs = 0;
    printf("self-test (no compositor): alloc -> encode -> %s -> map+compare, %d loop(s)\n",
           write_map ? "gbm_bo_map/unmap" : "gbm_bo_write", loops);
    for (int loop = 0; loop < loops; loop++)
    for (int i = 0; i < n; i++) {
        const struct fmt *f = sel[i];
        runs++;
            struct gbm_bo *bo = carrier(gbm, f, W, H);
        if (!bo) { printf("  %-5s %4dx%-4d alloc FAILED\n", f->code, W, H); fails++; continue; }
        uint32_t stride = gbm_bo_get_stride(bo);
        uint8_t *px = calloc((size_t)stride * H, 1);
        fill(f, px, stride, W, H, 0);
        int wrote = put_pixels(bo, px, stride, H);
        size_t bad = 0; int bad_rows = 0; uint32_t mstride = 0;
        int bad_row = compare_bo(bo, px, stride, W, H, f->bpp, &bad, &bad_rows, &mstride);
        if (bad_row >= 0) {
            // RE-READ. If the same BO now matches, the data was in flight and the first
            // map saw a stale view; if it still differs, the write was lost.
            size_t bad2 = 0; int rows2 = 0; uint32_t ms2 = 0;
            int again = compare_bo(bo, px, stride, W, H, f->bpp, &bad2, &rows2, &ms2);
            printf("  %-5s %4dx%-4d bo %ux%u stride %u/%u write=%d  MISMATCH"
                   " rows %d..%d (%d row(s), %zu bytes); re-read: %s\n",
                   f->code, W, H, gbm_bo_get_width(bo), gbm_bo_get_height(bo), stride, mstride,
                   wrote, bad_row, H - 1, bad_rows, bad,
                   again < 0 ? "NOW MATCHES (write was in flight)" : "still differs (write lost)");
            fails++;
        } else if (bad_row == -2) {
            printf("  %-5s %4dx%-4d map(READ) FAILED - cannot verify\n", f->code, W, H);
        } else if (loops == 1) {
            printf("  %-5s %4dx%-4d bo %ux%u stride %u/%u write=%d  byte-perfect\n",
                   f->code, W, H, gbm_bo_get_width(bo), gbm_bo_get_height(bo), stride, mstride, wrote);
        }
        free(px);
        gbm_bo_destroy(bo);
    }
    printf("%s: %d/%d buffer(s) mismatched\n",
           fails ? "SELF-TEST FAILED" : "self-test clean", fails, runs);
    return fails;
}

int main(int argc, char **argv) {
    const char *uniform = NULL;
    const char *sock = NULL, *dev = "/dev/dri/renderD128";
    int W = 256, H = 128, hold = 0, all = 0, use_grid = 0, concurrent = 0, cols = 4, gap = 4;
    int selftest_only = 0;
    const char *want[32]; int nwant = 0;
    for (int i = 1; i < argc; i++) {
        if (!strncmp(argv[i], "--uniform=", 10)) uniform = argv[i] + 10;
        else if (!strncmp(argv[i], "--socket=", 9)) sock = argv[i] + 9;
        else if (!strncmp(argv[i], "--device=", 9)) dev = argv[i] + 9;
        else if (!strncmp(argv[i], "--size=", 7)) sscanf(argv[i] + 7, "%dx%d", &W, &H);
        else if (!strncmp(argv[i], "--hold=", 7)) hold = atoi(argv[i] + 7);
        else if (!strncmp(argv[i], "--cols=", 7)) cols = atoi(argv[i] + 7);
        else if (!strncmp(argv[i], "--gap=", 6)) gap = atoi(argv[i] + 6);
        else if (!strcmp(argv[i], "--verify")) verify = 1;
        else if (!strcmp(argv[i], "--selftest")) selftest_only = 1;
        else if (!strcmp(argv[i], "--shm")) use_shm = 1;
        else if (!strcmp(argv[i], "--write=map")) write_map = 1;
        else if (!strcmp(argv[i], "--linear")) force_linear = 1;
        else if (!strncmp(argv[i], "--slack=", 8)) slack_rows = atoi(argv[i] + 8);
        else if (!strcmp(argv[i], "--rewrite")) rewrite = 1;
        else if (!strncmp(argv[i], "--settle=", 9)) settle_ms = atoi(argv[i] + 9);
        else if (!strncmp(argv[i], "--loops=", 8)) loops = atoi(argv[i] + 8);
        else if (!strncmp(argv[i], "--watch=", 8)) watch_secs = atoi(argv[i] + 8);
        else if (!strcmp(argv[i], "--pattern=check")) no_black = 1;
        else if (!strcmp(argv[i], "--blend")) blend_mode = 1;
        else if (!strcmp(argv[i], "--straight")) straight_alpha = 1;
        else if (!strcmp(argv[i], "--gpu")) gpu_render = 1;
        else if (!strncmp(argv[i], "--modifier=", 11)) {
            const char *v = argv[i] + 11;
            have_modifier = 1;
            if (!strcasecmp(v, "linear")) want_modifier = 0;
            else if (!strcasecmp(v, "invalid")) want_modifier = 0x00ffffffffffffffULL;
            else want_modifier = strtoull(v, NULL, 0);
        }
        else if (!strcmp(argv[i], "--all")) all = 1;
        else if (!strcmp(argv[i], "--all-concurrent")) { all = 1; concurrent = 1; }
        else if (!strcmp(argv[i], "--grid")) { all = 1; use_grid = 1; }
        else if (nwant < 32) want[nwant++] = argv[i];
    }
    if (selftest_only && !nwant) all = 1;
    if (watch_secs > 0 && !nwant) all = 1;
    if (!nwant && !all) {
        printf("usage: dmabuf-draw [--socket=wayland-N] [--size=WxH] [--hold=ms] [--cols=N]\n"
               "                   [--gap=N] --grid | --all-concurrent | --all | FOURCC...\n"
               "  --grid            one window, a subsurface per format (one screenshot)\n"
               "  --all-concurrent  every format at once, one toplevel each\n"
               "  --all             every format, one at a time\n"
               "  --hold=0          hold until closed (default; --hold=ms to time out)\n"
               "  --verify          read the BO back after writing; proves what landed\n"
               "  --selftest        write+readback only, NO compositor needed\n"
               "  --shm             same pattern over wl_shm (no dmabuf at all)\n"
               "  --write=map       gbm_bo_map/unmap instead of gbm_bo_write\n"
               "  --rewrite         re-write the BO at each re-damage tick\n"
               "  --settle=ms       wait+verify the BO is complete BEFORE committing\n"
               "  --watch=secs      write once, re-compare on a timer (no compositor)\n"
               "  --loops=N         repeat the self-test N times for a failure rate\n"
               "  --pattern=check   magenta/yellow checks in a white border: NO black,\n"
               "                    so any black on screen is not from this client\n"
               "  --slack=N         allocate N extra carrier rows past the declared height.\n"
               "                    Tests whether a black band is really the buffer being\n"
               "                    SHORTER than Vulkan requirements.size for that format.\n"
               "  --blend           TWO surfaces: an opaque XR24 checkerboard with a\n"
               "                    subsurface of the format under test over it, carrying an\n"
               "                    alpha staircase and NO opaque region. The only mode here\n"
               "                    that actually makes the compositor blend; an X-format is\n"
               "                    the control and must come out fully opaque.\n"
               "  --modifier=X      allocate the carrier with EXACTLY this modifier\n"
               "                    (hex, or `linear` / `invalid`). One modifier is offered,\n"
               "                    so a refusal is reported instead of being satisfied with\n"
               "                    some other layout.\n"
               "  --linear          force the carrier BO to DRM_FORMAT_MOD_LINEAR, so the\n"
               "                    only thing that changed is tiling. If black bars survive\n"
               "                    this, they are not a tiling/kind problem.\n"
               "  --uniform=FOURCC  keep the tile/window COUNT of --grid/--all/--all-concurrent\n"
               "                    but make every one of them the SAME format. Isolates\n"
               "                    format bugs from timing/concurrency bugs: pick a format\n"
               "                    you know renders clean, then anything still going black\n"
               "                    is not about the format. Tiles are labelled CODE#N so a\n"
               "                    bad one can be named.\n\n");
        for (size_t i = 0; i < sizeof FORMATS / sizeof *FORMATS; i++)
            printf("  %-5s %s\n", FORMATS[i].code, FORMATS[i].note);
        return 0;
    }

    int fd = open(dev, O_RDWR | O_CLOEXEC);
    if (fd < 0) { perror(dev); return 1; }
    struct gbm_device *gbm = gbm_create_device(fd);
    if (!gbm) { fprintf(stderr, "gbm_create_device failed\n"); return 1; }
    setvbuf(stdout, NULL, _IONBF, 0);

    const struct fmt *sel[64]; int nsel = 0;
    for (size_t i = 0; i < sizeof FORMATS / sizeof *FORMATS; i++) {
        if (!all) {
            int hit = 0;
            for (int j = 0; j < nwant; j++) if (!strcmp(want[j], FORMATS[i].code)) hit = 1;
            if (!hit) continue;
        }
        sel[nsel++] = &FORMATS[i];
    }
    // --uniform: same count, one format. Distinct labels (CODE#N) so a tile that
    // misbehaves can be identified even though every tile is the same format.
    static struct fmt uni[64];
    static char uni_label[64][16];
    if (uniform) {
        const struct fmt *base = NULL;
        for (size_t i = 0; i < sizeof FORMATS / sizeof *FORMATS; i++)
            if (!strcmp(uniform, FORMATS[i].code)) base = &FORMATS[i];
        if (!base) {
            fprintf(stderr, "--uniform=%s: unknown fourcc\n", uniform);
            return 1;
        }
        if (nsel == 0) nsel = (int)(sizeof FORMATS / sizeof *FORMATS);
        if (nsel > 64) nsel = 64;
        for (int i = 0; i < nsel; i++) {
            uni[i] = *base;
            snprintf(uni_label[i], sizeof uni_label[i], "%s#%d", base->code, i + 1);
            uni[i].code = uni_label[i];
            sel[i] = &uni[i];
        }
        printf("uniform: %d x %s (%s)\n", nsel, base->code, base->note);
    }
    printf("%d format(s), tiles %dx%d, device %s\n\n", nsel, W, H, dev);

    if (watch_secs > 0) {
        int rc = 0;
        for (int i = 0; i < nsel; i++) rc |= watch_one(gbm, sel[i], W, H);
        gbm_device_destroy(gbm); close(fd);
        return rc;
    }
    if (selftest_only) {
        int rc = selftest(gbm, sel, nsel, W, H);
        gbm_device_destroy(gbm); close(fd);
        return rc ? 1 : 0;
    }
    if (blend_mode) {
        // One format per run: the point is to compare two runs side by side (an
        // X-format against its A-sibling), not to tile them.
        const char *why;
        struct client *c = blend(gbm, sock, sel[0], W, H, &why);
        if (c) { struct client *one[1] = { c }; pump(one, 1, hold); client_close(c); }
        gbm_device_destroy(gbm); close(fd);
        return c ? 0 : 1;
    }
    if (use_grid) {
        grid(gbm, sock, sel, nsel, W, H, cols, gap, hold);
    } else if (concurrent) {
        struct client *cs[64]; int n = 0;
        for (int i = 0; i < nsel; i++) {
            const char *why;
            struct client *c = window_for(gbm, sock, sel[i], W, H, &why);
            printf("  %-5s %-42s %s\n", sel[i]->code, sel[i]->note, why);
            if (c) cs[n++] = c;
        }
        printf("\n%d window(s) up - screenshot with y5's capture (~/Pictures/y5-capture-*.png)\n", n);
        pump(cs, n, hold);
        for (int i = 0; i < n; i++) client_close(cs[i]);
    } else {
        for (int i = 0; i < nsel; i++) {
            const char *why;
            struct client *c = window_for(gbm, sock, sel[i], W, H, &why);
            printf("  %-5s %-42s %s\n", sel[i]->code, sel[i]->note, why);
            if (!c) continue;
            struct client *one[1] = { c };
            pump(one, 1, hold > 0 ? hold : 3000);
            client_close(c);
        }
    }
    gbm_device_destroy(gbm);
    close(fd);
    return 0;
}
