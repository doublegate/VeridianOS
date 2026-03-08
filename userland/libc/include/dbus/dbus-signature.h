/*
 * VeridianOS libc -- <dbus/dbus-signature.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus type signature parsing and validation.
 */

#ifndef _DBUS_DBUS_SIGNATURE_H
#define _DBUS_DBUS_SIGNATURE_H

#include <dbus/dbus-macros.h>
#include <dbus/dbus-types.h>
#include <dbus/dbus-errors.h>

DBUS_BEGIN_DECLS

typedef struct {
    void *dummy1;
    void *dummy2;
    dbus_uint32_t dummy8;
    int dummy12;
    int dummy17;
} DBusSignatureIter;

DBUS_EXPORT void         dbus_signature_iter_init(DBusSignatureIter *iter,
                                                  const char *signature);
DBUS_EXPORT int          dbus_signature_iter_get_current_type(
                             const DBusSignatureIter *iter);
DBUS_EXPORT char        *dbus_signature_iter_get_signature(
                             const DBusSignatureIter *iter);
DBUS_EXPORT int          dbus_signature_iter_get_element_type(
                             const DBusSignatureIter *iter);
DBUS_EXPORT dbus_bool_t  dbus_signature_iter_next(DBusSignatureIter *iter);
DBUS_EXPORT void         dbus_signature_iter_recurse(
                             const DBusSignatureIter *iter,
                             DBusSignatureIter *sub_iter);
DBUS_EXPORT dbus_bool_t  dbus_signature_validate(const char *signature,
                                                 DBusError *error);
DBUS_EXPORT dbus_bool_t  dbus_signature_validate_single(const char *signature,
                                                        DBusError *error);
DBUS_EXPORT dbus_bool_t  dbus_type_is_valid(int typecode);
DBUS_EXPORT dbus_bool_t  dbus_type_is_basic(int typecode);
DBUS_EXPORT dbus_bool_t  dbus_type_is_container(int typecode);
DBUS_EXPORT dbus_bool_t  dbus_type_is_fixed(int typecode);

DBUS_END_DECLS

#endif /* _DBUS_DBUS_SIGNATURE_H */
