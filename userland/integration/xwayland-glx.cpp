/*
 * xwayland-glx.cpp -- GLX-over-EGL Translation Layer for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Translates GLX 1.4 API calls to EGL equivalents, enabling legacy
 * OpenGL X11 applications to render through XWayland on VeridianOS.
 *
 * Architecture:
 *   Application calls glXCreateContext()
 *     -> We call eglCreateContext() with translated attributes
 *     -> Return an opaque GLXContext wrapping the EGLContext
 *   Application calls glXMakeCurrent(drawable)
 *     -> We find/create EGLSurface for the drawable
 *     -> Call eglMakeCurrent()
 *   Application calls glXSwapBuffers()
 *     -> We call eglSwapBuffers()
 *   Application calls glXGetProcAddress("glFoo")
 *     -> We call eglGetProcAddress("glFoo")
 *
 * Limitations:
 *   - Color index mode not supported (RGBA only)
 *   - Accumulation buffers not supported
 *   - Stereo rendering not supported
 *   - Indirect rendering not supported (all contexts are direct)
 */

#include "xwayland-glx.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>

/* ======================================================================
 * Internal structures
 * ====================================================================== */

/* Maximum tracked contexts and surfaces */
#define GLX_MAX_CONTEXTS    256
#define GLX_MAX_SURFACES    512
#define GLX_MAX_FBCONFIGS   64

/* Internal GLX context record */
struct __GLXcontextRec {
    EGLContext  egl_context;
    EGLDisplay  egl_display;
    EGLConfig   egl_config;
    GLXContext  share_context;
    int         major_version;  /* Requested GL major version */
    int         minor_version;  /* Requested GL minor version */
    int         profile_mask;   /* Core or compatibility */
    int         context_flags;  /* Debug, forward-compat */
    bool        direct;
    bool        in_use;
};

/* Internal FBConfig record */
struct __GLXFBConfigRec {
    EGLConfig   egl_config;
    EGLDisplay  egl_display;
    int         fbconfig_id;
    int         red_size;
    int         green_size;
    int         blue_size;
    int         alpha_size;
    int         depth_size;
    int         stencil_size;
    int         samples;
    int         sample_buffers;
    int         buffer_size;
    int         render_type;
    int         drawable_type;
    int         visual_type;
    int         config_caveat;
    bool        double_buffer;
    bool        in_use;
};

/* Drawable-to-EGLSurface mapping */
typedef struct {
    GLXDrawable drawable;
    EGLSurface  egl_surface;
    EGLDisplay  egl_display;
    bool        in_use;
} glx_surface_map_t;

/* ======================================================================
 * Global state
 * ====================================================================== */

static struct {
    bool                initialized;
    EGLDisplay          egl_display;

    /* Context pool */
    struct __GLXcontextRec contexts[GLX_MAX_CONTEXTS];
    int                 context_count;

    /* FBConfig pool */
    struct __GLXFBConfigRec fbconfigs[GLX_MAX_FBCONFIGS];
    int                 fbconfig_count;

    /* Surface mapping */
    glx_surface_map_t   surfaces[GLX_MAX_SURFACES];
    int                 surface_count;

    /* Extension strings */
    char                client_extensions[1024];
    char                server_extensions[1024];
    char                combined_extensions[2048];

    /* Vendor string */
    char                vendor_string[128];
    char                version_string[128];
} g_glx_state;

/* Thread-local current context */
static __thread GLXContext  tls_current_context  = NULL;
static __thread GLXDrawable tls_current_drawable = 0;

/* Mutex for state modifications */
static pthread_mutex_t g_glx_mutex = PTHREAD_MUTEX_INITIALIZER;

/* ======================================================================
 * Internal helpers
 * ====================================================================== */

