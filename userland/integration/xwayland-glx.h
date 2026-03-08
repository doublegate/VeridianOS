/*
 * xwayland-glx.h -- GLX API for VeridianOS XWayland
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * GLX 1.4 core API + GLX_ARB_create_context extension.
 * This header provides the GLX interface that X11 applications expect,
 * backed by EGL on VeridianOS.  The implementation translates GLX calls
 * to EGL equivalents, allowing legacy OpenGL applications to render
 * through XWayland without requiring a native GLX server extension.
 *
 * Supported:
 *   - GLX 1.4 core (contexts, visuals, FBConfigs, windows, swap)
 *   - GLX_ARB_create_context (versioned context creation)
 *   - Extension string queries
 *   - Procedure address lookup (glXGetProcAddress)
 */

#ifndef XWAYLAND_GLX_H
#define XWAYLAND_GLX_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ======================================================================
 * X11/EGL stub types (avoid pulling in full X11/EGL headers)
 * ====================================================================== */

/* X11 types */
typedef unsigned long XID;
typedef struct _XDisplay Display;
typedef XID Window;
typedef XID Pixmap;
typedef unsigned long VisualID;

/* Minimal XVisualInfo for GLX */
typedef struct {
    void       *visual;
    VisualID    visualid;
    int         screen;
    int         depth;
    int         c_class;       /* X11 visual class */
    unsigned long red_mask;
    unsigned long green_mask;
    unsigned long blue_mask;
    int         colormap_size;
    int         bits_per_rgb;
} XVisualInfo;

/* Bool type for X11 compatibility */
#ifndef Bool
typedef int Bool;
#endif
#ifndef True
#define True  1
#define False 0
#endif

/* EGL forward declarations */
typedef void *EGLDisplay;
typedef void *EGLContext;
typedef void *EGLSurface;
typedef void *EGLConfig;

/* ======================================================================
 * GLX types
 * ====================================================================== */

typedef XID          GLXDrawable;
typedef XID          GLXWindow;
typedef XID          GLXPixmap;
typedef struct __GLXcontextRec *GLXContext;
typedef struct __GLXFBConfigRec *GLXFBConfig;

/* Function pointer type for glXGetProcAddress */
typedef void (*__GLXextFuncPtr)(void);

/* ======================================================================
 * GLX constants
 * ====================================================================== */

/* Visual attributes for glXChooseVisual / glXGetConfig */
#define GLX_USE_GL              1
#define GLX_BUFFER_SIZE         2
#define GLX_LEVEL               3
#define GLX_RGBA                4
#define GLX_DOUBLEBUFFER        5
#define GLX_STEREO              6
#define GLX_AUX_BUFFERS         7
#define GLX_RED_SIZE            8
#define GLX_GREEN_SIZE          9
#define GLX_BLUE_SIZE           10
#define GLX_ALPHA_SIZE          11
#define GLX_DEPTH_SIZE          12
#define GLX_STENCIL_SIZE        13
#define GLX_ACCUM_RED_SIZE      14
#define GLX_ACCUM_GREEN_SIZE    15
#define GLX_ACCUM_BLUE_SIZE     16
#define GLX_ACCUM_ALPHA_SIZE    17

/* FBConfig attributes */
#define GLX_X_RENDERABLE        0x8012
#define GLX_FBCONFIG_ID         0x8013
#define GLX_RENDER_TYPE         0x8011
#define GLX_DRAWABLE_TYPE       0x8010
#define GLX_X_VISUAL_TYPE       0x22
#define GLX_CONFIG_CAVEAT       0x20
#define GLX_TRANSPARENT_TYPE    0x23
#define GLX_SAMPLES             100001
#define GLX_SAMPLE_BUFFERS      100000

/* GLX_RENDER_TYPE bits */
#define GLX_RGBA_BIT            0x0001
#define GLX_COLOR_INDEX_BIT     0x0002

/* GLX_DRAWABLE_TYPE bits */
#define GLX_WINDOW_BIT          0x0001
#define GLX_PIXMAP_BIT          0x0002
#define GLX_PBUFFER_BIT         0x0004

/* GLX_CONFIG_CAVEAT values */
#define GLX_NONE                0x8000
#define GLX_SLOW_CONFIG         0x8001
#define GLX_NON_CONFORMANT_CONFIG 0x800D

/* GLX_X_VISUAL_TYPE values */
#define GLX_TRUE_COLOR          0x8002
#define GLX_DIRECT_COLOR        0x8003

/* GLX_TRANSPARENT_TYPE values */
#define GLX_TRANSPARENT_RGB     0x8008
#define GLX_TRANSPARENT_INDEX   0x8009

/* GLX_ARB_create_context attributes */
#define GLX_CONTEXT_MAJOR_VERSION_ARB             0x2091
#define GLX_CONTEXT_MINOR_VERSION_ARB             0x2092
#define GLX_CONTEXT_FLAGS_ARB                     0x2094
#define GLX_CONTEXT_PROFILE_MASK_ARB              0x9126

/* GLX_CONTEXT_FLAGS_ARB bits */
#define GLX_CONTEXT_DEBUG_BIT_ARB                 0x0001
#define GLX_CONTEXT_FORWARD_COMPATIBLE_BIT_ARB    0x0002

