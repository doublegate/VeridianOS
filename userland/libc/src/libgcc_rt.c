/*
 * VeridianOS libc -- libgcc_rt.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * GCC runtime support functions.
 *
 * GCC emits calls to these helper functions for operations that the
 * target hardware cannot perform directly (e.g., 64-bit division on
 * 32-bit targets, 128-bit arithmetic, float<->int conversions).
 * On x86_64, most 64-bit ops are native, but 128-bit helpers and
 * float conversions are still needed by GCC-compiled code.
 *
 * These implement the same ABI as libgcc_s.so.1 so that C and C++
 * code linked against VeridianOS libc can resolve these symbols
 * without needing a separate libgcc_s shared library.
 *
 * Reference: GCC Internals -- Integer library routines
 * https://gcc.gnu.org/onlinedocs/gccint/Integer-library-routines.html
 */

#include <stdint.h>
#include <stddef.h>

/* ========================================================================= */
/* Internal types                                                            */
/* ========================================================================= */

typedef          int      si_int;
typedef unsigned int      su_int;
typedef          long long di_int;
typedef unsigned long long du_int;

#ifdef __SIZEOF_INT128__
typedef          __int128 ti_int;
typedef unsigned __int128 tu_int;
#endif

typedef union {
    di_int all;
    struct {
        su_int low;
        si_int high;
    } s;
} dwords;

typedef union {
    du_int all;
    struct {
        su_int low;
        su_int high;
    } s;
} udwords;

#ifdef __SIZEOF_INT128__
typedef union {
    ti_int all;
    struct {
        du_int low;
        di_int high;
    } s;
} twords;

typedef union {
    tu_int all;
    struct {
        du_int low;
        du_int high;
    } s;
} utwords;
#endif

/* ========================================================================= */
/* 64-bit integer division                                                   */
/* ========================================================================= */

/*
 * Unsigned 64-bit division helper -- used by all div/mod functions.
 *
 * Uses binary long division (shift-and-subtract).  On x86_64 this is
 * rarely called because the hardware has native 64-bit div, but GCC
 * may still reference the symbol for generic code paths.
 */
static du_int __udivmoddi4(du_int a, du_int b, du_int *rem)
{
    if (b == 0) {
        /* Division by zero -- undefined behavior, but don't crash */
        if (rem)
            *rem = 0;
        return 0;
    }

    if (b > a) {
        if (rem)
            *rem = a;
        return 0;
    }

    if (b == a) {
        if (rem)
            *rem = 0;
        return 1;
    }

    /* Count leading zeros to align divisor */
    int sr = __builtin_clzll(b) - __builtin_clzll(a);
    du_int q = 0;
    du_int r = 0;

    /* Binary long division */
    for (int i = 63; i >= 0; i--) {
        r = (r << 1) | ((a >> i) & 1);
        if (r >= b) {
            r -= b;
            q |= (du_int)1 << i;
        }
    }

    if (rem)
        *rem = r;
    return q;
}

/* Signed 64-bit division: a / b */
di_int __divdi3(di_int a, di_int b)
{
    int neg = 0;
    du_int ua = (du_int)a;
    du_int ub = (du_int)b;

    if (a < 0) {
        ua = (du_int)(-(a + 1)) + 1u;  /* safe negate avoiding overflow */
        neg = !neg;
    }
    if (b < 0) {
        ub = (du_int)(-(b + 1)) + 1u;
        neg = !neg;
    }

    du_int q = __udivmoddi4(ua, ub, (du_int *)0);
    return neg ? -(di_int)q : (di_int)q;
}

/* Signed 64-bit modulus: a % b */
di_int __moddi3(di_int a, di_int b)
{
    int neg = 0;
    du_int ua = (du_int)a;
    du_int ub = (du_int)b;

    if (a < 0) {
        ua = (du_int)(-(a + 1)) + 1u;
        neg = 1;
    }
    if (b < 0) {
        ub = (du_int)(-(b + 1)) + 1u;
    }

    du_int rem;
    __udivmoddi4(ua, ub, &rem);
    return neg ? -(di_int)rem : (di_int)rem;
}

/* Unsigned 64-bit division: a / b */
du_int __udivdi3(du_int a, du_int b)
{
    return __udivmoddi4(a, b, (du_int *)0);
}

/* Unsigned 64-bit modulus: a % b */
du_int __umoddi3(du_int a, du_int b)
{
    du_int rem;
    __udivmoddi4(a, b, &rem);
    return rem;
}

/* ========================================================================= */
/* 128-bit integer division                                                  */
/* ========================================================================= */

#ifdef __SIZEOF_INT128__

static tu_int __udivmodti4(tu_int a, tu_int b, tu_int *rem)
{
    if (b == 0) {
        if (rem)
            *rem = 0;
        return 0;
    }

    if (b > a) {
        if (rem)
            *rem = a;
        return 0;
    }

    if (b == a) {
        if (rem)
            *rem = 0;
        return 1;
    }

    /* Use __builtin_clzll on the high/low halves */
    utwords ua, ub;
    ua.all = a;
    ub.all = b;

    tu_int q = 0;
    tu_int r = 0;

    for (int i = 127; i >= 0; i--) {
        r = (r << 1) | ((a >> i) & 1);
        if (r >= b) {
            r -= b;
            q |= (tu_int)1 << i;
        }
    }

    if (rem)
        *rem = r;
    return q;
}

ti_int __divti3(ti_int a, ti_int b)
{
    int neg = 0;
    tu_int ua = (tu_int)a;
    tu_int ub = (tu_int)b;

    if (a < 0) {
        ua = -ua;
        neg = !neg;
    }
    if (b < 0) {
        ub = -ub;
        neg = !neg;
    }

    tu_int q = __udivmodti4(ua, ub, (tu_int *)0);
    return neg ? -(ti_int)q : (ti_int)q;
}