static void glx_ensure_initialized(void)
{
    if (g_glx_state.initialized) {
        return;
    }

    pthread_mutex_lock(&g_glx_mutex);
    if (g_glx_state.initialized) {
        pthread_mutex_unlock(&g_glx_mutex);
        return;
    }

    memset(&g_glx_state, 0, sizeof(g_glx_state));

    /*
     * On VeridianOS, EGL is initialized by the compositor (KWin).
     * We obtain the EGL display from the platform.  In this shim layer,
     * we use EGL_DEFAULT_DISPLAY as a placeholder; the real integration
     * would use the KWin-provided EGLDisplay.
     */
    /* g_glx_state.egl_display = eglGetDisplay(EGL_DEFAULT_DISPLAY); */
    g_glx_state.egl_display = NULL;  /* Set by platform init */

    /* Set up extension strings */
    snprintf(g_glx_state.client_extensions,
             sizeof(g_glx_state.client_extensions),
             "GLX_ARB_create_context "
             "GLX_ARB_create_context_profile "
             "GLX_ARB_get_proc_address "
             "GLX_EXT_visual_info "
             "GLX_EXT_visual_rating "
             "GLX_EXT_swap_control "
             "GLX_SGI_swap_control");

    snprintf(g_glx_state.server_extensions,
             sizeof(g_glx_state.server_extensions),
             "GLX_ARB_create_context "
             "GLX_ARB_create_context_profile "
             "GLX_ARB_multisample "
             "GLX_EXT_visual_info "
             "GLX_EXT_visual_rating");

    snprintf(g_glx_state.combined_extensions,
             sizeof(g_glx_state.combined_extensions),
             "%s", g_glx_state.client_extensions);

    snprintf(g_glx_state.vendor_string,
             sizeof(g_glx_state.vendor_string),
             "VeridianOS GLX-over-EGL");

    snprintf(g_glx_state.version_string,
             sizeof(g_glx_state.version_string),
             "1.4");

    g_glx_state.initialized = true;
    pthread_mutex_unlock(&g_glx_mutex);

    fprintf(stderr, "[glx] GLX-over-EGL layer initialized\n");
}

static struct __GLXcontextRec *glx_alloc_context(void)
{
    for (int i = 0; i < GLX_MAX_CONTEXTS; i++) {
        if (!g_glx_state.contexts[i].in_use) {
            memset(&g_glx_state.contexts[i], 0,
                   sizeof(struct __GLXcontextRec));
            g_glx_state.contexts[i].in_use = true;
            g_glx_state.context_count++;
            return &g_glx_state.contexts[i];
        }
    }
    return NULL;
}

static void glx_free_context(struct __GLXcontextRec *ctx)
{
    if (ctx && ctx->in_use) {
        ctx->in_use = false;
        g_glx_state.context_count--;
    }
}

static struct __GLXFBConfigRec *glx_alloc_fbconfig(void)
{
    for (int i = 0; i < GLX_MAX_FBCONFIGS; i++) {
        if (!g_glx_state.fbconfigs[i].in_use) {
            memset(&g_glx_state.fbconfigs[i], 0,
                   sizeof(struct __GLXFBConfigRec));
            g_glx_state.fbconfigs[i].in_use = true;
            g_glx_state.fbconfig_count++;
            return &g_glx_state.fbconfigs[i];
        }
    }
    return NULL;
}

static glx_surface_map_t *glx_find_surface(GLXDrawable drawable)
{
    for (int i = 0; i < GLX_MAX_SURFACES; i++) {
        if (g_glx_state.surfaces[i].in_use &&
            g_glx_state.surfaces[i].drawable == drawable) {
            return &g_glx_state.surfaces[i];
        }
    }
    return NULL;
}

static glx_surface_map_t *glx_create_surface(GLXDrawable drawable,
                                               EGLDisplay egl_dpy,
                                               EGLSurface egl_surf)
{
    for (int i = 0; i < GLX_MAX_SURFACES; i++) {
        if (!g_glx_state.surfaces[i].in_use) {
            g_glx_state.surfaces[i].drawable = drawable;
            g_glx_state.surfaces[i].egl_surface = egl_surf;
            g_glx_state.surfaces[i].egl_display = egl_dpy;
            g_glx_state.surfaces[i].in_use = true;
            g_glx_state.surface_count++;
            return &g_glx_state.surfaces[i];
        }
    }
    return NULL;
}

/*
 * Map GLX visual attribute list to EGL config attributes.
 * Parses the old-style glXChooseVisual attribute list (tokens
 * like GLX_RGBA, GLX_DOUBLEBUFFER as bare flags, others as
 * key-value pairs).
 */
