/*
 * VeridianOS libc -- openssl/pem.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x PEM file I/O API.
 */

#ifndef _OPENSSL_PEM_H
#define _OPENSSL_PEM_H

#include "evp.h"
#include "x509.h"
#include <stdio.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef int pem_password_cb(char *buf, int size, int rwflag, void *userdata);

/** Read an X509 certificate from a PEM file. */
X509 *PEM_read_X509(FILE *fp, X509 **x, pem_password_cb *cb, void *u);

/** Read a private key from a PEM file. */
EVP_PKEY *PEM_read_PrivateKey(FILE *fp, EVP_PKEY **x,
                               pem_password_cb *cb, void *u);

/** Write an X509 certificate to a PEM file. */
int PEM_write_X509(FILE *fp, const X509 *x);

/** Write a private key to a PEM file. */
int PEM_write_PrivateKey(FILE *fp, const EVP_PKEY *x,
                         const EVP_CIPHER *enc,
                         const unsigned char *kstr, int klen,
                         pem_password_cb *cb, void *u);

#ifdef __cplusplus
}
#endif

#endif /* _OPENSSL_PEM_H */
