// sync-probe — does a VULKAN-written dmabuf survive the trip to this compositor?
//
// Emulates what Chrome does with `--enable-features=Vulkan,VulkanFromANGLE`: it
// allocates its buffers through Vulkan (not GBM/GL), renders into them with a
// Vulkan queue submit, and hands them straight to the compositor. Vulkan is an
// EXPLICIT-sync API: the driver is not obliged to attach a fence to the dma_buf's
// reservation object the way a GL driver does, so a compositor that relies on
// implicit sync has nothing to wait on and may sample the buffer before the clear
// has landed.
//
// The experiment is the `--sync` switch, and it is the whole point of this tool:
//
//   --sync=wait     block on a VkFence before committing. The content is
//                   definitely in the buffer by the time the compositor sees it.
//   --sync=nowait   commit immediately after vkQueueSubmit, with no fence wait
//                   and no explicit-sync protocol. This is the condition under
//                   test.
//
// Run both and compare what is on screen. `wait` correct + `nowait` empty means
// the compositor is sampling unsynchronised, and explicit sync
// (linux-drm-syncobj-v1) is the fix. Both correct means sync is not the problem
// here and the empty window has another cause.
//
// It also paces like a well-behaved client: the next frame is only committed once
// the previous one has been reported PRESENTED (or the frame callback fires when
// wp_presentation is absent), so a compositor that never presents visibly stalls
// the client rather than letting it free-run.
//
// Each frame clears to a different, fully opaque, easily-named colour, so "wrong
// content" and "no content" are distinguishable by eye.

#define _GNU_SOURCE
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include <vulkan/vulkan.h>
#include <wayland-client.h>

#include "linux-dmabuf-v1-client-protocol.h"
#include "presentation-time-client-protocol.h"
#include "viewporter-client-protocol.h"
#include "xdg-shell-client-protocol.h"

#define DRM_FORMAT_ARGB8888 0x34325241  // 'AR24'
#define DRM_FORMAT_ABGR8888 0x34324241  // 'AB24'
#define MAX_MODS 256

// --format=argb|abgr. The two differ ONLY in channel order, so a window that renders
// under one and is black under the other isolates the fourcc as the variable with the
// modifier, size, sync mode and everything else held fixed.
//
// Chrome commits AB24 on this stack; every other client on the machine — the iced and
// bevy surfaces, this probe's own default — commits AR24, and those all render. That
// is the only field in which Chrome differs, so it is the one worth testing directly
// rather than reasoning about.
static uint32_t g_drm_fourcc = DRM_FORMAT_ARGB8888;
static VkFormat g_vk_format = VK_FORMAT_B8G8R8A8_UNORM;
static const char *g_fmt_name = "ARGB8888";

// ------------------------------------------------------------------ wayland

static struct wl_display *d_display;
static struct wl_compositor *d_compositor;
static struct xdg_wm_base *d_wm_base;
static struct zwp_linux_dmabuf_v1 *d_dmabuf;
static struct wp_presentation *d_presentation;
static struct wl_surface *d_surface;
static struct xdg_surface *d_xdg_surface;
static struct xdg_toplevel *d_toplevel;
static int d_has_syncobj;          // wp_linux_drm_syncobj_manager_v1 advertised?
static int d_configured, d_running = 1;

// Modifiers the compositor advertises for ARGB8888 (dmabuf v3 `modifier` events).
static uint64_t d_mods[MAX_MODS];
static int d_mod_count;

static int g_width = 512, g_height = 384;
static int g_frames = 8;
static int g_wait_fence = 1;   // --sync=wait (default) / --sync=nowait
static int g_verbose;
static double g_hold = 4.0;   // --hold=N: seconds to leave the final frame on screen
// Set when a submit fails (in practice VK_ERROR_DEVICE_LOST). Once it is set this
// probe's on-screen pixels prove nothing, so the summary has to disclaim them.
static int g_gpu_lost;
// Chrome-emulation switches. Each isolates ONE way this tool differed from chrome
// in the captured traces, so a matrix run says which difference matters.
static uint64_t g_force_mod = 0;   // --modifier=0x...: use exactly this modifier
static int g_viewport;             // --viewport: wp_viewport src=whole buffer, dst=buffer/1.25
static int g_legacy_damage;        // --legacy-damage: wl_surface.damage, not damage_buffer
// A buffer with an ALPHA channel whose alpha is 0 is fully transparent if the
// compositor blends it, and fully opaque if the compositor honours the surface's
// declared opaque region. Chrome commits ARGB8888 AND declares its surface opaque,
// so these two switches together isolate which of those the compositor does.
static double g_alpha = 1.0;       // --alpha=N: the clear's alpha
static int g_opaque;               // --opaque: declare the whole surface opaque
// GPU LOAD. A single clear finishes in microseconds, so a compositor sampling ~16ms
// later wins that race whether or not any fence exists — a trivial workload cannot
// test synchronisation at all. This piles up work BEFORE the final colour so the
// submit takes long enough to still be running when the compositor samples: an
// early sample sees the black under-paint, a correctly synchronised one sees the
// colour. Chrome submits a whole frame of real rendering, which is why it can lose
// a race this tool otherwise always wins.
static int g_load;                 // --load=N: N black clears before the real one
static const char *g_label = "";   // --label=S: prefix the window title, for reporting
static int g_number;               // --number=N: draw N huge and centred in the window
// --black: every frame is RGB 0,0,0 instead of the colour cycle.
//
// NOTE: this only means anything at alpha 1. Combined with --alpha=0 the two
// cancel — black at alpha 0 is not black, it is nothing — so a "black" variant
// that also passes --alpha=0 is not the control it looks like. Pair --black with
// --alpha=1 to ask "is this window black because we painted it black, or because
// nothing of ours arrived", and with --opaque --alpha=0 to ask the different
// question "does the compositor honour the declared opaque region", where the
// correct answer is an OPAQUE BLACK window.
static int g_black;
// Target GPU time per frame, in milliseconds. Guessing a clear COUNT is useless
// across GPUs — 4000 clears is an age on one card and nothing on a 4090 — and the
// count only matters insofar as the submit outlasts several compositor frames
// (~16.7ms each). So state the duration and let the tool calibrate the count.
static double g_load_ms;           // --load-ms=N
static struct wp_viewporter *d_viewporter;
static struct wp_viewport *d_viewport;

static unsigned g_committed, g_presented, g_discarded, g_frame_cb;

