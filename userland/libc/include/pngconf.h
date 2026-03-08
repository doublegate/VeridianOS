/*
 * VeridianOS libc -- pngconf.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libpng configuration header.
 */

#ifndef _PNGCONF_H
#define _PNGCONF_H

#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Feature support */
#define PNG_READ_SUPPORTED
#define PNG_WRITE_SUPPORTED
#define PNG_READ_TRANSFORMS_SUPPORTED
#define PNG_WRITE_TRANSFORMS_SUPPORTED
#define PNG_READ_EXPAND_SUPPORTED
#define PNG_READ_STRIP_16_TO_8_SUPPORTED
#define PNG_READ_BGR_SUPPORTED
#define PNG_READ_FILLER_SUPPORTED

/* API decoration */
#define PNG_EXPORT(ordinal, type, name, args) type name args
#define PNG_EXPORTA(ordinal, type, name, args, attr) type name args
#define PNG_CALLBACK(type, name, args) type (*name) args

#endif /* _PNGCONF_H */