static void glx_parse_visual_attribs(const int *attribs,
                                      int *red, int *green,
                                      int *blue, int *alpha,
                                      int *depth, int *stencil,
                                      bool *double_buf, bool *rgba)
{
    *red = 0;
    *green = 0;
    *blue = 0;
    *alpha = 0;
    *depth = 0;
    *stencil = 0;
    *double_buf = false;
    *rgba = false;

    if (!attribs) {
        return;
    }

    for (int i = 0; attribs[i] != 0 /* None */; ) {
        switch (attribs[i]) {
        case GLX_RGBA:
            *rgba = true;
            i++;
            break;
        case GLX_DOUBLEBUFFER:
            *double_buf = true;
            i++;
            break;
        case GLX_RED_SIZE:
            *red = attribs[i + 1];
            i += 2;
            break;
        case GLX_GREEN_SIZE:
            *green = attribs[i + 1];
            i += 2;
            break;
        case GLX_BLUE_SIZE:
            *blue = attribs[i + 1];
            i += 2;
            break;
        case GLX_ALPHA_SIZE:
            *alpha = attribs[i + 1];
            i += 2;
            break;
        case GLX_DEPTH_SIZE:
            *depth = attribs[i + 1];
            i += 2;
            break;
        case GLX_STENCIL_SIZE:
            *stencil = attribs[i + 1];
            i += 2;
            break;
        case GLX_BUFFER_SIZE:
        case GLX_LEVEL:
        case GLX_AUX_BUFFERS:
        case GLX_ACCUM_RED_SIZE:
        case GLX_ACCUM_GREEN_SIZE:
        case GLX_ACCUM_BLUE_SIZE:
        case GLX_ACCUM_ALPHA_SIZE:
            /* Skip unsupported attributes with values */
            i += 2;
            break;
        case GLX_STEREO:
        case GLX_USE_GL:
            /* Bare flag, skip */
            i++;
            break;
        default:
            /* Unknown attribute; try to skip key+value */
            i += 2;
            break;
        }
    }
}

/*
 * Map EGL error code to a descriptive string for logging.
 */
static const char *glx_egl_error_string(int error)
{
    switch (error) {
    case 0x3000: return "EGL_SUCCESS";
    case 0x3001: return "EGL_NOT_INITIALIZED";
    case 0x3002: return "EGL_BAD_ACCESS";
    case 0x3003: return "EGL_BAD_ALLOC";
    case 0x3004: return "EGL_BAD_ATTRIBUTE";
    case 0x3005: return "EGL_BAD_CONFIG";
    case 0x3006: return "EGL_BAD_CONTEXT";
    case 0x3007: return "EGL_BAD_CURRENT_SURFACE";
    case 0x3008: return "EGL_BAD_DISPLAY";
    case 0x3009: return "EGL_BAD_MATCH";
    case 0x300A: return "EGL_BAD_NATIVE_PIXMAP";
    case 0x300B: return "EGL_BAD_NATIVE_WINDOW";
    case 0x300C: return "EGL_BAD_PARAMETER";
    case 0x300D: return "EGL_BAD_SURFACE";
    default:     return "EGL_UNKNOWN";
    }
}

/* ======================================================================
 * GLX 1.4 core implementation
 * ====================================================================== */

XVisualInfo *glXChooseVisual(Display *dpy, int screen,
                             int *attrib_list)
{
    glx_ensure_initialized();
    (void)dpy;

    int red, green, blue, alpha, depth, stencil;
    bool double_buf, rgba;
    glx_parse_visual_attribs(attrib_list, &red, &green, &blue, &alpha,
                              &depth, &stencil, &double_buf, &rgba);

    /*
     * Build an XVisualInfo matching the requested attributes.
     * On VeridianOS, we always return a 32-bit RGBA TrueColor visual
     * since our EGL backend supports RGBA rendering.
     */
    XVisualInfo *vis = (XVisualInfo *)calloc(1, sizeof(XVisualInfo));
    if (!vis) {
        return NULL;
    }

    vis->visualid = 0x21;  /* Synthetic visual ID */
    vis->screen = screen;
    vis->depth = (alpha > 0) ? 32 : 24;
    vis->c_class = 4;      /* TrueColor */
    vis->red_mask   = 0x00FF0000;
    vis->green_mask = 0x0000FF00;
    vis->blue_mask  = 0x000000FF;
    vis->bits_per_rgb = 8;

    fprintf(stderr, "[glx] glXChooseVisual: RGBA=%d/%d/%d/%d "
            "depth=%d stencil=%d double=%d\n",
            red, green, blue, alpha, depth, stencil, double_buf);

    return vis;
}