// ------------------------------------------------------------------- vulkan

static VkInstance vk_instance;
static VkPhysicalDevice vk_phys;
static VkDevice vk_dev;
static VkQueue vk_queue;
static uint32_t vk_qfam;
static VkCommandPool vk_pool;
// Ballast: a large off-screen image the load clears hammer instead of the presented
// one. GPU cost scales with AREA, so a 4096x4096 clear is ~55x a 640x480 clear —
// which means a one-second stall costs thousands of commands to record, not
// hundreds of thousands. Recording cost was the real limit on how long we could
// stall, and it is CPU time, which is not what we want to be measuring.
static VkImage vk_ballast;
static VkDeviceMemory vk_ballast_mem;
static int vk_ballast_dim = 4096;

static PFN_vkGetMemoryFdKHR p_vkGetMemoryFdKHR;
static PFN_vkGetImageDrmFormatModifierPropertiesEXT p_vkGetImageDrmFormatModifierPropertiesEXT;

struct frame_buf {
    VkImage image;
    VkDeviceMemory memory;
    VkCommandBuffer cmd;
    VkFence fence;
    int fd;
    uint64_t modifier;
    uint32_t stride, offset;
    struct wl_buffer *wl_buffer;
    int busy;
    // Host-visible staging for the centred number. The image is device-local and
    // DRM-modifier tiled, so it cannot be written from the CPU directly; a copy
    // lets Vulkan deal with the tiling.
    VkBuffer stage;
    VkDeviceMemory stage_mem;
    uint32_t *stage_px;
};
static struct frame_buf g_bufs[2];

#define VKCHECK(expr, what)                                                     \
    do {                                                                        \
        VkResult _r = (expr);                                                   \
        if (_r != VK_SUCCESS) {                                                 \
            fprintf(stderr, "sync-probe: %s failed (VkResult %d)\n", what, _r); \
            return -1;                                                          \
        }                                                                       \
    } while (0)

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + ts.tv_nsec;
}

// Which of the compositor's advertised modifiers can Vulkan actually render into?
// Asking this rather than assuming is the same discipline the compositor's own
// bridge uses: a modifier both ends merely *know about* is not necessarily one
// both ends can use.
static int pick_modifier(uint64_t *out) {
    VkDrmFormatModifierPropertiesListEXT list = {
        .sType = VK_STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT,
    };
    VkFormatProperties2 fp = { .sType = VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2, .pNext = &list };
    vkGetPhysicalDeviceFormatProperties2(vk_phys, g_vk_format, &fp);
    if (!list.drmFormatModifierCount) return -1;

    VkDrmFormatModifierPropertiesEXT *props =
        calloc(list.drmFormatModifierCount, sizeof(*props));
    list.pDrmFormatModifierProperties = props;
    vkGetPhysicalDeviceFormatProperties2(vk_phys, g_vk_format, &fp);

    // A forced modifier is still CHECKED, not trusted: if Vulkan cannot colour-attach
    // it the run would fail later and more confusingly.
    if (g_force_mod) {
        for (uint32_t j = 0; j < list.drmFormatModifierCount; j++) {
            if (props[j].drmFormatModifier != g_force_mod) continue;
            if (!(props[j].drmFormatModifierTilingFeatures &
                  VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT))
                continue;
            printf("modifier forced: 0x%016llx (%u plane(s))\n",
                   (unsigned long long)g_force_mod, props[j].drmFormatModifierPlaneCount);
            *out = g_force_mod;
            free(props);
            return 0;
        }
        fprintf(stderr, "sync-probe: forced modifier 0x%016llx is not colour-attachable here\n",
                (unsigned long long)g_force_mod);
        free(props);
        return -1;
    }

    // Prefer the compositor's order: it advertises best-first.
    for (int i = 0; i < d_mod_count; i++) {
        for (uint32_t j = 0; j < list.drmFormatModifierCount; j++) {
            if (props[j].drmFormatModifier != d_mods[i]) continue;
            if (props[j].drmFormatModifierPlaneCount != 1) continue;  // keep this tool single-plane
            if (!(props[j].drmFormatModifierTilingFeatures &
                  VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT))
                continue;
            *out = d_mods[i];
            free(props);
            return 0;
        }
    }
    free(props);
    return -1;
}

