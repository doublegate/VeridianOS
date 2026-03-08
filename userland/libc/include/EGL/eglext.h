/*
 * VeridianOS libc -- <EGL/eglext.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * EGL extension declarations.
 * Provides KHR, EXT, and MESA extensions used by Mesa, Qt, and KWin.
 */

#ifndef _EGL_EGLEXT_H
#define _EGL_EGLEXT_H

#ifdef __cplusplus
extern "C" {
#endif

#include <EGL/egl.h>

/* ========================================================================= */
/* EGL_KHR_image_base                                                        */
/* ========================================================================= */

#ifndef EGL_KHR_image_base
#define EGL_KHR_image_base 1

typedef void *EGLImageKHR;

#define EGL_IMAGE_PRESERVED_KHR         0x30D2
#define EGL_NO_IMAGE_KHR                ((EGLImageKHR)0)

EGLImageKHR eglCreateImageKHR(EGLDisplay dpy, EGLContext ctx,
                               unsigned int target,
                               intptr_t buffer,
                               const EGLint *attrib_list);
EGLBoolean  eglDestroyImageKHR(EGLDisplay dpy, EGLImageKHR image);

#endif /* EGL_KHR_image_base */

/* ========================================================================= */
/* EGL_KHR_image (depends on EGL_KHR_image_base)                             */
/* ========================================================================= */

#ifndef EGL_KHR_image
#define EGL_KHR_image 1
/* Same types and functions as EGL_KHR_image_base */
#endif

/* ========================================================================= */
/* EGL_KHR_gl_renderbuffer_image                                             */
/* ========================================================================= */

#ifndef EGL_KHR_gl_renderbuffer_image
#define EGL_KHR_gl_renderbuffer_image 1

#define EGL_GL_RENDERBUFFER_KHR         0x30B9

#endif

/* ========================================================================= */
/* EGL_KHR_gl_texture_2D_image                                               */
/* ========================================================================= */

#ifndef EGL_KHR_gl_texture_2D_image
#define EGL_KHR_gl_texture_2D_image 1

#define EGL_GL_TEXTURE_2D_KHR           0x30B1
#define EGL_GL_TEXTURE_LEVEL_KHR        0x30BC

#endif

/* ========================================================================= */
/* EGL_KHR_platform_wayland                                                  */
/* ========================================================================= */

#ifndef EGL_KHR_platform_wayland
#define EGL_KHR_platform_wayland 1

/* EGL_PLATFORM_WAYLAND_KHR defined in egl.h */

#endif

/* ========================================================================= */
/* EGL_MESA_platform_gbm                                                     */
/* ========================================================================= */

#ifndef EGL_MESA_platform_gbm
#define EGL_MESA_platform_gbm 1

/* EGL_PLATFORM_GBM_MESA defined in egl.h */

#endif

/* ========================================================================= */
/* EGL_EXT_image_dma_buf_import                                              */
/* ========================================================================= */

#ifndef EGL_EXT_image_dma_buf_import
#define EGL_EXT_image_dma_buf_import 1

#define EGL_LINUX_DMA_BUF_EXT           0x3270
#define EGL_LINUX_DRM_FOURCC_EXT        0x3271
#define EGL_DMA_BUF_PLANE0_FD_EXT       0x3272
#define EGL_DMA_BUF_PLANE0_OFFSET_EXT   0x3273
#define EGL_DMA_BUF_PLANE0_PITCH_EXT    0x3274
#define EGL_DMA_BUF_PLANE1_FD_EXT       0x3275
#define EGL_DMA_BUF_PLANE1_OFFSET_EXT   0x3276
#define EGL_DMA_BUF_PLANE1_PITCH_EXT    0x3277
#define EGL_DMA_BUF_PLANE2_FD_EXT       0x3278
#define EGL_DMA_BUF_PLANE2_OFFSET_EXT   0x3279
#define EGL_DMA_BUF_PLANE2_PITCH_EXT    0x327A

#endif

/* ========================================================================= */
/* EGL_EXT_image_dma_buf_import_modifiers                                    */
/* ========================================================================= */

#ifndef EGL_EXT_image_dma_buf_import_modifiers
#define EGL_EXT_image_dma_buf_import_modifiers 1

#define EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT 0x3443
#define EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT 0x3444
#define EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT 0x3445
#define EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT 0x3446
#define EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT 0x3447
#define EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT 0x3448
#define EGL_DMA_BUF_PLANE3_FD_EXT          0x3440
#define EGL_DMA_BUF_PLANE3_OFFSET_EXT      0x3441
#define EGL_DMA_BUF_PLANE3_PITCH_EXT       0x3442
#define EGL_DMA_BUF_PLANE3_MODIFIER_LO_EXT 0x3449
#define EGL_DMA_BUF_PLANE3_MODIFIER_HI_EXT 0x344A

#endif

/* ========================================================================= */
/* EGL_KHR_fence_sync                                                        */
/* ========================================================================= */

#ifndef EGL_KHR_fence_sync
#define EGL_KHR_fence_sync 1

typedef void *EGLSyncKHR;
typedef khronos_int64_t EGLTimeKHR;

#define EGL_SYNC_FENCE_KHR              0x30F9
#define EGL_SYNC_CONDITION_KHR          0x30F8
#define EGL_SYNC_PRIOR_COMMANDS_COMPLETE_KHR 0x30F0
#define EGL_SYNC_STATUS_KHR             0x30F1
#define EGL_SIGNALED_KHR                0x30F2
#define EGL_UNSIGNALED_KHR              0x30F3
#define EGL_TIMEOUT_EXPIRED_KHR         0x30F5
#define EGL_CONDITION_SATISFIED_KHR     0x30F6
#define EGL_FOREVER_KHR                 0xFFFFFFFFFFFFFFFFull
#define EGL_NO_SYNC_KHR                 ((EGLSyncKHR)0)

EGLSyncKHR eglCreateSyncKHR(EGLDisplay dpy, unsigned int type,
                              const EGLint *attrib_list);
EGLBoolean eglDestroySyncKHR(EGLDisplay dpy, EGLSyncKHR sync);
EGLint     eglClientWaitSyncKHR(EGLDisplay dpy, EGLSyncKHR sync,
                                 EGLint flags, EGLTimeKHR timeout);

#endif

/* ========================================================================= */
/* EGL_KHR_wait_sync                                                         */
/* ========================================================================= */

#ifndef EGL_KHR_wait_sync
#define EGL_KHR_wait_sync 1

EGLint eglWaitSyncKHR(EGLDisplay dpy, EGLSyncKHR sync, EGLint flags);

#endif

/* ========================================================================= */
/* EGL_KHR_surfaceless_context                                               */
/* ========================================================================= */

#ifndef EGL_KHR_surfaceless_context
#define EGL_KHR_surfaceless_context 1
/* No additional tokens -- just enables eglMakeCurrent with EGL_NO_SURFACE */
#endif

/* ========================================================================= */
/* EGL_KHR_create_context                                                    */
/* ========================================================================= */

#ifndef EGL_KHR_create_context
#define EGL_KHR_create_context 1

#define EGL_CONTEXT_MAJOR_VERSION_KHR           0x3098
#define EGL_CONTEXT_MINOR_VERSION_KHR           0x30FB
#define EGL_CONTEXT_FLAGS_KHR                   0x30FC
#define EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR     0x30FD
#define EGL_CONTEXT_OPENGL_RESET_NOTIFICATION_STRATEGY_KHR 0x31BD
#define EGL_CONTEXT_OPENGL_DEBUG_BIT_KHR        0x00000001
#define EGL_CONTEXT_OPENGL_FORWARD_COMPATIBLE_BIT_KHR 0x00000002
#define EGL_CONTEXT_OPENGL_ROBUST_ACCESS_BIT_KHR 0x00000004
#define EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR 0x00000001
#define EGL_CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT_KHR 0x00000002
#define EGL_NO_RESET_NOTIFICATION_KHR           0x31BE
#define EGL_LOSE_CONTEXT_ON_RESET_KHR           0x31BF

#endif

/* ========================================================================= */
/* EGL_EXT_buffer_age                                                        */
/* ========================================================================= */

#ifndef EGL_EXT_buffer_age
#define EGL_EXT_buffer_age 1

#define EGL_BUFFER_AGE_EXT              0x313D

#endif

/* ========================================================================= */
/* EGL_EXT_swap_buffers_with_damage                                          */
/* ========================================================================= */

#ifndef EGL_EXT_swap_buffers_with_damage
#define EGL_EXT_swap_buffers_with_damage 1

EGLBoolean eglSwapBuffersWithDamageEXT(EGLDisplay dpy, EGLSurface surface,
                                        const EGLint *rects, EGLint n_rects);

#endif

#ifdef __cplusplus
}
#endif

#endif /* _EGL_EGLEXT_H */
