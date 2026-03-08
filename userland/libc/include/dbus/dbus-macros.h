/*
 * VeridianOS libc -- <dbus/dbus-macros.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus macro definitions: export annotations, C linkage guards.
 */

#ifndef _DBUS_DBUS_MACROS_H
#define _DBUS_DBUS_MACROS_H

#define DBUS_EXPORT __attribute__((visibility("default")))

#ifdef __cplusplus
#define DBUS_BEGIN_DECLS extern "C" {
#define DBUS_END_DECLS   }
#else
#define DBUS_BEGIN_DECLS
#define DBUS_END_DECLS
#endif

#ifndef TRUE
#define TRUE  1
#endif

#ifndef FALSE
#define FALSE 0
#endif

#ifndef NULL
#ifdef __cplusplus
#define NULL 0
#else
#define NULL ((void *)0)
#endif
#endif

#define DBUS_DEPRECATED
#define DBUS_DEPRECATED_IN_FAVOUR_OF(x)
#define DBUS_GNUC_DEPRECATED

#endif /* _DBUS_DBUS_MACROS_H */