static int vk_setup(void) {
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                              .pApplicationName = "sync-probe",
                              .apiVersion = VK_API_VERSION_1_2 };
    VkInstanceCreateInfo ici = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                 .pApplicationInfo = &app };
    VKCHECK(vkCreateInstance(&ici, NULL, &vk_instance), "vkCreateInstance");

    uint32_t n = 0;
    vkEnumeratePhysicalDevices(vk_instance, &n, NULL);
    if (!n) { fprintf(stderr, "sync-probe: no Vulkan physical device\n"); return -1; }
    VkPhysicalDevice *pds = calloc(n, sizeof(*pds));
    vkEnumeratePhysicalDevices(vk_instance, &n, pds);
    vk_phys = pds[0];
    free(pds);

    VkPhysicalDeviceProperties props;
    vkGetPhysicalDeviceProperties(vk_phys, &props);
    printf("vulkan device: %s\n", props.deviceName);

    uint32_t qn = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(vk_phys, &qn, NULL);
    VkQueueFamilyProperties *qs = calloc(qn, sizeof(*qs));
    vkGetPhysicalDeviceQueueFamilyProperties(vk_phys, &qn, qs);
    vk_qfam = 0;
    for (uint32_t i = 0; i < qn; i++)
        if (qs[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) { vk_qfam = i; break; }
    free(qs);

    const char *exts[] = {
        VK_KHR_EXTERNAL_MEMORY_FD_EXTENSION_NAME,
        VK_EXT_EXTERNAL_MEMORY_DMA_BUF_EXTENSION_NAME,
        VK_EXT_IMAGE_DRM_FORMAT_MODIFIER_EXTENSION_NAME,
        VK_KHR_IMAGE_FORMAT_LIST_EXTENSION_NAME,
    };
    float prio = 1.0f;
    VkDeviceQueueCreateInfo qci = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                    .queueFamilyIndex = vk_qfam,
                                    .queueCount = 1,
                                    .pQueuePriorities = &prio };
    VkDeviceCreateInfo dci = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                               .queueCreateInfoCount = 1,
                               .pQueueCreateInfos = &qci,
                               .enabledExtensionCount = 4,
                               .ppEnabledExtensionNames = exts };
    VKCHECK(vkCreateDevice(vk_phys, &dci, NULL, &vk_dev), "vkCreateDevice");
    vkGetDeviceQueue(vk_dev, vk_qfam, 0, &vk_queue);

    p_vkGetMemoryFdKHR = (PFN_vkGetMemoryFdKHR)vkGetDeviceProcAddr(vk_dev, "vkGetMemoryFdKHR");
    p_vkGetImageDrmFormatModifierPropertiesEXT =
        (PFN_vkGetImageDrmFormatModifierPropertiesEXT)vkGetDeviceProcAddr(
            vk_dev, "vkGetImageDrmFormatModifierPropertiesEXT");
    if (!p_vkGetMemoryFdKHR || !p_vkGetImageDrmFormatModifierPropertiesEXT) {
        fprintf(stderr, "sync-probe: required Vulkan entry points missing\n");
        return -1;
    }

    VkCommandPoolCreateInfo pci = { .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
                                    .flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
                                    .queueFamilyIndex = vk_qfam };
    VKCHECK(vkCreateCommandPool(vk_dev, &pci, NULL, &vk_pool), "vkCreateCommandPool");

    VkImageCreateInfo bci = { .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                              .imageType = VK_IMAGE_TYPE_2D,
                              .format = g_vk_format,
                              .extent = { vk_ballast_dim, vk_ballast_dim, 1 },
                              .mipLevels = 1,
                              .arrayLayers = 1,
                              .samples = VK_SAMPLE_COUNT_1_BIT,
                              .tiling = VK_IMAGE_TILING_OPTIMAL,
                              .usage = VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                              .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
                              .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED };
    VKCHECK(vkCreateImage(vk_dev, &bci, NULL, &vk_ballast), "vkCreateImage(ballast)");
    VkMemoryRequirements breq;
    vkGetImageMemoryRequirements(vk_dev, vk_ballast, &breq);
    VkPhysicalDeviceMemoryProperties bmp;
    vkGetPhysicalDeviceMemoryProperties(vk_phys, &bmp);
    uint32_t bt = UINT32_MAX;
    for (uint32_t i = 0; i < bmp.memoryTypeCount; i++)
        if ((breq.memoryTypeBits & (1u << i)) &&
            (bmp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)) { bt = i; break; }
    if (bt == UINT32_MAX) { fprintf(stderr, "sync-probe: no memory for ballast\n"); return -1; }
    VkMemoryAllocateInfo bmai = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                                  .allocationSize = breq.size, .memoryTypeIndex = bt };
    VKCHECK(vkAllocateMemory(vk_dev, &bmai, NULL, &vk_ballast_mem), "vkAllocateMemory(ballast)");
    VKCHECK(vkBindImageMemory(vk_dev, vk_ballast, vk_ballast_mem, 0), "vkBindImageMemory(ballast)");
    return 0;
}

static int make_buffer(struct frame_buf *b, uint64_t modifier) {
    VkExternalMemoryImageCreateInfo emi = {
        .sType = VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO,
        .handleTypes = VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    VkImageDrmFormatModifierListCreateInfoEXT mod_list = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_LIST_CREATE_INFO_EXT,
        .pNext = &emi,
        .drmFormatModifierCount = 1,
        .pDrmFormatModifiers = &modifier,
    };
    VkImageCreateInfo ici = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        .pNext = &mod_list,
        .imageType = VK_IMAGE_TYPE_2D,
        .format = g_vk_format,
        .extent = { g_width, g_height, 1 },
        .mipLevels = 1,
        .arrayLayers = 1,
        .samples = VK_SAMPLE_COUNT_1_BIT,
        .tiling = VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
        .usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
        .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED,
    };
    VKCHECK(vkCreateImage(vk_dev, &ici, NULL, &b->image), "vkCreateImage");

    VkMemoryRequirements req;
    vkGetImageMemoryRequirements(vk_dev, b->image, &req);
    VkPhysicalDeviceMemoryProperties mp;
    vkGetPhysicalDeviceMemoryProperties(vk_phys, &mp);
    uint32_t type = UINT32_MAX;
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
        if ((req.memoryTypeBits & (1u << i)) &&
            (mp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)) {
            type = i;
            break;
        }
    if (type == UINT32_MAX) { fprintf(stderr, "sync-probe: no device-local memory type\n"); return -1; }

    VkExportMemoryAllocateInfo exp = {
        .sType = VK_STRUCTURE_TYPE_EXPORT_MEMORY_ALLOCATE_INFO,
        .handleTypes = VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    VkMemoryDedicatedAllocateInfo ded = {
        .sType = VK_STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO,
        .pNext = &exp,
        .image = b->image,
    };
    VkMemoryAllocateInfo mai = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                                 .pNext = &ded,
                                 .allocationSize = req.size,
                                 .memoryTypeIndex = type };
    VKCHECK(vkAllocateMemory(vk_dev, &mai, NULL, &b->memory), "vkAllocateMemory");
    VKCHECK(vkBindImageMemory(vk_dev, b->image, b->memory, 0), "vkBindImageMemory");

    VkMemoryGetFdInfoKHR gfi = { .sType = VK_STRUCTURE_TYPE_MEMORY_GET_FD_INFO_KHR,
                                 .memory = b->memory,
                                 .handleType = VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT };
    VKCHECK(p_vkGetMemoryFdKHR(vk_dev, &gfi, &b->fd), "vkGetMemoryFdKHR");

    VkImageDrmFormatModifierPropertiesEXT dmp = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_PROPERTIES_EXT,
    };
    VKCHECK(p_vkGetImageDrmFormatModifierPropertiesEXT(vk_dev, b->image, &dmp),
            "vkGetImageDrmFormatModifierPropertiesEXT");
    b->modifier = dmp.drmFormatModifier;

    VkImageSubresource sub = { .aspectMask = VK_IMAGE_ASPECT_MEMORY_PLANE_0_BIT_EXT };
    VkSubresourceLayout lay;
    vkGetImageSubresourceLayout(vk_dev, b->image, &sub, &lay);
    b->stride = (uint32_t)lay.rowPitch;
    b->offset = (uint32_t)lay.offset;

    VkBufferCreateInfo bci = { .sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
                               .size = (VkDeviceSize)g_width * g_height * 4,
                               .usage = VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
                               .sharingMode = VK_SHARING_MODE_EXCLUSIVE };
    VKCHECK(vkCreateBuffer(vk_dev, &bci, NULL, &b->stage), "vkCreateBuffer");
    VkMemoryRequirements breq;
    vkGetBufferMemoryRequirements(vk_dev, b->stage, &breq);
    uint32_t htype = UINT32_MAX;
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
        if ((breq.memoryTypeBits & (1u << i)) &&
            (mp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT) &&
            (mp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_HOST_COHERENT_BIT)) {
            htype = i;
            break;
        }
    if (htype == UINT32_MAX) { fprintf(stderr, "sync-probe: no host-visible memory\n"); return -1; }
    VkMemoryAllocateInfo bmai = { .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                                  .allocationSize = breq.size,
                                  .memoryTypeIndex = htype };
    VKCHECK(vkAllocateMemory(vk_dev, &bmai, NULL, &b->stage_mem), "vkAllocateMemory(stage)");
    VKCHECK(vkBindBufferMemory(vk_dev, b->stage, b->stage_mem, 0), "vkBindBufferMemory");
    VKCHECK(vkMapMemory(vk_dev, b->stage_mem, 0, VK_WHOLE_SIZE, 0, (void **)&b->stage_px),
            "vkMapMemory");

    VkCommandBufferAllocateInfo cai = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                                        .commandPool = vk_pool,
                                        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
                                        .commandBufferCount = 1 };
    VKCHECK(vkAllocateCommandBuffers(vk_dev, &cai, &b->cmd), "vkAllocateCommandBuffers");
    VkFenceCreateInfo fci = { .sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO };
    VKCHECK(vkCreateFence(vk_dev, &fci, NULL, &b->fence), "vkCreateFence");
    return 0;
}

