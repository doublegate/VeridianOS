/*
 * VeridianOS libm -- math.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Software implementations of standard math functions.
 * All implementations are freestanding -- no host libc/libm dependency.
 *
 * Precision: >= 10 digits for all functions (sufficient for GCC bootstrap).
 * These are correct reference implementations, not speed-optimized.
 */

#include <math.h>

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

/* Union for double bit manipulation (IEEE 754 double: 1 sign, 11 exp, 52 mantissa) */
typedef union {
    double   d;
    unsigned long long u;
} double_bits;

/* Union for float bit manipulation (IEEE 754 float: 1 sign, 8 exp, 23 mantissa) */
typedef union {
    float        f;
    unsigned int u;
} float_bits;

/* Extract biased exponent from double */
static int _double_exponent(double x)
{
    double_bits db;
    db.d = x;
    return (int)((db.u >> 52) & 0x7FF);
}

/* ========================================================================= */
/* Absolute value                                                            */
/* ========================================================================= */

double fabs(double x)
{
    double_bits db;
    db.d = x;
    db.u &= ~(1ULL << 63);  /* Clear sign bit */
    return db.d;
}

float fabsf(float x)
{
    float_bits fb;
    fb.f = x;
    fb.u &= ~(1U << 31);  /* Clear sign bit */
    return fb.f;
}

/* ========================================================================= */
/* Floor / Ceil                                                              */
/* ========================================================================= */

double floor(double x)
{
    /* Handle special values */
    if (isnan(x) || isinf(x))
        return x;

    /* If |x| >= 2^52, it is already an integer (no fractional bits in double) */
    if (x >= 4503599627370496.0 || x <= -4503599627370496.0)
        return x;

    /* Truncate toward zero */
    long long i = (long long)x;
    double truncated = (double)i;

    /* floor: if negative and had a fractional part, subtract 1 */
    if (truncated > x)
        truncated -= 1.0;

    return truncated;
}

double ceil(double x)
{
    /* Handle special values */
    if (isnan(x) || isinf(x))
        return x;

    if (x >= 4503599627370496.0 || x <= -4503599627370496.0)
        return x;

    long long i = (long long)x;
    double truncated = (double)i;

    /* ceil: if positive and had a fractional part, add 1 */
    if (truncated < x)
        truncated += 1.0;

    return truncated;
}

float floorf(float x)
{
    return (float)floor((double)x);
}

float ceilf(float x)
{
    return (float)ceil((double)x);
}

/* ========================================================================= */
/* Floating-point remainder                                                  */
/* ========================================================================= */

double fmod(double x, double y)
{
    /* Handle special cases */
    if (isnan(x) || isnan(y) || isinf(x) || y == 0.0)
        return NAN;
    if (isinf(y))
        return x;

    /* fmod(x, y) = x - trunc(x/y) * y */
    double quotient = x / y;
    long long trunc_q;

    /* Truncate toward zero */
    if (quotient >= 0.0)
        trunc_q = (long long)quotient;
    else
        trunc_q = (long long)quotient;  /* C truncation is toward zero */

    return x - (double)trunc_q * y;
}

/* ========================================================================= */
/* Square root -- Newton-Raphson iteration                                   */
/* ========================================================================= */

double sqrt(double x)
{
    /* Handle special cases */
    if (x < 0.0)
        return NAN;
    if (x == 0.0 || isnan(x) || isinf(x))
        return x;

    /* Initial guess using bit manipulation:
     * Halve the exponent for a rough sqrt estimate */
    double_bits db;
    db.d = x;
    db.u = (db.u >> 1) + (0x1FF8000000000000ULL);  /* Rough sqrt via exponent halving */
    double guess = db.d;

    /* Newton-Raphson: x_{n+1} = (x_n + S/x_n) / 2
     * 6 iterations gives >15 digits of precision */
    for (int i = 0; i < 6; i++)
        guess = 0.5 * (guess + x / guess);

    return guess;
}

/* ========================================================================= */
/* Exponential -- exp(x) via range reduction + Taylor series                 */
/* ========================================================================= */

