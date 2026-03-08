/*
 * VeridianOS libc -- openssl/err.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x error handling API.
 */

#ifndef _OPENSSL_ERR_H
#define _OPENSSL_ERR_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Get the earliest error code from the error queue. */
unsigned long ERR_get_error(void);

/** Peek at the earliest error code without removing it. */
unsigned long ERR_peek_error(void);

/** Get a human-readable error string. */
char *ERR_error_string(unsigned long e, char *buf);

/** Get a human-readable error string (reentrant). */
void ERR_error_string_n(unsigned long e, char *buf, size_t len);

/** Clear the error queue for the current thread. */
void ERR_clear_error(void);

/** Print errors to stderr. */
void ERR_print_errors_fp(void *fp);

/** Get the library name for an error code. */
const char *ERR_lib_error_string(unsigned long e);

/** Get the reason string for an error code. */
const char *ERR_reason_error_string(unsigned long e);

#ifdef __cplusplus
}
#endif

#endif /* _OPENSSL_ERR_H */