// Clear the image and release it to "foreign" ownership, which is the handover a
// dmabuf consumer outside Vulkan requires.
// Seven-segment digits, drawn as plain rectangles — no font, no dependencies, and
// legible at any size. Segment order: a(top) b(tr) c(br) d(bottom) e(bl) f(tl) g(mid).
static const unsigned char SEG[10] = {
    /*0*/ 0x3f, /*1*/ 0x06, /*2*/ 0x5b, /*3*/ 0x4f, /*4*/ 0x66,
    /*5*/ 0x6d, /*6*/ 0x7d, /*7*/ 0x07, /*8*/ 0x7f, /*9*/ 0x6f,
};

static void fill_rect(uint32_t *px, int W, int H, int x, int y, int w, int h, uint32_t c) {
    for (int j = y; j < y + h; j++) {
        if (j < 0 || j >= H) continue;
        for (int i = x; i < x + w; i++) {
            if (i < 0 || i >= W) continue;
            px[(size_t)j * W + i] = c;
        }
    }
}

static void draw_digit(uint32_t *px, int W, int H, int d, int x, int y, int dw, int dh,
                       int t, uint32_t c) {
    unsigned char m = SEG[d % 10];
    if (m & 0x01) fill_rect(px, W, H, x, y, dw, t, c);                        // a
    if (m & 0x02) fill_rect(px, W, H, x + dw - t, y, t, dh / 2, c);           // b
    if (m & 0x04) fill_rect(px, W, H, x + dw - t, y + dh / 2, t, dh / 2, c);  // c
    if (m & 0x08) fill_rect(px, W, H, x, y + dh - t, dw, t, c);               // d
    if (m & 0x10) fill_rect(px, W, H, x, y + dh / 2, t, dh / 2, c);           // e
    if (m & 0x20) fill_rect(px, W, H, x, y, t, dh / 2, c);                    // f
    if (m & 0x40) fill_rect(px, W, H, x, y + dh / 2 - t / 2, dw, t, c);       // g
}

// Paint the frame colour plus the window's number, centred, into the staging buffer.
// Channel order follows the chosen format, so --format=abgr paints the SAME colours
// rather than a red/blue swap that would be mistaken for a compositor bug: AR24 is
// VK_FORMAT_B8G8R8A8_UNORM (bytes B,G,R,A), AB24 is VK_FORMAT_R8G8B8A8_UNORM (bytes
// R,G,B,A), so only the red and blue lanes trade places in the packed word.
static void paint(struct frame_buf *b, float r, float g, float bl, float a) {
    int abgr = g_drm_fourcc == DRM_FORMAT_ABGR8888;
    uint32_t lo = (uint32_t)((abgr ? r : bl) * 255);
    uint32_t hi = (uint32_t)((abgr ? bl : r) * 255);
    uint32_t bg = ((uint32_t)(a * 255) << 24) | (hi << 16) |
                  ((uint32_t)(g * 255) << 8) | lo;
    int W = g_width, H = g_height;
    for (size_t i = 0; i < (size_t)W * H; i++) b->stage_px[i] = bg;
    if (g_number <= 0) return;

    int dh = H / 2, dw = dh / 2, t = dh / 8;
    int digits = g_number >= 10 ? 2 : 1;
    int gap = dw / 4;
    int total = digits * dw + (digits - 1) * gap;
    int x0 = (W - total) / 2, y0 = (H - dh) / 2;

    // A dark plate behind the glyphs, so the number reads on any frame colour —
    // including the alpha-0 variants, where the colour itself may be invisible.
    int pad = t * 2;
    fill_rect(b->stage_px, W, H, x0 - pad, y0 - pad, total + 2 * pad, dh + 2 * pad,
              ((uint32_t)(a * 255) << 24));
    uint32_t fg = ((uint32_t)(a * 255) << 24) | 0x00ffffff;
    if (digits == 2) {
        draw_digit(b->stage_px, W, H, g_number / 10, x0, y0, dw, dh, t, fg);
        draw_digit(b->stage_px, W, H, g_number % 10, x0 + dw + gap, y0, dw, dh, t, fg);
    } else {
        draw_digit(b->stage_px, W, H, g_number, x0, y0, dw, dh, t, fg);
    }
}

static int vk_clear_inner(struct frame_buf *b, float r, float g, float bl);

// Time one submit of `count` clears, so the load can be scaled to a real duration.
static double time_clears(struct frame_buf *b, int count) {
    int saved_load = g_load, saved_wait = g_wait_fence;
    g_load = count;
    g_wait_fence = 1;
    uint64_t t0 = now_ns();
    int rc = vk_clear_inner(b, 0, 0, 0);
    double ms = (now_ns() - t0) / 1e6;
    g_load = saved_load;
    g_wait_fence = saved_wait;
    return rc < 0 ? -1.0 : ms;
}

