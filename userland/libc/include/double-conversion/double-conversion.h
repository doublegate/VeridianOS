/*
 * VeridianOS libc -- double-conversion/double-conversion.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * double-conversion 3.3.x compatible API.
 * High-quality double-to-string and string-to-double conversions.
 * VeridianOS wraps the standard strtod/snprintf functions.
 */

#ifndef _DOUBLE_CONVERSION_H
#define _DOUBLE_CONVERSION_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Convert a string to double with high precision.
 * Wraps strtod() from stdlib.
 */
double dc_strtod(const char *str, char **endptr);

/**
 * Convert a string to float with high precision.
 * Wraps strtof() from stdlib.
 */
float dc_strtof(const char *str, char **endptr);

/**
 * Convert a double to string with specified precision.
 * Returns number of characters written (excluding NUL).
 */
int dc_dtoa(double value, char *buf, size_t bufsize, int precision);

/**
 * Convert a float to string with specified precision.
 * Returns number of characters written (excluding NUL).
 */
int dc_ftoa(float value, char *buf, size_t bufsize, int precision);

#ifdef __cplusplus
}
#endif

#endif /* _DOUBLE_CONVERSION_H */