double exp(double x)
{
    /* Handle special cases */
    if (isnan(x))
        return NAN;
    if (x > 709.0)
        return HUGE_VAL;  /* Overflow */
    if (x < -745.0)
        return 0.0;       /* Underflow */

    /* Range reduction: exp(x) = 2^k * exp(r)
     * where x = k * ln(2) + r, |r| <= ln(2)/2 */
    int k = (int)(x * M_LOG2E + (x >= 0.0 ? 0.5 : -0.5));
    double r = x - (double)k * M_LN2;

    /* Taylor series for exp(r): 1 + r + r^2/2! + r^3/3! + ...
     * With |r| <= 0.347, 13 terms give >15 digits */
    double term = 1.0;
    double sum = 1.0;
    for (int i = 1; i <= 20; i++) {
        term *= r / (double)i;
        sum += term;
        if (fabs(term) < 1e-16 * fabs(sum))
            break;
    }

    /* Multiply by 2^k using ldexp */
    return ldexp(sum, k);
}

/* ========================================================================= */
/* Natural logarithm -- series expansion                                     */
/* ========================================================================= */

double log(double x)
{
    /* Handle special cases */
    if (isnan(x) || x < 0.0)
        return NAN;
    if (x == 0.0)
        return -HUGE_VAL;
    if (isinf(x))
        return HUGE_VAL;

    /* Decompose: x = m * 2^e, where 0.5 <= m < 1.0
     * log(x) = log(m) + e * log(2) */
    int e;
    double m = frexp(x, &e);

    /* Adjust so m is in [sqrt(2)/2, sqrt(2)] for better convergence:
     * If m < sqrt(2)/2 (~0.7071), multiply by 2 and decrement e */
    if (m < M_SQRT1_2) {
        m *= 2.0;
        e--;
    }

    /* Now compute log(m) where m is near 1.
     * Let f = (m - 1) / (m + 1), then:
     * log(m) = 2 * (f + f^3/3 + f^5/5 + f^7/7 + ...)
     * This converges well since |f| < 0.172 */
    double f = (m - 1.0) / (m + 1.0);
    double f2 = f * f;
    double sum = 0.0;
    double term = f;

    for (int i = 0; i < 30; i++) {
        sum += term / (double)(2 * i + 1);
        term *= f2;
        if (fabs(term) < 1e-16)
            break;
    }
    sum *= 2.0;

    return sum + (double)e * M_LN2;
}

/* ========================================================================= */
/* Power -- pow(base, exp)                                                   */
/* ========================================================================= */

double pow(double base, double exponent)
{
    /* Handle special cases */
    if (isnan(base) || isnan(exponent))
        return NAN;

    /* Anything to the power of 0 is 1 */
    if (exponent == 0.0)
        return 1.0;

    /* 1 to any power is 1 */
    if (base == 1.0)
        return 1.0;

    /* 0 to positive power is 0, 0 to negative power is inf */
    if (base == 0.0) {
        if (exponent > 0.0)
            return 0.0;
        else
            return HUGE_VAL;
    }

    /* Integer exponent fast path */
    double int_part;
    if (modf(exponent, &int_part) == 0.0 && fabs(int_part) <= 2147483647.0) {
        int n = (int)int_part;
        int neg = 0;
        double result = 1.0;
        double b = base;

        if (n < 0) {
            neg = 1;
            n = -n;
        }

        /* Exponentiation by squaring */
        while (n > 0) {
            if (n & 1)
                result *= b;
            b *= b;
            n >>= 1;
        }

        return neg ? 1.0 / result : result;
    }

    /* Negative base with non-integer exponent is NaN */
    if (base < 0.0)
        return NAN;

    /* General case: pow(base, exp) = exp(exp * log(base)) */
    return exp(exponent * log(base));
}

/* ========================================================================= */
/* Trigonometric functions                                                   */
/* ========================================================================= */

/*
 * Reduce angle x to the range [-pi, pi] using:
 *   x = x - 2*pi * round(x / (2*pi))
 */