// Choose the clear count that makes a submit take about `g_load_ms`.
static int calibrate_load(struct frame_buf *b) {
    if (g_load_ms <= 0) return 0;
    // Iterate rather than extrapolate once. The first submit carries warm-up costs
    // (allocation, first-touch of the image), so a single measurement over-predicts
    // the per-clear cost badly — the initial guess here came out 4x too slow. Each
    // round rescales from what was actually measured and re-measures, which
    // converges in two or three rounds on any GPU.
    int count = 64;
    double ms = 0;
    for (int round = 0; round < 6; round++) {
        ms = time_clears(b, count);
        if (ms < 0) { fprintf(stderr, "sync-probe: load calibration failed\n"); return -1; }
        if (ms >= g_load_ms * 0.85 && ms <= g_load_ms * 1.5) break;
        double per = ms / count;
        if (per <= 0) per = 1e-6;
        double next = g_load_ms / per;
        if (next > 4e6) next = 4e6;      // don't build an unbounded command buffer
        if (next < 1) next = 1;
        count = (int)next;
    }
    g_load = count;
    printf("gpu load:   %d clears, measured %.1f ms/frame (%.1f frames at 60Hz)\n",
           g_load, ms, ms / 16.67);
    return 0;
}

static int vk_clear_inner(struct frame_buf *b, float r, float g, float bl) {
    const float a = (float)g_alpha;
    // PREMULTIPLY. wayland requires ARGB8888 to carry premultiplied alpha, and the
    // compositor blends src=ONE, dst=ONE_MINUS_SRC_ALPHA accordingly. Writing a full
    // -strength colour with alpha 0 is not "transparent", it is the invalid state
    // colour>alpha, and that blend reads it as "add this colour, occlude nothing" —
    // an additive wash. The alpha-0 window then looks SOLID, which is exactly the
    // false alarm this produced: it looked like the compositor forcing opacity when
    // it was the probe emitting garbage.
    r *= a;
    g *= a;
    bl *= a;
    vkResetCommandBuffer(b->cmd, 0);
    VkCommandBufferBeginInfo bi = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                    .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT };
    VKCHECK(vkBeginCommandBuffer(b->cmd, &bi), "vkBeginCommandBuffer");

    VkImageSubresourceRange range = { .aspectMask = VK_IMAGE_ASPECT_COLOR_BIT,
                                      .levelCount = 1, .layerCount = 1 };
    VkImageMemoryBarrier to_dst = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
        .newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        .srcQueueFamilyIndex = VK_QUEUE_FAMILY_FOREIGN_EXT,
        .dstQueueFamilyIndex = vk_qfam,
        .image = b->image,
        .subresourceRange = range,
        .dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT,
    };
    vkCmdPipelineBarrier(b->cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                         VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &to_dst);

    // Burn GPU time on the BALLAST, so the stall is long without the presented image
    // being repeatedly overwritten and without a vast command buffer to record.
    VkClearColorValue black = { .float32 = { 0.0f, 0.0f, 0.0f, a } };
    if (g_load > 0) {
        VkImageMemoryBarrier bb = { .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                                    .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
                                    .newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                                    .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                    .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                    .image = vk_ballast,
                                    .subresourceRange = range,
                                    .dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT };
        vkCmdPipelineBarrier(b->cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                             VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &bb);
    }
    for (int i = 0; i < g_load; i++) {
        vkCmdClearColorImage(b->cmd, vk_ballast, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &black, 1,
                             &range);
        // Force each clear to actually happen rather than be coalesced away.
        VkMemoryBarrier mb = { .sType = VK_STRUCTURE_TYPE_MEMORY_BARRIER,
                               .srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT,
                               .dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT };
        vkCmdPipelineBarrier(b->cmd, VK_PIPELINE_STAGE_TRANSFER_BIT,
                             VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 1, &mb, 0, NULL, 0, NULL);
    }

    VkClearColorValue color = { .float32 = { r, g, bl, a } };
    vkCmdClearColorImage(b->cmd, b->image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &color, 1, &range);

    // The number goes on LAST, so what is on screen is unambiguous.
    paint(b, r, g, bl, a);
    VkBufferMemoryBarrier host_done = { .sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER,
                                        .srcAccessMask = VK_ACCESS_HOST_WRITE_BIT,
                                        .dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT,
                                        .buffer = b->stage,
                                        .size = VK_WHOLE_SIZE };
    vkCmdPipelineBarrier(b->cmd, VK_PIPELINE_STAGE_HOST_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0,
                         NULL, 1, &host_done, 0, NULL);
    VkBufferImageCopy region = {
        .imageSubresource = { .aspectMask = VK_IMAGE_ASPECT_COLOR_BIT, .layerCount = 1 },
        .imageExtent = { g_width, g_height, 1 },
    };
    vkCmdCopyBufferToImage(b->cmd, b->stage, b->image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1,
                           &region);

    VkImageMemoryBarrier to_foreign = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        .oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        .newLayout = VK_IMAGE_LAYOUT_GENERAL,
        .srcQueueFamilyIndex = vk_qfam,
        .dstQueueFamilyIndex = VK_QUEUE_FAMILY_FOREIGN_EXT,
        .image = b->image,
        .subresourceRange = range,
        .srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT,
    };
    vkCmdPipelineBarrier(b->cmd, VK_PIPELINE_STAGE_TRANSFER_BIT,
                         VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, 0, 0, NULL, 0, NULL, 1,
                         &to_foreign);
    VKCHECK(vkEndCommandBuffer(b->cmd), "vkEndCommandBuffer");

    vkResetFences(vk_dev, 1, &b->fence);
    VkSubmitInfo si = { .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                        .commandBufferCount = 1,
                        .pCommandBuffers = &b->cmd };
    VKCHECK(vkQueueSubmit(vk_queue, 1, &si, b->fence), "vkQueueSubmit");

    // THE experiment. Waiting guarantees the clear has landed before the
    // compositor can sample; not waiting leaves it to implicit sync, which
    // Vulkan does not promise.
    if (g_wait_fence)
        VKCHECK(vkWaitForFences(vk_dev, 1, &b->fence, VK_TRUE, UINT64_MAX), "vkWaitForFences");
    return 0;
}

// ------------------------------------------------------------- wl callbacks

static void buffer_release(void *data, struct wl_buffer *b) {
    (void)b;
    ((struct frame_buf *)data)->busy = 0;
}
static const struct wl_buffer_listener buffer_listener = { buffer_release };

