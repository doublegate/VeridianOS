/*
 * VeridianOS libc -- <sys/types.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Re-exports the canonical type definitions from <veridian/types.h> and adds
 * a few POSIX types that live traditionally in <sys/types.h>.
 */

#ifndef _SYS_TYPES_H
#define _SYS_TYPES_H

#include <veridian/types.h>

/*
 * intptr_t / uintptr_t are usually provided by <stdint.h> which
 * <veridian/types.h> already includes.  Guard against compilers that
 * don't expose them without an explicit request.
 */
#ifndef __intptr_t_defined
#define __intptr_t_defined
typedef long            intptr_t;
typedef unsigned long   uintptr_t;
#endif

#endif /* _SYS_TYPES_H */