static double _reduce_angle(double x)
{
    static const double TWO_PI     = 6.28318530717958647693;
    static const double INV_TWO_PI = 0.15915494309189533577;

    if (x >= -M_PI && x <= M_PI)
        return x;

    /* Round to nearest integer */
    double k = x * INV_TWO_PI;
    if (k >= 0.0)
        k = (double)(long long)(k + 0.5);
    else
        k = (double)(long long)(k - 0.5);

    return x - k * TWO_PI;
}

double sin(double x)
{
    /* Handle special cases */
    if (isnan(x) || isinf(x))
        return NAN;

    /* Reduce to [-pi, pi] */
    x = _reduce_angle(x);

    /* Taylor series: sin(x) = x - x^3/3! + x^5/5! - x^7/7! + ...
     * With |x| <= pi, 15 terms give >15 digits */
    double x2 = x * x;
    double term = x;
    double sum = x;

    for (int i = 1; i <= 15; i++) {
        term *= -x2 / (double)(2 * i * (2 * i + 1));
        sum += term;
        if (fabs(term) < 1e-16 * fabs(sum))
            break;
    }

    return sum;
}

double cos(double x)
{
    /* cos(x) = sin(x + pi/2) */
    return sin(x + M_PI_2);
}

/* ========================================================================= */
/* Decomposition functions                                                   */
/* ========================================================================= */

/*
 * frexp -- Extract mantissa and exponent.
 * Returns m such that x = m * 2^(*exp), where 0.5 <= |m| < 1.0.
 */
double frexp(double x, int *exp)
{
    double_bits db;
    db.d = x;

    /* Handle zero */
    if (x == 0.0) {
        *exp = 0;
        return 0.0;
    }

    /* Handle special values */
    if (isnan(x) || isinf(x)) {
        *exp = 0;
        return x;
    }

    int biased_exp = _double_exponent(x);

    /* Handle subnormals: multiply by 2^64 to normalize, then adjust */
    if (biased_exp == 0) {
        db.d *= 18446744073709551616.0;  /* 2^64 */
        biased_exp = _double_exponent(db.d);
        *exp = biased_exp - 1022 - 64;
    } else {
        *exp = biased_exp - 1022;
    }

    /* Set exponent to -1 (biased 1022) so result is in [0.5, 1.0) */
    db.u = (db.u & 0x800FFFFFFFFFFFFFULL) | 0x3FE0000000000000ULL;

    return db.d;
}

/*
 * ldexp -- Load exponent: return x * 2^exp.
 */
double ldexp(double x, int exp)
{
    /* Handle special values */
    if (x == 0.0 || isnan(x) || isinf(x))
        return x;

    /* Apply exponent in steps to avoid overflow in intermediate */
    while (exp > 1023) {
        x *= 8.98846567431158e+307;  /* 2^1023 */
        exp -= 1023;
    }
    while (exp < -1022) {
        x *= 2.2250738585072014e-308;  /* 2^-1022 */
        exp += 1022;
    }

    /* Final scaling: construct 2^exp as a double and multiply */
    double_bits db;
    db.u = (unsigned long long)(exp + 1023) << 52;
    return x * db.d;
}

/*
 * modf -- Split into integer and fractional parts.
 * Both have the same sign as x.
 */
double modf(double x, double *iptr)
{
    /* Handle special values */
    if (isnan(x)) {
        *iptr = NAN;
        return NAN;
    }
    if (isinf(x)) {
        *iptr = x;
        return 0.0;  /* Fractional part is zero (with correct sign handled below) */
    }

    double i = (x >= 0.0) ? floor(x) : ceil(x);
    *iptr = i;

    /* Preserve sign of zero: modf(-0.0, &i) should return -0.0 */
    double frac = x - i;
    if (frac == 0.0 && x < 0.0) {
        double_bits db;
        db.d = 0.0;
        db.u |= (1ULL << 63);  /* Set sign bit for -0.0 */
        return db.d;
    }
    return frac;
}
