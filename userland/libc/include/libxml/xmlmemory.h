/*
 * VeridianOS libc -- libxml/xmlmemory.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libxml2 memory management API.
 */

#ifndef _LIBXML_XMLMEMORY_H
#define _LIBXML_XMLMEMORY_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Free memory allocated by libxml2. */
void xmlFree(void *mem);

/** Allocate memory through libxml2. */
void *xmlMalloc(size_t size);

/** Reallocate memory through libxml2. */
void *xmlRealloc(void *mem, size_t size);

/** Duplicate a string through libxml2 allocator. */
char *xmlMemStrdup(const char *str);

/** Initialize the memory layer. */
int xmlMemSetup(void (*freeFunc)(void *),
                void *(*mallocFunc)(size_t),
                void *(*reallocFunc)(void *, size_t),
                char *(*strdupFunc)(const char *));

/** Cleanup the memory layer. */
void xmlMemoryDump(void);

#ifdef __cplusplus
}
#endif

#endif /* _LIBXML_XMLMEMORY_H */
