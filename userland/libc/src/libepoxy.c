/*
 * VeridianOS libc -- libepoxy.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libepoxy GL/EGL function dispatch implementation.
 * On VeridianOS, this is a thin layer over the native GLES2 and EGL shims.
 * Provides version queries, extension checks, and dispatch functions.
 */

#include <epoxy/gl.h>
#include <epoxy/egl.h>
#include <string.h>

/* ========================================================================= */
/* GL version and capability queries                                         */
/* ========================================================================= */

EPOXY_PUBLIC int epoxy_gl_version(void)
{
    /*
     * Returns the GL version as (major * 10 + minor).
     * VeridianOS provides OpenGL ES 2.0 -> return 20.
     */
    return 20;
}

EPOXY_PUBLIC bool epoxy_is_desktop_gl(void)
{
    /*
     * VeridianOS provides OpenGL ES, not desktop GL.
     */
    return false;
}

EPOXY_PUBLIC bool epoxy_has_gl_extension(const char *extension)
{
    const char *ext_string;
    const char *found;
    size_t len;

    if (!extension)
        return false;

    ext_string = (const char *)glGetString(GL_EXTENSIONS);
    if (!ext_string)
        return false;

    len = strlen(extension);

    found = ext_string;
    while ((found = strstr(found, extension)) != NULL) {
        /* Verify it's a complete match (not a substring of another ext) */
        if ((found == ext_string || found[-1] == ' ') &&
            (found[len] == ' ' || found[len] == '\0'))
            return true;
        found += len;
    }

    return false;
}

/* ========================================================================= */
/* EGL version and extension queries                                         */
/* ========================================================================= */

EPOXY_PUBLIC int epoxy_egl_version(EGLDisplay dpy)
{
    /*
     * Returns the EGL version as (major * 10 + minor).
     * VeridianOS provides EGL 1.5 -> return 15.
     */
    (void)dpy;
    return 15;
}

EPOXY_PUBLIC bool epoxy_has_egl_extension(EGLDisplay dpy, const char *extension)
{
    const char *ext_string;
    const char *found;
    size_t len;

    if (!extension)
        return false;

    ext_string = eglQueryString(dpy, EGL_EXTENSIONS);
    if (!ext_string)
        return false;

    len = strlen(extension);

    found = ext_string;
    while ((found = strstr(found, extension)) != NULL) {
        if ((found == ext_string || found[-1] == ' ') &&
            (found[len] == ' ' || found[len] == '\0'))
            return true;
        found += len;
    }

    return false;
}
