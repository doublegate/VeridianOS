/*
 * VeridianOS libc -- <xf86drm.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal libdrm interface for DRM/KMS access.
 * Provides ioctl wrappers for /dev/dri/card0 operations.
 */

#ifndef _XF86DRM_H
#define _XF86DRM_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>

/* ========================================================================= */
/* DRM ioctl command numbers                                                 */
/* ========================================================================= */

#define DRM_IOCTL_BASE          'd'
#define DRM_IOCTL_VERSION       0x00
#define DRM_IOCTL_GEM_CLOSE     0x09
#define DRM_IOCTL_GET_CAP       0x0C
#define DRM_IOCTL_SET_MASTER    0x1E
#define DRM_IOCTL_DROP_MASTER   0x1F
#define DRM_IOCTL_PRIME_HANDLE_TO_FD 0x2D
#define DRM_IOCTL_PRIME_FD_TO_HANDLE 0x2E

/* Mode-setting ioctls */
#define DRM_IOCTL_MODE_GETRESOURCES  0xA0
#define DRM_IOCTL_MODE_GETCRTC       0xA1
#define DRM_IOCTL_MODE_SETCRTC       0xA2
#define DRM_IOCTL_MODE_GETENCODER    0xA6
#define DRM_IOCTL_MODE_GETCONNECTOR  0xA7
#define DRM_IOCTL_MODE_PAGE_FLIP     0xB0
#define DRM_IOCTL_MODE_CREATE_DUMB   0xB2
#define DRM_IOCTL_MODE_MAP_DUMB      0xB3
#define DRM_IOCTL_MODE_DESTROY_DUMB  0xB4

/* ========================================================================= */
/* DRM capability constants                                                  */
/* ========================================================================= */

#define DRM_CAP_DUMB_BUFFER          0x01
#define DRM_CAP_PRIME                0x05
#define DRM_CAP_TIMESTAMP_MONOTONIC  0x06

/* ========================================================================= */
/* DRM version                                                               */
/* ========================================================================= */

typedef struct _drmVersion {
    int    version_major;
    int    version_minor;
    int    version_patchlevel;
    int    name_len;
    char  *name;
    int    date_len;
    char  *date;
    int    desc_len;
    char  *desc;
} drmVersion, *drmVersionPtr;

/** Get DRM driver version. Caller must free with drmFreeVersion(). */
drmVersionPtr drmGetVersion(int fd);

/** Free a drmVersion structure. */
void drmFreeVersion(drmVersionPtr v);

/* ========================================================================= */
/* DRM capability query                                                      */
/* ========================================================================= */

/** Query a DRM capability. Returns 0 on success, -1 on failure. */
int drmGetCap(int fd, uint64_t capability, uint64_t *value);

/* ========================================================================= */
/* DRM master control                                                        */
/* ========================================================================= */

/** Acquire DRM master. Returns 0 on success. */
int drmSetMaster(int fd);

/** Release DRM master. Returns 0 on success. */
int drmDropMaster(int fd);

/* ========================================================================= */
/* GEM / PRIME buffer management                                             */
/* ========================================================================= */

/** Close a GEM buffer handle. */
int drmGemClose(int fd, uint32_t handle);

/** Export a GEM handle as a DMA-BUF fd. Returns 0 on success. */
int drmPrimeHandleToFD(int fd, uint32_t handle, uint32_t flags, int *prime_fd);

/** Import a DMA-BUF fd as a GEM handle. Returns 0 on success. */
int drmPrimeFDToHandle(int fd, int prime_fd, uint32_t *handle);

/* ========================================================================= */
/* DRM device open                                                           */
/* ========================================================================= */

/** Open a DRM device by path. Returns fd or -1 on error. */
int drmOpen(const char *name, const char *busid);

/** Close a DRM device fd. */
int drmClose(int fd);

/* ========================================================================= */
/* Page flip                                                                 */
/* ========================================================================= */

#define DRM_MODE_PAGE_FLIP_EVENT  0x01
#define DRM_MODE_PAGE_FLIP_ASYNC  0x02

/** Request a page flip. Returns 0 on success. */
int drmModePageFlip(int fd, uint32_t crtc_id, uint32_t fb_id,
                    uint32_t flags, void *user_data);

#ifdef __cplusplus
}
#endif

#endif /* _XF86DRM_H */
