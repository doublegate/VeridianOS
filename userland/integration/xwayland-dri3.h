/*
 * xwayland-dri3.h -- DRI3 Extension for VeridianOS XWayland
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * DRI3 (Direct Rendering Infrastructure 3) extension support for
 * XWayland on VeridianOS.  Provides buffer allocation and sharing
 * via DMA-BUF file descriptors between X11 clients and the
 * compositor (KWin).
 *
 * DRI3 replaces DRI2's buffer management with fd-passing:
 *   - Client allocates GBM buffer -> gets DMA-BUF fd
 *   - fd passed to Xwayland via DRI3PixmapFromBuffer
 *   - Xwayland imports the fd as an X11 Pixmap
 *   - Compositor can scanout or texture from the DMA-BUF directly
 *
 * This avoids copies and enables zero-copy rendering for Mesa-based
 * OpenGL/Vulkan applications.
 */

#ifndef XWAYLAND_DRI3_H
#define XWAYLAND_DRI3_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ======================================================================
 * DRI3 types
 * ====================================================================== */

/* DRM fourcc format codes (subset) */
#define DRI3_FORMAT_XRGB8888    0x34325258  /* XR24 */
#define DRI3_FORMAT_ARGB8888    0x34325241  /* AR24 */
#define DRI3_FORMAT_XBGR8888    0x34324258  /* XB24 */
#define DRI3_FORMAT_ABGR8888    0x34324241  /* AB24 */
#define DRI3_FORMAT_RGB565      0x36314752  /* RG16 */

/* Invalid modifier sentinel */
#define DRI3_FORMAT_MOD_INVALID 0x00ffffffffffffffULL
#define DRI3_FORMAT_MOD_LINEAR  0x0000000000000000ULL

/* Maximum planes for multi-planar formats */
#define DRI3_MAX_PLANES         4

/* DRI3 buffer descriptor */
typedef struct {
    int         fd;             /* DMA-BUF file descriptor */
    uint32_t    width;
    uint32_t    height;
    uint32_t    stride;         /* Row pitch in bytes */
    uint32_t    format;         /* DRM fourcc format code */
    uint64_t    modifier;       /* DRM format modifier */
    uint32_t    bpp;            /* Bits per pixel */
    uint32_t    depth;          /* X11 drawable depth */
} dri3_buffer_t;

/* Multi-plane DRI3 buffer (DRI3 1.2+) */
typedef struct {
    int         fds[DRI3_MAX_PLANES];
    uint32_t    strides[DRI3_MAX_PLANES];
    uint32_t    offsets[DRI3_MAX_PLANES];
    uint32_t    num_planes;
    uint32_t    width;
    uint32_t    height;
    uint32_t    format;         /* DRM fourcc */
    uint64_t    modifier;       /* DRM format modifier */
} dri3_multi_buffer_t;

/* DRI3 fence for synchronization */
typedef struct {
    int         fd;             /* eventfd or sync fd */
    bool        triggered;
} dri3_fence_t;

/* Supported modifier list for format negotiation */
typedef struct {
    uint64_t   *modifiers;
    int         count;
} dri3_modifier_list_t;

/* ======================================================================
 * DRI3 functions
 * ====================================================================== */

/*
 * Open the DRI3 render node.
 * Returns a file descriptor to the GPU render node (/dev/dri/renderD128)
 * that clients use for buffer allocation.  Returns -1 on error.
 */
int dri3_open(void);

/*
 * Create an X11 Pixmap from a DMA-BUF file descriptor.
 * The fd is imported by the server; the caller retains ownership
 * of the fd and must close it separately.
 * Returns a Pixmap XID, or 0 on error.
 */
uint32_t dri3_pixmap_from_buffer(const dri3_buffer_t *buffer);

/*
 * Export an existing X11 Pixmap as a DMA-BUF.
 * Fills in the buffer descriptor with the fd, stride, and format.
 * Returns 0 on success, -1 on error.
 */
int dri3_buffer_from_pixmap(uint32_t pixmap, dri3_buffer_t *buffer);

/*
 * Create a DRI3 fence from a file descriptor.
 * Used for synchronization between client and server rendering.
 * Returns 0 on success, -1 on error.
 */
int dri3_fence_from_fd(int fd, dri3_fence_t *fence);

/*
 * Query supported DRM format modifiers for a given format.
 * Returns 0 on success, -1 on error.
 * Caller must free modifiers->modifiers when done.
 */
int dri3_get_supported_modifiers(uint32_t format, uint32_t depth,
                                  dri3_modifier_list_t *modifiers);

/*
 * Create an X11 Pixmap from a multi-plane DMA-BUF (DRI3 1.2).
 * Supports multi-planar formats (e.g., NV12, YUV420).
 * Returns a Pixmap XID, or 0 on error.
 */
uint32_t dri3_pixmap_from_buffers(const dri3_multi_buffer_t *buffers);

/*
 * Allocate a GBM buffer suitable for DRI3 use.
 * Wraps gbm_bo_create() with appropriate flags.
 * Returns 0 on success, -1 on error.
 * The caller must close buffer->fd when done.
 */
int dri3_alloc_buffer(uint32_t width, uint32_t height,
                      uint32_t format, uint64_t modifier,
                      dri3_buffer_t *buffer);

/*
 * Free resources associated with a DRI3 buffer.
 * Closes the fd if >= 0.
 */
void dri3_free_buffer(dri3_buffer_t *buffer);

/*
 * Convert DRM fourcc format to X11 depth and bpp.
 * Returns 0 on success, -1 for unknown formats.
 */
int dri3_format_to_depth_bpp(uint32_t format, uint32_t *depth,
                              uint32_t *bpp);

/*
 * Convert X11 depth and bpp to DRM fourcc format.
 * Returns the fourcc code, or 0 for unsupported combinations.
 */
uint32_t dri3_depth_bpp_to_format(uint32_t depth, uint32_t bpp);

#ifdef __cplusplus
}
#endif

#endif /* XWAYLAND_DRI3_H */