static void dmabuf_format(void *d, struct zwp_linux_dmabuf_v1 *z, uint32_t fmt) {
    (void)d; (void)z; (void)fmt;
}
static void dmabuf_modifier(void *d, struct zwp_linux_dmabuf_v1 *z, uint32_t fmt,
                            uint32_t hi, uint32_t lo) {
    (void)d; (void)z;
    if (fmt != g_drm_fourcc || d_mod_count >= MAX_MODS) return;
    d_mods[d_mod_count++] = ((uint64_t)hi << 32) | lo;
}
static const struct zwp_linux_dmabuf_v1_listener dmabuf_listener = { dmabuf_format, dmabuf_modifier };

static void commit_next(void);

static void fb_sync_output(void *d, struct wp_presentation_feedback *f, struct wl_output *o) {
    (void)d; (void)f; (void)o;
}
static void fb_presented(void *data, struct wp_presentation_feedback *fb, uint32_t sh,
                         uint32_t sl, uint32_t ns, uint32_t refresh, uint32_t qh,
                         uint32_t ql, uint32_t flags) {
    (void)data; (void)sh; (void)sl; (void)ns; (void)refresh; (void)qh; (void)ql;
    g_presented++;
    if (g_verbose) printf("  frame %u presented (flags 0x%x)\n", g_presented, flags);
    wp_presentation_feedback_destroy(fb);
    commit_next();  // pace on presentation, like a well-behaved client
}
static void fb_discarded(void *data, struct wp_presentation_feedback *fb) {
    (void)data;
    g_discarded++;
    if (g_verbose) printf("  frame discarded (#%u)\n", g_discarded);
    wp_presentation_feedback_destroy(fb);
    commit_next();  // otherwise a never-presenting compositor stalls us forever
}
static const struct wp_presentation_feedback_listener fb_listener = {
    fb_sync_output, fb_presented, fb_discarded,
};

static void frame_done(void *data, struct wl_callback *cb, uint32_t t) {
    (void)data; (void)t;
    wl_callback_destroy(cb);
    g_frame_cb++;
    if (!d_presentation) commit_next();  // no feedback to pace on
}
static const struct wl_callback_listener frame_listener = { frame_done };

static const struct { float r, g, b; const char *name; } COLOURS[] = {
    { 1, 0, 0, "red" },   { 0, 1, 0, "green" }, { 0, 0, 1, "blue" },
    { 1, 1, 0, "yellow" },{ 1, 0, 1, "magenta" },{ 0, 1, 1, "cyan" },
};

static void commit_next(void) {
    if (!d_configured || !d_running) return;
    if (g_committed >= (unsigned)g_frames) { d_running = 0; return; }

    struct frame_buf *b = NULL;
    for (int i = 0; i < 2; i++)
        if (!g_bufs[i].busy) { b = &g_bufs[i]; break; }
    if (!b) return;  // both held; a release will re-arm us

    int c = g_committed % (int)(sizeof(COLOURS) / sizeof(COLOURS[0]));
    float col_r = g_black ? 0.0f : COLOURS[c].r;
    float col_g = g_black ? 0.0f : COLOURS[c].g;
    float col_b = g_black ? 0.0f : COLOURS[c].b;
    printf("  commit %u: clear to %s (%s)\n", g_committed + 1,
           g_black ? "black" : COLOURS[c].name,
           g_wait_fence ? "fence waited" : "NO fence wait");
    // A failure here is almost always VK_ERROR_DEVICE_LOST (-4): the calibrated load
    // is heavy enough that, with the whole matrix submitting at once, the driver's
    // channel watchdog resets the GPU. Say so LOUDLY — after a reset this process's
    // buffer contents are undefined, so whatever the window shows from here on is a
    // statement about the probe, not about the compositor.
    if (vk_clear_inner(b, col_r, col_g, col_b) < 0) {
        g_gpu_lost = 1;
        printf("  !! GPU LOST on commit %u — pixels from here on are UNDEFINED\n",
               g_committed + 1);
        d_running = 0;
        return;
    }

    struct wl_callback *cb = wl_surface_frame(d_surface);
    wl_callback_add_listener(cb, &frame_listener, NULL);
    if (d_presentation) {
        struct wp_presentation_feedback *fb = wp_presentation_feedback(d_presentation, d_surface);
        wp_presentation_feedback_add_listener(fb, &fb_listener, NULL);
    }

    b->busy = 1;
    wl_surface_attach(d_surface, b->wl_buffer, 0, 0);
    if (g_legacy_damage) {
        // Surface coordinates, the deprecated form — which is what chrome sends.
        wl_surface_damage(d_surface, 0, 0, g_width, g_height);
    } else {
        wl_surface_damage_buffer(d_surface, 0, 0, g_width, g_height);
    }
    wl_surface_commit(d_surface);
    g_committed++;
}

static void xdg_surface_configure(void *data, struct xdg_surface *xs, uint32_t serial) {
    (void)data;
    xdg_surface_ack_configure(xs, serial);
    if (!d_configured) { d_configured = 1; commit_next(); }
}
static const struct xdg_surface_listener xdg_surface_listener = { xdg_surface_configure };
static void toplevel_configure(void *d, struct xdg_toplevel *t, int32_t w, int32_t h,
                               struct wl_array *s) {
    (void)d; (void)t; (void)w; (void)h; (void)s;
}
static void toplevel_close(void *d, struct xdg_toplevel *t) { (void)d; (void)t; d_running = 0; }
static const struct xdg_toplevel_listener toplevel_listener = { toplevel_configure, toplevel_close };
static void wm_base_ping(void *d, struct xdg_wm_base *b, uint32_t s) {
    (void)d;
    xdg_wm_base_pong(b, s);
}
static const struct xdg_wm_base_listener wm_base_listener = { wm_base_ping };

