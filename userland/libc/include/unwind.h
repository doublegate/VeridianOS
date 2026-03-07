/*
 * VeridianOS libc -- <unwind.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Itanium C++ ABI unwinding interface (Level I & II).
 *
 * This header declares the types and functions used by the C++ runtime
 * for stack unwinding during exception handling.  It follows the
 * specification at:
 *   https://itanium-cxx-abi.github.io/cxx-abi/abi-eh.html
 *
 * The implementation is in unwind.c and cxa_exception.c.
 */

#ifndef _UNWIND_H
#define _UNWIND_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Reason codes                                                              */
/* ========================================================================= */

typedef enum {
    _URC_NO_REASON                = 0,
    _URC_FOREIGN_EXCEPTION_CAUGHT = 1,
    _URC_FATAL_PHASE2_ERROR       = 2,
    _URC_FATAL_PHASE1_ERROR       = 3,
    _URC_NORMAL_STOP              = 4,
    _URC_END_OF_STACK             = 5,
    _URC_HANDLER_FOUND            = 6,
    _URC_INSTALL_CONTEXT          = 7,
    _URC_CONTINUE_UNWIND          = 8
} _Unwind_Reason_Code;

/* ========================================================================= */
/* Action flags (passed to personality routine in phase 2)                   */
/* ========================================================================= */

typedef int _Unwind_Action;

#define _UA_SEARCH_PHASE    1
#define _UA_CLEANUP_PHASE   2
#define _UA_HANDLER_FRAME   4
#define _UA_FORCE_UNWIND    8

/* ========================================================================= */
/* Exception class -- 8-byte vendor + language identifier                    */
/* ========================================================================= */

typedef uint64_t _Unwind_Exception_Class;

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

struct _Unwind_Exception;
struct _Unwind_Context;

/* ========================================================================= */
/* Exception cleanup function                                                */
/* ========================================================================= */

typedef void (*_Unwind_Exception_Cleanup_Fn)(
    _Unwind_Reason_Code reason,
    struct _Unwind_Exception *exc
);

/* ========================================================================= */
/* Exception header (prepended to every thrown exception object)              */
/* ========================================================================= */

struct _Unwind_Exception {
    _Unwind_Exception_Class exception_class;
    _Unwind_Exception_Cleanup_Fn exception_cleanup;

    /* Saved state for the unwinder -- opaque to personality routines */
    uint64_t private_1;
    uint64_t private_2;
};

/* ========================================================================= */
/* Personality routine type                                                  */
/* ========================================================================= */

typedef _Unwind_Reason_Code (*_Unwind_Personality_Fn)(
    int version,
    _Unwind_Action actions,
    _Unwind_Exception_Class exception_class,
    struct _Unwind_Exception *exception_object,
    struct _Unwind_Context *context
);

/* ========================================================================= */
/* Stop function type (for _Unwind_ForcedUnwind)                             */
/* ========================================================================= */

typedef _Unwind_Reason_Code (*_Unwind_Stop_Fn)(
    int version,
    _Unwind_Action actions,
    _Unwind_Exception_Class exception_class,
    struct _Unwind_Exception *exception_object,
    struct _Unwind_Context *context,
    void *stop_parameter
);

/* ========================================================================= */
/* Core unwinding functions                                                  */
/* ========================================================================= */

/**
 * Raise an exception -- performs two-phase unwinding:
 *   Phase 1: Search for a handler (personality returns _URC_HANDLER_FOUND)
 *   Phase 2: Cleanup and transfer control to the handler
 */
_Unwind_Reason_Code _Unwind_RaiseException(struct _Unwind_Exception *exc);

/**
 * Resume propagation after a cleanup (called at end of landing pad
 * that did not install a handler).
 */
void _Unwind_Resume(struct _Unwind_Exception *exc)
    __attribute__((noreturn));

/**
 * Delete an exception object after it has been caught.
 */
void _Unwind_DeleteException(struct _Unwind_Exception *exc);

/**
 * Force unwinding (e.g., for thread cancellation or longjmp).
 */
_Unwind_Reason_Code _Unwind_ForcedUnwind(
    struct _Unwind_Exception *exc,
    _Unwind_Stop_Fn stop,
    void *stop_parameter
);

/* ========================================================================= */
/* Context accessors (used by personality routines and landing pads)          */
/* ========================================================================= */

/** Get instruction pointer for this frame. */
uint64_t _Unwind_GetIP(struct _Unwind_Context *ctx);

/** Set instruction pointer (used to redirect to landing pad). */
void _Unwind_SetIP(struct _Unwind_Context *ctx, uint64_t new_ip);

/** Get general register value. */
uint64_t _Unwind_GetGR(struct _Unwind_Context *ctx, int index);

/** Set general register value (used to pass exception info to landing pad). */
void _Unwind_SetGR(struct _Unwind_Context *ctx, int index, uint64_t value);

/** Get pointer to language-specific data area (LSDA) for this frame. */
uint64_t _Unwind_GetLanguageSpecificData(struct _Unwind_Context *ctx);

/** Get start address of the function for this frame. */
uint64_t _Unwind_GetRegionStart(struct _Unwind_Context *ctx);

/** Get canonical frame address. */
uint64_t _Unwind_GetCFA(struct _Unwind_Context *ctx);

/** Get data-relative base for this frame. */
uint64_t _Unwind_GetDataRelBase(struct _Unwind_Context *ctx);

/** Get text-relative base for this frame. */
uint64_t _Unwind_GetTextRelBase(struct _Unwind_Context *ctx);

/* ========================================================================= */
/* Backtrace support                                                         */
/* ========================================================================= */

typedef _Unwind_Reason_Code (*_Unwind_Trace_Fn)(
    struct _Unwind_Context *ctx,
    void *arg
);

/**
 * Walk the call stack, invoking callback for each frame.
 */
_Unwind_Reason_Code _Unwind_Backtrace(
    _Unwind_Trace_Fn callback,
    void *arg
);

#ifdef __cplusplus
}
#endif

#endif /* _UNWIND_H */