GLXContext glXCreateContext(Display *dpy, XVisualInfo *vis,
                            GLXContext share_list, Bool direct)
{
    glx_ensure_initialized();
    (void)dpy;
    (void)vis;

    pthread_mutex_lock(&g_glx_mutex);

    struct __GLXcontextRec *ctx = glx_alloc_context();
    if (!ctx) {
        pthread_mutex_unlock(&g_glx_mutex);
        fprintf(stderr, "[glx] glXCreateContext: out of context slots\n");
        return NULL;
    }

    ctx->egl_display = g_glx_state.egl_display;
    ctx->share_context = share_list;
    ctx->direct = (direct != False);
    ctx->major_version = 2;
    ctx->minor_version = 1;
    ctx->profile_mask = GLX_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB;

    /*
     * Translate to EGL context creation:
     *   EGL_CONTEXT_CLIENT_VERSION = 2 (for GLES2 compat)
     *   Share group from share_list->egl_context
     *
     * In the real implementation:
     *   EGLContext share = share_list ? share_list->egl_context : EGL_NO_CONTEXT;
     *   EGLint attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
     *   ctx->egl_context = eglCreateContext(dpy, config, share, attribs);
     */
    ctx->egl_context = NULL;  /* Placeholder -- set by platform */

    pthread_mutex_unlock(&g_glx_mutex);

    fprintf(stderr, "[glx] glXCreateContext: created context %p "
            "(share=%p, direct=%d)\n",
            (void *)ctx, (void *)share_list, direct);

    return ctx;
}

void glXDestroyContext(Display *dpy, GLXContext ctx)
{
    if (!ctx) {
        return;
    }
    (void)dpy;

    pthread_mutex_lock(&g_glx_mutex);

    /* Unbind if this is the current context */
    if (tls_current_context == ctx) {
        tls_current_context = NULL;
        tls_current_drawable = 0;
        /*
         * eglMakeCurrent(ctx->egl_display, EGL_NO_SURFACE,
         *                EGL_NO_SURFACE, EGL_NO_CONTEXT);
         */
    }

    /*
     * eglDestroyContext(ctx->egl_display, ctx->egl_context);
     */

    fprintf(stderr, "[glx] glXDestroyContext: destroyed context %p\n",
            (void *)ctx);

    glx_free_context(ctx);

    pthread_mutex_unlock(&g_glx_mutex);
}

Bool glXMakeCurrent(Display *dpy, GLXDrawable drawable, GLXContext ctx)
{
    glx_ensure_initialized();
    (void)dpy;

    if (!ctx && drawable == 0) {
        /* Unbind current context */
        tls_current_context = NULL;
        tls_current_drawable = 0;
        /*
         * eglMakeCurrent(g_glx_state.egl_display,
         *                EGL_NO_SURFACE, EGL_NO_SURFACE,
         *                EGL_NO_CONTEXT);
         */
        return True;
    }

    if (!ctx) {
        return False;
    }

    /*
     * Find or create an EGL surface for the drawable.
     * On VeridianOS, the drawable (X11 Window XID) maps to
     * a Wayland surface via XWayland, and we create an
     * EGL window surface from the native window handle.
     */
    glx_surface_map_t *surf = glx_find_surface(drawable);
    if (!surf) {
        /*
         * Create a new EGL surface for this drawable.
         * In the real implementation:
         *   EGLSurface egl_surf = eglCreateWindowSurface(
         *       ctx->egl_display, ctx->egl_config,
         *       (EGLNativeWindowType)drawable, NULL);
         */
        EGLSurface egl_surf = NULL;  /* Placeholder */
        surf = glx_create_surface(drawable, ctx->egl_display, egl_surf);
        if (!surf) {
            fprintf(stderr, "[glx] glXMakeCurrent: "
                    "failed to create surface for drawable 0x%lx\n",
                    (unsigned long)drawable);
            return False;
        }
    }

    /*
     * eglMakeCurrent(ctx->egl_display, surf->egl_surface,
     *                surf->egl_surface, ctx->egl_context);
     */

    tls_current_context = ctx;
    tls_current_drawable = drawable;

    return True;
}

void glXSwapBuffers(Display *dpy, GLXDrawable drawable)
{
    (void)dpy;

    glx_surface_map_t *surf = glx_find_surface(drawable);
    if (!surf) {
        fprintf(stderr, "[glx] glXSwapBuffers: unknown drawable 0x%lx\n",
                (unsigned long)drawable);
        return;
    }

    /*
     * eglSwapBuffers(surf->egl_display, surf->egl_surface);
     */
    (void)surf;
}

Bool glXIsDirect(Display *dpy, GLXContext ctx)
{
    (void)dpy;
    /* All contexts on VeridianOS are direct (EGL-backed) */
    if (ctx) {
        return ctx->direct ? True : False;
    }
    return False;
}

GLXContext glXGetCurrentContext(void)
{
    return tls_current_context;
}

GLXDrawable glXGetCurrentDrawable(void)
{
    return tls_current_drawable;
}

/* ======================================================================
 * FBConfig functions
 * ====================================================================== */

