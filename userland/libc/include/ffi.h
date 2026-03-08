/*
 * VeridianOS libc -- ffi.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libffi 3.4.x compatible API.
 * Foreign function interface for dynamic function calls.
 */

#ifndef _FFI_H
#define _FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Status codes                                                              */
/* ========================================================================= */

typedef enum {
    FFI_OK = 0,
    FFI_BAD_TYPEDEF,
    FFI_BAD_ABI,
    FFI_BAD_ARGTYPE
} ffi_status;

/* ========================================================================= */
/* ABI definitions                                                           */
/* ========================================================================= */

typedef enum {
    FFI_FIRST_ABI = 0,
    FFI_SYSV = 1,
    FFI_UNIX64 = 2,
    FFI_DEFAULT_ABI = FFI_UNIX64,
    FFI_LAST_ABI
} ffi_abi;

/* ========================================================================= */
/* Type definitions                                                          */
/* ========================================================================= */

#define FFI_TYPE_VOID       0
#define FFI_TYPE_INT        1
#define FFI_TYPE_FLOAT      2
#define FFI_TYPE_DOUBLE     3
#define FFI_TYPE_LONGDOUBLE 4
#define FFI_TYPE_UINT8      5
#define FFI_TYPE_SINT8      6
#define FFI_TYPE_UINT16     7
#define FFI_TYPE_SINT16     8
#define FFI_TYPE_UINT32     9
#define FFI_TYPE_SINT32     10
#define FFI_TYPE_UINT64     11
#define FFI_TYPE_SINT64     12
#define FFI_TYPE_STRUCT     13
#define FFI_TYPE_POINTER    14
#define FFI_TYPE_COMPLEX    15

typedef struct _ffi_type {
    size_t              size;
    unsigned short      alignment;
    unsigned short      type;
    struct _ffi_type  **elements;
} ffi_type;

/* Predefined type descriptors */
extern ffi_type ffi_type_void;
extern ffi_type ffi_type_uint8;
extern ffi_type ffi_type_sint8;
extern ffi_type ffi_type_uint16;
extern ffi_type ffi_type_sint16;
extern ffi_type ffi_type_uint32;
extern ffi_type ffi_type_sint32;
extern ffi_type ffi_type_uint64;
extern ffi_type ffi_type_sint64;
extern ffi_type ffi_type_float;
extern ffi_type ffi_type_double;
extern ffi_type ffi_type_pointer;

/* ========================================================================= */
/* Call interface (CIF)                                                      */
/* ========================================================================= */

typedef struct {
    ffi_abi     abi;
    unsigned    nargs;
    ffi_type  **arg_types;
    ffi_type   *rtype;
    unsigned    bytes;
    unsigned    flags;
} ffi_cif;

/* ========================================================================= */
/* Closure                                                                   */
/* ========================================================================= */

#define FFI_TRAMPOLINE_SIZE 24

typedef struct {
    char tramp[FFI_TRAMPOLINE_SIZE];
    ffi_cif *cif;
    void (*fun)(ffi_cif *, void *, void **, void *);
    void *user_data;
} ffi_closure;

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

/** Prepare a call interface descriptor. */
ffi_status ffi_prep_cif(ffi_cif *cif, ffi_abi abi, unsigned int nargs,
                        ffi_type *rtype, ffi_type **atypes);

/** Prepare a variadic call interface descriptor. */
ffi_status ffi_prep_cif_var(ffi_cif *cif, ffi_abi abi,
                            unsigned int nfixedargs, unsigned int ntotalargs,
                            ffi_type *rtype, ffi_type **atypes);

/** Call a function through a prepared CIF. */
void ffi_call(ffi_cif *cif, void (*fn)(void), void *rvalue, void **avalue);

/** Allocate a closure. */
ffi_closure *ffi_closure_alloc(size_t size, void **code);

/** Free a closure. */
void ffi_closure_free(void *closure);

/** Prepare a closure for invocation. */
ffi_status ffi_prep_closure_loc(ffi_closure *closure, ffi_cif *cif,
                                void (*fun)(ffi_cif *, void *, void **, void *),
                                void *user_data, void *codeloc);

#ifdef __cplusplus
}
#endif

#endif /* _FFI_H */
