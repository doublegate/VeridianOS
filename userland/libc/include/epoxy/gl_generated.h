/*
 * VeridianOS libc -- <epoxy/gl_generated.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Generated-style GL dispatch declarations for libepoxy.
 * On VeridianOS, epoxy_glFoo() maps directly to glFoo().
 */

#ifndef _EPOXY_GL_GENERATED_H
#define _EPOXY_GL_GENERATED_H

/*
 * libepoxy normally generates dispatch function pointers for every GL
 * entry point. On VeridianOS we link directly to the GLES2 shim, so
 * the epoxy_glFoo symbols are provided as aliases in libepoxy.c.
 * No additional declarations are needed beyond what GLES2/gl2.h and
 * GLES3/gl3.h already provide.
 */

#endif /* _EPOXY_GL_GENERATED_H */