GLXFBConfig *glXChooseFBConfig(Display *dpy, int screen,
                                const int *attrib_list,
                                int *nelements)
{
    glx_ensure_initialized();
    (void)dpy;
    (void)screen;

    /*
     * Parse the FBConfig attribute list and translate to EGL.
     * FBConfig attributes use key/value pairs (unlike the old
     * glXChooseVisual which uses bare flags).
     */
    int red = 0, green = 0, blue = 0, alpha = 0;
    int depth = 0, stencil = 0;
    int samples = 0, sample_buffers = 0;
    int render_type = GLX_RGBA_BIT;
    int drawable_type = GLX_WINDOW_BIT;
    bool double_buf = false;

    if (attrib_list) {
        for (int i = 0; attrib_list[i] != 0 /* None */; i += 2) {
            switch (attrib_list[i]) {
            case GLX_RED_SIZE:       red = attrib_list[i + 1]; break;
            case GLX_GREEN_SIZE:     green = attrib_list[i + 1]; break;
            case GLX_BLUE_SIZE:      blue = attrib_list[i + 1]; break;
            case GLX_ALPHA_SIZE:     alpha = attrib_list[i + 1]; break;
            case GLX_DEPTH_SIZE:     depth = attrib_list[i + 1]; break;
            case GLX_STENCIL_SIZE:   stencil = attrib_list[i + 1]; break;
            case GLX_SAMPLES:        samples = attrib_list[i + 1]; break;
            case GLX_SAMPLE_BUFFERS: sample_buffers = attrib_list[i + 1]; break;
            case GLX_RENDER_TYPE:    render_type = attrib_list[i + 1]; break;
            case GLX_DRAWABLE_TYPE:  drawable_type = attrib_list[i + 1]; break;
            case GLX_DOUBLEBUFFER:   double_buf = (attrib_list[i + 1] != 0); break;
            case GLX_X_RENDERABLE:
            case GLX_X_VISUAL_TYPE:
            case GLX_CONFIG_CAVEAT:
            case GLX_TRANSPARENT_TYPE:
                /* Noted but not filtered */
                break;
            default:
                break;
            }
        }
    }

    /*
     * Translate to eglChooseConfig():
     *   EGLint egl_attribs[] = {
     *       EGL_RED_SIZE, red,
     *       EGL_GREEN_SIZE, green,
     *       EGL_BLUE_SIZE, blue,
     *       EGL_ALPHA_SIZE, alpha,
     *       EGL_DEPTH_SIZE, depth,
     *       EGL_STENCIL_SIZE, stencil,
     *       EGL_SAMPLES, samples,
     *       EGL_SAMPLE_BUFFERS, sample_buffers,
     *       EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
     *       EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
     *       EGL_NONE
     *   };
     *   EGLConfig egl_configs[GLX_MAX_FBCONFIGS];
     *   EGLint num;
     *   eglChooseConfig(g_glx_state.egl_display, egl_attribs,
     *                   egl_configs, GLX_MAX_FBCONFIGS, &num);
     */

    /* Create a single matching FBConfig for now */
    pthread_mutex_lock(&g_glx_mutex);

    struct __GLXFBConfigRec *cfg = glx_alloc_fbconfig();
    if (!cfg) {
        pthread_mutex_unlock(&g_glx_mutex);
        if (nelements) *nelements = 0;
        return NULL;
    }

    static int next_fbconfig_id = 1;
    cfg->fbconfig_id = next_fbconfig_id++;
    cfg->egl_display = g_glx_state.egl_display;
    cfg->egl_config = NULL;  /* Set by platform */
    cfg->red_size = (red > 0) ? red : 8;
    cfg->green_size = (green > 0) ? green : 8;
    cfg->blue_size = (blue > 0) ? blue : 8;
    cfg->alpha_size = alpha;
    cfg->depth_size = depth;
    cfg->stencil_size = stencil;
    cfg->samples = samples;
    cfg->sample_buffers = sample_buffers;
    cfg->buffer_size = cfg->red_size + cfg->green_size +
                       cfg->blue_size + cfg->alpha_size;
    cfg->render_type = render_type;
    cfg->drawable_type = drawable_type;
    cfg->visual_type = GLX_TRUE_COLOR;
    cfg->config_caveat = GLX_NONE;
    cfg->double_buffer = double_buf;

    pthread_mutex_unlock(&g_glx_mutex);

    /* Return array of GLXFBConfig pointers */
    GLXFBConfig *result = (GLXFBConfig *)calloc(1, sizeof(GLXFBConfig));
    if (!result) {
        if (nelements) *nelements = 0;
        return NULL;
    }

    result[0] = cfg;
    if (nelements) *nelements = 1;

    fprintf(stderr, "[glx] glXChooseFBConfig: returning 1 config "
            "(RGBA=%d/%d/%d/%d depth=%d stencil=%d)\n",
            cfg->red_size, cfg->green_size, cfg->blue_size,
            cfg->alpha_size, cfg->depth_size, cfg->stencil_size);

    return result;
}

