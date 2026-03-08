/*
 * VeridianOS libc -- libffi_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libffi 3.4.x shim -- foreign function interface for x86_64.
 * Implements ffi_prep_cif, ffi_call (System V AMD64 ABI), and closures.
 *
 * The System V AMD64 ABI passes the first 6 integer/pointer args in:
 *   rdi, rsi, rdx, rcx, r8, r9
 * The first 8 float/double args go in xmm0-xmm7.
 * Return value in rax (integer) or xmm0 (float/double).
 */

#include <ffi.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Predefined type descriptors                                               */
/* ========================================================================= */

ffi_type ffi_type_void    = { 0, 0, FFI_TYPE_VOID,       NULL };
ffi_type ffi_type_uint8   = { 1, 1, FFI_TYPE_UINT8,      NULL };
ffi_type ffi_type_sint8   = { 1, 1, FFI_TYPE_SINT8,      NULL };
ffi_type ffi_type_uint16  = { 2, 2, FFI_TYPE_UINT16,     NULL };
ffi_type ffi_type_sint16  = { 2, 2, FFI_TYPE_SINT16,     NULL };
ffi_type ffi_type_uint32  = { 4, 4, FFI_TYPE_UINT32,     NULL };
ffi_type ffi_type_sint32  = { 4, 4, FFI_TYPE_SINT32,     NULL };
ffi_type ffi_type_uint64  = { 8, 8, FFI_TYPE_UINT64,     NULL };
ffi_type ffi_type_sint64  = { 8, 8, FFI_TYPE_SINT64,     NULL };
ffi_type ffi_type_float   = { 4, 4, FFI_TYPE_FLOAT,      NULL };
ffi_type ffi_type_double  = { 8, 8, FFI_TYPE_DOUBLE,     NULL };
ffi_type ffi_type_pointer = { 8, 8, FFI_TYPE_POINTER,    NULL };

/* ========================================================================= */
/* ffi_prep_cif -- prepare a call interface                                  */
/* ========================================================================= */

static int is_integer_type(ffi_type *t)
{
    switch (t->type) {
    case FFI_TYPE_UINT8:  case FFI_TYPE_SINT8:
    case FFI_TYPE_UINT16: case FFI_TYPE_SINT16:
    case FFI_TYPE_UINT32: case FFI_TYPE_SINT32:
    case FFI_TYPE_UINT64: case FFI_TYPE_SINT64:
    case FFI_TYPE_POINTER:
    case FFI_TYPE_INT:
        return 1;
    default:
        return 0;
    }
}

ffi_status ffi_prep_cif(ffi_cif *cif, ffi_abi abi, unsigned int nargs,
                        ffi_type *rtype, ffi_type **atypes)
{
    unsigned int i;
    unsigned int stack_bytes = 0;
    unsigned int gpr_count = 0;

    if (cif == NULL || rtype == NULL)
        return FFI_BAD_TYPEDEF;
    if (abi <= FFI_FIRST_ABI || abi >= FFI_LAST_ABI)
        return FFI_BAD_ABI;

    cif->abi = abi;
    cif->nargs = nargs;
    cif->arg_types = atypes;
    cif->rtype = rtype;
    cif->flags = 0;

    /* Compute stack space needed for overflow args */
    for (i = 0; i < nargs; i++) {
        if (atypes[i] == NULL)
            return FFI_BAD_TYPEDEF;

        if (is_integer_type(atypes[i]) || atypes[i]->type == FFI_TYPE_STRUCT) {
            if (gpr_count < 6) {
                gpr_count++;
            } else {
                stack_bytes += 8;  /* Everything is 8-byte aligned on stack */
            }
        } else {
            /* Float/double -- simplified: treat as stack for now */
            stack_bytes += 8;
        }
    }

    /* Align stack to 16 bytes */
    stack_bytes = (stack_bytes + 15) & ~15u;
    cif->bytes = stack_bytes;

    return FFI_OK;
}

ffi_status ffi_prep_cif_var(ffi_cif *cif, ffi_abi abi,
                            unsigned int nfixedargs, unsigned int ntotalargs,
                            ffi_type *rtype, ffi_type **atypes)
{
    (void)nfixedargs;
    return ffi_prep_cif(cif, abi, ntotalargs, rtype, atypes);
}

/* ========================================================================= */
/* ffi_call -- invoke a function through a CIF (x86_64 System V ABI)        */
/* ========================================================================= */

