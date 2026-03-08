/*
 * VeridianOS libc -- <dbus/dbus-message.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus message creation, iteration, and argument handling.
 */

#ifndef _DBUS_DBUS_MESSAGE_H
#define _DBUS_DBUS_MESSAGE_H

#include <dbus/dbus-macros.h>
#include <dbus/dbus-types.h>
#include <dbus/dbus-protocol.h>
#include <dbus/dbus-errors.h>
#include <stdarg.h>

DBUS_BEGIN_DECLS

/* Message iterator for reading/appending arguments */
typedef struct {
    void          *dummy1;
    void          *dummy2;
    dbus_uint32_t  dummy3;
    int            dummy4;
    int            dummy5;
    int            dummy6;
    int            dummy7;
    int            dummy8;
    int            dummy9;
    int            dummy10;
    int            dummy11;
    int            pad1;
    void          *pad2;
    void          *pad3;
} DBusMessageIter;

/* ---- Message lifecycle ---- */

DBUS_EXPORT DBusMessage *dbus_message_new(int message_type);
DBUS_EXPORT DBusMessage *dbus_message_new_method_call(const char *destination,
                                                      const char *path,
                                                      const char *iface,
                                                      const char *method);
DBUS_EXPORT DBusMessage *dbus_message_new_method_return(DBusMessage *method_call);
DBUS_EXPORT DBusMessage *dbus_message_new_signal(const char *path,
                                                 const char *iface,
                                                 const char *name);
DBUS_EXPORT DBusMessage *dbus_message_new_error(DBusMessage *reply_to,
                                                const char *error_name,
                                                const char *error_message);
DBUS_EXPORT DBusMessage *dbus_message_new_error_printf(DBusMessage *reply_to,
                                                       const char *error_name,
                                                       const char *error_format,
                                                       ...);
DBUS_EXPORT DBusMessage *dbus_message_ref(DBusMessage *message);
DBUS_EXPORT void         dbus_message_unref(DBusMessage *message);
DBUS_EXPORT DBusMessage *dbus_message_copy(const DBusMessage *message);

/* ---- Message metadata ---- */

