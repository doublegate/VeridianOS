/*
 * VeridianOS libc -- <dbus/dbus-shared.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Shared enums and types used by both D-Bus client and server code.
 */

#ifndef _DBUS_DBUS_SHARED_H
#define _DBUS_DBUS_SHARED_H

#include <dbus/dbus-macros.h>

DBUS_BEGIN_DECLS

/* Bus types for dbus_bus_get() */
typedef enum {
    DBUS_BUS_SESSION  = 0,
    DBUS_BUS_SYSTEM   = 1,
    DBUS_BUS_STARTER  = 2
} DBusBusType;

/* Handler result for message filters and object path handlers */
typedef enum {
    DBUS_HANDLER_RESULT_HANDLED          = 0,
    DBUS_HANDLER_RESULT_NOT_YET_HANDLED  = 1,
    DBUS_HANDLER_RESULT_NEED_MEMORY      = 2
} DBusHandlerResult;

DBUS_END_DECLS

#endif /* _DBUS_DBUS_SHARED_H */
