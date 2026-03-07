/*
 * VeridianOS libc -- libstdcxx_support.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libstdc++ OS-level support functions.
 *
 * These are the functions that libstdc++ expects the OS/runtime to
 * provide.  They include:
 *   - operator new / operator delete (all C++17 variants)
 *   - std::terminate / std::unexpected handlers
 *   - std::get_new_handler / std::set_new_handler
 *   - __cxa_demangle (minimal passthrough)
 *   - __cxa_thread_atexit_impl (thread-local destructor registration)
 *   - uncaught_exceptions / uncaught_exception
 *
 * All memory allocation ultimately goes through malloc/free from stdlib.c.
 *
 * Reference: https://itanium-cxx-abi.github.io/cxx-abi/abi.html
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>

/* Forward-declare to avoid pulling in full headers */
void *malloc(size_t size);
void free(void *ptr);
void *calloc(size_t count, size_t size);
void *aligned_alloc(size_t alignment, size_t size);
long write(int fd, const void *buf, unsigned long count);
void abort(void) __attribute__((noreturn));
void _Exit(int status) __attribute__((noreturn));

/* ========================================================================= */
/* New handler                                                               */
/* ========================================================================= */

/*
 * The new-handler is called when operator new fails to allocate memory.
 * If set, it may free some memory and return (allowing retry), or it
 * may throw std::bad_alloc, or call abort().
 */

typedef void (*new_handler_t)(void);
static new_handler_t current_new_handler = (new_handler_t)0;

new_handler_t __cxa_set_new_handler(new_handler_t handler)
{
    new_handler_t old = current_new_handler;
    current_new_handler = handler;
    return old;
}

new_handler_t __cxa_get_new_handler(void)
{
    return current_new_handler;
}

/*
 * These are the C++ mangled names that libstdc++ actually references.
 * std::set_new_handler -> _ZSt15set_new_handlerPFvvE
 * std::get_new_handler -> _ZSt15get_new_handlerv
 */
new_handler_t _ZSt15set_new_handlerPFvvE(new_handler_t handler)
    __attribute__((alias("__cxa_set_new_handler")));

new_handler_t _ZSt15get_new_handlerv(void)
    __attribute__((alias("__cxa_get_new_handler")));

/* ========================================================================= */
/* std::terminate / std::unexpected                                          */
/* ========================================================================= */

typedef void (*terminate_handler_t)(void);
typedef void (*unexpected_handler_t)(void);

static terminate_handler_t current_terminate_handler = (void *)0;
static unexpected_handler_t current_unexpected_handler = (void *)0;

/*
 * std::terminate -- called when exception handling fails.
 * If a terminate handler is installed, call it; otherwise abort.
 */
void __cxa_call_terminate(void *exception_header)
{
    (void)exception_header;

    if (current_terminate_handler) {
        current_terminate_handler();
    }

    static const char msg[] = "std::terminate() called\n";
    write(2, msg, sizeof(msg) - 1);
    abort();
}

/*
 * Mangled names for libstdc++ references:
 *   std::terminate -> _ZSt9terminatev
 *   std::unexpected -> _ZSt10unexpectedv
 *   std::set_terminate -> _ZSt13set_terminatePFvvE
 *   std::set_unexpected -> _ZSt14set_unexpectedPFvvE
 */
void _ZSt9terminatev(void)
{
    __cxa_call_terminate((void *)0);
}

void _ZSt10unexpectedv(void)
{
    if (current_unexpected_handler)
        current_unexpected_handler();

    /* Default: call terminate */
    _ZSt9terminatev();
}

terminate_handler_t _ZSt13set_terminatePFvvE(terminate_handler_t handler)
{
    terminate_handler_t old = current_terminate_handler;
    current_terminate_handler = handler;
    return old;
}

unexpected_handler_t _ZSt14set_unexpectedPFvvE(unexpected_handler_t handler)
{
    unexpected_handler_t old = current_unexpected_handler;
    current_unexpected_handler = handler;
    return old;
}

/* ========================================================================= */
/* operator new / operator delete                                            */
/* ========================================================================= */

