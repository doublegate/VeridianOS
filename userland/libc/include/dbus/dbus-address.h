/*
 * VeridianOS libc -- <dbus/dbus-address.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus server address parsing.
 */

#ifndef _DBUS_DBUS_ADDRESS_H
#define _DBUS_DBUS_ADDRESS_H

#include <dbus/dbus-macros.h>
#include <dbus/dbus-types.h>
#include <dbus/dbus-errors.h>

DBUS_BEGIN_DECLS

typedef struct DBusAddressEntry DBusAddressEntry;

DBUS_EXPORT dbus_bool_t  dbus_parse_address(const char *address,
                                            DBusAddressEntry ***entry,
                                            int *array_len,
                                            DBusError *error);
DBUS_EXPORT const char  *dbus_address_entry_get_value(DBusAddressEntry *entry,
                                                      const char *key);
DBUS_EXPORT const char  *dbus_address_entry_get_method(DBusAddressEntry *entry);
DBUS_EXPORT void         dbus_address_entries_free(DBusAddressEntry **entries);
DBUS_EXPORT char        *dbus_address_escape_value(const char *value);
DBUS_EXPORT char        *dbus_address_unescape_value(const char *value,
                                                     DBusError *error);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_ADDRESS_H */
