/*
 * VeridianOS libc -- openssl/crypto.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x crypto utility API.
 */

#ifndef _OPENSSL_CRYPTO_H
#define _OPENSSL_CRYPTO_H

#include "opensslv.h"
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Version query types */
#define OPENSSL_VERSION     0
#define OPENSSL_CFLAGS      1
#define OPENSSL_BUILT_ON    2
#define OPENSSL_PLATFORM    3
#define OPENSSL_DIR         4
#define OPENSSL_ENGINES_DIR 5

/** Get the OpenSSL version string. */
const char *OpenSSL_version(int type);

/** Library initialization (no-op for 3.x). */
int OPENSSL_init_crypto(uint64_t opts, const void *settings);

/** Secure memory allocation. */
void *OPENSSL_malloc(size_t num);

/** Secure memory free. */
void OPENSSL_free(void *addr);

/** Secure memory zeroing. */
void OPENSSL_cleanse(void *ptr, size_t len);

/** Get the number of available CPUs. */
int OPENSSL_get_cpu_count(void);

#ifdef __cplusplus
}
#endif

#endif /* _OPENSSL_CRYPTO_H */