/*
 * C++ operator new/delete -- all variants required by C++17.
 *
 * The mangled names are:
 *   operator new(size_t)                        -> _Znwm
 *   operator new[](size_t)                      -> _Znam
 *   operator new(size_t, nothrow_t)             -> _ZnwmRKSt9nothrow_t
 *   operator new[](size_t, nothrow_t)           -> _ZnamRKSt9nothrow_t
 *   operator delete(void*)                      -> _ZdlPv
 *   operator delete[](void*)                    -> _ZdaPv
 *   operator delete(void*, size_t)              -> _ZdlPvm
 *   operator delete[](void*, size_t)            -> _ZdaPvm
 *   operator new(size_t, align_val_t)           -> _ZnwmSt11align_val_t
 *   operator new[](size_t, align_val_t)         -> _ZnamSt11align_val_t
 *   operator delete(void*, align_val_t)         -> _ZdlPvSt11align_val_t
 *   operator delete[](void*, align_val_t)       -> _ZdaPvSt11align_val_t
 *   operator delete(void*, size_t, align_val_t) -> _ZdlPvmSt11align_val_t
 *   operator delete[](void*, size_t, align_val_t) -> _ZdaPvmSt11align_val_t
 */

/* ---- Throwing operator new ---- */

/* operator new(size_t) */
void *_Znwm(size_t size)
{
    if (size == 0)
        size = 1;

    for (;;) {
        void *p = malloc(size);
        if (p)
            return p;

        new_handler_t handler = current_new_handler;
        if (handler) {
            handler();
            /* handler may free memory and return, so retry */
        } else {
            /* No handler -- would throw std::bad_alloc, but we abort */
            static const char msg[] = "operator new: allocation failed\n";
            write(2, msg, sizeof(msg) - 1);
            abort();
        }
    }
}

/* operator new[](size_t) */
void *_Znam(size_t size)
{
    return _Znwm(size);
}

/* ---- Nothrow operator new ---- */

/* operator new(size_t, nothrow_t) */
void *_ZnwmRKSt9nothrow_t(size_t size, void *nothrow_tag)
{
    (void)nothrow_tag;
    if (size == 0)
        size = 1;
    return malloc(size);
}

/* operator new[](size_t, nothrow_t) */
void *_ZnamRKSt9nothrow_t(size_t size, void *nothrow_tag)
{
    return _ZnwmRKSt9nothrow_t(size, nothrow_tag);
}

/* ---- Aligned operator new (C++17) ---- */

/* operator new(size_t, align_val_t) */
void *_ZnwmSt11align_val_t(size_t size, size_t alignment)
{
    if (size == 0)
        size = 1;
    if (alignment < sizeof(void *))
        alignment = sizeof(void *);

    for (;;) {
        void *p = aligned_alloc(alignment, size);
        if (p)
            return p;

        new_handler_t handler = current_new_handler;
        if (handler) {
            handler();
        } else {
            static const char msg[] = "operator new(aligned): allocation failed\n";
            write(2, msg, sizeof(msg) - 1);
            abort();
        }
    }
}

/* operator new[](size_t, align_val_t) */
void *_ZnamSt11align_val_t(size_t size, size_t alignment)
{
    return _ZnwmSt11align_val_t(size, alignment);
}

/* operator new(size_t, align_val_t, nothrow_t) */
void *_ZnwmSt11align_val_tRKSt9nothrow_t(size_t size, size_t alignment,
                                           void *nothrow_tag)
{
    (void)nothrow_tag;
    if (size == 0)
        size = 1;
    if (alignment < sizeof(void *))
        alignment = sizeof(void *);
    return aligned_alloc(alignment, size);
}

/* operator new[](size_t, align_val_t, nothrow_t) */
void *_ZnamSt11align_val_tRKSt9nothrow_t(size_t size, size_t alignment,
                                           void *nothrow_tag)
{
    return _ZnwmSt11align_val_tRKSt9nothrow_t(size, alignment, nothrow_tag);
}

/* ---- operator delete ---- */

/* operator delete(void*) */
void _ZdlPv(void *ptr)
{
    free(ptr);
}

/* operator delete[](void*) */
void _ZdaPv(void *ptr)
{
    free(ptr);
}

/* operator delete(void*, size_t) -- C++14 sized deallocation */
void _ZdlPvm(void *ptr, size_t size)
{
    (void)size;
    free(ptr);
}

/* operator delete[](void*, size_t) */
void _ZdaPvm(void *ptr, size_t size)
{
    (void)size;
    free(ptr);
}

/* operator delete(void*, nothrow_t) */
void _ZdlPvRKSt9nothrow_t(void *ptr, void *nothrow_tag)
{
    (void)nothrow_tag;
    free(ptr);
}

/* operator delete[](void*, nothrow_t) */
void _ZdaPvRKSt9nothrow_t(void *ptr, void *nothrow_tag)
{
    (void)nothrow_tag;
    free(ptr);
}

/* operator delete(void*, align_val_t) */
void _ZdlPvSt11align_val_t(void *ptr, size_t alignment)
{
    (void)alignment;
    free(ptr);
}

/* operator delete[](void*, align_val_t) */
void _ZdaPvSt11align_val_t(void *ptr, size_t alignment)
{
    (void)alignment;
    free(ptr);
}

