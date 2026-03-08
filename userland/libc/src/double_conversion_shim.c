/*
 * VeridianOS libc -- double_conversion_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * double-conversion 3.3.x wrappers.
 * Wraps stdlib strtod/strtof and snprintf for double-to-string and
 * string-to-double conversions.
 */

#include <double-conversion/double-conversion.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

double dc_strtod(const char *str, char **endptr)
{
    if (str == NULL) return 0.0;
    return strtod(str, endptr);
}

float dc_strtof(const char *str, char **endptr)
{
    if (str == NULL) return 0.0f;
    return strtof(str, endptr);
}

int dc_dtoa(double value, char *buf, size_t bufsize, int precision)
{
    if (buf == NULL || bufsize == 0) return 0;
    if (precision < 0) precision = 6;
    return snprintf(buf, bufsize, "%.*g", precision, value);
}

int dc_ftoa(float value, char *buf, size_t bufsize, int precision)
{
    if (buf == NULL || bufsize == 0) return 0;
    if (precision < 0) precision = 6;
    return snprintf(buf, bufsize, "%.*g", precision, (double)value);
}
