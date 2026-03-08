/*
 * VeridianOS libc -- openssl/x509.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x X.509 certificate API.
 */

#ifndef _OPENSSL_X509_H
#define _OPENSSL_X509_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct x509_st          X509;
typedef struct x509_name_st     X509_NAME;
typedef struct x509_store_st    X509_STORE;
typedef struct x509_store_ctx_st X509_STORE_CTX;

/** Free an X509 certificate. */
void X509_free(X509 *a);

/** Get the subject name from a certificate. */
X509_NAME *X509_get_subject_name(const X509 *a);

/** Get the issuer name from a certificate. */
X509_NAME *X509_get_issuer_name(const X509 *a);

/** Get a one-line string from an X509_NAME. */
char *X509_NAME_oneline(const X509_NAME *a, char *buf, int size);

/** Get the version of a certificate. */
long X509_get_version(const X509 *x);

/** Verify a certificate. */
int X509_verify_cert(X509_STORE_CTX *ctx);

/** Create a new X509 store. */
X509_STORE *X509_STORE_new(void);

/** Free an X509 store. */
void X509_STORE_free(X509_STORE *v);

/** V_OK constant for verify result */
#define X509_V_OK 0

#ifdef __cplusplus
}
#endif

#endif /* _OPENSSL_X509_H */
