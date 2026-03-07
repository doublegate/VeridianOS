/*
 * VeridianOS libc -- libdrm.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Thin wrappers around ioctl() calls to /dev/dri/card0.
 * Implements the libdrm API for user-space DRM/KMS access.
 */

#include <xf86drm.h>
#include <xf86drmMode.h>
#include <sys/ioctl.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* ========================================================================= */
/* Internal ioctl data structures (match kernel repr(C) structs)             */
/* ========================================================================= */

struct drm_version_ioctl {
    int      version_major;
    int      version_minor;
    int      version_patchlevel;
    uint32_t name_len;
    uint64_t name_ptr;
    uint32_t date_len;
    uint64_t date_ptr;
    uint32_t desc_len;
    uint64_t desc_ptr;
};

struct drm_get_cap_ioctl {
    uint64_t capability;
    uint64_t value;
};

struct drm_gem_close_ioctl {
    uint32_t handle;
    uint32_t pad;
};

struct drm_prime_handle_to_fd_ioctl {
    uint32_t handle;
    uint32_t flags;
    int      fd;
    uint32_t pad;
};

struct drm_prime_fd_to_handle_ioctl {
    int      fd;
    uint32_t pad;
    uint32_t handle;
    uint32_t pad2;
};

struct drm_mode_card_res {
    uint64_t fb_id_ptr;
    uint64_t crtc_id_ptr;
    uint64_t connector_id_ptr;
    uint64_t encoder_id_ptr;
    uint32_t count_fbs;
    uint32_t count_crtcs;
    uint32_t count_connectors;
    uint32_t count_encoders;
    uint32_t min_width;
    uint32_t max_width;
    uint32_t min_height;
    uint32_t max_height;
};

/* Kernel-ABI mode info (matches DrmModeInfo in drm_ioctl.rs) */
struct drm_mode_modeinfo {
    uint32_t clock;
    uint16_t hdisplay;
    uint16_t hsync_start;
    uint16_t hsync_end;
    uint16_t htotal;
    uint16_t hskew;
    uint16_t vdisplay;
    uint16_t vsync_start;
    uint16_t vsync_end;
    uint16_t vtotal;
    uint16_t vscan;
    uint32_t vrefresh;
    uint32_t flags;
    uint32_t type;
    char     name[32];
};

struct drm_mode_get_crtc {
    uint64_t set_connectors_ptr;
    uint32_t count_connectors;
    uint32_t crtc_id;
    uint32_t fb_id;
    uint32_t x;
    uint32_t y;
    uint32_t gamma_size;
    uint32_t mode_valid;
    struct drm_mode_modeinfo mode;
};

struct drm_mode_get_encoder {
    uint32_t encoder_id;
    uint32_t encoder_type;
    uint32_t crtc_id;
    uint32_t possible_crtcs;
    uint32_t possible_clones;
};

struct drm_mode_get_connector {
    uint64_t encoders_ptr;
    uint64_t modes_ptr;
    uint64_t props_ptr;
    uint64_t prop_values_ptr;
    uint32_t count_modes;
    uint32_t count_props;
    uint32_t count_encoders;
    uint32_t encoder_id;
    uint32_t connector_id;
    uint32_t connector_type;
    uint32_t connector_type_id;
    uint32_t connection;
    uint32_t mm_width;
    uint32_t mm_height;
    uint32_t subpixel;
    uint32_t pad;
};

struct drm_mode_page_flip_ioctl {
    uint32_t crtc_id;
    uint32_t fb_id;
    uint32_t flags;
    uint32_t reserved;
    uint64_t user_data;
};

/* ========================================================================= */
/* Helper: copy mode info between kernel and user structs                    */
/* ========================================================================= */

static void copy_modeinfo_to_user(drmModeModeInfo *dst,
                                  const struct drm_mode_modeinfo *src)
{
    dst->clock      = src->clock;
    dst->hdisplay   = src->hdisplay;
    dst->hsync_start = src->hsync_start;
    dst->hsync_end  = src->hsync_end;
    dst->htotal     = src->htotal;
    dst->hskew      = src->hskew;
    dst->vdisplay   = src->vdisplay;
    dst->vsync_start = src->vsync_start;
    dst->vsync_end  = src->vsync_end;
    dst->vtotal     = src->vtotal;
    dst->vscan      = src->vscan;
    dst->vrefresh   = src->vrefresh;
    dst->flags      = src->flags;
    dst->type       = src->type;
    memcpy(dst->name, src->name, 32);
}