int glXGetFBConfigAttrib(Display *dpy, GLXFBConfig config,
                         int attribute, int *value)
{
    (void)dpy;

    if (!config || !value) {
        return -1;  /* GLXBadFBConfig */
    }

    switch (attribute) {
    case GLX_FBCONFIG_ID:     *value = config->fbconfig_id; break;
    case GLX_RED_SIZE:        *value = config->red_size; break;
    case GLX_GREEN_SIZE:      *value = config->green_size; break;
    case GLX_BLUE_SIZE:       *value = config->blue_size; break;
    case GLX_ALPHA_SIZE:      *value = config->alpha_size; break;
    case GLX_DEPTH_SIZE:      *value = config->depth_size; break;
    case GLX_STENCIL_SIZE:    *value = config->stencil_size; break;
    case GLX_BUFFER_SIZE:     *value = config->buffer_size; break;
    case GLX_DOUBLEBUFFER:    *value = config->double_buffer ? 1 : 0; break;
    case GLX_RENDER_TYPE:     *value = config->render_type; break;
    case GLX_DRAWABLE_TYPE:   *value = config->drawable_type; break;
    case GLX_X_VISUAL_TYPE:   *value = config->visual_type; break;
    case GLX_CONFIG_CAVEAT:   *value = config->config_caveat; break;
    case GLX_X_RENDERABLE:    *value = 1; break;
    case GLX_SAMPLES:         *value = config->samples; break;
    case GLX_SAMPLE_BUFFERS:  *value = config->sample_buffers; break;
    case GLX_TRANSPARENT_TYPE: *value = GLX_NONE; break;
    case GLX_STEREO:          *value = 0; break;
    case GLX_AUX_BUFFERS:     *value = 0; break;
    case GLX_LEVEL:           *value = 0; break;
    default:
        fprintf(stderr, "[glx] glXGetFBConfigAttrib: "
                "unknown attribute 0x%x\n", attribute);
        return -1;
    }

    return 0;
}

XVisualInfo *glXGetVisualFromFBConfig(Display *dpy, GLXFBConfig config)
{
    (void)dpy;

    if (!config) {
        return NULL;
    }

    XVisualInfo *vis = (XVisualInfo *)calloc(1, sizeof(XVisualInfo));
    if (!vis) {
        return NULL;
    }

    vis->visualid = 0x21;
    vis->screen = 0;
    vis->depth = (config->alpha_size > 0) ? 32 : 24;
    vis->c_class = 4;  /* TrueColor */
    vis->red_mask   = 0x00FF0000;
    vis->green_mask = 0x0000FF00;
    vis->blue_mask  = 0x000000FF;
    vis->bits_per_rgb = 8;

    return vis;
}

GLXContext glXCreateNewContext(Display *dpy, GLXFBConfig config,
                               int render_type,
                               GLXContext share_list, Bool direct)
{
    glx_ensure_initialized();
    (void)dpy;
    (void)render_type;

    pthread_mutex_lock(&g_glx_mutex);

    struct __GLXcontextRec *ctx = glx_alloc_context();
    if (!ctx) {
        pthread_mutex_unlock(&g_glx_mutex);
        fprintf(stderr, "[glx] glXCreateNewContext: "
                "out of context slots\n");
        return NULL;
    }

    ctx->egl_display = g_glx_state.egl_display;
    ctx->egl_config = config ? config->egl_config : NULL;
    ctx->share_context = share_list;
    ctx->direct = (direct != False);
    ctx->major_version = 2;
    ctx->minor_version = 1;
    ctx->profile_mask = GLX_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB;

    /*
     * EGLContext share = share_list ? share_list->egl_context
     *                               : EGL_NO_CONTEXT;
     * EGLint attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
     * ctx->egl_context = eglCreateContext(
     *     ctx->egl_display, ctx->egl_config, share, attribs);
     */
    ctx->egl_context = NULL;

    pthread_mutex_unlock(&g_glx_mutex);

    fprintf(stderr, "[glx] glXCreateNewContext: created context %p\n",
            (void *)ctx);

    return ctx;
}

GLXWindow glXCreateWindow(Display *dpy, GLXFBConfig config,
                           Window win, const int *attrib_list)
{
    glx_ensure_initialized();
    (void)dpy;
    (void)config;
    (void)attrib_list;

    /*
     * GLXWindow is an XID alias for the window.  On VeridianOS,
     * the EGL surface is created lazily in glXMakeCurrent().
     * We just return the X11 Window XID as the GLXWindow.
     */
    fprintf(stderr, "[glx] glXCreateWindow: window 0x%lx\n",
            (unsigned long)win);

    return (GLXWindow)win;
}

