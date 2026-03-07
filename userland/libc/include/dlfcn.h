/*
 * VeridianOS C Library -- <dlfcn.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _DLFCN_H
#define _DLFCN_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* dlopen flags                                                              */
/* ========================================================================= */

#define RTLD_LAZY    0x0001
#define RTLD_NOW     0x0002
#define RTLD_GLOBAL  0x0100
#define RTLD_LOCAL   0x0000

/** Special handle: search default symbol scope. */
#define RTLD_DEFAULT ((void *)0)

/** Special handle: find next occurrence of symbol after caller. */
#define RTLD_NEXT    ((void *)-1L)

/* ========================================================================= */
/* Core dlopen API                                                           */
/* ========================================================================= */

/** Open a shared library and return a handle. */
void *dlopen(const char *filename, int flags);

/** Look up a symbol in a loaded library. */
void *dlsym(void *handle, const char *symbol);

/** Close a previously opened library handle. */
int   dlclose(void *handle);

/** Return a human-readable error message from the last dl* call. */
char *dlerror(void);

/* ========================================================================= */
/* dladdr -- symbol/file resolution from address                             */
/* ========================================================================= */

/** Information returned by dladdr(). */
typedef struct {
    const char *dli_fname;  /**< Pathname of shared object containing addr. */
    void       *dli_fbase;  /**< Base address of shared object. */
    const char *dli_sname;  /**< Name of nearest symbol with addr <= given. */
    void       *dli_saddr;  /**< Address of the symbol named in dli_sname. */
} Dl_info;

/**
 * Determine the shared object and symbol containing a given address.
 * @return Non-zero on success, 0 on failure.
 */
int dladdr(const void *addr, Dl_info *info);

/* ========================================================================= */
/* dl_iterate_phdr -- program header iteration                               */
/* ========================================================================= */

/** Information passed to dl_iterate_phdr callback. */
struct dl_phdr_info {
    uint64_t        dlpi_addr;   /**< Base address of the object. */
    const char     *dlpi_name;   /**< Object file name. */
    const void     *dlpi_phdr;   /**< Pointer to program header array. */
    uint16_t        dlpi_phnum;  /**< Number of program headers. */
};

/**
 * Iterate over loaded program headers.
 * Calls callback for each loaded object.
 * @return Last non-zero callback return value, or 0.
 */
int dl_iterate_phdr(int (*callback)(struct dl_phdr_info *info,
                                     size_t size, void *data),
                    void *data);

#ifdef __cplusplus
}
#endif

#endif /* _DLFCN_H */
