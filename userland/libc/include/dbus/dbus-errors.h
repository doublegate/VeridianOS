/*
 * VeridianOS libc -- <dbus/dbus-errors.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus error reporting.
 */

#ifndef _DBUS_DBUS_ERRORS_H
#define _DBUS_DBUS_ERRORS_H

#include <dbus/dbus-macros.h>
#include <dbus/dbus-types.h>

DBUS_BEGIN_DECLS

typedef struct DBusError {
    const char *name;
    const char *message;
    unsigned int dummy1 : 1;
    unsigned int dummy2 : 1;
    unsigned int dummy3 : 1;
    unsigned int dummy4 : 1;
    unsigned int dummy5 : 1;
    void *padding1;
} DBusError;

DBUS_EXPORT void         dbus_error_init(DBusError *error);
DBUS_EXPORT void         dbus_error_free(DBusError *error);
DBUS_EXPORT void         dbus_set_error(DBusError *error, const char *name,
                                        const char *format, ...);
DBUS_EXPORT void         dbus_set_error_const(DBusError *error,
                                              const char *name,
                                              const char *message);
DBUS_EXPORT void         dbus_move_error(DBusError *src, DBusError *dest);
DBUS_EXPORT dbus_bool_t  dbus_error_has_name(const DBusError *error,
                                             const char *name);
DBUS_EXPORT dbus_bool_t  dbus_error_is_set(const DBusError *error);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_ERRORS_H */