static void registry_global(void *data, struct wl_registry *reg, uint32_t name,
                            const char *iface, uint32_t version) {
    (void)data;
    if (!strcmp(iface, wl_compositor_interface.name))
        d_compositor = wl_registry_bind(reg, name, &wl_compositor_interface, version < 4 ? version : 4);
    else if (!strcmp(iface, xdg_wm_base_interface.name))
        d_wm_base = wl_registry_bind(reg, name, &xdg_wm_base_interface, 1);
    else if (!strcmp(iface, zwp_linux_dmabuf_v1_interface.name))
        d_dmabuf = wl_registry_bind(reg, name, &zwp_linux_dmabuf_v1_interface, version < 3 ? version : 3);
    else if (!strcmp(iface, wp_presentation_interface.name))
        d_presentation = wl_registry_bind(reg, name, &wp_presentation_interface, 1);
    else if (!strcmp(iface, wp_viewporter_interface.name))
        d_viewporter = wl_registry_bind(reg, name, &wp_viewporter_interface, 1);
    else if (!strcmp(iface, "wp_linux_drm_syncobj_manager_v1"))
        d_has_syncobj = 1;
}
static void registry_global_remove(void *d, struct wl_registry *r, uint32_t n) {
    (void)d; (void)r; (void)n;
}
static const struct wl_registry_listener registry_listener = { registry_global, registry_global_remove };

// -------------------------------------------------------------------- main

static void usage(const char *me) {
    fprintf(stderr,
            "usage: %s [--socket=NAME] [--sync=wait|nowait] [--frames=N] [--size=WxH] [-v]\n"
            "  --sync=wait    block on a VkFence before committing (default)\n"
            "  --sync=nowait  commit straight after vkQueueSubmit — the condition under test\n"
            "  --format=argb|abgr  buffer fourcc: AR24 (default) or AB24, the one chrome uses\n"
            "  --hold=N       seconds to leave the final frame on screen (default 4)\n"
            "  --modifier=0x..  force one DRM modifier instead of picking the first usable\n"
            "  --viewport       add a wp_viewport like chrome's (src=buffer, dst=buffer/1.25)\n"
            "  --legacy-damage  use wl_surface.damage instead of damage_buffer\n"
            "  --alpha=N        clear alpha, 0..1 (default 1). 0 = fully transparent if blended\n"
            "  --opaque         declare the whole surface opaque, as chrome does\n"
            "  --label=S        prefix the window title (e.g. \"[7] \") for easy reporting\n"
            "  --number=N       draw N huge and centred, to identify the window on screen\n"
            "  --black          paint every frame black (the number plate colour), not the cycle\n"
            "  --load-ms=N      calibrate the clear count so each submit takes ~N ms\n"
            "  --load=N         N black clears before the real one, to lengthen the submit\n"
            "                   so an unsynchronised sample sees BLACK instead of the colour\n",
            me);
}