/* operator delete(void*, size_t, align_val_t) */
void _ZdlPvmSt11align_val_t(void *ptr, size_t size, size_t alignment)
{
    (void)size;
    (void)alignment;
    free(ptr);
}

/* operator delete[](void*, size_t, align_val_t) */
void _ZdaPvmSt11align_val_t(void *ptr, size_t size, size_t alignment)
{
    (void)size;
    (void)alignment;
    free(ptr);
}

/* ========================================================================= */
/* __cxa_demangle -- minimal C++ name demangling                             */
/* ========================================================================= */

/*
 * Full C++ name demangling is complex (~2000 lines in libiberty).
 * This provides a minimal implementation that:
 *   1. Returns the mangled name as-is (sufficient for basic diagnostics)
 *   2. Sets *status to 0 (success) so callers don't error out
 *
 * A full implementation can be added later when needed for better
 * diagnostic output.
 */
char *__cxa_demangle(const char *mangled_name,
                     char *output_buffer,
                     size_t *length,
                     int *status)
{
    if (!mangled_name || mangled_name[0] == '\0') {
        if (status)
            *status = -2; /* invalid mangled name */
        return (char *)0;
    }

    size_t name_len = strlen(mangled_name);
    size_t needed = name_len + 1;

    if (output_buffer && length && *length >= needed) {
        /* Use provided buffer */
        memcpy(output_buffer, mangled_name, needed);
        *length = needed;
        if (status)
            *status = 0;
        return output_buffer;
    }

    /* Allocate new buffer */
    char *result = (char *)malloc(needed);
    if (!result) {
        if (status)
            *status = -1; /* memory allocation failure */
        return (char *)0;
    }

    memcpy(result, mangled_name, needed);

    if (length)
        *length = needed;
    if (status)
        *status = 0;

    return result;
}

/* ========================================================================= */
/* Thread-local destructor registration                                      */
/* ========================================================================= */

/*
 * __cxa_thread_atexit_impl -- register a destructor for a thread-local
 * object.  Called by the compiler for thread_local objects with
 * non-trivial destructors.
 *
 * For now, we delegate to __cxa_atexit since thread-local storage
 * destructors at thread exit are handled by pthread_exit.
 */

extern int __cxa_atexit(void (*)(void *), void *, void *);
extern void *__dso_handle;

int __cxa_thread_atexit_impl(void (*dtor)(void *), void *obj, void *dso)
{
    /* Fall back to process-level atexit registration */
    return __cxa_atexit(dtor, obj, dso ? dso : __dso_handle);
}

/* ========================================================================= */
/* Uncaught exception counting                                               */
/* ========================================================================= */

/*
 * std::uncaught_exceptions() returns the number of uncaught exceptions
 * on the current thread.  This is used by scope guards and similar
 * RAII constructs.
 *
 * Mangled: _ZSt20uncaught_exceptionsv (C++17)
 *          _ZSt18uncaught_exceptionv (C++98, returns bool)
 */

struct __cxa_eh_globals {
    void    *caughtExceptions;
    unsigned int uncaughtExceptions;
};

extern struct __cxa_eh_globals *__cxa_get_globals_fast(void);

int _ZSt20uncaught_exceptionsv(void)
{
    struct __cxa_eh_globals *globals = __cxa_get_globals_fast();
    if (!globals)
        return 0;
    return (int)globals->uncaughtExceptions;
}

/* C++98 version: returns 1 if any uncaught exception exists */
int _ZSt18uncaught_exceptionv(void)
{
    return _ZSt20uncaught_exceptionsv() > 0;
}

/* ========================================================================= */
/* Verbose terminate handler (libstdc++ default)                             */
/* ========================================================================= */

/*
 * __gnu_cxx::__verbose_terminate_handler -- the default terminate handler
 * installed by libstdc++ that prints exception info before aborting.
 *
 * Mangled: _ZN9__gnu_cxx27__verbose_terminate_handlerEv
 */
void _ZN9__gnu_cxx27__verbose_terminate_handlerEv(void)
{
    static const char msg[] = "terminate called after throwing an exception\n";
    write(2, msg, sizeof(msg) - 1);
    abort();
}

/* ========================================================================= */
/* __cxa_call_unexpected                                                     */
/* ========================================================================= */

/*
 * Called when a function throws an exception not in its exception
 * specification (deprecated in C++17, removed in C++20).
 */
void __cxa_call_unexpected(void *exception_header)
{
    (void)exception_header;
    _ZSt10unexpectedv();
    /* unexpected() may return if the handler throws -- terminate if so */
    _ZSt9terminatev();
    __builtin_unreachable();
}
