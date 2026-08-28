// vkfmt — what a VkFormat can actually do on this device, per DRM modifier.
//
// Answers the two questions y5's format gates ask, without a compositor, a
// surface, or a window — so it cannot wedge a session the way dmabuf-draw and
// dmabuf-format-test can:
//
//   query::renderable(phd, vk)  ->  optimalTilingFeatures has COLOR_ATTACHMENT
//   query::sampleable(phd, vk)  ->  either tiling has SAMPLED_IMAGE
//   modifier::modifiers(phd,vk) ->  the per-modifier tiling feature list
//
// The per-modifier list is the one that matters: a format can report
// COLOR_ATTACHMENT in optimalTilingFeatures and still be unusable as a dmabuf
// target if no DRM modifier carries the feature.
//
//   cc -O2 -o vkfmt vkfmt.c -lvulkan && ./vkfmt

#define VK_USE_PLATFORM_XLIB_KHR 0
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>

struct entry {
    const char *drm;   // the DRM fourcc(s) this VkFormat backs
    const char *name;
    VkFormat fmt;
};

// The formats the scanout ladder and the background worker ladder can name.
static const struct entry FORMATS[] = {
    {"XB30/AB30", "A2B10G10R10_UNORM_PACK32", VK_FORMAT_A2B10G10R10_UNORM_PACK32},
    {"XR30/AR30", "A2R10G10B10_UNORM_PACK32", VK_FORMAT_A2R10G10B10_UNORM_PACK32},
    {"XR24/AR24", "B8G8R8A8_UNORM         ", VK_FORMAT_B8G8R8A8_UNORM},
    {"XB24/AB24", "R8G8B8A8_UNORM         ", VK_FORMAT_R8G8B8A8_UNORM},
    {"XB4H/AB4H", "R16G16B16A16_SFLOAT    ", VK_FORMAT_R16G16B16A16_SFLOAT},
};

static void feats(VkFormatFeatureFlags f, char *out, size_t n) {
    snprintf(out, n, "%s%s%s%s%s%s%s%s",
             (f & VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT) ? "COLOR_ATTACHMENT " : "",
             (f & VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT) ? "SAMPLED " : "",
             (f & VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT) ? "BLEND " : "",
             (f & VK_FORMAT_FEATURE_TRANSFER_SRC_BIT) ? "TRANSFER_SRC " : "",
             (f & VK_FORMAT_FEATURE_TRANSFER_DST_BIT) ? "TRANSFER_DST " : "",
             (f & VK_FORMAT_FEATURE_BLIT_SRC_BIT) ? "BLIT_SRC " : "",
             (f & VK_FORMAT_FEATURE_BLIT_DST_BIT) ? "BLIT_DST " : "",
             f ? "" : "(none)");
}

// Cross-device modifier overlap. THE question on a PRIME box: do the two GPUs share any
// modifier beyond LINEAR? DRM modifiers are vendor-namespaced (top 8 bits: 0x01 Intel,
// 0x03 NVIDIA), and only LINEAR (0) and INVALID are vendor-neutral — so the expected answer
// is "no". Expected is not measured, and the whole scanout story turns on it: a shared TILED
// modifier means the scanout buffer can be async-flipped, LINEAR-only means it cannot.
#define MAX_DEV 8
#define MAX_MOD 64
struct devmods {
    char name[256];
    uint32_t vendor;        // 0x10de NVIDIA, 0x8086 Intel, 0x1002 AMD
    uint8_t uuid[VK_UUID_SIZE];  // identifies the PHYSICAL device, so the same GPU
                                 // enumerated twice is not compared against itself
    uint64_t mods[sizeof FORMATS / sizeof *FORMATS][MAX_MOD];
    unsigned count[sizeof FORMATS / sizeof *FORMATS];
};
static struct devmods DEVS[MAX_DEV];
static unsigned NDEV;

static int has_mod(const struct devmods *d, size_t k, uint64_t m) {
    for (unsigned i = 0; i < d->count[k]; i++)
        if (d->mods[k][i] == m) return 1;
    return 0;
}

