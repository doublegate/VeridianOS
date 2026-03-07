/*
 * VeridianOS libc -- unwind.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Stack unwinding implementation for C++ exception handling.
 *
 * This provides the Itanium C++ ABI Level I (base) unwinding interface.
 * The implementation uses the frame-pointer chain (RBP on x86_64) to
 * walk the stack.  Each frame is examined by calling its personality
 * routine (if any) to determine whether it contains a catch handler
 * or cleanup code.
 *
 * Two-phase unwinding:
 *   Phase 1 (search): Walk frames looking for a handler.  Personality
 *     routines are called with _UA_SEARCH_PHASE.  If one returns
 *     _URC_HANDLER_FOUND, we record the frame and proceed to phase 2.
 *   Phase 2 (cleanup): Walk frames again, this time calling personality
 *     routines with _UA_CLEANUP_PHASE.  Cleanups run destructors for
 *     local objects.  When we reach the handler frame, the personality
 *     routine returns _URC_INSTALL_CONTEXT and we jump to the landing pad.
 *
 * Limitations:
 *   - Uses frame pointers only (no DWARF .eh_frame parsing yet).
 *     Code MUST be compiled with -fno-omit-frame-pointer.
 *   - No support for signal frames or cross-DSO unwinding.
 *   - _Unwind_ForcedUnwind is a minimal implementation.
 *
 * When a full DWARF unwinder is needed (e.g., for optimized code without
 * frame pointers), this can be extended to parse .eh_frame / .eh_frame_hdr.
 */

#include <unwind.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>

/* Forward-declare to avoid header deps */
long write(int fd, const void *buf, unsigned long count);
void abort(void) __attribute__((noreturn));

/* ========================================================================= */
/* Unwind context -- internal representation of a stack frame                */
/* ========================================================================= */

/*
 * We store enough state to identify and resume at any frame:
 *   - ip: instruction pointer (return address)
 *   - cfa: canonical frame address (caller's stack pointer)
 *   - rbp: frame pointer (for walking the chain)
 *   - regs[]: scratch space for personality routines to set GR values
 *   - lsda: language-specific data area pointer (set by personality)
 *   - func_start: start of function (set by personality)
 *   - personality: the personality routine for this frame
 */

#define UNWIND_MAX_REGS 16

struct _Unwind_Context {
    uint64_t ip;
    uint64_t cfa;
    uint64_t rbp;
    uint64_t regs[UNWIND_MAX_REGS];
    uint64_t lsda;
    uint64_t func_start;
    _Unwind_Personality_Fn personality;
};

/* ========================================================================= */
/* Context accessors                                                         */
/* ========================================================================= */

uint64_t _Unwind_GetIP(struct _Unwind_Context *ctx)
{
    return ctx->ip;
}

void _Unwind_SetIP(struct _Unwind_Context *ctx, uint64_t new_ip)
{
    ctx->ip = new_ip;
}

uint64_t _Unwind_GetGR(struct _Unwind_Context *ctx, int index)
{
    if (index < 0 || index >= UNWIND_MAX_REGS)
        return 0;
    return ctx->regs[index];
}

void _Unwind_SetGR(struct _Unwind_Context *ctx, int index, uint64_t value)
{
    if (index >= 0 && index < UNWIND_MAX_REGS)
        ctx->regs[index] = value;
}

uint64_t _Unwind_GetLanguageSpecificData(struct _Unwind_Context *ctx)
{
    return ctx->lsda;
}

uint64_t _Unwind_GetRegionStart(struct _Unwind_Context *ctx)
{
    return ctx->func_start;
}

uint64_t _Unwind_GetCFA(struct _Unwind_Context *ctx)
{
    return ctx->cfa;
}

uint64_t _Unwind_GetDataRelBase(struct _Unwind_Context *ctx)
{
    (void)ctx;
    return 0; /* Not needed for static position-dependent code */
}

uint64_t _Unwind_GetTextRelBase(struct _Unwind_Context *ctx)
{
    (void)ctx;
    return 0;
}

/* ========================================================================= */
/* Frame walking (frame-pointer chain)                                       */
/* ========================================================================= */

/*
 * On x86_64 with frame pointers:
 *   [rbp+0] = saved rbp (previous frame)
 *   [rbp+8] = return address
 *
 * The CFA (canonical frame address) is rbp+16 (the caller's RSP at
 * the point of the CALL instruction).
 */

