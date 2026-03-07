/*
 * VeridianOS libc -- gbm.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Generic Buffer Manager implementation.
 * Backed by DRM dumb buffers and GEM handles.
 */

#include <gbm.h>
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <sys/ioctl.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct gbm_device {
    int      fd;          /* DRM device fd */
    int      refcount;
};

struct gbm_bo {
    struct gbm_device *device;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint32_t format;
    uint32_t handle;      /* GEM handle */
    uint64_t size;
    int      fd;          /* DMA-BUF fd, -1 if not exported */

    void    *user_data;
    void   (*destroy_user_data)(struct gbm_bo *, void *);
};

struct gbm_surface {
    struct gbm_device *device;
    uint32_t width;
    uint32_t height;
    uint32_t format;
    uint32_t flags;

    /* Double-buffered: two backing buffer objects */
    struct gbm_bo *buffers[2];
    int            front;       /* Index of current front buffer */
    int            locked;      /* Whether front is locked */
};

/* ========================================================================= */
/* Device                                                                    */
/* ========================================================================= */

struct gbm_device *gbm_create_device(int fd)
{
    struct gbm_device *dev = calloc(1, sizeof(*dev));
    if (!dev)
        return NULL;

    dev->fd = fd;
    dev->refcount = 1;
    return dev;
}

void gbm_device_destroy(struct gbm_device *gbm)
{
    if (!gbm)
        return;
    gbm->refcount--;
    if (gbm->refcount <= 0)
        free(gbm);
}

int gbm_device_get_fd(struct gbm_device *gbm)
{
    return gbm ? gbm->fd : -1;
}

int gbm_device_is_format_supported(struct gbm_device *gbm,
                                   uint32_t format, uint32_t usage)
{
    (void)gbm;
    (void)usage;

    /* We support the common formats via dumb buffers */
    switch (format) {
    case GBM_FORMAT_XRGB8888:
    case GBM_FORMAT_ARGB8888:
    case GBM_FORMAT_XBGR8888:
    case GBM_FORMAT_ABGR8888:
    case GBM_FORMAT_RGB565:
        return 1;
    default:
        return 0;
    }
}

/* ========================================================================= */
/* Buffer object                                                             */
/* ========================================================================= */

/** Get bytes per pixel for a GBM format. */
static uint32_t format_bpp(uint32_t format)
{
    switch (format) {
    case GBM_FORMAT_XRGB8888:
    case GBM_FORMAT_ARGB8888:
    case GBM_FORMAT_XBGR8888:
    case GBM_FORMAT_ABGR8888:
        return 32;
    case GBM_FORMAT_RGB565:
        return 16;
    default:
        return 32;
    }
}

struct gbm_bo *gbm_bo_create(struct gbm_device *gbm,
                             uint32_t width, uint32_t height,
                             uint32_t format, uint32_t flags)
{
    struct drm_mode_create_dumb create;
    struct gbm_bo *bo;

    if (!gbm)
        return NULL;

    (void)flags;

    memset(&create, 0, sizeof(create));
    create.width  = width;
    create.height = height;
    create.bpp    = format_bpp(format);

    if (ioctl(gbm->fd, DRM_IOCTL_MODE_CREATE_DUMB, &create) < 0)
        return NULL;

    bo = calloc(1, sizeof(*bo));
    if (!bo) {
        /* Clean up the dumb buffer on allocation failure */
        drmModeDestroyDumb(gbm->fd, create.handle);
        return NULL;
    }

    bo->device  = gbm;
    bo->width   = width;
    bo->height  = height;
    bo->stride  = create.pitch;
    bo->format  = format;
    bo->handle  = create.handle;
    bo->size    = create.size;
    bo->fd      = -1;

    return bo;
}

void gbm_bo_destroy(struct gbm_bo *bo)
{
    if (!bo)
        return;

    if (bo->destroy_user_data)
        bo->destroy_user_data(bo, bo->user_data);

    if (bo->fd >= 0)
        close(bo->fd);

    drmModeDestroyDumb(bo->device->fd, bo->handle);
    free(bo);
}

uint32_t gbm_bo_get_width(struct gbm_bo *bo)
{
    return bo ? bo->width : 0;
}

