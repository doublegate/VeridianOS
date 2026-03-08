/*
 * VeridianOS libc -- egl.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * EGL 1.5 implementation backed by VeridianOS DRM/GBM.
 * Provides display lifecycle, config selection, context management,
 * surface management, and extension queries.
 */

#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <gbm.h>
#include <string.h>

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

#define MAX_EGL_CONFIGS   8
#define MAX_EGL_CONTEXTS  16
#define MAX_EGL_SURFACES  32
#define MAX_EGL_IMAGES    32

struct egl_config {
    EGLint id;
    EGLint red_size;
    EGLint green_size;
    EGLint blue_size;
    EGLint alpha_size;
    EGLint depth_size;
    EGLint stencil_size;
    EGLint buffer_size;
    EGLint surface_type;
    EGLint renderable_type;
    EGLint samples;
    EGLint sample_buffers;
    EGLint native_visual_id;
    EGLint conformant;
    EGLint color_buffer_type;
};

struct egl_context {
    int           in_use;
    EGLint        client_version;
    struct egl_config *config;
};

struct egl_surface {
    int           in_use;
    int           type;            /* 0=window, 1=pbuffer */
    EGLint        width;
    EGLint        height;
    void         *native_window;   /* gbm_surface * or wl_egl_window * */
    struct egl_config *config;
};

struct egl_image {
    int           in_use;
    unsigned int  target;
    intptr_t      buffer;
    int           dma_buf_fd;
    EGLint        width;
    EGLint        height;
    EGLint        fourcc;
};

struct egl_display {
    int                initialized;
    void              *native_display;    /* gbm_device * or wl_display * */
    unsigned int       platform;

    struct egl_config  configs[MAX_EGL_CONFIGS];
    int                num_configs;

    struct egl_context contexts[MAX_EGL_CONTEXTS];
    struct egl_surface surfaces[MAX_EGL_SURFACES];
    struct egl_image   images[MAX_EGL_IMAGES];
};

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static EGLint          g_egl_error = EGL_SUCCESS;
static unsigned int    g_current_api = EGL_OPENGL_ES_API;

/* Single display instance (sufficient for embedded/OS use) */
static struct egl_display g_display;
static int                g_display_allocated = 0;

/* Current context per-thread (single-threaded for now) */
static struct egl_context *g_current_context = NULL;
static struct egl_surface *g_current_draw    = NULL;
static struct egl_surface *g_current_read    = NULL;

/* ========================================================================= */
/* Extension and vendor strings                                              */
/* ========================================================================= */

static const char *EGL_VENDOR_STRING =
    "VeridianOS Mesa 24.2 (llvmpipe)";

static const char *EGL_VERSION_STRING =
    "1.5 VeridianOS";

static const char *EGL_EXTENSIONS_STRING =
    "EGL_KHR_platform_wayland "
    "EGL_MESA_platform_gbm "
    "EGL_KHR_image_base "
    "EGL_KHR_image "
    "EGL_KHR_gl_renderbuffer_image "
    "EGL_KHR_gl_texture_2D_image "
    "EGL_EXT_image_dma_buf_import "
    "EGL_EXT_image_dma_buf_import_modifiers "
    "EGL_KHR_fence_sync "
    "EGL_KHR_wait_sync "
    "EGL_KHR_surfaceless_context "
    "EGL_KHR_create_context "
    "EGL_EXT_buffer_age "
    "EGL_EXT_swap_buffers_with_damage";

static const char *EGL_CLIENT_APIS_STRING =
    "OpenGL_ES";

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static void set_error(EGLint err)
{
    g_egl_error = err;
}

static struct egl_display *get_display(EGLDisplay dpy)
{
    if (dpy != (EGLDisplay)&g_display || !g_display_allocated)
        return NULL;
    return &g_display;
}

static struct egl_display *get_initialized_display(EGLDisplay dpy)
{
    struct egl_display *d = get_display(dpy);
    if (!d) {
        set_error(EGL_BAD_DISPLAY);
        return NULL;
    }
    if (!d->initialized) {
        set_error(EGL_NOT_INITIALIZED);
        return NULL;
    }
    return d;
}

