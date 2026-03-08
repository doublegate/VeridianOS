/*
 * VeridianOS libc -- openssl/ssl.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x SSL/TLS API.
 */

#ifndef _OPENSSL_SSL_H
#define _OPENSSL_SSL_H

#include "opensslv.h"
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

typedef struct ssl_st          SSL;
typedef struct ssl_ctx_st      SSL_CTX;
typedef struct ssl_method_st   SSL_METHOD;

/* ========================================================================= */
/* SSL method constructors                                                   */
/* ========================================================================= */

const SSL_METHOD *TLS_method(void);
const SSL_METHOD *TLS_client_method(void);
const SSL_METHOD *TLS_server_method(void);

/* ========================================================================= */
/* SSL_CTX functions                                                         */
/* ========================================================================= */

SSL_CTX *SSL_CTX_new(const SSL_METHOD *method);
void SSL_CTX_free(SSL_CTX *ctx);

int SSL_CTX_set_min_proto_version(SSL_CTX *ctx, int version);
int SSL_CTX_set_max_proto_version(SSL_CTX *ctx, int version);

int SSL_CTX_use_certificate_file(SSL_CTX *ctx, const char *file,
                                 int type);
int SSL_CTX_use_PrivateKey_file(SSL_CTX *ctx, const char *file,
                                int type);
int SSL_CTX_check_private_key(const SSL_CTX *ctx);
int SSL_CTX_load_verify_locations(SSL_CTX *ctx, const char *CAfile,
                                  const char *CApath);

long SSL_CTX_set_options(SSL_CTX *ctx, long options);
long SSL_CTX_clear_options(SSL_CTX *ctx, long options);

/* ========================================================================= */
/* SSL connection functions                                                  */
/* ========================================================================= */

SSL *SSL_new(SSL_CTX *ctx);
void SSL_free(SSL *ssl);

int SSL_set_fd(SSL *ssl, int fd);
int SSL_connect(SSL *ssl);
int SSL_accept(SSL *ssl);
int SSL_read(SSL *ssl, void *buf, int num);
int SSL_write(SSL *ssl, const void *buf, int num);
int SSL_shutdown(SSL *ssl);

int SSL_get_error(const SSL *ssl, int ret);
long SSL_get_verify_result(const SSL *ssl);
void *SSL_get_peer_certificate(const SSL *ssl);

const char *SSL_get_version(const SSL *ssl);
const char *SSL_get_cipher_name(const SSL *ssl);

/* ========================================================================= */
/* Protocol version constants                                                */
/* ========================================================================= */

#define TLS1_VERSION        0x0301
#define TLS1_1_VERSION      0x0302
#define TLS1_2_VERSION      0x0303
#define TLS1_3_VERSION      0x0304

/* File type constants */
#define SSL_FILETYPE_PEM    1
#define SSL_FILETYPE_ASN1   2

/* Error codes */
#define SSL_ERROR_NONE              0
#define SSL_ERROR_SSL               1
#define SSL_ERROR_WANT_READ         2
#define SSL_ERROR_WANT_WRITE        3
#define SSL_ERROR_WANT_X509_LOOKUP  4
#define SSL_ERROR_SYSCALL           5
#define SSL_ERROR_ZERO_RETURN       6
#define SSL_ERROR_WANT_CONNECT      7
#define SSL_ERROR_WANT_ACCEPT       8

/* Options */
#define SSL_OP_NO_SSLv2     0x01000000L
#define SSL_OP_NO_SSLv3     0x02000000L
#define SSL_OP_NO_TLSv1     0x04000000L
#define SSL_OP_NO_TLSv1_1   0x10000000L
#define SSL_OP_NO_TLSv1_2   0x08000000L

/* ========================================================================= */
/* Library initialization                                                    */
/* ========================================================================= */

int SSL_library_init(void);
void SSL_load_error_strings(void);

int OPENSSL_init_ssl(uint64_t opts, const void *settings);

#ifdef __cplusplus
}
#endif

#endif /* _OPENSSL_SSL_H */
