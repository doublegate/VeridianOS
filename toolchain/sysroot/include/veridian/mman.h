/*
 * VeridianOS Memory Management Definitions
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Memory mapping flags, protection bits, and mmap/munmap/mprotect declarations.
 * Corresponds to kernel SYS_MEMORY_MAP (20) and SYS_MEMORY_UNMAP (21).
 */

#ifndef VERIDIAN_MMAN_H
#define VERIDIAN_MMAN_H

#include <veridian/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Memory Protection Flags                                                   */
/* ========================================================================= */

#define PROT_NONE       0   /* No access */
#define PROT_READ       1   /* Read permission */
#define PROT_WRITE      2   /* Write permission */
#define PROT_EXEC       4   /* Execute permission */

/* ========================================================================= */
/* Memory Mapping Flags                                                      */
/* ========================================================================= */

/** Modifications are shared (visible to other mappings) */
#define MAP_SHARED      0x01

/** Modifications are private (copy-on-write) */
#define MAP_PRIVATE     0x02

/** Place mapping at exactly the specified address */
#define MAP_FIXED       0x10

/** Mapping is not backed by any file (zeroed pages) */
#define MAP_ANONYMOUS   0x20

/** Alias for MAP_ANONYMOUS */
#define MAP_ANON        MAP_ANONYMOUS

/* ========================================================================= */
/* Return Values                                                             */
/* ========================================================================= */

/** Returned by mmap() on failure */
#define MAP_FAILED      ((void *)-1)

/* ========================================================================= */
/* msync Flags                                                               */
/* ========================================================================= */

#define MS_ASYNC        1   /* Schedule flush, return immediately */
#define MS_SYNC         2   /* Flush synchronously */
#define MS_INVALIDATE   4   /* Invalidate cached data */

/* ========================================================================= */
/* madvise Advice Values                                                     */
/* ========================================================================= */

#define MADV_NORMAL     0   /* No special treatment */
#define MADV_RANDOM     1   /* Expect random access */
#define MADV_SEQUENTIAL 2   /* Expect sequential access */
#define MADV_WILLNEED   3   /* Will need pages soon */
#define MADV_DONTNEED   4   /* Do not need pages soon */

/* ========================================================================= */
/* Function Declarations                                                     */
/* ========================================================================= */

/**
 * Map files or devices into memory.
 *
 * @param addr      Desired mapping address (NULL for kernel-chosen).
 * @param length    Length of the mapping in bytes.
 * @param prot      Memory protection (PROT_READ | PROT_WRITE | PROT_EXEC).
 * @param flags     Mapping flags (MAP_SHARED, MAP_PRIVATE, MAP_ANONYMOUS, etc.).
 * @param fd        File descriptor (-1 for MAP_ANONYMOUS).
 * @param offset    File offset (must be page-aligned).
 * @return Pointer to mapped region on success, MAP_FAILED on error.
 */
void *mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset);

/**
 * Unmap a previously mapped memory region.
 *
 * @param addr      Start of the mapping (must be page-aligned).
 * @param length    Length of the mapping to unmap.
 * @return 0 on success, -1 on error.
 */
int munmap(void *addr, size_t length);

/**
 * Change protection on a memory region.
 *
 * @param addr      Start of the region (must be page-aligned).
 * @param length    Length of the region.
 * @param prot      New protection (PROT_READ | PROT_WRITE | PROT_EXEC).
 * @return 0 on success, -1 on error.
 */
int mprotect(void *addr, size_t length, int prot);

/**
 * Synchronize a mapped region with its backing store.
 *
 * @param addr      Start of the region (must be page-aligned).
 * @param length    Length of the region.
 * @param flags     MS_ASYNC, MS_SYNC, or MS_INVALIDATE.
 * @return 0 on success, -1 on error.
 */
int msync(void *addr, size_t length, int flags);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_MMAN_H */