/* Initialize context from the current frame (inline asm for x86_64) */
static void init_context_here(struct _Unwind_Context *ctx)
{
    memset(ctx, 0, sizeof(*ctx));

#if defined(__x86_64__)
    uint64_t rbp_val, rip_val;
    __asm__ volatile (
        "movq %%rbp, %0\n\t"
        "leaq (%%rip), %1\n\t"
        : "=r"(rbp_val), "=r"(rip_val)
    );
    ctx->rbp = rbp_val;
    ctx->ip  = rip_val;
    ctx->cfa = rbp_val + 16;
#elif defined(__aarch64__)
    uint64_t fp_val, lr_val;
    __asm__ volatile (
        "mov %0, x29\n\t"
        "mov %1, x30\n\t"
        : "=r"(fp_val), "=r"(lr_val)
    );
    ctx->rbp = fp_val;
    ctx->ip  = lr_val;
    ctx->cfa = fp_val + 16;
#elif defined(__riscv)
    uint64_t fp_val, ra_val;
    __asm__ volatile (
        "mv %0, s0\n\t"
        "mv %1, ra\n\t"
        : "=r"(fp_val), "=r"(ra_val)
    );
    ctx->rbp = fp_val;
    ctx->ip  = ra_val;
    ctx->cfa = fp_val + 16;
#endif
}

/*
 * Step to the next (caller) frame.
 * Returns 0 on success, -1 if we've reached the bottom of the stack.
 */
static int step_frame(struct _Unwind_Context *ctx)
{
    uint64_t bp = ctx->rbp;

    /* Sanity: frame pointer must be non-null and reasonably aligned */
    if (bp == 0 || (bp & 0x7) != 0)
        return -1;

    /*
     * Read saved frame pointer and return address from the stack.
     * These are at [bp] and [bp+8] respectively on x86_64.
     * On AArch64 and RISC-V with frame pointers, the layout is similar.
     */
    uint64_t *frame = (uint64_t *)bp;
    uint64_t saved_bp = frame[0];
    uint64_t ret_addr = frame[1];

    /* End of chain */
    if (ret_addr == 0 || saved_bp == 0)
        return -1;

    /* Detect stack corruption: saved_bp should be above current bp */
    if (saved_bp != 0 && saved_bp <= bp)
        return -1;

    ctx->rbp = saved_bp;
    ctx->ip  = ret_addr;
    ctx->cfa = saved_bp + 16;

    /* Clear per-frame state */
    ctx->lsda = 0;
    ctx->func_start = 0;
    ctx->personality = (void *)0;

    return 0;
}

/* ========================================================================= */
/* _Unwind_RaiseException -- two-phase exception raising                     */
/* ========================================================================= */

_Unwind_Reason_Code _Unwind_RaiseException(struct _Unwind_Exception *exc)
{
    struct _Unwind_Context ctx;
    _Unwind_Reason_Code rc;

    if (!exc)
        return _URC_FATAL_PHASE1_ERROR;

    init_context_here(&ctx);

    /*
     * Skip past our own frame and the caller's frame (__cxa_throw)
     * so we start searching from the frame that actually threw.
     */
    if (step_frame(&ctx) < 0)
        return _URC_END_OF_STACK;
    if (step_frame(&ctx) < 0)
        return _URC_END_OF_STACK;

    /* ---- Phase 1: Search for a handler ---- */
    struct _Unwind_Context search_ctx = ctx;
    uint64_t handler_cfa = 0;
    int found = 0;

    while (step_frame(&search_ctx) == 0) {
        if (!search_ctx.personality)
            continue;

        rc = search_ctx.personality(
            1,                          /* version */
            _UA_SEARCH_PHASE,
            exc->exception_class,
            exc,
            &search_ctx
        );

        if (rc == _URC_HANDLER_FOUND) {
            handler_cfa = search_ctx.cfa;
            found = 1;
            break;
        } else if (rc != _URC_CONTINUE_UNWIND) {
            return _URC_FATAL_PHASE1_ERROR;
        }
    }

    if (!found)
        return _URC_END_OF_STACK;

    /* ---- Phase 2: Cleanup and transfer to handler ---- */

    /*
     * Save the handler frame CFA in the exception's private fields
     * so the personality routine can check if this is the handler frame.
     */
    exc->private_1 = 0;
    exc->private_2 = handler_cfa;

    while (step_frame(&ctx) == 0) {
        if (!ctx.personality)
            continue;

        _Unwind_Action actions = _UA_CLEANUP_PHASE;
        if (ctx.cfa == handler_cfa)
            actions |= _UA_HANDLER_FRAME;

        rc = ctx.personality(
            1,
            actions,
            exc->exception_class,
            exc,
            &ctx
        );

        if (rc == _URC_INSTALL_CONTEXT) {
            /*
             * The personality routine has set up the context to jump
             * to the landing pad.  Transfer control.
             *
             * On x86_64: set RSP to CFA, jump to the landing pad IP.
             * Registers GR[0] (RAX) and GR[1] (RDX) carry the exception
             * pointer and type selector to the landing pad.
             */
#if defined(__x86_64__)
            __asm__ volatile (
                "movq %0, %%rax\n\t"     /* exception pointer */
                "movq %1, %%rdx\n\t"     /* type selector */
                "movq %2, %%rsp\n\t"     /* restore stack */
                "jmpq *%3\n\t"           /* jump to landing pad */
                :
                : "r"(ctx.regs[0]),
                  "r"(ctx.regs[1]),
                  "r"(ctx.cfa),
                  "r"(ctx.ip)
                : "memory"
            );
#elif defined(__aarch64__)
            __asm__ volatile (
                "mov x0, %0\n\t"
                "mov x1, %1\n\t"
                "mov sp, %2\n\t"
                "br %3\n\t"
                :
                : "r"(ctx.regs[0]),
                  "r"(ctx.regs[1]),
                  "r"(ctx.cfa),
                  "r"(ctx.ip)
                : "memory"
            );
#elif defined(__riscv)
            __asm__ volatile (
                "mv a0, %0\n\t"
                "mv a1, %1\n\t"
                "mv sp, %2\n\t"
                "jr %3\n\t"
                :
                : "r"(ctx.regs[0]),
                  "r"(ctx.regs[1]),
                  "r"(ctx.cfa),
                  "r"(ctx.ip)
                : "memory"
            );
#endif
            __builtin_unreachable();
        } else if (rc != _URC_CONTINUE_UNWIND) {
            return _URC_FATAL_PHASE2_ERROR;
        }
    }

    return _URC_FATAL_PHASE2_ERROR;
}