static void init_default_configs(struct egl_display *d)
{
    /* Config 0: RGBA8888, depth 24, stencil 8 (window + pbuffer) */
    d->configs[0].id              = 1;
    d->configs[0].red_size        = 8;
    d->configs[0].green_size      = 8;
    d->configs[0].blue_size       = 8;
    d->configs[0].alpha_size      = 8;
    d->configs[0].depth_size      = 24;
    d->configs[0].stencil_size    = 8;
    d->configs[0].buffer_size     = 32;
    d->configs[0].surface_type    = EGL_WINDOW_BIT | EGL_PBUFFER_BIT;
    d->configs[0].renderable_type = EGL_OPENGL_ES2_BIT | EGL_OPENGL_ES3_BIT;
    d->configs[0].samples         = 0;
    d->configs[0].sample_buffers  = 0;
    d->configs[0].native_visual_id = 0x34325241; /* GBM_FORMAT_ARGB8888 */
    d->configs[0].conformant      = EGL_OPENGL_ES2_BIT;
    d->configs[0].color_buffer_type = EGL_RGB_BUFFER;

    /* Config 1: RGBX8888, depth 24, stencil 8 (window + pbuffer) */
    d->configs[1] = d->configs[0];
    d->configs[1].id              = 2;
    d->configs[1].alpha_size      = 0;
    d->configs[1].native_visual_id = 0x34325258; /* GBM_FORMAT_XRGB8888 */

    /* Config 2: RGBA8888, no depth/stencil (lightweight) */
    d->configs[2] = d->configs[0];
    d->configs[2].id              = 3;
    d->configs[2].depth_size      = 0;
    d->configs[2].stencil_size    = 0;

    /* Config 3: RGB565, depth 16, no stencil */
    d->configs[3].id              = 4;
    d->configs[3].red_size        = 5;
    d->configs[3].green_size      = 6;
    d->configs[3].blue_size       = 5;
    d->configs[3].alpha_size      = 0;
    d->configs[3].depth_size      = 16;
    d->configs[3].stencil_size    = 0;
    d->configs[3].buffer_size     = 16;
    d->configs[3].surface_type    = EGL_WINDOW_BIT | EGL_PBUFFER_BIT;
    d->configs[3].renderable_type = EGL_OPENGL_ES2_BIT;
    d->configs[3].samples         = 0;
    d->configs[3].sample_buffers  = 0;
    d->configs[3].native_visual_id = 0x36314752; /* GBM_FORMAT_RGB565 */
    d->configs[3].conformant      = EGL_OPENGL_ES2_BIT;
    d->configs[3].color_buffer_type = EGL_RGB_BUFFER;

    d->num_configs = 4;
}

static int config_matches(const struct egl_config *cfg,
                          const EGLint *attrib_list)
{
    const EGLint *p;

    if (!attrib_list)
        return 1;

    for (p = attrib_list; p[0] != EGL_NONE; p += 2) {
        EGLint attr = p[0];
        EGLint val  = p[1];

        if (val == EGL_DONT_CARE)
            continue;

        switch (attr) {
        case EGL_RED_SIZE:
            if (cfg->red_size < val) return 0;
            break;
        case EGL_GREEN_SIZE:
            if (cfg->green_size < val) return 0;
            break;
        case EGL_BLUE_SIZE:
            if (cfg->blue_size < val) return 0;
            break;
        case EGL_ALPHA_SIZE:
            if (cfg->alpha_size < val) return 0;
            break;
        case EGL_DEPTH_SIZE:
            if (cfg->depth_size < val) return 0;
            break;
        case EGL_STENCIL_SIZE:
            if (cfg->stencil_size < val) return 0;
            break;
        case EGL_BUFFER_SIZE:
            if (cfg->buffer_size < val) return 0;
            break;
        case EGL_SURFACE_TYPE:
            if ((cfg->surface_type & val) != val) return 0;
            break;
        case EGL_RENDERABLE_TYPE:
            if ((cfg->renderable_type & val) != val) return 0;
            break;
        case EGL_SAMPLES:
            if (cfg->samples < val) return 0;
            break;
        case EGL_SAMPLE_BUFFERS:
            if (cfg->sample_buffers < val) return 0;
            break;
        case EGL_COLOR_BUFFER_TYPE:
            if (cfg->color_buffer_type != val) return 0;
            break;
        case EGL_NATIVE_VISUAL_ID:
            if (cfg->native_visual_id != val) return 0;
            break;
        case EGL_CONFIG_CAVEAT:
        case EGL_CONFORMANT:
        case EGL_NATIVE_RENDERABLE:
        case EGL_TRANSPARENT_TYPE:
        case EGL_LEVEL:
        case EGL_MIN_SWAP_INTERVAL:
        case EGL_MAX_SWAP_INTERVAL:
            /* Accept any value for these */
            break;
        default:
            /* Unknown attribute -- ignore */
            break;
        }
    }

    return 1;
}

