/*
 * VeridianOS libc -- <dbus/dbus-threads.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus threading support.
 */

#ifndef _DBUS_DBUS_THREADS_H
#define _DBUS_DBUS_THREADS_H

#include <dbus/dbus-macros.h>
#include <dbus/dbus-types.h>

DBUS_BEGIN_DECLS

DBUS_EXPORT dbus_bool_t dbus_threads_init_default(void);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_THREADS_H */