uint32_t gbm_bo_get_height(struct gbm_bo *bo)
{
    return bo ? bo->height : 0;
}

uint32_t gbm_bo_get_stride(struct gbm_bo *bo)
{
    return bo ? bo->stride : 0;
}

uint32_t gbm_bo_get_format(struct gbm_bo *bo)
{
    return bo ? bo->format : 0;
}

uint32_t gbm_bo_get_handle(struct gbm_bo *bo)
{
    return bo ? bo->handle : 0;
}

int gbm_bo_get_fd(struct gbm_bo *bo)
{
    if (!bo)
        return -1;

    if (bo->fd < 0) {
        int prime_fd = -1;
        if (drmPrimeHandleToFD(bo->device->fd, bo->handle, 0, &prime_fd) == 0)
            bo->fd = prime_fd;
    }

    return bo->fd;
}

void gbm_bo_set_user_data(struct gbm_bo *bo, void *data,
                          void (*destroy)(struct gbm_bo *, void *))
{
    if (!bo)
        return;
    bo->user_data = data;
    bo->destroy_user_data = destroy;
}

void *gbm_bo_get_user_data(struct gbm_bo *bo)
{
    return bo ? bo->user_data : NULL;
}

struct gbm_device *gbm_bo_get_device(struct gbm_bo *bo)
{
    return bo ? bo->device : NULL;
}

int gbm_bo_write(struct gbm_bo *bo, const void *buf, size_t count)
{
    /* Writing to dumb buffers requires mmap; for now, return error */
    (void)bo;
    (void)buf;
    (void)count;
    return -1;
}

void *gbm_bo_map(struct gbm_bo *bo, uint32_t x, uint32_t y,
                 uint32_t width, uint32_t height,
                 uint32_t flags, uint32_t *stride,
                 void **map_data)
{
    /* Mapping requires mmap support on the DRM device; stub for now */
    (void)bo;
    (void)x;
    (void)y;
    (void)width;
    (void)height;
    (void)flags;
    (void)stride;
    (void)map_data;
    return NULL;
}

void gbm_bo_unmap(struct gbm_bo *bo, void *map_data)
{
    (void)bo;
    (void)map_data;
}

/* ========================================================================= */
/* Surface                                                                   */
/* ========================================================================= */

struct gbm_surface *gbm_surface_create(struct gbm_device *gbm,
                                       uint32_t width, uint32_t height,
                                       uint32_t format, uint32_t flags)
{
    struct gbm_surface *surface;

    if (!gbm)
        return NULL;

    surface = calloc(1, sizeof(*surface));
    if (!surface)
        return NULL;

    surface->device = gbm;
    surface->width  = width;
    surface->height = height;
    surface->format = format;
    surface->flags  = flags;
    surface->front  = 0;
    surface->locked = 0;

    /* Pre-allocate two backing buffers for double-buffering */
    surface->buffers[0] = gbm_bo_create(gbm, width, height, format, flags);
    surface->buffers[1] = gbm_bo_create(gbm, width, height, format, flags);

    if (!surface->buffers[0] || !surface->buffers[1]) {
        gbm_surface_destroy(surface);
        return NULL;
    }

    return surface;
}

void gbm_surface_destroy(struct gbm_surface *surface)
{
    if (!surface)
        return;

    if (surface->buffers[0])
        gbm_bo_destroy(surface->buffers[0]);
    if (surface->buffers[1])
        gbm_bo_destroy(surface->buffers[1]);

    free(surface);
}

struct gbm_bo *gbm_surface_lock_front_buffer(struct gbm_surface *surface)
{
    if (!surface || surface->locked)
        return NULL;

    surface->locked = 1;
    return surface->buffers[surface->front];
}

void gbm_surface_release_buffer(struct gbm_surface *surface,
                                struct gbm_bo *bo)
{
    if (!surface)
        return;

    (void)bo;
    surface->locked = 0;
    /* Swap front/back */
    surface->front = 1 - surface->front;
}

int gbm_surface_has_free_buffers(struct gbm_surface *surface)
{
    if (!surface)
        return 0;
    return surface->locked ? 0 : 1;
}