ti_int __modti3(ti_int a, ti_int b)
{
    int neg = 0;
    tu_int ua = (tu_int)a;
    tu_int ub = (tu_int)b;

    if (a < 0) {
        ua = -ua;
        neg = 1;
    }
    if (b < 0) {
        ub = -ub;
    }

    tu_int rem;
    __udivmodti4(ua, ub, &rem);
    return neg ? -(ti_int)rem : (ti_int)rem;
}

tu_int __udivti3(tu_int a, tu_int b)
{
    return __udivmodti4(a, b, (tu_int *)0);
}

tu_int __umodti3(tu_int a, tu_int b)
{
    tu_int rem;
    __udivmodti4(a, b, &rem);
    return rem;
}

#endif /* __SIZEOF_INT128__ */

/* ========================================================================= */
/* 64-bit / 128-bit multiplication                                           */
/* ========================================================================= */

/*
 * On x86_64, 64-bit multiply is native (imulq), but GCC may still
 * reference these symbols in generic code or when targeting 32-bit.
 */
di_int __muldi3(di_int a, di_int b)
{
    return a * b;
}

#ifdef __SIZEOF_INT128__
ti_int __multi3(ti_int a, ti_int b)
{
    return a * b;
}
#endif

/* ========================================================================= */
/* 64-bit shifts                                                             */
/* ========================================================================= */

/*
 * These are needed on 32-bit targets where 64-bit shifts are not native.
 * On x86_64 they compile to trivial wrappers but the symbols must exist.
 */
di_int __ashldi3(di_int a, int b)
{
    if (b >= 64) return 0;
    if (b == 0) return a;
    return a << b;
}

di_int __ashrdi3(di_int a, int b)
{
    if (b >= 64) return a < 0 ? -1 : 0;
    if (b == 0) return a;
    return a >> b;
}

di_int __lshrdi3(di_int a, int b)
{
    du_int ua = (du_int)a;
    if (b >= 64) return 0;
    if (b == 0) return (di_int)ua;
    return (di_int)(ua >> b);
}

/* ========================================================================= */
/* Bit counting operations                                                   */
/* ========================================================================= */

int __clzdi2(di_int a)
{
    return __builtin_clzll((du_int)a);
}

int __ctzdi2(di_int a)
{
    return __builtin_ctzll((du_int)a);
}

int __popcountdi2(di_int a)
{
    return __builtin_popcountll((du_int)a);
}

/* ========================================================================= */
/* Float-to-integer conversions                                              */
/* ========================================================================= */

/* float -> signed 64-bit */
di_int __fixsfdi(float a)
{
    return (di_int)a;
}

/* double -> signed 64-bit */
di_int __fixdfdi(double a)
{
    return (di_int)a;
}

/* float -> unsigned 64-bit */
du_int __fixunssfdi(float a)
{
    if (a < 0.0f) return 0;
    return (du_int)a;
}

/* double -> unsigned 64-bit */
du_int __fixunsdfdi(double a)
{
    if (a < 0.0) return 0;
    return (du_int)a;
}

/* ========================================================================= */
/* Integer-to-float conversions                                              */
/* ========================================================================= */

/* signed 64-bit -> double */
double __floatdidf(di_int a)
{
    return (double)a;
}

/* signed 64-bit -> float */
float __floatdisf(di_int a)
{
    return (float)a;
}

/* unsigned 64-bit -> double */
double __floatundidf(du_int a)
{
    return (double)a;
}

/* unsigned 64-bit -> float */
float __floatundisf(du_int a)
{
    return (float)a;
}

/* ========================================================================= */
/* Absolute value with overflow check                                        */
/* ========================================================================= */

/*
 * Forward-declare abort -- avoid pulling in full stdlib.h to keep
 * this translation unit self-contained.
 */
void abort(void) __attribute__((noreturn));

si_int __absvsi2(si_int a)
{
    if (a == (si_int)0x80000000)
        abort(); /* overflow: abs(INT_MIN) is undefined */
    return a < 0 ? -a : a;
}

di_int __absvdi2(di_int a)
{
    if (a == (di_int)0x8000000000000000LL)
        abort(); /* overflow: abs(LLONG_MIN) is undefined */
    return a < 0 ? -a : a;
}

/* ========================================================================= */
/* 64-bit negation                                                           */
/* ========================================================================= */

di_int __negdi2(di_int a)
{
    return -a;
}

/* ========================================================================= */
/* 64-bit comparison                                                         */
/* ========================================================================= */

/*
 * Returns: 0 if a < b, 1 if a == b, 2 if a > b
 * (Itanium ABI convention for __cmpdi2)
 */
si_int __cmpdi2(di_int a, di_int b)
{
    if (a < b) return 0;
    if (a > b) return 2;
    return 1;
}

si_int __ucmpdi2(du_int a, du_int b)
{
    if (a < b) return 0;
    if (a > b) return 2;
    return 1;
}

/* ========================================================================= */
/* Stack unwinding frame registration (no-op stubs)                          */
/* ========================================================================= */

/*
 * GCC-generated code may call __register_frame / __deregister_frame to
 * register DWARF .eh_frame sections for stack unwinding.  With our
 * integrated unwinder (unwind.c), we use the frame-pointer chain instead
 * of DWARF tables, so these are no-ops.  Once a full DWARF unwinder is
 * implemented, these should maintain a frame table.
 */
void __register_frame(void *begin)
{
    (void)begin;
}

void __deregister_frame(void *begin)
{
    (void)begin;
}

void __register_frame_info(void *begin, void *ob)
{
    (void)begin;
    (void)ob;
}

void *__deregister_frame_info(void *begin)
{
    (void)begin;
    return (void *)0;
}
