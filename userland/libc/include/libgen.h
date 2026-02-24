/*
 * VeridianOS libc -- <libgen.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX basename() and dirname() declarations.
 * Note: These modify the input string in-place per POSIX spec.
 */

#ifndef _LIBGEN_H
#define _LIBGEN_H

#ifdef __cplusplus
extern "C" {
#endif

char *basename(char *path);
char *dirname(char *path);

#ifdef __cplusplus
}
#endif

#endif /* _LIBGEN_H */
