/*
 * VeridianOS libc -- <cxxabi.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * C++ ABI public header.
 *
 * Declares the __cxa_* functions from the Itanium C++ ABI that are
 * used by the compiler and standard library for:
 *   - Exception handling (throw, catch, rethrow)
 *   - Static initialization guards
 *   - atexit registration for global/static destructors
 *   - RTTI support (dynamic_cast, typeid)
 *   - Name demangling
 *
 * Reference: https://itanium-cxx-abi.github.io/cxx-abi/abi.html
 */

#ifndef _CXXABI_H
#define _CXXABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Exception handling                                                        */
/* ========================================================================= */

/**
 * Allocate storage for an exception object of the given size.
 * The returned pointer has space for both the __cxa_exception header
 * and the user exception object.
 */
void *__cxa_allocate_exception(size_t thrown_size);

/** Free storage allocated by __cxa_allocate_exception. */
void __cxa_free_exception(void *thrown_exception);

/**
 * Throw a C++ exception.
 *
 * @param thrown_exception  Pointer to the exception object (as returned
 *                          by __cxa_allocate_exception).
 * @param tinfo             Pointer to the std::type_info for the exception.
 * @param dest              Destructor for the exception object, or NULL.
 */
void __cxa_throw(void *thrown_exception, void *tinfo, void (*dest)(void *))
    __attribute__((noreturn));

/** Begin a catch handler -- returns pointer to the exception object. */
void *__cxa_begin_catch(void *exception_header);

/** End a catch handler -- decrements handler count, may free exception. */
void __cxa_end_catch(void);

/** Rethrow the current exception. */
void __cxa_rethrow(void) __attribute__((noreturn));

/** Get pointer to exception object from exception header. */
void *__cxa_get_exception_ptr(void *exception_header);

/** Get the type_info of the current exception (or NULL). */
void *__cxa_current_exception_type(void);

/* ========================================================================= */
/* Per-thread exception state                                                */
/* ========================================================================= */

struct __cxa_eh_globals {
    void    *caughtExceptions;      /* linked list of caught exceptions */
    unsigned int uncaughtExceptions; /* count of uncaught exceptions */
};

/** Get per-thread exception globals. */
struct __cxa_eh_globals *__cxa_get_globals(void);

/** Get per-thread exception globals (fast path, assumes initialized). */
struct __cxa_eh_globals *__cxa_get_globals_fast(void);

/* ========================================================================= */
/* Static initialization guards                                              */
/* ========================================================================= */

/** Acquire initialization guard (returns 1 if caller should init). */
int __cxa_guard_acquire(uint64_t *guard);

/** Release initialization guard (mark object as initialized). */
void __cxa_guard_release(uint64_t *guard);

/** Abort initialization (initialization threw an exception). */
void __cxa_guard_abort(uint64_t *guard);

/* ========================================================================= */
/* atexit / finalize                                                         */
/* ========================================================================= */

typedef void (*cxa_dtor_fn)(void *);

/** Register a destructor for a static/global object. */
int __cxa_atexit(cxa_dtor_fn destructor, void *arg, void *dso_handle);

/** Run registered destructors. */
void __cxa_finalize(void *dso_handle);

/** DSO handle for the main program. */
extern void *__dso_handle;

/* ========================================================================= */
/* Pure virtual / deleted virtual                                            */
/* ========================================================================= */

/** Called when a pure virtual function is invoked (aborts). */
void __cxa_pure_virtual(void);

/* ========================================================================= */
/* RTTI support                                                              */
/* ========================================================================= */

/**
 * Perform a dynamic_cast.
 *
 * @param src_ptr       Pointer to the object being cast.
 * @param src_type      type_info of the static type.
 * @param dst_type      type_info of the target type.
 * @param src2dst_offset Offset hint (-1 for unknown, -2 for virtual base).
 * @return Adjusted pointer, or NULL on failure.
 */
void *__dynamic_cast(const void *src_ptr,
                     const void *src_type,
                     const void *dst_type,
                     long src2dst_offset);

/** Thrown on failed dynamic_cast<T&>. */
void __cxa_bad_cast(void) __attribute__((noreturn));

/** Thrown on typeid(*null_ptr). */
void __cxa_bad_typeid(void) __attribute__((noreturn));

/* ========================================================================= */
/* Name demangling                                                           */
/* ========================================================================= */

/**
 * Demangle a C++ symbol name.
 *
 * @param mangled_name  The mangled name (e.g., "_Z3fooi").
 * @param output_buffer Pre-allocated buffer, or NULL for malloc.
 * @param length        Pointer to buffer length (updated on return).
 * @param status        0 on success, -1 memory error, -2 invalid name,
 *                      -3 invalid argument.
 * @return Demangled name (malloc'd if output_buffer is NULL).
 */
char *__cxa_demangle(const char *mangled_name,
                     char *output_buffer,
                     size_t *length,
                     int *status);

/* ========================================================================= */
/* Personality routine                                                       */
/* ========================================================================= */

struct _Unwind_Exception;
struct _Unwind_Context;

/** GCC C++ personality routine for exception handling. */
int __gxx_personality_v0(int version, int actions, uint64_t exception_class,
                         struct _Unwind_Exception *ue_header,
                         struct _Unwind_Context *context);

#ifdef __cplusplus
}
#endif

#endif /* _CXXABI_H */
