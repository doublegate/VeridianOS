/*
 * VeridianOS libc -- <dbus/dbus-types.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Core D-Bus type definitions.
 */

#ifndef _DBUS_DBUS_TYPES_H
#define _DBUS_DBUS_TYPES_H

#include <dbus/dbus-macros.h>
#include <stdint.h>

typedef uint32_t  dbus_uint32_t;
typedef int32_t   dbus_int32_t;
typedef uint16_t  dbus_uint16_t;
typedef int16_t   dbus_int16_t;
typedef uint64_t  dbus_uint64_t;
typedef int64_t   dbus_int64_t;
typedef uint32_t  dbus_bool_t;
typedef uint32_t  dbus_unichar_t;

/* Opaque handle types */
typedef struct DBusConnection  DBusConnection;
typedef struct DBusMessage     DBusMessage;
typedef struct DBusPendingCall DBusPendingCall;

/* 8-byte aligned union for basic type extraction */
typedef union {
    unsigned char    bytes[8];
    dbus_int16_t     i16;
    dbus_uint16_t    u16;
    dbus_int32_t     i32;
    dbus_uint32_t    u32;
    dbus_int64_t     i64;
    dbus_uint64_t    u64;
    double           dbl;
    unsigned char    byt;
    const char      *str;
    int              fd;
} DBusBasicValue;

#endif /* _DBUS_DBUS_TYPES_H */