/* GLX_CONTEXT_PROFILE_MASK_ARB bits */
#define GLX_CONTEXT_CORE_PROFILE_BIT_ARB          0x0001
#define GLX_CONTEXT_COMPATIBILITY_PROFILE_BIT_ARB 0x0002

/* ======================================================================
 * GLX 1.4 core functions
 * ====================================================================== */

/*
 * Choose a visual matching the desired attributes.
 * attrib_list is a list of attribute/value pairs terminated by None (0).
 * Returns a pointer to an XVisualInfo (caller must XFree), or NULL.
 */
XVisualInfo *glXChooseVisual(Display *dpy, int screen,
                             int *attrib_list);

/*
 * Create a GLX rendering context.
 * share_list: context to share display lists with, or NULL.
 * direct: True for direct rendering (always True on VeridianOS).
 */
GLXContext glXCreateContext(Display *dpy, XVisualInfo *vis,
                            GLXContext share_list, Bool direct);

/*
 * Destroy a GLX rendering context.
 */
void glXDestroyContext(Display *dpy, GLXContext ctx);

/*
 * Make a GLX context current for the calling thread.
 * drawable: the GLX drawable to render to.
 * Returns True on success, False on error.
 */
Bool glXMakeCurrent(Display *dpy, GLXDrawable drawable,
                    GLXContext ctx);

/*
 * Swap front and back buffers for a double-buffered drawable.
 */
void glXSwapBuffers(Display *dpy, GLXDrawable drawable);

/*
 * Check if a context uses direct rendering.
 * Always returns True on VeridianOS (EGL backend).
 */
Bool glXIsDirect(Display *dpy, GLXContext ctx);

/*
 * Return the current GLX context for the calling thread.
 * Returns NULL if no context is current.
 */
GLXContext glXGetCurrentContext(void);

/*
 * Return the current GLX drawable for the calling thread.
 * Returns None if no context is current.
 */
GLXDrawable glXGetCurrentDrawable(void);

/* ======================================================================
 * GLX 1.3+ FBConfig functions
 * ====================================================================== */

/*
 * Return a list of FBConfigs matching the given attributes.
 * attrib_list: attribute/value pairs terminated by None.
 * nelements: receives the number of returned configs.
 * Caller must XFree the returned array.
 */
GLXFBConfig *glXChooseFBConfig(Display *dpy, int screen,
                                const int *attrib_list,
                                int *nelements);

/*
 * Query an attribute value from an FBConfig.
 * Returns 0 on success.
 */
int glXGetFBConfigAttrib(Display *dpy, GLXFBConfig config,
                         int attribute, int *value);

/*
 * Return the XVisualInfo associated with an FBConfig.
 * Caller must XFree the returned XVisualInfo.
 */
XVisualInfo *glXGetVisualFromFBConfig(Display *dpy,
                                       GLXFBConfig config);

/*
 * Create a new GLX context with an FBConfig.
 */
GLXContext glXCreateNewContext(Display *dpy, GLXFBConfig config,
                               int render_type,
                               GLXContext share_list,
                               Bool direct);

/*
 * Create a GLX window from an X11 Window and FBConfig.
 */
GLXWindow glXCreateWindow(Display *dpy, GLXFBConfig config,
                           Window win, const int *attrib_list);

/*
 * Destroy a GLX window.
 */
void glXDestroyWindow(Display *dpy, GLXWindow win);

/* ======================================================================
 * Extension and version queries
 * ====================================================================== */

/*
 * Query whether GLX is supported.
 * Sets *error_base and *event_base.
 * Returns True if GLX is supported.
 */
Bool glXQueryExtension(Display *dpy, int *error_base,
                       int *event_base);

/*
 * Query the GLX version.
 * Returns True on success; *major and *minor set to version numbers.
 */
Bool glXQueryVersion(Display *dpy, int *major, int *minor);

/*
 * Return the client-side GLX extension string.
 */
const char *glXGetClientString(Display *dpy, int name);

/*
 * Return the server-side GLX extension string for a screen.
 */
const char *glXQueryServerString(Display *dpy, int screen,
                                  int name);

/*
 * Return the combined GLX extension string for a screen.
 */
const char *glXQueryExtensionsString(Display *dpy, int screen);

/* ======================================================================
 * Procedure address lookup
 * ====================================================================== */

/*
 * Return the address of an OpenGL or GLX extension function.
 * proc_name: the function name to look up.
 * Returns NULL if not found.
 */
__GLXextFuncPtr glXGetProcAddress(const unsigned char *proc_name);

/*
 * ARB variant of glXGetProcAddress (identical behavior).
 */
__GLXextFuncPtr glXGetProcAddressARB(const unsigned char *proc_name);

/* ======================================================================
 * GLX_ARB_create_context
 * ====================================================================== */

/*
 * Create a GLX context with explicit version and profile attributes.
 * attrib_list: GLX_CONTEXT_MAJOR_VERSION_ARB, etc., terminated by None.
 */
GLXContext glXCreateContextAttribsARB(Display *dpy,
                                       GLXFBConfig config,
                                       GLXContext share_context,
                                       Bool direct,
                                       const int *attrib_list);

#ifdef __cplusplus
}
#endif

#endif /* XWAYLAND_GLX_H */
