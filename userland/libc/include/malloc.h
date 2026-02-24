/*
 * VeridianOS libc -- <malloc.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Compatibility header -- on VeridianOS, malloc/free/realloc/calloc
 * are declared in <stdlib.h>.  This header exists for software that
 * includes <malloc.h> directly (e.g. BusyBox).
 */

#ifndef _MALLOC_H
#define _MALLOC_H

#include <stdlib.h>

#ifdef __cplusplus
extern "C" {
#endif

/* memalign (obsolete; prefer posix_memalign or aligned_alloc) */
void *memalign(size_t alignment, size_t size);

/* valloc (obsolete page-aligned allocation) */
void *valloc(size_t size);

/* pvalloc (obsolete page-aligned allocation, size rounded up) */
void *pvalloc(size_t size);

/* malloc_usable_size -- return the usable size of a malloc'd block */
size_t malloc_usable_size(void *ptr);

#ifdef __cplusplus
}
#endif

#endif /* _MALLOC_H */