void ffi_call(ffi_cif *cif, void (*fn)(void), void *rvalue, void **avalue)
{
    uint64_t gpr_args[6];
    unsigned int gpr_count = 0;
    unsigned int stack_count = 0;
    uint64_t *stack_args;
    unsigned int i;
    uint64_t result;

    if (cif == NULL || fn == NULL)
        return;

    /* Allocate stack overflow area */
    stack_args = NULL;
    if (cif->bytes > 0) {
        stack_args = (uint64_t *)alloca(cif->bytes);
        memset(stack_args, 0, cif->bytes);
    }

    memset(gpr_args, 0, sizeof(gpr_args));

    /* Distribute arguments to registers and stack */
    for (i = 0; i < cif->nargs; i++) {
        uint64_t val = 0;

        if (avalue[i] != NULL) {
            switch (cif->arg_types[i]->type) {
            case FFI_TYPE_UINT8:   val = *(uint8_t *)avalue[i];  break;
            case FFI_TYPE_SINT8:   val = (uint64_t)(int64_t)*(int8_t *)avalue[i]; break;
            case FFI_TYPE_UINT16:  val = *(uint16_t *)avalue[i]; break;
            case FFI_TYPE_SINT16:  val = (uint64_t)(int64_t)*(int16_t *)avalue[i]; break;
            case FFI_TYPE_UINT32:
            case FFI_TYPE_INT:     val = *(uint32_t *)avalue[i]; break;
            case FFI_TYPE_SINT32:  val = (uint64_t)(int64_t)*(int32_t *)avalue[i]; break;
            case FFI_TYPE_UINT64:
            case FFI_TYPE_SINT64:  val = *(uint64_t *)avalue[i]; break;
            case FFI_TYPE_POINTER: val = (uint64_t)(uintptr_t)*(void **)avalue[i]; break;
            default:
                /* For struct/float/double, just copy the raw bits */
                memcpy(&val, avalue[i], cif->arg_types[i]->size < 8
                       ? cif->arg_types[i]->size : 8);
                break;
            }
        }

        if (gpr_count < 6 && is_integer_type(cif->arg_types[i])) {
            gpr_args[gpr_count++] = val;
        } else {
            if (stack_args != NULL && stack_count < cif->bytes / 8)
                stack_args[stack_count++] = val;
        }
    }

    /*
     * Call the function using inline assembly.
     * Load the 6 GPR arguments from gpr_args[], set up the stack
     * overflow area, call fn, and capture the result in rax.
     *
     * Note: This is a simplified implementation that handles the common
     * case of integer/pointer arguments.  Float/double via XMM registers
     * is not yet implemented (would need separate handling).
     */
    __asm__ volatile (
        /* Save callee-saved registers we'll clobber */
        "push %%rbp\n\t"
        "mov %%rsp, %%rbp\n\t"

        /* Copy stack overflow args if any */
        "mov %[stack_bytes], %%ecx\n\t"
        "test %%ecx, %%ecx\n\t"
        "jz 1f\n\t"
        "sub %%rcx, %%rsp\n\t"
        "and $-16, %%rsp\n\t"   /* Align to 16 */
        "mov %[stack_args], %%rsi\n\t"
        "mov %%rsp, %%rdi\n\t"
        "rep movsb\n\t"
        "1:\n\t"

        /* Load GPR args */
        "mov %[gpr], %%r11\n\t"
        "mov 0(%%r11), %%rdi\n\t"
        "mov 8(%%r11), %%rsi\n\t"
        "mov 16(%%r11), %%rdx\n\t"
        "mov 24(%%r11), %%rcx\n\t"
        "mov 32(%%r11), %%r8\n\t"
        "mov 40(%%r11), %%r9\n\t"

        /* al = 0 for non-variadic (no float args in XMM) */
        "xor %%eax, %%eax\n\t"

        /* Call the function */
        "call *%[fn]\n\t"

        /* Capture result */
        "mov %%rax, %[result]\n\t"

        /* Restore stack */
        "mov %%rbp, %%rsp\n\t"
        "pop %%rbp\n\t"

        : [result] "=r" (result)
        : [fn] "r" (fn),
          [gpr] "r" (gpr_args),
          [stack_args] "r" (stack_args),
          [stack_bytes] "r" ((uint32_t)cif->bytes)
        : "rdi", "rsi", "rdx", "rcx", "r8", "r9", "r10", "r11",
          "rax", "memory", "cc"
    );

    /* Store the return value */
    if (rvalue != NULL && cif->rtype->type != FFI_TYPE_VOID) {
        switch (cif->rtype->type) {
        case FFI_TYPE_UINT8:   *(uint8_t *)rvalue  = (uint8_t)result;  break;
        case FFI_TYPE_SINT8:   *(int8_t *)rvalue   = (int8_t)result;   break;
        case FFI_TYPE_UINT16:  *(uint16_t *)rvalue  = (uint16_t)result; break;
        case FFI_TYPE_SINT16:  *(int16_t *)rvalue   = (int16_t)result;  break;
        case FFI_TYPE_UINT32:
        case FFI_TYPE_INT:     *(uint32_t *)rvalue  = (uint32_t)result; break;
        case FFI_TYPE_SINT32:  *(int32_t *)rvalue   = (int32_t)result;  break;
        default:               *(uint64_t *)rvalue  = result;           break;
        }
    }
}

/* ========================================================================= */
/* Closures (minimal implementation)                                         */
/* ========================================================================= */

ffi_closure *ffi_closure_alloc(size_t size, void **code)
{
    ffi_closure *closure;

    if (size < sizeof(ffi_closure))
        size = sizeof(ffi_closure);

    closure = (ffi_closure *)malloc(size);
    if (closure == NULL)
        return NULL;

    memset(closure, 0, size);

    /* For a real implementation, the code pointer would be an executable
     * trampoline.  For now, just point it at the closure itself. */
    if (code != NULL)
        *code = closure;

    return closure;
}

void ffi_closure_free(void *closure)
{
    free(closure);
}

ffi_status ffi_prep_closure_loc(ffi_closure *closure, ffi_cif *cif,
                                void (*fun)(ffi_cif *, void *, void **, void *),
                                void *user_data, void *codeloc)
{
    if (closure == NULL || cif == NULL || fun == NULL)
        return FFI_BAD_TYPEDEF;

    closure->cif = cif;
    closure->fun = fun;
    closure->user_data = user_data;

    (void)codeloc;

    /*
     * A full implementation would write a trampoline at codeloc that:
     * 1. Saves the register args to the stack
     * 2. Calls the closure->fun with (cif, rvalue, avalue, user_data)
     * 3. Returns the result
     *
     * For now, the closure is prepared but the trampoline is a stub.
     */

    return FFI_OK;
}