/* ========================================================================= */
/* _Unwind_Resume                                                            */
/* ========================================================================= */

/*
 * Called at the end of a cleanup landing pad to continue unwinding.
 * The exception object carries the saved state needed to continue.
 */
void _Unwind_Resume(struct _Unwind_Exception *exc)
{
    /*
     * Re-raise the exception.  In a full implementation this would
     * resume from where phase 2 left off.  For our frame-pointer-based
     * approach, we restart the search (which is correct but slightly
     * slower than optimal).
     */
    _Unwind_Reason_Code rc = _Unwind_RaiseException(exc);

    /* If we get here, unwinding failed -- this is fatal */
    (void)rc;
    static const char msg[] = "_Unwind_Resume: unwinding failed\n";
    write(2, msg, sizeof(msg) - 1);
    abort();
}

/* ========================================================================= */
/* _Unwind_DeleteException                                                   */
/* ========================================================================= */

void _Unwind_DeleteException(struct _Unwind_Exception *exc)
{
    if (exc && exc->exception_cleanup)
        exc->exception_cleanup(_URC_FOREIGN_EXCEPTION_CAUGHT, exc);
}

/* ========================================================================= */
/* _Unwind_ForcedUnwind                                                      */
/* ========================================================================= */

_Unwind_Reason_Code _Unwind_ForcedUnwind(
    struct _Unwind_Exception *exc,
    _Unwind_Stop_Fn stop,
    void *stop_parameter)
{
    struct _Unwind_Context ctx;

    if (!exc || !stop)
        return _URC_FATAL_PHASE2_ERROR;

    init_context_here(&ctx);

    /* Skip our own frame */
    if (step_frame(&ctx) < 0)
        return _URC_END_OF_STACK;

    while (step_frame(&ctx) == 0) {
        _Unwind_Reason_Code rc;

        /* Call the stop function first */
        rc = stop(1, _UA_FORCE_UNWIND | _UA_CLEANUP_PHASE,
                  exc->exception_class, exc, &ctx, stop_parameter);

        if (rc != _URC_NO_REASON && rc != _URC_CONTINUE_UNWIND)
            return rc;

        /* Then call the personality for cleanup */
        if (ctx.personality) {
            rc = ctx.personality(
                1,
                _UA_FORCE_UNWIND | _UA_CLEANUP_PHASE,
                exc->exception_class,
                exc,
                &ctx
            );

            if (rc == _URC_INSTALL_CONTEXT) {
#if defined(__x86_64__)
                __asm__ volatile (
                    "movq %0, %%rax\n\t"
                    "movq %1, %%rdx\n\t"
                    "movq %2, %%rsp\n\t"
                    "jmpq *%3\n\t"
                    :
                    : "r"(ctx.regs[0]),
                      "r"(ctx.regs[1]),
                      "r"(ctx.cfa),
                      "r"(ctx.ip)
                    : "memory"
                );
#endif
                __builtin_unreachable();
            }
        }
    }

    return _URC_END_OF_STACK;
}

/* ========================================================================= */
/* _Unwind_Backtrace                                                         */
/* ========================================================================= */

_Unwind_Reason_Code _Unwind_Backtrace(
    _Unwind_Trace_Fn callback,
    void *arg)
{
    struct _Unwind_Context ctx;
    _Unwind_Reason_Code rc;

    init_context_here(&ctx);

    /* Skip our own frame */
    if (step_frame(&ctx) < 0)
        return _URC_END_OF_STACK;

    while (step_frame(&ctx) == 0) {
        rc = callback(&ctx, arg);
        if (rc != _URC_NO_REASON)
            return rc;
    }

    return _URC_END_OF_STACK;
}
