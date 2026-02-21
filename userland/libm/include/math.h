/*
 * VeridianOS libm -- math.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Standard math library declarations for VeridianOS user-space programs.
 * All implementations are freestanding -- no host libc/libm dependency.
 */

#ifndef _VERIDIAN_MATH_H
#define _VERIDIAN_MATH_H

/* ========================================================================= */
/* Mathematical constants                                                    */
/* ========================================================================= */

#define M_PI        3.14159265358979323846
#define M_PI_2      1.57079632679489661923
#define M_PI_4      0.78539816339744830962
#define M_1_PI      0.31830988618379067154
#define M_2_PI      0.63661977236758134308
#define M_E         2.71828182845904523536
#define M_LOG2E     1.44269504088896340736
#define M_LOG10E    0.43429448190325182765
#define M_LN2       0.69314718055994530942
#define M_LN10      2.30258509299404568402
#define M_SQRT2     1.41421356237309504880
#define M_SQRT1_2   0.70710678118654752440
#define M_SQRT3     1.73205080756887729353
#define M_2_SQRTPI  1.12837916709551257390

/* ========================================================================= */
/* IEEE 754 special values                                                   */
/* ========================================================================= */

#define HUGE_VAL    (__builtin_huge_val())
#define HUGE_VALF   (__builtin_huge_valf())
#define HUGE_VALL   (__builtin_huge_vall())
#define INFINITY    (__builtin_inff())
#define NAN         (__builtin_nanf(""))

/* ========================================================================= */
/* Classification macros                                                     */
/* ========================================================================= */

#define FP_NAN       0
#define FP_INFINITE  1
#define FP_ZERO      2
#define FP_SUBNORMAL 3
#define FP_NORMAL    4

#define isnan(x)    (__builtin_isnan(x))
#define isinf(x)    (__builtin_isinf(x))
#define isfinite(x) (__builtin_isfinite(x))
#define isnormal(x) (__builtin_isnormal(x))
#define signbit(x)  (__builtin_signbit(x))

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Function declarations -- double precision                                 */
/* ========================================================================= */

/* Absolute value */
double fabs(double x);

/* Rounding */
double floor(double x);
double ceil(double x);

/* Remainder */
double fmod(double x, double y);

/* Square root */
double sqrt(double x);

/* Powers and logarithms */
double pow(double base, double exp);
double log(double x);
double exp(double x);

/* Trigonometric */
double sin(double x);
double cos(double x);
double tan(double x);
double asin(double x);
double acos(double x);
double atan(double x);
double atan2(double y, double x);

/* Logarithmic variants */
double log10(double x);
double log2(double x);

/* Rounding and value manipulation */
double round(double x);
double trunc(double x);
double copysign(double x, double y);
double fmin(double x, double y);
double fmax(double x, double y);
double remainder(double x, double y);
double fdim(double x, double y);
double hypot(double x, double y);

/* Decomposition */
double frexp(double x, int *exp);
double ldexp(double x, int exp);
double modf(double x, double *iptr);

/* ========================================================================= */
/* Function declarations -- single precision                                 */
/* ========================================================================= */

float fabsf(float x);
float floorf(float x);
float ceilf(float x);
float sqrtf(float x);
float sinf(float x);
float cosf(float x);
float tanf(float x);
float asinf(float x);
float acosf(float x);
float atanf(float x);
float atan2f(float y, float x);
float logf(float x);
float log10f(float x);
float log2f(float x);
float expf(float x);
float powf(float base, float exp);
float fmodf(float x, float y);
float roundf(float x);
float truncf(float x);
float copysignf(float x, float y);
float fminf(float x, float y);
float fmaxf(float x, float y);
float hypotf(float x, float y);

#ifdef __cplusplus
}
#endif

#endif /* _VERIDIAN_MATH_H */