DBUS_EXPORT int          dbus_message_get_type(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_set_path(DBusMessage *message,
                                               const char *object_path);
DBUS_EXPORT const char  *dbus_message_get_path(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_has_path(DBusMessage *message,
                                               const char *path);
DBUS_EXPORT dbus_bool_t  dbus_message_set_interface(DBusMessage *message,
                                                    const char *iface);
DBUS_EXPORT const char  *dbus_message_get_interface(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_has_interface(DBusMessage *message,
                                                    const char *iface);
DBUS_EXPORT dbus_bool_t  dbus_message_set_member(DBusMessage *message,
                                                 const char *member);
DBUS_EXPORT const char  *dbus_message_get_member(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_has_member(DBusMessage *message,
                                                 const char *member);
DBUS_EXPORT dbus_bool_t  dbus_message_set_error_name(DBusMessage *message,
                                                     const char *error_name);
DBUS_EXPORT const char  *dbus_message_get_error_name(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_set_destination(DBusMessage *message,
                                                      const char *destination);
DBUS_EXPORT const char  *dbus_message_get_destination(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_set_sender(DBusMessage *message,
                                                 const char *sender);
DBUS_EXPORT const char  *dbus_message_get_sender(DBusMessage *message);
DBUS_EXPORT const char  *dbus_message_get_signature(DBusMessage *message);
DBUS_EXPORT void         dbus_message_set_no_reply(DBusMessage *message,
                                                   dbus_bool_t no_reply);
DBUS_EXPORT dbus_bool_t  dbus_message_get_no_reply(DBusMessage *message);
DBUS_EXPORT void         dbus_message_set_auto_start(DBusMessage *message,
                                                     dbus_bool_t auto_start);
DBUS_EXPORT dbus_bool_t  dbus_message_get_auto_start(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_set_reply_serial(DBusMessage *message,
                                                       dbus_uint32_t serial);
DBUS_EXPORT dbus_uint32_t dbus_message_get_reply_serial(DBusMessage *message);
DBUS_EXPORT dbus_uint32_t dbus_message_get_serial(DBusMessage *message);
DBUS_EXPORT dbus_bool_t  dbus_message_is_method_call(DBusMessage *message,
                                                     const char *iface,
                                                     const char *method);
DBUS_EXPORT dbus_bool_t  dbus_message_is_signal(DBusMessage *message,
                                                const char *iface,
                                                const char *signal_name);
DBUS_EXPORT dbus_bool_t  dbus_message_is_error(DBusMessage *message,
                                               const char *error_name);
DBUS_EXPORT dbus_bool_t  dbus_message_has_destination(DBusMessage *message,
                                                      const char *name);
DBUS_EXPORT dbus_bool_t  dbus_message_has_sender(DBusMessage *message,
                                                 const char *name);
DBUS_EXPORT dbus_bool_t  dbus_message_has_signature(DBusMessage *message,
                                                    const char *signature);

/* ---- Message argument convenience ---- */

DBUS_EXPORT dbus_bool_t  dbus_message_get_args(DBusMessage *message,
                                               DBusError *error,
                                               int first_arg_type, ...);
DBUS_EXPORT dbus_bool_t  dbus_message_get_args_valist(DBusMessage *message,
                                                      DBusError *error,
                                                      int first_arg_type,
                                                      va_list var_args);
DBUS_EXPORT dbus_bool_t  dbus_message_append_args(DBusMessage *message,
                                                  int first_arg_type, ...);
DBUS_EXPORT dbus_bool_t  dbus_message_append_args_valist(DBusMessage *message,
                                                         int first_arg_type,
                                                         va_list var_args);

/* ---- Message iterator API ---- */

DBUS_EXPORT dbus_bool_t  dbus_message_iter_init(DBusMessage *message,
                                                DBusMessageIter *iter);
DBUS_EXPORT dbus_bool_t  dbus_message_iter_has_next(DBusMessageIter *iter);
DBUS_EXPORT dbus_bool_t  dbus_message_iter_next(DBusMessageIter *iter);
DBUS_EXPORT int          dbus_message_iter_get_arg_type(DBusMessageIter *iter);
DBUS_EXPORT int          dbus_message_iter_get_element_type(
                             DBusMessageIter *iter);
DBUS_EXPORT void         dbus_message_iter_recurse(DBusMessageIter *iter,
                                                   DBusMessageIter *sub);
DBUS_EXPORT int          dbus_message_iter_get_element_count(
                             DBusMessageIter *iter);
DBUS_EXPORT void         dbus_message_iter_get_basic(DBusMessageIter *iter,
                                                     void *value);
DBUS_EXPORT void         dbus_message_iter_get_fixed_array(
                             DBusMessageIter *iter, void *value, int *n_elements);
DBUS_EXPORT char        *dbus_message_iter_get_signature(
                             DBusMessageIter *iter);

DBUS_EXPORT void         dbus_message_iter_init_append(DBusMessage *message,
                                                       DBusMessageIter *iter);
DBUS_EXPORT dbus_bool_t  dbus_message_iter_append_basic(DBusMessageIter *iter,
                                                        int type,
                                                        const void *value);
DBUS_EXPORT dbus_bool_t  dbus_message_iter_append_fixed_array(
                             DBusMessageIter *iter, int element_type,
                             const void *value, int n_elements);
DBUS_EXPORT dbus_bool_t  dbus_message_iter_open_container(
                             DBusMessageIter *iter, int type,
                             const char *contained_signature,
                             DBusMessageIter *sub);
DBUS_EXPORT dbus_bool_t  dbus_message_iter_close_container(
                             DBusMessageIter *iter,
                             DBusMessageIter *sub);
DBUS_EXPORT void         dbus_message_iter_abandon_container(
                             DBusMessageIter *iter,
                             DBusMessageIter *sub);
DBUS_EXPORT dbus_bool_t  dbus_message_iter_abandon_container_if_open(
                             DBusMessageIter *iter,
                             DBusMessageIter *sub);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_MESSAGE_H */