static void copy_modeinfo_to_kernel(struct drm_mode_modeinfo *dst,
                                    const drmModeModeInfo *src)
{
    dst->clock      = src->clock;
    dst->hdisplay   = src->hdisplay;
    dst->hsync_start = src->hsync_start;
    dst->hsync_end  = src->hsync_end;
    dst->htotal     = src->htotal;
    dst->hskew      = src->hskew;
    dst->vdisplay   = src->vdisplay;
    dst->vsync_start = src->vsync_start;
    dst->vsync_end  = src->vsync_end;
    dst->vtotal     = src->vtotal;
    dst->vscan      = src->vscan;
    dst->vrefresh   = src->vrefresh;
    dst->flags      = src->flags;
    dst->type       = src->type;
    memcpy(dst->name, src->name, 32);
}

/* ========================================================================= */
/* DRM device open/close                                                     */
/* ========================================================================= */

int drmOpen(const char *name, const char *busid)
{
    (void)name;
    (void)busid;
    return open("/dev/dri/card0", O_RDWR);
}

int drmClose(int fd)
{
    return close(fd);
}

/* ========================================================================= */
/* DRM version                                                               */
/* ========================================================================= */

drmVersionPtr drmGetVersion(int fd)
{
    struct drm_version_ioctl ver;
    drmVersionPtr ret;
    char name_buf[64];
    char date_buf[32];
    char desc_buf[128];

    memset(&ver, 0, sizeof(ver));
    ver.name_len = sizeof(name_buf) - 1;
    ver.name_ptr = (uint64_t)(uintptr_t)name_buf;
    ver.date_len = sizeof(date_buf) - 1;
    ver.date_ptr = (uint64_t)(uintptr_t)date_buf;
    ver.desc_len = sizeof(desc_buf) - 1;
    ver.desc_ptr = (uint64_t)(uintptr_t)desc_buf;

    if (ioctl(fd, DRM_IOCTL_VERSION, &ver) < 0)
        return NULL;

    ret = calloc(1, sizeof(*ret));
    if (!ret)
        return NULL;

    ret->version_major    = ver.version_major;
    ret->version_minor    = ver.version_minor;
    ret->version_patchlevel = ver.version_patchlevel;

    ret->name_len = (int)ver.name_len;
    ret->name = calloc(1, ver.name_len + 1);
    if (ret->name)
        memcpy(ret->name, name_buf, ver.name_len);

    ret->date_len = (int)ver.date_len;
    ret->date = calloc(1, ver.date_len + 1);
    if (ret->date)
        memcpy(ret->date, date_buf, ver.date_len);

    ret->desc_len = (int)ver.desc_len;
    ret->desc = calloc(1, ver.desc_len + 1);
    if (ret->desc)
        memcpy(ret->desc, desc_buf, ver.desc_len);

    return ret;
}

void drmFreeVersion(drmVersionPtr v)
{
    if (!v) return;
    free(v->name);
    free(v->date);
    free(v->desc);
    free(v);
}

/* ========================================================================= */
/* DRM capabilities                                                          */
/* ========================================================================= */

int drmGetCap(int fd, uint64_t capability, uint64_t *value)
{
    struct drm_get_cap_ioctl cap;
    cap.capability = capability;
    cap.value = 0;

    if (ioctl(fd, DRM_IOCTL_GET_CAP, &cap) < 0)
        return -1;

    *value = cap.value;
    return 0;
}

/* ========================================================================= */
/* DRM master                                                                */
/* ========================================================================= */

int drmSetMaster(int fd)
{
    return ioctl(fd, DRM_IOCTL_SET_MASTER, NULL);
}

int drmDropMaster(int fd)
{
    return ioctl(fd, DRM_IOCTL_DROP_MASTER, NULL);
}

/* ========================================================================= */
/* GEM / PRIME                                                               */
/* ========================================================================= */

int drmGemClose(int fd, uint32_t handle)
{
    struct drm_gem_close_ioctl arg;
    arg.handle = handle;
    arg.pad = 0;
    return ioctl(fd, DRM_IOCTL_GEM_CLOSE, &arg);
}

int drmPrimeHandleToFD(int fd, uint32_t handle, uint32_t flags, int *prime_fd)
{
    struct drm_prime_handle_to_fd_ioctl arg;
    arg.handle = handle;
    arg.flags = flags;
    arg.fd = -1;
    arg.pad = 0;

    if (ioctl(fd, DRM_IOCTL_PRIME_HANDLE_TO_FD, &arg) < 0)
        return -1;

    *prime_fd = arg.fd;
    return 0;
}

