// eglvk — what EGL enumerates versus what Vulkan enumerates, for ONE device.
//
// The question behind "Vulkan is the authority, not EGL": we source scanout and client
// format sets from smithay's EGL lists, but the composite is Vulkan. If EGL under-reports,
// every intersection built on it is narrower than the hardware allows, and nothing
// downstream can recover the difference. If EGL over-reports, we advertise what will not
// import. This measures which, per (fourcc, modifier), on the same DRM node.
//
// Both halves are asked of the SAME device on purpose — pass the render node and it opens
// EGL on it via GBM and matches the Vulkan physical device by DRM minor
// (VK_EXT_physical_device_drm). Comparing two different GPUs answers nothing.
//
//   cc -O2 -o eglvk eglvk.c -lvulkan -lEGL -lgbm && ./eglvk [--device=/dev/dri/renderD128]

#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>   // major()/minor()

#include <gbm.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <vulkan/vulkan.h>

#define FOURCC(a, b, c, d) ((uint32_t)(a) | ((uint32_t)(b) << 8) | ((uint32_t)(c) << 16) | ((uint32_t)(d) << 24))
#define MAX_MOD 64

struct entry {
    const char *name;
    uint32_t fourcc;
    VkFormat vk;
};

// The formats the scanout ladder, the client advertisement and the producer ladders name.
static const struct entry FORMATS[] = {
    {"XR24", FOURCC('X','R','2','4'), VK_FORMAT_B8G8R8A8_UNORM},
    {"AR24", FOURCC('A','R','2','4'), VK_FORMAT_B8G8R8A8_UNORM},
    {"XB24", FOURCC('X','B','2','4'), VK_FORMAT_R8G8B8A8_UNORM},
    {"AB24", FOURCC('A','B','2','4'), VK_FORMAT_R8G8B8A8_UNORM},
    {"XR30", FOURCC('X','R','3','0'), VK_FORMAT_A2R10G10B10_UNORM_PACK32},
    {"AR30", FOURCC('A','R','3','0'), VK_FORMAT_A2R10G10B10_UNORM_PACK32},
    {"XB30", FOURCC('X','B','3','0'), VK_FORMAT_A2B10G10R10_UNORM_PACK32},
    {"AB30", FOURCC('A','B','3','0'), VK_FORMAT_A2B10G10R10_UNORM_PACK32},
    {"RG16", FOURCC('R','G','1','6'), VK_FORMAT_R5G6B5_UNORM_PACK16},
    {"XR15", FOURCC('X','R','1','5'), VK_FORMAT_A1R5G5B5_UNORM_PACK16},
    {"GR88", FOURCC('G','R','8','8'), VK_FORMAT_R8G8_UNORM},
    {"R8  ", FOURCC('R','8',' ',' '), VK_FORMAT_R8_UNORM},
    {"XB4H", FOURCC('X','B','4','H'), VK_FORMAT_R16G16B16A16_SFLOAT},
    {"AB4H", FOURCC('A','B','4','H'), VK_FORMAT_R16G16B16A16_SFLOAT},
};
#define NFMT (sizeof FORMATS / sizeof *FORMATS)

struct set {
    uint64_t mod[MAX_MOD];
    unsigned n;
};

static int has(const struct set *s, uint64_t m) {
    for (unsigned i = 0; i < s->n; i++) if (s->mod[i] == m) return 1;
    return 0;
}
static void add(struct set *s, uint64_t m) {
    if (!has(s, m) && s->n < MAX_MOD) s->mod[s->n++] = m;
}

static const char *modname(uint64_t m) {
    static char buf[32];
    if (m == 0) return "LINEAR";
    if (m == 0x00ffffffffffffffULL) return "INVALID";
    snprintf(buf, sizeof buf, "0x%016llx", (unsigned long long)m);
    return buf;
}

