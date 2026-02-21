/*
 * VeridianOS C Library -- <alloca.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _ALLOCA_H
#define _ALLOCA_H

#include <stddef.h>

#ifdef __GNUC__
#define alloca(size) __builtin_alloca(size)
#else
void *alloca(size_t size);
#endif

#endif /* _ALLOCA_H */
