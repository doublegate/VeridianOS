/*
 * VeridianOS libc -- jconfig.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libjpeg-turbo configuration header.
 */

#ifndef _JCONFIG_H
#define _JCONFIG_H

#define HAVE_PROTOTYPES
#define HAVE_UNSIGNED_CHAR
#define HAVE_UNSIGNED_SHORT
#define HAVE_STDDEF_H
#define HAVE_STDLIB_H

#define BITS_IN_JSAMPLE 8

typedef unsigned char JSAMPLE;
typedef short JCOEF;
typedef unsigned char JOCTET;
typedef unsigned int JDIMENSION;

#endif /* _JCONFIG_H */
