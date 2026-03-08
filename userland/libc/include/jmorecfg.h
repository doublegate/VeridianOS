/*
 * VeridianOS libc -- jmorecfg.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libjpeg-turbo additional configuration.
 */

#ifndef _JMORECFG_H
#define _JMORECFG_H

/* Boolean type */
#ifndef HAVE_BOOLEAN
typedef int boolean;
#endif
#ifndef FALSE
#define FALSE 0
#endif
#ifndef TRUE
#define TRUE  1
#endif

/* Data types */
typedef short INT16;
typedef int   INT32;

#define MAXJSAMPLE  255
#define CENTERJSAMPLE 128

#define MAX_COMPONENTS 10

/* Method selection */
#define JMETHOD(type, methodname, arglist) type (*methodname) arglist

/* RGB pixel ordering */
#define RGB_RED       0
#define RGB_GREEN     1
#define RGB_BLUE      2
#define RGB_PIXELSIZE 3

#endif /* _JMORECFG_H */