// ------------------------------------------------------------------ EGL half ----
// smithay's sets come from exactly these two calls (backend/egl/display.rs), so this is
// what it would see. `external_only` splits them: smithay keeps external_only == false as
// the RENDER set and everything as the TEXTURE set.
static int egl_query(const char *dev, struct set render[NFMT], struct set texture[NFMT]) {
    int fd = open(dev, O_RDWR | O_CLOEXEC);
    if (fd < 0) { perror(dev); return -1; }
    struct gbm_device *gbm = gbm_create_device(fd);
    if (!gbm) { fprintf(stderr, "gbm_create_device failed\n"); return -1; }

    PFNEGLGETPLATFORMDISPLAYEXTPROC get_disp =
        (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    if (!get_disp) { fprintf(stderr, "no eglGetPlatformDisplayEXT\n"); return -1; }
    EGLDisplay dpy = get_disp(EGL_PLATFORM_GBM_KHR, gbm, NULL);
    if (dpy == EGL_NO_DISPLAY) { fprintf(stderr, "eglGetPlatformDisplay failed\n"); return -1; }
    EGLint major, minor;
    if (!eglInitialize(dpy, &major, &minor)) { fprintf(stderr, "eglInitialize failed\n"); return -1; }
    printf("EGL %d.%d on %s\n", major, minor, dev);

    const char *exts = eglQueryString(dpy, EGL_EXTENSIONS);
    if (!exts || !strstr(exts, "EGL_EXT_image_dma_buf_import_modifiers")) {
        fprintf(stderr, "EGL_EXT_image_dma_buf_import_modifiers absent — EGL cannot "
                        "enumerate modifiers at all here\n");
        return -1;
    }
    PFNEGLQUERYDMABUFFORMATSEXTPROC q_fmt =
        (void *)eglGetProcAddress("eglQueryDmaBufFormatsEXT");
    PFNEGLQUERYDMABUFMODIFIERSEXTPROC q_mod =
        (void *)eglGetProcAddress("eglQueryDmaBufModifiersEXT");
    if (!q_fmt || !q_mod) { fprintf(stderr, "missing EGL query entrypoints\n"); return -1; }

    for (size_t k = 0; k < NFMT; k++) {
        EGLint n = 0;
        if (!q_mod(dpy, (EGLint)FORMATS[k].fourcc, 0, NULL, NULL, &n) || n <= 0) continue;
        EGLuint64KHR *mods = calloc(n, sizeof *mods);
        EGLBoolean *ext = calloc(n, sizeof *ext);
        if (q_mod(dpy, (EGLint)FORMATS[k].fourcc, n, mods, ext, &n)) {
            for (EGLint i = 0; i < n; i++) {
                add(&texture[k], mods[i]);
                if (!ext[i]) add(&render[k], mods[i]);
            }
        }
        free(mods); free(ext);
    }
    return 0;
}

// --------------------------------------------------------------- Vulkan half ----
// Matched to the same DRM node by render minor, so the comparison is one device.
static int vk_query(const char *dev, struct set attach[NFMT], struct set sample[NFMT],
                    char *name, size_t namelen) {
    struct stat st;
    if (stat(dev, &st) != 0) { perror(dev); return -1; }
    unsigned want_minor = minor(st.st_rdev);

    VkApplicationInfo app = {.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                             .apiVersion = VK_API_VERSION_1_3};
    VkInstanceCreateInfo ici = {.sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                .pApplicationInfo = &app};
    VkInstance inst;
    if (vkCreateInstance(&ici, NULL, &inst) != VK_SUCCESS) {
        fprintf(stderr, "vkCreateInstance failed\n"); return -1;
    }
    uint32_t n = 0;
    vkEnumeratePhysicalDevices(inst, &n, NULL);
    VkPhysicalDevice *pds = calloc(n, sizeof *pds);
    vkEnumeratePhysicalDevices(inst, &n, pds);

    VkPhysicalDevice match = VK_NULL_HANDLE;
    for (uint32_t i = 0; i < n; i++) {
        VkPhysicalDeviceDrmPropertiesEXT drm = {
            .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT};
        VkPhysicalDeviceProperties2 p2 = {
            .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2, .pNext = &drm};
        vkGetPhysicalDeviceProperties2(pds[i], &p2);
        if (drm.hasRender && (unsigned)drm.renderMinor == want_minor) {
            match = pds[i];
            snprintf(name, namelen, "%s", p2.properties.deviceName);
            break;
        }
    }
    if (match == VK_NULL_HANDLE) {
        fprintf(stderr, "no Vulkan physical device reports render minor %u for %s "
                        "(VK_EXT_physical_device_drm missing, or no Vulkan driver for it)\n",
                want_minor, dev);
        free(pds); return -1;
    }

    for (size_t k = 0; k < NFMT; k++) {
        VkDrmFormatModifierPropertiesListEXT list = {
            .sType = VK_STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT};
        VkFormatProperties2 fp2 = {.sType = VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2,
                                   .pNext = &list};
        vkGetPhysicalDeviceFormatProperties2(match, FORMATS[k].vk, &fp2);
        if (!list.drmFormatModifierCount) continue;
        VkDrmFormatModifierPropertiesEXT *mods =
            calloc(list.drmFormatModifierCount, sizeof *mods);
        list.pDrmFormatModifierProperties = mods;
        vkGetPhysicalDeviceFormatProperties2(match, FORMATS[k].vk, &fp2);
        for (uint32_t i = 0; i < list.drmFormatModifierCount; i++) {
            VkFormatFeatureFlags f = mods[i].drmFormatModifierTilingFeatures;
            if (f & VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT) add(&attach[k], mods[i].drmFormatModifier);
            if (f & VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT) add(&sample[k], mods[i].drmFormatModifier);
        }
        free(mods);
    }
    free(pds);
    return 0;
}

static void diff(const char *label, const struct set *vk, const struct set *egl) {
    unsigned only_vk = 0, only_egl = 0, both = 0;
    for (unsigned i = 0; i < vk->n; i++) {
        if (has(egl, vk->mod[i])) both++; else only_vk++;
    }
    for (unsigned i = 0; i < egl->n; i++) if (!has(vk, egl->mod[i])) only_egl++;
    printf("    %-22s vk=%-2u egl=%-2u shared=%-2u", label, vk->n, egl->n, both);
    if (only_vk) {
        printf("  VULKAN-ONLY:");
        for (unsigned i = 0; i < vk->n; i++)
            if (!has(egl, vk->mod[i])) printf(" %s", modname(vk->mod[i]));
    }
    if (only_egl) {
        printf("  EGL-ONLY:");
        for (unsigned i = 0; i < egl->n; i++)
            if (!has(vk, egl->mod[i])) printf(" %s", modname(egl->mod[i]));
    }
    printf("\n");
}

int main(int argc, char **argv) {
    const char *dev = "/dev/dri/renderD128";
    for (int i = 1; i < argc; i++)
        if (!strncmp(argv[i], "--device=", 9)) dev = argv[i] + 9;

    static struct set egl_render[NFMT], egl_texture[NFMT];
    static struct set vk_attach[NFMT], vk_sample[NFMT];
    char vkname[256] = "?";

    setvbuf(stdout, NULL, _IONBF, 0);
    if (egl_query(dev, egl_render, egl_texture) != 0) return 1;
    if (vk_query(dev, vk_attach, vk_sample, vkname, sizeof vkname) != 0) return 1;
    printf("Vulkan device: %s\n\n", vkname);

    // Paired by ROLE, because that is what the compositor actually asks:
    //   render  : EGL dmabuf_render_formats  <-> Vulkan COLOR_ATTACHMENT  (scanout target)
    //   texture : EGL dmabuf_texture_formats <-> Vulkan SAMPLED_IMAGE     (client/producer)
    unsigned gained_r = 0, lost_r = 0, gained_s = 0, lost_s = 0;
    for (size_t k = 0; k < NFMT; k++) {
        if (!egl_texture[k].n && !vk_sample[k].n) continue;
        printf("  %s\n", FORMATS[k].name);
        diff("render/ATTACH", &vk_attach[k], &egl_render[k]);
        diff("texture/SAMPLED", &vk_sample[k], &egl_texture[k]);
        for (unsigned i = 0; i < vk_attach[k].n; i++)
            if (!has(&egl_render[k], vk_attach[k].mod[i])) gained_r++;
        for (unsigned i = 0; i < egl_render[k].n; i++)
            if (!has(&vk_attach[k], egl_render[k].mod[i])) lost_r++;
        for (unsigned i = 0; i < vk_sample[k].n; i++)
            if (!has(&egl_texture[k], vk_sample[k].mod[i])) gained_s++;
        for (unsigned i = 0; i < egl_texture[k].n; i++)
            if (!has(&vk_sample[k], egl_texture[k].mod[i])) lost_s++;
    }

    printf("\n=== totals ===\n");
    printf("  render : %u pair(s) Vulkan has and EGL does not (sourcing from Vulkan GAINS these)\n", gained_r);
    printf("           %u pair(s) EGL has and Vulkan does not (sourcing from Vulkan LOSES these)\n", lost_r);
    printf("  texture: %u pair(s) Vulkan has and EGL does not\n", gained_s);
    printf("           %u pair(s) EGL has and Vulkan does not\n", lost_s);
    printf("\nEGL-ONLY entries are the ones to explain before switching: either EGL reports a\n"
           "modifier the device cannot really use for that role, or Vulkan is stricter for a\n"
           "reason (a per-modifier feature bit EGL's flat list cannot express).\n"
           "INVALID appearing only on the EGL side is EXPECTED — Vulkan has no way to name it.\n");
    return 0;
}