void glXDestroyWindow(Display *dpy, GLXWindow win)
{
    (void)dpy;

    /* Remove the surface mapping if it exists */
    pthread_mutex_lock(&g_glx_mutex);

    glx_surface_map_t *surf = glx_find_surface((GLXDrawable)win);
    if (surf) {
        /*
         * eglDestroySurface(surf->egl_display, surf->egl_surface);
         */
        surf->in_use = false;
        g_glx_state.surface_count--;
    }

    pthread_mutex_unlock(&g_glx_mutex);

    fprintf(stderr, "[glx] glXDestroyWindow: window 0x%lx\n",
            (unsigned long)win);
}

/* ======================================================================
 * Extension and version queries
 * ====================================================================== */

/* String name constants for glXGetClientString / glXQueryServerString */
#define GLX_VENDOR      1
#define GLX_VERSION     2
#define GLX_EXTENSIONS  3

Bool glXQueryExtension(Display *dpy, int *error_base, int *event_base)
{
    glx_ensure_initialized();
    (void)dpy;

    /* GLX uses X11 extension error/event base offsets */
    if (error_base) *error_base = 0;
    if (event_base) *event_base = 0;

    return True;
}

Bool glXQueryVersion(Display *dpy, int *major, int *minor)
{
    glx_ensure_initialized();
    (void)dpy;

    if (major) *major = 1;
    if (minor) *minor = 4;

    return True;
}

const char *glXGetClientString(Display *dpy, int name)
{
    glx_ensure_initialized();
    (void)dpy;

    switch (name) {
    case GLX_VENDOR:     return g_glx_state.vendor_string;
    case GLX_VERSION:    return g_glx_state.version_string;
    case GLX_EXTENSIONS: return g_glx_state.client_extensions;
    default:             return NULL;
    }
}

const char *glXQueryServerString(Display *dpy, int screen, int name)
{
    glx_ensure_initialized();
    (void)dpy;
    (void)screen;

    switch (name) {
    case GLX_VENDOR:     return g_glx_state.vendor_string;
    case GLX_VERSION:    return g_glx_state.version_string;
    case GLX_EXTENSIONS: return g_glx_state.server_extensions;
    default:             return NULL;
    }
}

const char *glXQueryExtensionsString(Display *dpy, int screen)
{
    glx_ensure_initialized();
    (void)dpy;
    (void)screen;

    return g_glx_state.combined_extensions;
}

/* ======================================================================
 * Procedure address lookup
 * ====================================================================== */

/*
 * GLX function name -> function pointer mapping table.
 * Used by glXGetProcAddress to return addresses of GLX functions
 * that the application might query at runtime.
 */
typedef struct {
    const char     *name;
    __GLXextFuncPtr func;
} glx_proc_entry_t;

static const glx_proc_entry_t glx_proc_table[] = {
    { "glXChooseVisual",          (__GLXextFuncPtr)glXChooseVisual },
    { "glXCreateContext",         (__GLXextFuncPtr)glXCreateContext },
    { "glXDestroyContext",        (__GLXextFuncPtr)glXDestroyContext },
    { "glXMakeCurrent",           (__GLXextFuncPtr)glXMakeCurrent },
    { "glXSwapBuffers",           (__GLXextFuncPtr)glXSwapBuffers },
    { "glXIsDirect",              (__GLXextFuncPtr)glXIsDirect },
    { "glXGetCurrentContext",     (__GLXextFuncPtr)glXGetCurrentContext },
    { "glXGetCurrentDrawable",    (__GLXextFuncPtr)glXGetCurrentDrawable },
    { "glXChooseFBConfig",        (__GLXextFuncPtr)glXChooseFBConfig },
    { "glXGetFBConfigAttrib",     (__GLXextFuncPtr)glXGetFBConfigAttrib },
    { "glXGetVisualFromFBConfig", (__GLXextFuncPtr)glXGetVisualFromFBConfig },
    { "glXCreateNewContext",      (__GLXextFuncPtr)glXCreateNewContext },
    { "glXCreateWindow",          (__GLXextFuncPtr)glXCreateWindow },
    { "glXDestroyWindow",         (__GLXextFuncPtr)glXDestroyWindow },
    { "glXQueryExtension",        (__GLXextFuncPtr)glXQueryExtension },
    { "glXQueryVersion",          (__GLXextFuncPtr)glXQueryVersion },
    { "glXGetClientString",       (__GLXextFuncPtr)glXGetClientString },
    { "glXQueryServerString",     (__GLXextFuncPtr)glXQueryServerString },
    { "glXQueryExtensionsString", (__GLXextFuncPtr)glXQueryExtensionsString },
    { "glXGetProcAddress",        (__GLXextFuncPtr)glXGetProcAddress },
    { "glXGetProcAddressARB",     (__GLXextFuncPtr)glXGetProcAddressARB },
    { "glXCreateContextAttribsARB",
                                  (__GLXextFuncPtr)glXCreateContextAttribsARB },
    { NULL, NULL }
};