/* ========================================================================= */
/* Display management                                                        */
/* ========================================================================= */

EGLDisplay eglGetDisplay(EGLNativeDisplayType display_id)
{
    if (g_display_allocated)
        return (EGLDisplay)&g_display;

    memset(&g_display, 0, sizeof(g_display));
    g_display.native_display = (void *)display_id;
    g_display.platform = 0;
    g_display_allocated = 1;

    return (EGLDisplay)&g_display;
}

EGLDisplay eglGetPlatformDisplay(unsigned int platform, void *native_display,
                                  const EGLAttrib *attrib_list)
{
    (void)attrib_list;

    if (platform != EGL_PLATFORM_WAYLAND_KHR &&
        platform != EGL_PLATFORM_GBM_MESA) {
        set_error(EGL_BAD_PARAMETER);
        return EGL_NO_DISPLAY;
    }

    if (g_display_allocated)
        return (EGLDisplay)&g_display;

    memset(&g_display, 0, sizeof(g_display));
    g_display.native_display = native_display;
    g_display.platform = platform;
    g_display_allocated = 1;

    return (EGLDisplay)&g_display;
}

EGLBoolean eglInitialize(EGLDisplay dpy, EGLint *major, EGLint *minor)
{
    struct egl_display *d = get_display(dpy);
    if (!d) {
        set_error(EGL_BAD_DISPLAY);
        return EGL_FALSE;
    }

    if (!d->initialized) {
        init_default_configs(d);
        d->initialized = 1;
    }

    if (major) *major = 1;
    if (minor) *minor = 5;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglTerminate(EGLDisplay dpy)
{
    struct egl_display *d = get_display(dpy);
    int i;

    if (!d) {
        set_error(EGL_BAD_DISPLAY);
        return EGL_FALSE;
    }

    /* Release current context if it belongs to this display */
    g_current_context = NULL;
    g_current_draw    = NULL;
    g_current_read    = NULL;

    /* Mark all contexts and surfaces as unused */
    for (i = 0; i < MAX_EGL_CONTEXTS; i++)
        d->contexts[i].in_use = 0;
    for (i = 0; i < MAX_EGL_SURFACES; i++)
        d->surfaces[i].in_use = 0;
    for (i = 0; i < MAX_EGL_IMAGES; i++)
        d->images[i].in_use = 0;

    d->initialized = 0;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* ========================================================================= */
/* Config management                                                         */
/* ========================================================================= */

EGLBoolean eglChooseConfig(EGLDisplay dpy, const EGLint *attrib_list,
                           EGLConfig *configs, EGLint config_size,
                           EGLint *num_config)
{
    struct egl_display *d = get_initialized_display(dpy);
    int count = 0;
    int i;

    if (!d) return EGL_FALSE;

    if (!num_config) {
        set_error(EGL_BAD_PARAMETER);
        return EGL_FALSE;
    }

    for (i = 0; i < d->num_configs && count < config_size; i++) {
        if (config_matches(&d->configs[i], attrib_list)) {
            if (configs)
                configs[count] = (EGLConfig)&d->configs[i];
            count++;
        }
    }

    *num_config = count;
    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglGetConfigs(EGLDisplay dpy, EGLConfig *configs,
                         EGLint config_size, EGLint *num_config)
{
    struct egl_display *d = get_initialized_display(dpy);
    int count;
    int i;

    if (!d) return EGL_FALSE;

    if (!num_config) {
        set_error(EGL_BAD_PARAMETER);
        return EGL_FALSE;
    }

    count = d->num_configs;
    if (configs) {
        if (count > config_size)
            count = config_size;
        for (i = 0; i < count; i++)
            configs[i] = (EGLConfig)&d->configs[i];
    }

    *num_config = d->num_configs;
    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglGetConfigAttrib(EGLDisplay dpy, EGLConfig config,
                              EGLint attribute, EGLint *value)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_config *cfg = (struct egl_config *)config;

    if (!d) return EGL_FALSE;

    if (!cfg || !value) {
        set_error(EGL_BAD_PARAMETER);
        return EGL_FALSE;
    }

    switch (attribute) {
    case EGL_CONFIG_ID:         *value = cfg->id;              break;
    case EGL_RED_SIZE:          *value = cfg->red_size;        break;
    case EGL_GREEN_SIZE:        *value = cfg->green_size;      break;
    case EGL_BLUE_SIZE:         *value = cfg->blue_size;       break;
    case EGL_ALPHA_SIZE:        *value = cfg->alpha_size;      break;
    case EGL_DEPTH_SIZE:        *value = cfg->depth_size;      break;
    case EGL_STENCIL_SIZE:      *value = cfg->stencil_size;    break;
    case EGL_BUFFER_SIZE:       *value = cfg->buffer_size;     break;
    case EGL_SURFACE_TYPE:      *value = cfg->surface_type;    break;
    case EGL_RENDERABLE_TYPE:   *value = cfg->renderable_type; break;
    case EGL_CONFORMANT:        *value = cfg->conformant;      break;
    case EGL_SAMPLES:           *value = cfg->samples;         break;
    case EGL_SAMPLE_BUFFERS:    *value = cfg->sample_buffers;  break;
    case EGL_NATIVE_VISUAL_ID:  *value = cfg->native_visual_id; break;
    case EGL_NATIVE_VISUAL_TYPE: *value = 0;                   break;
    case EGL_NATIVE_RENDERABLE: *value = EGL_TRUE;             break;
    case EGL_COLOR_BUFFER_TYPE: *value = cfg->color_buffer_type; break;
    case EGL_CONFIG_CAVEAT:     *value = EGL_NONE;             break;
    case EGL_LEVEL:             *value = 0;                    break;
    case EGL_TRANSPARENT_TYPE:  *value = EGL_NONE;             break;
    case EGL_TRANSPARENT_RED_VALUE:   *value = 0;              break;
    case EGL_TRANSPARENT_GREEN_VALUE: *value = 0;              break;
    case EGL_TRANSPARENT_BLUE_VALUE:  *value = 0;              break;
    case EGL_LUMINANCE_SIZE:    *value = 0;                    break;
    case EGL_ALPHA_MASK_SIZE:   *value = 0;                    break;
    case EGL_BIND_TO_TEXTURE_RGB:  *value = EGL_FALSE;         break;
    case EGL_BIND_TO_TEXTURE_RGBA: *value = EGL_FALSE;         break;
    case EGL_MIN_SWAP_INTERVAL: *value = 0;                    break;
    case EGL_MAX_SWAP_INTERVAL: *value = 1;                    break;
    case EGL_MAX_PBUFFER_WIDTH: *value = 4096;                 break;
    case EGL_MAX_PBUFFER_HEIGHT: *value = 4096;                break;
    case EGL_MAX_PBUFFER_PIXELS: *value = 4096 * 4096;        break;
    default:
        set_error(EGL_BAD_ATTRIBUTE);
        return EGL_FALSE;
    }

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* ========================================================================= */
/* Context management                                                        */
/* ========================================================================= */

EGLContext eglCreateContext(EGLDisplay dpy, EGLConfig config,
                            EGLContext share_context,
                            const EGLint *attrib_list)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_context *ctx;
    EGLint client_version = 2;
    const EGLint *p;
    int i;

    if (!d) return EGL_NO_CONTEXT;

    (void)share_context;

    /* Parse attributes */
    if (attrib_list) {
        for (p = attrib_list; p[0] != EGL_NONE; p += 2) {
            switch (p[0]) {
            case EGL_CONTEXT_MAJOR_VERSION:  /* == EGL_CONTEXT_CLIENT_VERSION */
                client_version = p[1];
                break;
            case EGL_CONTEXT_MINOR_VERSION:
                /* Accept but ignore minor version */
                break;
            default:
                break;
            }
        }
    }

    /* Find a free context slot */
    ctx = NULL;
    for (i = 0; i < MAX_EGL_CONTEXTS; i++) {
        if (!d->contexts[i].in_use) {
            ctx = &d->contexts[i];
            break;
        }
    }

    if (!ctx) {
        set_error(EGL_BAD_ALLOC);
        return EGL_NO_CONTEXT;
    }

    ctx->in_use = 1;
    ctx->client_version = client_version;
    ctx->config = (struct egl_config *)config;

    set_error(EGL_SUCCESS);
    return (EGLContext)ctx;
}

EGLBoolean eglDestroyContext(EGLDisplay dpy, EGLContext ctx)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_context *c = (struct egl_context *)ctx;

    if (!d) return EGL_FALSE;

    if (!c || !c->in_use) {
        set_error(EGL_BAD_CONTEXT);
        return EGL_FALSE;
    }

    if (g_current_context == c) {
        g_current_context = NULL;
        g_current_draw    = NULL;
        g_current_read    = NULL;
    }

    c->in_use = 0;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* ========================================================================= */
/* Surface management                                                        */
/* ========================================================================= */

EGLSurface eglCreateWindowSurface(EGLDisplay dpy, EGLConfig config,
                                   EGLNativeWindowType win,
                                   const EGLint *attrib_list)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_surface *surf;
    int i;

    if (!d) return EGL_NO_SURFACE;

    (void)attrib_list;

    /* Find a free surface slot */
    surf = NULL;
    for (i = 0; i < MAX_EGL_SURFACES; i++) {
        if (!d->surfaces[i].in_use) {
            surf = &d->surfaces[i];
            break;
        }
    }

    if (!surf) {
        set_error(EGL_BAD_ALLOC);
        return EGL_NO_SURFACE;
    }

    surf->in_use        = 1;
    surf->type          = 0; /* window */
    surf->native_window = (void *)win;
    surf->config        = (struct egl_config *)config;
    surf->width         = 0;
    surf->height        = 0;

    set_error(EGL_SUCCESS);
    return (EGLSurface)surf;
}

EGLSurface eglCreatePbufferSurface(EGLDisplay dpy, EGLConfig config,
                                    const EGLint *attrib_list)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_surface *surf;
    const EGLint *p;
    EGLint width = 0, height = 0;
    int i;

    if (!d) return EGL_NO_SURFACE;

    if (attrib_list) {
        for (p = attrib_list; p[0] != EGL_NONE; p += 2) {
            switch (p[0]) {
            case EGL_WIDTH:  width  = p[1]; break;
            case EGL_HEIGHT: height = p[1]; break;
            default: break;
            }
        }
    }

    /* Find a free surface slot */
    surf = NULL;
    for (i = 0; i < MAX_EGL_SURFACES; i++) {
        if (!d->surfaces[i].in_use) {
            surf = &d->surfaces[i];
            break;
        }
    }

    if (!surf) {
        set_error(EGL_BAD_ALLOC);
        return EGL_NO_SURFACE;
    }

    surf->in_use        = 1;
    surf->type          = 1; /* pbuffer */
    surf->native_window = NULL;
    surf->config        = (struct egl_config *)config;
    surf->width         = width;
    surf->height        = height;

    set_error(EGL_SUCCESS);
    return (EGLSurface)surf;
}

EGLBoolean eglDestroySurface(EGLDisplay dpy, EGLSurface surface)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_surface *s = (struct egl_surface *)surface;

    if (!d) return EGL_FALSE;

    if (!s || !s->in_use) {
        set_error(EGL_BAD_SURFACE);
        return EGL_FALSE;
    }

    if (g_current_draw == s) g_current_draw = NULL;
    if (g_current_read == s) g_current_read = NULL;

    s->in_use = 0;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* ========================================================================= */
/* Rendering control                                                         */
/* ========================================================================= */

EGLBoolean eglMakeCurrent(EGLDisplay dpy, EGLSurface draw,
                           EGLSurface read, EGLContext ctx)
{
    struct egl_display *d = get_initialized_display(dpy);

    if (!d) return EGL_FALSE;

    /* Allow unbinding with all NULL */
    if (ctx == EGL_NO_CONTEXT) {
        g_current_context = NULL;
        g_current_draw    = NULL;
        g_current_read    = NULL;
        set_error(EGL_SUCCESS);
        return EGL_TRUE;
    }

    g_current_context = (struct egl_context *)ctx;
    g_current_draw    = (struct egl_surface *)draw;
    g_current_read    = (struct egl_surface *)read;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglSwapBuffers(EGLDisplay dpy, EGLSurface surface)
{
    struct egl_display *d = get_initialized_display(dpy);

    if (!d) return EGL_FALSE;

    (void)surface;

    /*
     * In a full implementation, this would trigger a page flip via DRM
     * and swap the GBM surface buffers. For the shim, this is a no-op
     * that always succeeds.
     */

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglSwapInterval(EGLDisplay dpy, EGLint interval)
{
    struct egl_display *d = get_initialized_display(dpy);

    if (!d) return EGL_FALSE;

    (void)interval;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* ========================================================================= */
/* Query functions                                                           */
/* ========================================================================= */

EGLint eglGetError(void)
{
    EGLint err = g_egl_error;
    g_egl_error = EGL_SUCCESS;
    return err;
}

const char *eglQueryString(EGLDisplay dpy, EGLint name)
{
    /* EGL 1.5 allows EGL_NO_DISPLAY for EGL_EXTENSIONS and EGL_VERSION */
    if (dpy == EGL_NO_DISPLAY) {
        switch (name) {
        case EGL_EXTENSIONS:  return EGL_EXTENSIONS_STRING;
        case EGL_VERSION:     return EGL_VERSION_STRING;
        default:
            set_error(EGL_BAD_DISPLAY);
            return NULL;
        }
    }

    if (!get_initialized_display(dpy))
        return NULL;

    switch (name) {
    case EGL_VENDOR:      return EGL_VENDOR_STRING;
    case EGL_VERSION:     return EGL_VERSION_STRING;
    case EGL_EXTENSIONS:  return EGL_EXTENSIONS_STRING;
    case EGL_CLIENT_APIS: return EGL_CLIENT_APIS_STRING;
    default:
        set_error(EGL_BAD_PARAMETER);
        return NULL;
    }
}

EGLBoolean eglQuerySurface(EGLDisplay dpy, EGLSurface surface,
                            EGLint attribute, EGLint *value)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_surface *s = (struct egl_surface *)surface;

    if (!d) return EGL_FALSE;

    if (!s || !s->in_use || !value) {
        set_error(EGL_BAD_SURFACE);
        return EGL_FALSE;
    }

    switch (attribute) {
    case EGL_WIDTH:          *value = s->width;                    break;
    case EGL_HEIGHT:         *value = s->height;                   break;
    case EGL_CONFIG_ID:      *value = s->config ? s->config->id : 0; break;
    case EGL_RENDER_BUFFER:  *value = EGL_BACK_BUFFER;             break;
    case EGL_SWAP_BEHAVIOR:  *value = EGL_BUFFER_DESTROYED;        break;
    default:
        set_error(EGL_BAD_ATTRIBUTE);
        return EGL_FALSE;
    }

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglQueryContext(EGLDisplay dpy, EGLContext ctx,
                            EGLint attribute, EGLint *value)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_context *c = (struct egl_context *)ctx;

    if (!d) return EGL_FALSE;

    if (!c || !c->in_use || !value) {
        set_error(EGL_BAD_CONTEXT);
        return EGL_FALSE;
    }

    switch (attribute) {
    case EGL_CONFIG_ID:
        *value = c->config ? c->config->id : 0;
        break;
    case EGL_CONTEXT_CLIENT_VERSION:
        *value = c->client_version;
        break;
    case EGL_RENDER_BUFFER:
        *value = EGL_BACK_BUFFER;
        break;
    default:
        set_error(EGL_BAD_ATTRIBUTE);
        return EGL_FALSE;
    }

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* ========================================================================= */
/* API binding                                                               */
/* ========================================================================= */

EGLBoolean eglBindAPI(unsigned int api)
{
    if (api != EGL_OPENGL_ES_API && api != EGL_OPENGL_API &&
        api != EGL_OPENVG_API) {
        set_error(EGL_BAD_PARAMETER);
        return EGL_FALSE;
    }

    g_current_api = api;
    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

unsigned int eglQueryAPI(void)
{
    return g_current_api;
}

EGLBoolean eglWaitClient(void)
{
    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglWaitGL(void)
{
    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglWaitNative(EGLint engine)
{
    (void)engine;
    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLBoolean eglReleaseThread(void)
{
    g_current_context = NULL;
    g_current_draw    = NULL;
    g_current_read    = NULL;
    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* ========================================================================= */
/* Current context queries                                                   */
/* ========================================================================= */

EGLContext eglGetCurrentContext(void)
{
    return (EGLContext)g_current_context;
}

EGLSurface eglGetCurrentSurface(EGLint readdraw)
{
    if (readdraw == EGL_READ)
        return (EGLSurface)g_current_read;
    return (EGLSurface)g_current_draw;
}

EGLDisplay eglGetCurrentDisplay(void)
{
    if (!g_current_context)
        return EGL_NO_DISPLAY;
    return (EGLDisplay)&g_display;
}

/* ========================================================================= */
/* Proc address lookup                                                       */
/* ========================================================================= */

/* Forward declarations for extension functions */
static EGLImageKHR egl_create_image_khr(EGLDisplay, EGLContext,
                                         unsigned int, intptr_t,
                                         const EGLint *);
static EGLBoolean  egl_destroy_image_khr(EGLDisplay, EGLImageKHR);
static EGLBoolean  egl_swap_buffers_with_damage(EGLDisplay, EGLSurface,
                                                 const EGLint *, EGLint);

struct egl_proc_entry {
    const char *name;
    void       (*func)(void);
};

static const struct egl_proc_entry g_proc_table[] = {
    { "eglCreateImageKHR",            (void(*)(void))egl_create_image_khr },
    { "eglDestroyImageKHR",           (void(*)(void))egl_destroy_image_khr },
    { "eglSwapBuffersWithDamageEXT",  (void(*)(void))egl_swap_buffers_with_damage },
    { "eglGetPlatformDisplay",        (void(*)(void))eglGetPlatformDisplay },
    { NULL, NULL }
};

__eglMustCastToProperFunctionPointerType eglGetProcAddress(const char *procname)
{
    const struct egl_proc_entry *entry;

    if (!procname)
        return NULL;

    for (entry = g_proc_table; entry->name; entry++) {
        if (strcmp(entry->name, procname) == 0)
            return (__eglMustCastToProperFunctionPointerType)entry->func;
    }

    return NULL;
}

/* ========================================================================= */
/* EGL_KHR_image_base extension                                              */
/* ========================================================================= */

static EGLImageKHR egl_create_image_khr(EGLDisplay dpy, EGLContext ctx,
                                         unsigned int target,
                                         intptr_t buffer,
                                         const EGLint *attrib_list)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_image *img;
    int i;

    if (!d) return EGL_NO_IMAGE_KHR;

    (void)ctx;

    /* Find a free image slot */
    img = NULL;
    for (i = 0; i < MAX_EGL_IMAGES; i++) {
        if (!d->images[i].in_use) {
            img = &d->images[i];
            break;
        }
    }

    if (!img) {
        set_error(EGL_BAD_ALLOC);
        return EGL_NO_IMAGE_KHR;
    }

    img->in_use     = 1;
    img->target     = target;
    img->buffer     = buffer;
    img->dma_buf_fd = -1;
    img->width      = 0;
    img->height     = 0;
    img->fourcc     = 0;

    /* Parse DMA-BUF attributes if applicable */
    if (target == EGL_LINUX_DMA_BUF_EXT && attrib_list) {
        const EGLint *p;
        for (p = attrib_list; p[0] != EGL_NONE; p += 2) {
            switch (p[0]) {
            case EGL_WIDTH:                   img->width      = p[1]; break;
            case EGL_HEIGHT:                  img->height     = p[1]; break;
            case EGL_LINUX_DRM_FOURCC_EXT:    img->fourcc     = p[1]; break;
            case EGL_DMA_BUF_PLANE0_FD_EXT:   img->dma_buf_fd = p[1]; break;
            default: break;
            }
        }
    }

    set_error(EGL_SUCCESS);
    return (EGLImageKHR)img;
}

static EGLBoolean egl_destroy_image_khr(EGLDisplay dpy, EGLImageKHR image)
{
    struct egl_display *d = get_initialized_display(dpy);
    struct egl_image *img = (struct egl_image *)image;

    if (!d) return EGL_FALSE;

    if (!img || !img->in_use) {
        set_error(EGL_BAD_PARAMETER);
        return EGL_FALSE;
    }

    img->in_use = 0;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

/* Public wrappers for the extension functions */
EGLImageKHR eglCreateImageKHR(EGLDisplay dpy, EGLContext ctx,
                               unsigned int target,
                               intptr_t buffer,
                               const EGLint *attrib_list)
{
    return egl_create_image_khr(dpy, ctx, target, buffer, attrib_list);
}

EGLBoolean eglDestroyImageKHR(EGLDisplay dpy, EGLImageKHR image)
{
    return egl_destroy_image_khr(dpy, image);
}

/* ========================================================================= */
/* EGL_EXT_swap_buffers_with_damage                                          */
/* ========================================================================= */

static EGLBoolean egl_swap_buffers_with_damage(EGLDisplay dpy,
                                                EGLSurface surface,
                                                const EGLint *rects,
                                                EGLint n_rects)
{
    (void)rects;
    (void)n_rects;

    /* Damage tracking is a no-op; just do a regular swap */
    return eglSwapBuffers(dpy, surface);
}

EGLBoolean eglSwapBuffersWithDamageEXT(EGLDisplay dpy, EGLSurface surface,
                                        const EGLint *rects, EGLint n_rects)
{
    return egl_swap_buffers_with_damage(dpy, surface, rects, n_rects);
}

/* ========================================================================= */
/* EGL_KHR_fence_sync                                                        */
/* ========================================================================= */

EGLSyncKHR eglCreateSyncKHR(EGLDisplay dpy, unsigned int type,
                              const EGLint *attrib_list)
{
    struct egl_display *d = get_initialized_display(dpy);

    if (!d) return EGL_NO_SYNC_KHR;

    (void)type;
    (void)attrib_list;

    /* Return a non-NULL sentinel; sync is always "signaled" immediately */
    set_error(EGL_SUCCESS);
    return (EGLSyncKHR)(intptr_t)1;
}

EGLBoolean eglDestroySyncKHR(EGLDisplay dpy, EGLSyncKHR sync)
{
    struct egl_display *d = get_initialized_display(dpy);

    if (!d) return EGL_FALSE;

    (void)sync;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}

EGLint eglClientWaitSyncKHR(EGLDisplay dpy, EGLSyncKHR sync,
                              EGLint flags, EGLTimeKHR timeout)
{
    struct egl_display *d = get_initialized_display(dpy);

    if (!d) return EGL_FALSE;

    (void)sync;
    (void)flags;
    (void)timeout;

    /* Always immediately signaled */
    set_error(EGL_SUCCESS);
    return EGL_CONDITION_SATISFIED_KHR;
}

EGLint eglWaitSyncKHR(EGLDisplay dpy, EGLSyncKHR sync, EGLint flags)
{
    struct egl_display *d = get_initialized_display(dpy);

    if (!d) return EGL_FALSE;

    (void)sync;
    (void)flags;

    set_error(EGL_SUCCESS);
    return EGL_TRUE;
}
