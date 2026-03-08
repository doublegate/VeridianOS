/*
 * VeridianOS libc -- zconf.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * zlib configuration header.
 * Type definitions and platform configuration for zlib.
 */

#ifndef _ZCONF_H
#define _ZCONF_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Basic type definitions */
typedef unsigned char  Byte;
typedef unsigned char  Bytef;
typedef unsigned int   uInt;
typedef unsigned long  uLong;
typedef char           charf;
typedef int            intf;
typedef long           longf;
typedef void          *voidp;
typedef void          *voidpf;
typedef void const    *voidpc;
typedef unsigned char  uch;
typedef unsigned short ush;
typedef unsigned long  ulg;

/* Function calling convention */
#define ZEXTERN  extern
#define ZEXPORT
#define ZEXPORTVA
#define FAR
#define OF(args) args
#define z_off_t  long

/* Maximum memory allocation size */
#define MAX_MEM_LEVEL  9
#define MAX_WBITS     15

#ifdef __cplusplus
}
#endif

#endif /* _ZCONF_H */
