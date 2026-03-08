/*
 * VeridianOS libc -- <dbus/dbus-bus.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus bus-level operations: get connection, name ownership, matching.
 */

#ifndef _DBUS_DBUS_BUS_H
#define _DBUS_DBUS_BUS_H

#include <dbus/dbus-macros.h>
#include <dbus/dbus-types.h>
#include <dbus/dbus-connection.h>
#include <dbus/dbus-shared.h>
#include <dbus/dbus-errors.h>

DBUS_BEGIN_DECLS

/* ---- Bus connection ---- */

DBUS_EXPORT DBusConnection *dbus_bus_get(DBusBusType type, DBusError *error);
DBUS_EXPORT DBusConnection *dbus_bus_get_private(DBusBusType type,
                                                 DBusError *error);
DBUS_EXPORT dbus_bool_t     dbus_bus_register(DBusConnection *connection,
                                              DBusError *error);
DBUS_EXPORT void            dbus_bus_set_unique_name(DBusConnection *connection,
                                                     const char *unique_name);
DBUS_EXPORT const char     *dbus_bus_get_unique_name(
                                DBusConnection *connection);
DBUS_EXPORT unsigned long   dbus_bus_get_unix_user(DBusConnection *connection,
                                                   const char *name,
                                                   DBusError *error);
DBUS_EXPORT char           *dbus_bus_get_id(DBusConnection *connection,
                                            DBusError *error);

/* ---- Name ownership ---- */

DBUS_EXPORT int             dbus_bus_request_name(DBusConnection *connection,
                                                  const char *name,
                                                  unsigned int flags,
                                                  DBusError *error);
DBUS_EXPORT int             dbus_bus_release_name(DBusConnection *connection,
                                                  const char *name,
                                                  DBusError *error);
DBUS_EXPORT dbus_bool_t     dbus_bus_name_has_owner(DBusConnection *connection,
                                                    const char *name,
                                                    DBusError *error);
DBUS_EXPORT dbus_bool_t     dbus_bus_start_service_by_name(
                                DBusConnection *connection,
                                const char *name,
                                dbus_uint32_t flags,
                                dbus_uint32_t *result,
                                DBusError *error);

/* ---- Signal matching ---- */

DBUS_EXPORT void            dbus_bus_add_match(DBusConnection *connection,
                                               const char *rule,
                                               DBusError *error);
DBUS_EXPORT void            dbus_bus_remove_match(DBusConnection *connection,
                                                  const char *rule,
                                                  DBusError *error);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_BUS_H */
