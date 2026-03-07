/*
 * VeridianOS libc -- cxxabi.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * C++ ABI support functions required by the Itanium C++ ABI.
 * These are needed when linking C++ code (or C code compiled with
 * a C++ compiler) even in a freestanding/static environment.
 *
 * Provides:
 *   __cxa_atexit     -- register destructor for static/global objects
 *   __cxa_finalize   -- run registered destructors
 *   __cxa_guard_acquire / __cxa_guard_release / __cxa_guard_abort
 *                    -- thread-safe initialization of function-local statics
 *   __cxa_pure_virtual -- called when a pure virtual function is invoked
 *   __dso_handle     -- DSO handle for atexit registration scope
 */

#include <stdint.h>
#include <stddef.h>

/* ========================================================================= */
/* __cxa_atexit / __cxa_finalize                                             */
/* ========================================================================= */

/*
 * __cxa_atexit registers a destructor function to be called when a
 * shared library is unloaded (or at program exit for static linking).
 *
 * The Itanium C++ ABI requires this for global/static object destruction.
 */

#define CXA_ATEXIT_MAX 1024

typedef void (*cxa_dtor_fn)(void *);

struct cxa_atexit_entry {
    cxa_dtor_fn  destructor;
    void        *arg;
    void        *dso_handle;
};

static struct cxa_atexit_entry cxa_atexit_list[CXA_ATEXIT_MAX];
static int cxa_atexit_count = 0;

/*
 * DSO handle -- in a statically-linked program this is just a unique
 * address used to group atexit registrations by "module".
 */
void *__dso_handle = (void *)0;

int __cxa_atexit(cxa_dtor_fn destructor, void *arg, void *dso_handle)
{
    if (cxa_atexit_count >= CXA_ATEXIT_MAX)
        return -1;

    struct cxa_atexit_entry *e = &cxa_atexit_list[cxa_atexit_count++];
    e->destructor = destructor;
    e->arg        = arg;
    e->dso_handle = dso_handle;
    return 0;
}

/*
 * __cxa_finalize -- run destructors registered via __cxa_atexit.
 *
 * If dso_handle is NULL, run ALL registered destructors (program exit).
 * If dso_handle is non-NULL, run only those registered with that handle
 * (shared library unload via dlclose).
 *
 * Destructors are called in reverse registration order.
 */
void __cxa_finalize(void *dso_handle)
{
    for (int i = cxa_atexit_count - 1; i >= 0; i--) {
        struct cxa_atexit_entry *e = &cxa_atexit_list[i];
        if (!e->destructor) continue;

        if (dso_handle == NULL || e->dso_handle == dso_handle) {
            cxa_dtor_fn fn = e->destructor;
            void *arg = e->arg;
            e->destructor = NULL; /* prevent double-call */
            fn(arg);
        }
    }
}

/* ========================================================================= */
/* __cxa_guard_acquire / __cxa_guard_release / __cxa_guard_abort              */
/* ========================================================================= */

/*
 * Thread-safe initialization of function-local static variables.
 *
 * The compiler generates code like:
 *   if (__cxa_guard_acquire(&guard)) {
 *       // initialize the static
 *       __cxa_guard_release(&guard);
 *   }
 *
 * The guard object is a 64-bit integer where:
 *   - Byte 0 (bit 0): set to 1 when initialization is complete
 *   - Byte 1 (bit 8): set to 1 while initialization is in progress
 *
 * In a single-threaded environment (which VeridianOS userland currently is),
 * a simple flag check suffices.  When threading support is added, these
 * should use atomic operations and futex-based waiting.
 */

int __cxa_guard_acquire(uint64_t *guard)
{
    char *g = (char *)guard;

    /* Already initialized? */
    if (g[0] != 0)
        return 0;   /* No initialization needed */

    /* Mark as in-progress */
    g[1] = 1;
    return 1;   /* Caller should initialize */
}

void __cxa_guard_release(uint64_t *guard)
{
    char *g = (char *)guard;
    g[0] = 1;  /* Mark as fully initialized */
    g[1] = 0;  /* Clear in-progress flag */
}

void __cxa_guard_abort(uint64_t *guard)
{
    char *g = (char *)guard;
    g[1] = 0;  /* Clear in-progress flag, leave uninitialized */
}

/* ========================================================================= */
/* __cxa_pure_virtual                                                        */
/* ========================================================================= */

/*
 * Called when a pure virtual function is invoked.  This should never
 * happen in correct code.  We abort with a diagnostic message.
 */

/* Forward-declare to avoid pulling in extra headers */
long write(int fd, const void *buf, unsigned long count);
void _Exit(int status) __attribute__((noreturn));

void __cxa_pure_virtual(void)
{
    static const char msg[] = "pure virtual function called\n";
    write(2, msg, sizeof(msg) - 1);
    _Exit(134);  /* 128 + SIGABRT(6) */
}