int drmPrimeFDToHandle(int fd, int prime_fd, uint32_t *handle)
{
    struct drm_prime_fd_to_handle_ioctl arg;
    arg.fd = prime_fd;
    arg.pad = 0;
    arg.handle = 0;
    arg.pad2 = 0;

    if (ioctl(fd, DRM_IOCTL_PRIME_FD_TO_HANDLE, &arg) < 0)
        return -1;

    *handle = arg.handle;
    return 0;
}

/* ========================================================================= */
/* Mode resources                                                            */
/* ========================================================================= */

drmModeResPtr drmModeGetResources(int fd)
{
    struct drm_mode_card_res res;
    drmModeResPtr ret;

    /* First call: get counts */
    memset(&res, 0, sizeof(res));
    if (ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res) < 0)
        return NULL;

    ret = calloc(1, sizeof(*ret));
    if (!ret)
        return NULL;

    ret->count_fbs        = (int)res.count_fbs;
    ret->count_crtcs      = (int)res.count_crtcs;
    ret->count_connectors = (int)res.count_connectors;
    ret->count_encoders   = (int)res.count_encoders;
    ret->min_width        = res.min_width;
    ret->max_width        = res.max_width;
    ret->min_height       = res.min_height;
    ret->max_height       = res.max_height;

    /* Allocate arrays and second call to fill IDs */
    if (ret->count_fbs > 0) {
        ret->fbs = calloc((size_t)ret->count_fbs, sizeof(uint32_t));
        res.fb_id_ptr = (uint64_t)(uintptr_t)ret->fbs;
    }
    if (ret->count_crtcs > 0) {
        ret->crtcs = calloc((size_t)ret->count_crtcs, sizeof(uint32_t));
        res.crtc_id_ptr = (uint64_t)(uintptr_t)ret->crtcs;
    }
    if (ret->count_connectors > 0) {
        ret->connectors = calloc((size_t)ret->count_connectors, sizeof(uint32_t));
        res.connector_id_ptr = (uint64_t)(uintptr_t)ret->connectors;
    }
    if (ret->count_encoders > 0) {
        ret->encoders = calloc((size_t)ret->count_encoders, sizeof(uint32_t));
        res.encoder_id_ptr = (uint64_t)(uintptr_t)ret->encoders;
    }

    /* Second call: fill ID arrays */
    if (ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res) < 0) {
        drmModeFreeResources(ret);
        return NULL;
    }

    return ret;
}

void drmModeFreeResources(drmModeResPtr ptr)
{
    if (!ptr) return;
    free(ptr->fbs);
    free(ptr->crtcs);
    free(ptr->connectors);
    free(ptr->encoders);
    free(ptr);
}

/* ========================================================================= */
/* Connector                                                                 */
/* ========================================================================= */

drmModeConnectorPtr drmModeGetConnector(int fd, uint32_t connector_id)
{
    struct drm_mode_get_connector conn;
    drmModeConnectorPtr ret;

    /* First call: get counts */
    memset(&conn, 0, sizeof(conn));
    conn.connector_id = connector_id;
    if (ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &conn) < 0)
        return NULL;

    ret = calloc(1, sizeof(*ret));
    if (!ret)
        return NULL;

    ret->connector_id      = conn.connector_id;
    ret->encoder_id        = conn.encoder_id;
    ret->connector_type    = conn.connector_type;
    ret->connector_type_id = conn.connector_type_id;
    ret->connection        = conn.connection;
    ret->mm_width          = conn.mm_width;
    ret->mm_height         = conn.mm_height;
    ret->subpixel          = conn.subpixel;
    ret->count_modes       = (int)conn.count_modes;
    ret->count_props       = (int)conn.count_props;
    ret->count_encoders    = (int)conn.count_encoders;

    /* Allocate mode array and do second call */
    if (ret->count_modes > 0) {
        struct drm_mode_modeinfo *modes =
            calloc((size_t)ret->count_modes, sizeof(struct drm_mode_modeinfo));
        conn.modes_ptr = (uint64_t)(uintptr_t)modes;

        if (ret->count_encoders > 0) {
            ret->encoders = calloc((size_t)ret->count_encoders, sizeof(uint32_t));
            conn.encoders_ptr = (uint64_t)(uintptr_t)ret->encoders;
        }

        if (ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &conn) < 0) {
            free(modes);
            drmModeFreeConnector(ret);
            return NULL;
        }

        /* Convert kernel mode structs to user mode structs */
        ret->modes = calloc((size_t)ret->count_modes, sizeof(drmModeModeInfo));
        if (ret->modes && modes) {
            for (int i = 0; i < ret->count_modes; i++) {
                copy_modeinfo_to_user(&ret->modes[i], &modes[i]);
            }
        }
        free(modes);
    }

    return ret;
}

