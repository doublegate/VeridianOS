/*
 * VeridianOS libc -- <dbus/dbus-memory.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus memory allocation wrappers.
 */

#ifndef _DBUS_DBUS_MEMORY_H
#define _DBUS_DBUS_MEMORY_H

#include <dbus/dbus-macros.h>
#include <stddef.h>

DBUS_BEGIN_DECLS

DBUS_EXPORT void *dbus_malloc(size_t bytes);
DBUS_EXPORT void *dbus_malloc0(size_t bytes);
DBUS_EXPORT void *dbus_realloc(void *memory, size_t bytes);
DBUS_EXPORT void  dbus_free(void *memory);
DBUS_EXPORT void  dbus_free_string_array(char **str_array);

typedef void (*DBusFreeFunction)(void *memory);

DBUS_EXPORT void  dbus_shutdown(void);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_MEMORY_H */
