/*
 * VeridianOS libc -- <dbus/dbus-pending-call.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus pending call (async reply) management.
 */

#ifndef _DBUS_DBUS_PENDING_CALL_H
#define _DBUS_DBUS_PENDING_CALL_H

#include <dbus/dbus-macros.h>
#include <dbus/dbus-types.h>
#include <dbus/dbus-memory.h>

DBUS_BEGIN_DECLS

typedef void (*DBusPendingCallNotifyFunction)(DBusPendingCall *pending,
                                              void *user_data);

DBUS_EXPORT DBusPendingCall *dbus_pending_call_ref(DBusPendingCall *pending);
DBUS_EXPORT void             dbus_pending_call_unref(DBusPendingCall *pending);
DBUS_EXPORT dbus_bool_t      dbus_pending_call_set_notify(
                                 DBusPendingCall *pending,
                                 DBusPendingCallNotifyFunction function,
                                 void *user_data,
                                 DBusFreeFunction free_user_data);
DBUS_EXPORT void             dbus_pending_call_cancel(
                                 DBusPendingCall *pending);
DBUS_EXPORT dbus_bool_t      dbus_pending_call_get_completed(
                                 DBusPendingCall *pending);
DBUS_EXPORT DBusMessage     *dbus_pending_call_steal_reply(
                                 DBusPendingCall *pending);
DBUS_EXPORT void             dbus_pending_call_block(
                                 DBusPendingCall *pending);
DBUS_EXPORT dbus_bool_t      dbus_pending_call_set_data(
                                 DBusPendingCall *pending,
                                 dbus_int32_t slot,
                                 void *data,
                                 DBusFreeFunction free_data_func);
DBUS_EXPORT void            *dbus_pending_call_get_data(
                                 DBusPendingCall *pending,
                                 dbus_int32_t slot);
DBUS_EXPORT dbus_bool_t      dbus_pending_call_allocate_data_slot(
                                 dbus_int32_t *slot_p);
DBUS_EXPORT void             dbus_pending_call_free_data_slot(
                                 dbus_int32_t *slot_p);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_PENDING_CALL_H */