void drmModeFreeConnector(drmModeConnectorPtr ptr)
{
    if (!ptr) return;
    free(ptr->modes);
    free(ptr->props);
    free(ptr->prop_values);
    free(ptr->encoders);
    free(ptr);
}

/* ========================================================================= */
/* Encoder                                                                   */
/* ========================================================================= */

drmModeEncoderPtr drmModeGetEncoder(int fd, uint32_t encoder_id)
{
    struct drm_mode_get_encoder enc;
    drmModeEncoderPtr ret;

    memset(&enc, 0, sizeof(enc));
    enc.encoder_id = encoder_id;

    if (ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &enc) < 0)
        return NULL;

    ret = calloc(1, sizeof(*ret));
    if (!ret)
        return NULL;

    ret->encoder_id     = enc.encoder_id;
    ret->encoder_type   = enc.encoder_type;
    ret->crtc_id        = enc.crtc_id;
    ret->possible_crtcs = enc.possible_crtcs;
    ret->possible_clones = enc.possible_clones;

    return ret;
}

void drmModeFreeEncoder(drmModeEncoderPtr ptr)
{
    free(ptr);
}

/* ========================================================================= */
/* CRTC                                                                      */
/* ========================================================================= */

drmModeCrtcPtr drmModeGetCrtc(int fd, uint32_t crtc_id)
{
    struct drm_mode_get_crtc crtc;
    drmModeCrtcPtr ret;

    memset(&crtc, 0, sizeof(crtc));
    crtc.crtc_id = crtc_id;

    if (ioctl(fd, DRM_IOCTL_MODE_GETCRTC, &crtc) < 0)
        return NULL;

    ret = calloc(1, sizeof(*ret));
    if (!ret)
        return NULL;

    ret->crtc_id    = crtc.crtc_id;
    ret->buffer_id  = crtc.fb_id;
    ret->x          = crtc.x;
    ret->y          = crtc.y;
    ret->mode_valid = (int)crtc.mode_valid;
    ret->gamma_size = (int)crtc.gamma_size;

    if (crtc.mode_valid) {
        copy_modeinfo_to_user(&ret->mode, &crtc.mode);
        ret->width  = crtc.mode.hdisplay;
        ret->height = crtc.mode.vdisplay;
    }

    return ret;
}

void drmModeFreeCrtc(drmModeCrtcPtr ptr)
{
    free(ptr);
}

int drmModeSetCrtc(int fd, uint32_t crtc_id, uint32_t fb_id,
                   uint32_t x, uint32_t y,
                   uint32_t *connectors, int count,
                   drmModeModeInfoPtr mode)
{
    struct drm_mode_get_crtc crtc;

    memset(&crtc, 0, sizeof(crtc));
    crtc.crtc_id = crtc_id;
    crtc.fb_id = fb_id;
    crtc.x = x;
    crtc.y = y;

    if (connectors && count > 0) {
        crtc.set_connectors_ptr = (uint64_t)(uintptr_t)connectors;
        crtc.count_connectors = (uint32_t)count;
    }

    if (mode) {
        crtc.mode_valid = 1;
        copy_modeinfo_to_kernel(&crtc.mode, mode);
    }

    return ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &crtc);
}

/* ========================================================================= */
/* Dumb buffer management                                                    */
/* ========================================================================= */

int drmModeCreateDumb(int fd, struct drm_mode_create_dumb *create)
{
    return ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, create);
}

int drmModeMapDumb(int fd, struct drm_mode_map_dumb *map)
{
    return ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, map);
}

int drmModeDestroyDumb(int fd, uint32_t handle)
{
    struct drm_mode_destroy_dumb arg;
    arg.handle = handle;
    return ioctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &arg);
}

/* ========================================================================= */
/* Page flip                                                                 */
/* ========================================================================= */

int drmModePageFlip(int fd, uint32_t crtc_id, uint32_t fb_id,
                    uint32_t flags, void *user_data)
{
    struct drm_mode_page_flip_ioctl flip;
    flip.crtc_id = crtc_id;
    flip.fb_id = fb_id;
    flip.flags = flags;
    flip.reserved = 0;
    flip.user_data = (uint64_t)(uintptr_t)user_data;

    return ioctl(fd, DRM_IOCTL_MODE_PAGE_FLIP, &flip);
}