__GLXextFuncPtr glXGetProcAddress(const unsigned char *proc_name)
{
    glx_ensure_initialized();

    if (!proc_name) {
        return NULL;
    }

    const char *name = (const char *)proc_name;

    /* First check the GLX function table */
    for (int i = 0; glx_proc_table[i].name != NULL; i++) {
        if (strcmp(name, glx_proc_table[i].name) == 0) {
            return glx_proc_table[i].func;
        }
    }

    /*
     * Fall through to EGL for GL function addresses:
     *   return (__GLXextFuncPtr)eglGetProcAddress(name);
     */

    fprintf(stderr, "[glx] glXGetProcAddress: '%s' -> NULL "
            "(not in GLX table, would query EGL)\n", name);

    return NULL;
}

__GLXextFuncPtr glXGetProcAddressARB(const unsigned char *proc_name)
{
    /* ARB variant is identical to core */
    return glXGetProcAddress(proc_name);
}

/* ======================================================================
 * GLX_ARB_create_context
 * ====================================================================== */

GLXContext glXCreateContextAttribsARB(Display *dpy,
                                       GLXFBConfig config,
                                       GLXContext share_context,
                                       Bool direct,
                                       const int *attrib_list)
{
    glx_ensure_initialized();
    (void)dpy;

    int major = 2, minor = 1;
    int flags = 0;
    int profile = GLX_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB;

    /* Parse the attribute list */
    if (attrib_list) {
        for (int i = 0; attrib_list[i] != 0 /* None */; i += 2) {
            switch (attrib_list[i]) {
            case GLX_CONTEXT_MAJOR_VERSION_ARB:
                major = attrib_list[i + 1];
                break;
            case GLX_CONTEXT_MINOR_VERSION_ARB:
                minor = attrib_list[i + 1];
                break;
            case GLX_CONTEXT_FLAGS_ARB:
                flags = attrib_list[i + 1];
                break;
            case GLX_CONTEXT_PROFILE_MASK_ARB:
                profile = attrib_list[i + 1];
                break;
            default:
                fprintf(stderr, "[glx] glXCreateContextAttribsARB: "
                        "unknown attrib 0x%x\n", attrib_list[i]);
                break;
            }
        }
    }

    pthread_mutex_lock(&g_glx_mutex);

    struct __GLXcontextRec *ctx = glx_alloc_context();
    if (!ctx) {
        pthread_mutex_unlock(&g_glx_mutex);
        fprintf(stderr, "[glx] glXCreateContextAttribsARB: "
                "out of context slots\n");
        return NULL;
    }

    ctx->egl_display = g_glx_state.egl_display;
    ctx->egl_config = config ? config->egl_config : NULL;
    ctx->share_context = share_context;
    ctx->direct = (direct != False);
    ctx->major_version = major;
    ctx->minor_version = minor;
    ctx->context_flags = flags;
    ctx->profile_mask = profile;

    /*
     * Translate to EGL:
     *   EGLint egl_attribs[] = {
     *       EGL_CONTEXT_MAJOR_VERSION, major,
     *       EGL_CONTEXT_MINOR_VERSION, minor,
     *       EGL_NONE
     *   };
     *   EGLContext share = share_context ? share_context->egl_context
     *                                    : EGL_NO_CONTEXT;
     *   ctx->egl_context = eglCreateContext(
     *       ctx->egl_display, ctx->egl_config, share, egl_attribs);
     */
    ctx->egl_context = NULL;

    pthread_mutex_unlock(&g_glx_mutex);

    fprintf(stderr, "[glx] glXCreateContextAttribsARB: "
            "GL %d.%d %s%s (flags=0x%x)\n",
            major, minor,
            (profile & GLX_CONTEXT_CORE_PROFILE_BIT_ARB)
                ? "core" : "compat",
            (flags & GLX_CONTEXT_DEBUG_BIT_ARB) ? " debug" : "",
            flags);

    return ctx;
}
