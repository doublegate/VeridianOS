/*
 * VeridianOS libc -- openssl/bio.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x BIO (Basic I/O) API.
 */

#ifndef _OPENSSL_BIO_H
#define _OPENSSL_BIO_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct bio_st       BIO;
typedef struct bio_method_st BIO_METHOD;

/** Create a new BIO with the given method. */
BIO *BIO_new(const BIO_METHOD *type);

/** Free a BIO. */
int BIO_free(BIO *a);

/** Free a BIO chain. */
void BIO_free_all(BIO *a);

/** Read from a BIO. */
int BIO_read(BIO *b, void *data, int dlen);

/** Write to a BIO. */
int BIO_write(BIO *b, const void *data, int dlen);

/** Formatted output to a BIO. */
int BIO_printf(BIO *bio, const char *format, ...);

/** Get the memory BIO method. */
const BIO_METHOD *BIO_s_mem(void);

/** Get the file BIO method. */
const BIO_METHOD *BIO_s_file(void);

/** Get the socket BIO method. */
const BIO_METHOD *BIO_s_socket(void);

/** Create a BIO for a file. */
BIO *BIO_new_file(const char *filename, const char *mode);

/** Create a memory BIO from a buffer. */
BIO *BIO_new_mem_buf(const void *buf, int len);

/** Push a BIO onto a chain. */
BIO *BIO_push(BIO *b, BIO *next);

/** Pop a BIO from a chain. */
BIO *BIO_pop(BIO *b);

/** Flush pending output. */
int BIO_flush(BIO *b);

#ifdef __cplusplus
}
#endif

#endif /* _OPENSSL_BIO_H */