int main(int argc, char **argv) {
    // Line-buffered: this tool is normally run with stdout redirected to a file and
    // read WHILE it holds a frame on screen, so block buffering would hide every
    // line until exit — exactly the window in which the output is wanted.
    setvbuf(stdout, NULL, _IOLBF, 0);
    const char *socket_name = NULL;
    for (int i = 1; i < argc; i++) {
        if (!strncmp(argv[i], "--socket=", 9)) socket_name = argv[i] + 9;
        else if (!strcmp(argv[i], "--sync=wait")) g_wait_fence = 1;
        else if (!strcmp(argv[i], "--sync=nowait")) g_wait_fence = 0;
        else if (!strncmp(argv[i], "--frames=", 9)) g_frames = atoi(argv[i] + 9);
        else if (!strncmp(argv[i], "--size=", 7)) sscanf(argv[i] + 7, "%dx%d", &g_width, &g_height);
        else if (!strncmp(argv[i], "--hold=", 7)) g_hold = atof(argv[i] + 7);
        else if (!strncmp(argv[i], "--modifier=", 11)) g_force_mod = strtoull(argv[i] + 11, NULL, 0);
        else if (!strcmp(argv[i], "--viewport")) g_viewport = 1;
        else if (!strcmp(argv[i], "--legacy-damage")) g_legacy_damage = 1;
        else if (!strncmp(argv[i], "--alpha=", 8)) g_alpha = atof(argv[i] + 8);
        else if (!strcmp(argv[i], "--opaque")) g_opaque = 1;
        else if (!strncmp(argv[i], "--load=", 7)) g_load = atoi(argv[i] + 7);
        else if (!strncmp(argv[i], "--load-ms=", 10)) g_load_ms = atof(argv[i] + 10);
        else if (!strncmp(argv[i], "--label=", 8)) g_label = argv[i] + 8;
        else if (!strcmp(argv[i], "--format=abgr")) {
            g_drm_fourcc = DRM_FORMAT_ABGR8888;
            g_vk_format = VK_FORMAT_R8G8B8A8_UNORM;
            g_fmt_name = "ABGR8888";
        }
        else if (!strcmp(argv[i], "--format=argb")) { /* the default */ }
        else if (!strncmp(argv[i], "--number=", 9)) g_number = atoi(argv[i] + 9);
        else if (!strcmp(argv[i], "--black")) g_black = 1;
        else if (!strcmp(argv[i], "-v")) g_verbose = 1;
        else { usage(argv[0]); return 2; }
    }

    d_display = wl_display_connect(socket_name);
    if (!d_display) {
        fprintf(stderr, "sync-probe: cannot connect: %s\n", strerror(errno));
        return 1;
    }
    struct wl_registry *registry = wl_display_get_registry(d_display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(d_display);
    if (!d_compositor || !d_wm_base || !d_dmabuf) {
        fprintf(stderr, "sync-probe: missing wl_compositor/xdg_wm_base/zwp_linux_dmabuf_v1\n");
        return 1;
    }
    zwp_linux_dmabuf_v1_add_listener(d_dmabuf, &dmabuf_listener, NULL);
    wl_display_roundtrip(d_display);  // collect the modifier list

    printf("compositor: wp_presentation=%s  linux-drm-syncobj=%s  %s modifiers=%d\n",
           d_presentation ? "yes" : "NO", d_has_syncobj ? "yes" : "NO", g_fmt_name,
           d_mod_count);
    printf("sync mode:  %s\n", g_wait_fence ? "wait on VkFence before commit"
                                            : "NO wait — relying on implicit sync");
    printf("gpu load:   %d extra clear(s) per frame%s\n", g_load,
           g_load ? "  (early sample would show BLACK)" : "");
    printf("clear alpha: %.2f%s\n", g_alpha,
           g_alpha == 0.0 ? "  (transparent if the compositor blends)" : "");

    if (vk_setup() < 0) return 1;

    uint64_t modifier;
    if (pick_modifier(&modifier) < 0) {
        fprintf(stderr, "sync-probe: no %s modifier both the compositor advertises and "
                        "Vulkan can colour-attach (single-plane)\n", g_fmt_name);
        return 1;
    }
    printf("modifier:   0x%016llx\n", (unsigned long long)modifier);

    xdg_wm_base_add_listener(d_wm_base, &wm_base_listener, NULL);
    for (int i = 0; i < 2; i++)
        if (make_buffer(&g_bufs[i], modifier) < 0) return 1;

    for (int i = 0; i < 2; i++) {
        struct zwp_linux_buffer_params_v1 *p = zwp_linux_dmabuf_v1_create_params(d_dmabuf);
        zwp_linux_buffer_params_v1_add(p, g_bufs[i].fd, 0, g_bufs[i].offset, g_bufs[i].stride,
                                       (uint32_t)(g_bufs[i].modifier >> 32),
                                       (uint32_t)(g_bufs[i].modifier & 0xffffffff));
        g_bufs[i].wl_buffer = zwp_linux_buffer_params_v1_create_immed(
            p, g_width, g_height, g_drm_fourcc, 0);
        zwp_linux_buffer_params_v1_destroy(p);
        wl_buffer_add_listener(g_bufs[i].wl_buffer, &buffer_listener, &g_bufs[i]);
    }

    if (calibrate_load(&g_bufs[0]) < 0) return 1;

    d_surface = wl_compositor_create_surface(d_compositor);
    if (g_opaque) {
        // The whole surface. A compositor that honours this must ignore the alpha
        // channel inside it, so an alpha-0 buffer still shows its colour.
        struct wl_region *r = wl_compositor_create_region(d_compositor);
        wl_region_add(r, 0, 0, g_width, g_height);
        wl_surface_set_opaque_region(d_surface, r);
        wl_region_destroy(r);
        printf("opaque:     whole surface declared opaque\n");
    }
    d_xdg_surface = xdg_wm_base_get_xdg_surface(d_wm_base, d_surface);
    xdg_surface_add_listener(d_xdg_surface, &xdg_surface_listener, NULL);
    d_toplevel = xdg_surface_get_toplevel(d_xdg_surface);
    xdg_toplevel_add_listener(d_toplevel, &toplevel_listener, NULL);
    char title[128];
    // Every switch shows in the title: a variant you cannot identify on screen is a
    // variant you cannot trust the result of.
    snprintf(title, sizeof title, "%s%s%s%s%s%s%s", g_label, g_wait_fence ? "WAIT" : "NOWAIT",
             g_alpha == 0.0 ? " a=0" : "", g_opaque ? " +opaque" : "",
             g_force_mod ? " +mod" : "", g_viewport ? " +viewport" : "",
             g_legacy_damage ? " +legacydmg" : "");
    if (g_black) strncat(title, " BLACK", sizeof title - strlen(title) - 1);
    if (g_load) {
        char l[48];
        if (g_load_ms > 0) snprintf(l, sizeof l, " load=%.0fms", g_load_ms);
        else snprintf(l, sizeof l, " load=%d", g_load);
        strncat(title, l, sizeof title - strlen(title) - 1);
    }
    xdg_toplevel_set_title(d_toplevel, title);
    xdg_toplevel_set_app_id(d_toplevel, "sync-probe");
    if (g_viewport) {
        if (!d_viewporter) {
            fprintf(stderr, "sync-probe: --viewport but no wp_viewporter advertised\n");
            return 1;
        }
        // Chrome's shape: source is the WHOLE buffer, destination is the buffer
        // divided by the fractional scale, so the buffer is 1.25x the logical size.
        d_viewport = wp_viewporter_get_viewport(d_viewporter, d_surface);
        wp_viewport_set_source(d_viewport, wl_fixed_from_int(0), wl_fixed_from_int(0),
                               wl_fixed_from_int(g_width), wl_fixed_from_int(g_height));
        wp_viewport_set_destination(d_viewport, (int)(g_width / 1.25), (int)(g_height / 1.25));
        printf("viewport:   src 0,0 %dx%d -> dst %dx%d\n", g_width, g_height,
               (int)(g_width / 1.25), (int)(g_height / 1.25));
    }
    wl_surface_commit(d_surface);
    wl_display_roundtrip(d_display);

    uint64_t deadline = now_ns() + 30ull * 1000000000ull;
    while (d_running && now_ns() < deadline) {
        while (wl_display_prepare_read(d_display) != 0) wl_display_dispatch_pending(d_display);
        wl_display_flush(d_display);
        struct pollfd pfd = { .fd = wl_display_get_fd(d_display), .events = POLLIN };
        if (poll(&pfd, 1, 100) > 0 && (pfd.revents & POLLIN)) wl_display_read_events(d_display);
        else wl_display_cancel_read(d_display);
        if (wl_display_dispatch_pending(d_display) < 0) break;
    }
    // Hold the last frame on screen long enough to be looked at.
    uint64_t hold = now_ns() + (uint64_t)(g_hold * 1e9);
    while (now_ns() < hold) {
        wl_display_dispatch_pending(d_display);
        wl_display_flush(d_display);
        struct pollfd pfd = { .fd = wl_display_get_fd(d_display), .events = POLLIN };
        poll(&pfd, 1, 100);
    }

    // In nowait mode nothing ever waits on the GPU again after the final submit, so a
    // device loss during that last (long) clear is completely silent — the commits all
    // printed, the window went blank, and the log claims a clean run. Ask once, here,
    // so "it vanished" and "the producer died mid-frame" stop looking identical.
    if (vk_dev) {
        VkResult idle = vkDeviceWaitIdle(vk_dev);
        if (idle != VK_SUCCESS) {
            g_gpu_lost = 1;
            printf("  !! vkDeviceWaitIdle after hold: VkResult %d\n", idle);
        }
    }

    printf("\n--- summary (%s) ---\n", g_wait_fence ? "sync=wait" : "sync=nowait");
    printf("verdict            %s\n", g_gpu_lost
        ? "CONTAMINATED — GPU lost, this window's pixels prove nothing"
        : "usable — every frame rendered");
    printf("commits            %u\n", g_committed);
    printf("frame callbacks    %u\n", g_frame_cb);
    printf("presented          %u\n", g_presented);
    printf("discarded          %u\n", g_discarded);
    printf("\nLOOK AT THE WINDOW: the last colour committed was %s.\n",
           g_black ? "black" : COLOURS[(g_committed ? g_committed - 1 : 0) % 6].name);
    printf("If it is that colour, the buffer survived. If it is empty/black, the\n"
           "compositor sampled before the clear landed.\n");

    wl_display_disconnect(d_display);
    return 0;
}
