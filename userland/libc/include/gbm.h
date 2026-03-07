/*
 * VeridianOS libc -- <gbm.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Generic Buffer Manager (GBM) interface.
 * Backed by DRM dumb buffers and GEM on VeridianOS.
 */

#ifndef _GBM_H
#define _GBM_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>
#include <stddef.h>

/* ========================================================================= */
/* Format constants                                                          */
/* ========================================================================= */

/** FourCC format helpers */
#define GBM_FORMAT_XRGB8888  0x34325258  /* [31:0] X:R:G:B 8:8:8:8 */
#define GBM_FORMAT_ARGB8888  0x34325241  /* [31:0] A:R:G:B 8:8:8:8 */
#define GBM_FORMAT_XBGR8888  0x34324258  /* [31:0] X:B:G:R 8:8:8:8 */
#define GBM_FORMAT_ABGR8888  0x34324241  /* [31:0] A:B:G:R 8:8:8:8 */
#define GBM_FORMAT_RGB565    0x36314752  /* [15:0] R:G:B 5:6:5 */

/* ========================================================================= */
/* Buffer object flags                                                       */
/* ========================================================================= */

#define GBM_BO_USE_SCANOUT       (1 << 0)
#define GBM_BO_USE_CURSOR        (1 << 1)
#define GBM_BO_USE_RENDERING     (1 << 2)
#define GBM_BO_USE_WRITE         (1 << 3)
#define GBM_BO_USE_LINEAR        (1 << 4)

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

/** GBM device -- one per DRM fd. */
struct gbm_device;

/** GBM surface -- for EGL rendering targets. */
struct gbm_surface;

/** GBM buffer object. */
struct gbm_bo;

/* ========================================================================= */
/* Device management                                                         */
/* ========================================================================= */

/**
 * Create a GBM device from a DRM fd.
 * Returns NULL on failure.
 */
struct gbm_device *gbm_create_device(int fd);

/**
 * Destroy a GBM device.
 */
void gbm_device_destroy(struct gbm_device *gbm);

/**
 * Get the DRM fd associated with a GBM device.
 */
int gbm_device_get_fd(struct gbm_device *gbm);

/**
 * Check if a format+usage combination is supported.
 */
int gbm_device_is_format_supported(struct gbm_device *gbm,
                                   uint32_t format, uint32_t usage);

/* ========================================================================= */
/* Surface management (EGL integration)                                      */
/* ========================================================================= */

/**
 * Create a GBM surface for EGL rendering.
 */
struct gbm_surface *gbm_surface_create(struct gbm_device *gbm,
                                       uint32_t width, uint32_t height,
                                       uint32_t format, uint32_t flags);

/**
 * Destroy a GBM surface.
 */
void gbm_surface_destroy(struct gbm_surface *surface);

/**
 * Lock the front buffer after an EGL swap.
 * Returns a gbm_bo that can be used for scanout.
 */
struct gbm_bo *gbm_surface_lock_front_buffer(struct gbm_surface *surface);

/**
 * Release a previously locked front buffer.
 */
void gbm_surface_release_buffer(struct gbm_surface *surface,
                                struct gbm_bo *bo);

/**
 * Check if the surface has a free buffer available.
 */
int gbm_surface_has_free_buffers(struct gbm_surface *surface);

/* ========================================================================= */
/* Buffer object management                                                  */
/* ========================================================================= */

/**
 * Create a buffer object.
 */
struct gbm_bo *gbm_bo_create(struct gbm_device *gbm,
                             uint32_t width, uint32_t height,
                             uint32_t format, uint32_t flags);

/**
 * Destroy a buffer object.
 */
void gbm_bo_destroy(struct gbm_bo *bo);

/**
 * Get the width of a buffer object.
 */
uint32_t gbm_bo_get_width(struct gbm_bo *bo);

/**
 * Get the height of a buffer object.
 */
uint32_t gbm_bo_get_height(struct gbm_bo *bo);

/**
 * Get the stride (pitch) of a buffer object.
 */
uint32_t gbm_bo_get_stride(struct gbm_bo *bo);

/**
 * Get the format of a buffer object.
 */
uint32_t gbm_bo_get_format(struct gbm_bo *bo);

/**
 * Get the GEM handle of a buffer object.
 */
uint32_t gbm_bo_get_handle(struct gbm_bo *bo);

/**
 * Get the DMA-BUF fd of a buffer object.
 */
int gbm_bo_get_fd(struct gbm_bo *bo);

/**
 * Set user data on a buffer object.
 */
void gbm_bo_set_user_data(struct gbm_bo *bo, void *data,
                          void (*destroy_user_data)(struct gbm_bo *, void *));

/**
 * Get user data from a buffer object.
 */
void *gbm_bo_get_user_data(struct gbm_bo *bo);

/**
 * Get the GBM device a buffer object was created with.
 */
struct gbm_device *gbm_bo_get_device(struct gbm_bo *bo);

/**
 * Write data into a buffer object.
 */
int gbm_bo_write(struct gbm_bo *bo, const void *buf, size_t count);

/**
 * Map a buffer object for CPU access.
 */
void *gbm_bo_map(struct gbm_bo *bo, uint32_t x, uint32_t y,
                 uint32_t width, uint32_t height,
                 uint32_t flags, uint32_t *stride,
                 void **map_data);

/**
 * Unmap a previously mapped buffer object.
 */
void gbm_bo_unmap(struct gbm_bo *bo, void *map_data);

#ifdef __cplusplus
}
#endif

#endif /* _GBM_H */
