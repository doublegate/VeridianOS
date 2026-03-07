/*
 * VeridianOS libc -- <xf86drmMode.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * DRM mode-setting structures and functions.
 */

#ifndef _XF86DRM_MODE_H
#define _XF86DRM_MODE_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>

/* ========================================================================= */
/* Mode info                                                                 */
/* ========================================================================= */

/** DRM display mode information. */
typedef struct _drmModeModeInfo {
    uint32_t clock;          /**< Pixel clock in kHz */
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
    uint32_t vrefresh;       /**< Refresh rate in Hz */
    uint32_t flags;
    uint32_t type;
    char     name[32];
} drmModeModeInfo, *drmModeModeInfoPtr;

/* ========================================================================= */
/* Resources                                                                 */
/* ========================================================================= */

/** DRM mode resources -- lists all CRTCs, connectors, encoders. */
typedef struct _drmModeRes {
    int       count_fbs;
    uint32_t *fbs;
    int       count_crtcs;
    uint32_t *crtcs;
    int       count_connectors;
    uint32_t *connectors;
    int       count_encoders;
    uint32_t *encoders;
    uint32_t  min_width, max_width;
    uint32_t  min_height, max_height;
} drmModeRes, *drmModeResPtr;

/** Get mode resources. Caller must free with drmModeFreeResources(). */
drmModeResPtr drmModeGetResources(int fd);

/** Free mode resources. */
void drmModeFreeResources(drmModeResPtr ptr);

/* ========================================================================= */
/* Connector                                                                 */
/* ========================================================================= */

/** Connection status */
#define DRM_MODE_CONNECTED         1
#define DRM_MODE_DISCONNECTED      2
#define DRM_MODE_UNKNOWNCONNECTION 3

/** Connector types */
#define DRM_MODE_CONNECTOR_VGA         1
#define DRM_MODE_CONNECTOR_DVII        2
#define DRM_MODE_CONNECTOR_DVID        3
#define DRM_MODE_CONNECTOR_DVIA        4
#define DRM_MODE_CONNECTOR_LVDS        7
#define DRM_MODE_CONNECTOR_HDMIA       11
#define DRM_MODE_CONNECTOR_HDMIB       12
#define DRM_MODE_CONNECTOR_DisplayPort 14
#define DRM_MODE_CONNECTOR_VIRTUAL     15

/** DRM connector. */
typedef struct _drmModeConnector {
    uint32_t          connector_id;
    uint32_t          encoder_id;       /**< Currently bound encoder */
    uint32_t          connector_type;
    uint32_t          connector_type_id;
    uint32_t          connection;       /**< DRM_MODE_CONNECTED etc. */
    uint32_t          mm_width;
    uint32_t          mm_height;
    uint32_t          subpixel;
    int               count_modes;
    drmModeModeInfo  *modes;
    int               count_props;
    uint32_t         *props;
    uint64_t         *prop_values;
    int               count_encoders;
    uint32_t         *encoders;
} drmModeConnector, *drmModeConnectorPtr;

/** Get connector info. Caller must free with drmModeFreeConnector(). */
drmModeConnectorPtr drmModeGetConnector(int fd, uint32_t connector_id);

/** Free connector. */
void drmModeFreeConnector(drmModeConnectorPtr ptr);

/* ========================================================================= */
/* Encoder                                                                   */
/* ========================================================================= */

/** DRM encoder. */
typedef struct _drmModeEncoder {
    uint32_t encoder_id;
    uint32_t encoder_type;
    uint32_t crtc_id;
    uint32_t possible_crtcs;
    uint32_t possible_clones;
} drmModeEncoder, *drmModeEncoderPtr;

/** Get encoder info. Caller must free with drmModeFreeEncoder(). */
drmModeEncoderPtr drmModeGetEncoder(int fd, uint32_t encoder_id);

/** Free encoder. */
void drmModeFreeEncoder(drmModeEncoderPtr ptr);

/* ========================================================================= */
/* CRTC                                                                      */
/* ========================================================================= */

/** DRM CRTC. */
typedef struct _drmModeCrtc {
    uint32_t        crtc_id;
    uint32_t        buffer_id;    /**< Currently active framebuffer */
    uint32_t        x, y;
    uint32_t        width, height;
    int             mode_valid;
    drmModeModeInfo mode;
    int             gamma_size;
} drmModeCrtc, *drmModeCrtcPtr;

/** Get CRTC info. Caller must free with drmModeFreeCrtc(). */
drmModeCrtcPtr drmModeGetCrtc(int fd, uint32_t crtc_id);

/** Free CRTC. */
void drmModeFreeCrtc(drmModeCrtcPtr ptr);

/** Set CRTC mode and framebuffer. */
int drmModeSetCrtc(int fd, uint32_t crtc_id, uint32_t fb_id,
                   uint32_t x, uint32_t y,
                   uint32_t *connectors, int count,
                   drmModeModeInfoPtr mode);

/* ========================================================================= */
/* Framebuffer                                                               */
/* ========================================================================= */

/** DRM framebuffer. */
typedef struct _drmModeFB {
    uint32_t fb_id;
    uint32_t width, height;
    uint32_t pitch;
    uint32_t bpp;
    uint32_t depth;
    uint32_t handle;
} drmModeFB, *drmModeFBPtr;

/* ========================================================================= */
/* Dumb buffer management                                                    */
/* ========================================================================= */

/** Create dumb buffer request. */
struct drm_mode_create_dumb {
    uint32_t height;
    uint32_t width;
    uint32_t bpp;
    uint32_t flags;
    uint32_t handle;     /**< Output: GEM handle */
    uint32_t pitch;      /**< Output: bytes per row */
    uint64_t size;       /**< Output: total size */
};

/** Map dumb buffer request. */
struct drm_mode_map_dumb {
    uint32_t handle;
    uint32_t pad;
    uint64_t offset;     /**< Output: mmap offset */
};

/** Destroy dumb buffer request. */
struct drm_mode_destroy_dumb {
    uint32_t handle;
};

/** Create a dumb (scanout) buffer. Returns 0 on success. */
int drmModeCreateDumb(int fd, struct drm_mode_create_dumb *create);

/** Prepare a dumb buffer for mmap. Returns 0 on success. */
int drmModeMapDumb(int fd, struct drm_mode_map_dumb *map);

/** Destroy a dumb buffer. Returns 0 on success. */
int drmModeDestroyDumb(int fd, uint32_t handle);

#ifdef __cplusplus
}
#endif

#endif /* _XF86DRM_MODE_H */