int main(void) {
    VkApplicationInfo app = {.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                             .apiVersion = VK_API_VERSION_1_3};
    VkInstanceCreateInfo ici = {.sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                                .pApplicationInfo = &app};
    VkInstance inst;
    if (vkCreateInstance(&ici, NULL, &inst) != VK_SUCCESS) {
        fprintf(stderr, "vkCreateInstance failed\n");
        return 1;
    }

    uint32_t n = 0;
    vkEnumeratePhysicalDevices(inst, &n, NULL);
    VkPhysicalDevice *pds = calloc(n, sizeof *pds);
    vkEnumeratePhysicalDevices(inst, &n, pds);

    for (uint32_t i = 0; i < n; i++) {
        VkPhysicalDeviceProperties props;
        vkGetPhysicalDeviceProperties(pds[i], &props);
        if (props.deviceType != VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU &&
            props.deviceType != VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU)
            continue;

        printf("\n=== %s ===\n", props.deviceName);
        if (NDEV < MAX_DEV) {
            snprintf(DEVS[NDEV].name, sizeof DEVS[NDEV].name, "%s", props.deviceName);
            DEVS[NDEV].vendor = props.vendorID;
            VkPhysicalDeviceIDProperties idp = {
                .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES};
            VkPhysicalDeviceProperties2 p2 = {
                .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2, .pNext = &idp};
            vkGetPhysicalDeviceProperties2(pds[i], &p2);
            memcpy(DEVS[NDEV].uuid, idp.deviceUUID, VK_UUID_SIZE);
            printf("    vendor 0x%04x device 0x%04x\n", props.vendorID, props.deviceID);
        }
        for (size_t k = 0; k < sizeof FORMATS / sizeof *FORMATS; k++) {
            // Plain properties: what query::renderable / ::sampleable read.
            VkFormatProperties fp;
            vkGetPhysicalDeviceFormatProperties(pds[i], FORMATS[k].fmt, &fp);
            char lin[96], opt[96];
            feats(fp.linearTilingFeatures, lin, sizeof lin);
            feats(fp.optimalTilingFeatures, opt, sizeof opt);
            printf("\n%s  %s\n", FORMATS[k].drm, FORMATS[k].name);
            printf("  optimal: %s\n  linear : %s\n", opt, lin);
            printf("  renderable()=%s  sampleable()=%s\n",
                   (fp.optimalTilingFeatures & VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT) ? "YES" : "no",
                   ((fp.optimalTilingFeatures | fp.linearTilingFeatures) &
                    VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT) ? "YES" : "no");

            // Per-modifier list: what modifier::modifiers_with() filters on.
            VkDrmFormatModifierPropertiesListEXT list = {
                .sType = VK_STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT};
            VkFormatProperties2 fp2 = {.sType = VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2,
                                       .pNext = &list};
            vkGetPhysicalDeviceFormatProperties2(pds[i], FORMATS[k].fmt, &fp2);
            if (!list.drmFormatModifierCount) {
                printf("  modifiers: NONE  <- unusable as a dmabuf target/source\n");
                continue;
            }
            VkDrmFormatModifierPropertiesEXT *mods =
                calloc(list.drmFormatModifierCount, sizeof *mods);
            list.pDrmFormatModifierProperties = mods;
            vkGetPhysicalDeviceFormatProperties2(pds[i], FORMATS[k].fmt, &fp2);
            printf("  modifiers: %u\n", list.drmFormatModifierCount);
            unsigned ca = 0, sa = 0;
            for (uint32_t m = 0; m < list.drmFormatModifierCount; m++) {
                char mf[96];
                feats(mods[m].drmFormatModifierTilingFeatures, mf, sizeof mf);
                printf("    0x%016llx planes=%u  %s\n",
                       (unsigned long long)mods[m].drmFormatModifier,
                       mods[m].drmFormatModifierPlaneCount, mf);
                if (mods[m].drmFormatModifierTilingFeatures &
                    VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT) ca++;
                if (mods[m].drmFormatModifierTilingFeatures &
                    VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT) sa++;
                if (NDEV < MAX_DEV && DEVS[NDEV].count[k] < MAX_MOD)
                    DEVS[NDEV].mods[k][DEVS[NDEV].count[k]++] = mods[m].drmFormatModifier;
            }
            printf("    -> %u color-attachable, %u sampleable\n", ca, sa);
            free(mods);
        }
        if (NDEV < MAX_DEV) NDEV++;
    }

    // The cross-device answer, stated rather than left to the eye.
    if (NDEV > 1) {
        unsigned compared = 0;
        printf("\n=== cross-device modifier overlap ===\n");
        for (unsigned a = 0; a < NDEV; a++) {
            for (unsigned b = a + 1; b < NDEV; b++) {
                // The same physical GPU can be enumerated more than once (two ICDs, or a
                // driver listing it twice). Comparing it with itself yields a full overlap
                // that reads exactly like a real cross-vendor result — so skip it.
                if (!memcmp(DEVS[a].uuid, DEVS[b].uuid, VK_UUID_SIZE)) {
                    printf("\n%s vs itself (same deviceUUID, enumerated twice) — skipped\n",
                           DEVS[a].name);
                    continue;
                }
                compared++;
                printf("\n%s [0x%04x]\n  vs %s [0x%04x]%s\n",
                       DEVS[a].name, DEVS[a].vendor, DEVS[b].name, DEVS[b].vendor,
                       DEVS[a].vendor != DEVS[b].vendor ? "   (CROSS-VENDOR)" : "");
                for (size_t k = 0; k < sizeof FORMATS / sizeof *FORMATS; k++) {
                    unsigned shared = 0, tiled = 0;
                    printf("  %s %s:", FORMATS[k].drm, FORMATS[k].name);
                    for (unsigned i = 0; i < DEVS[a].count[k]; i++) {
                        uint64_t m = DEVS[a].mods[k][i];
                        if (!has_mod(&DEVS[b], k, m)) continue;
                        shared++;
                        if (m != 0) tiled++;
                        printf(" 0x%016llx%s", (unsigned long long)m, m == 0 ? "(LINEAR)" : "");
                    }
                    if (!shared) printf(" NONE");
                    printf("   [%u shared, %u non-LINEAR]%s\n", shared, tiled,
                           tiled ? "  <- async-flip capable" : "");
                }
            }
        }
        if (compared)
            printf("\nnon-LINEAR shared == 0 for every format means the scanout buffer is\n"
                   "forced to LINEAR on this pairing, and i915 will not async-flip LINEAR.\n");
        else
            printf("\nOnly one distinct physical device — nothing to compare. Run this on the\n"
                   "split-GPU machine, where both GPUs enumerate.\n");
    }
    free(pds);
    vkDestroyInstance(inst, NULL);
    return 0;
}
