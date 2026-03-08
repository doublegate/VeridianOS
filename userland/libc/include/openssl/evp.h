/*
 * VeridianOS libc -- openssl/evp.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x EVP (high-level crypto) API.
 */

#ifndef _OPENSSL_EVP_H
#define _OPENSSL_EVP_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

typedef struct evp_md_st        EVP_MD;
typedef struct evp_md_ctx_st    EVP_MD_CTX;
typedef struct evp_cipher_st    EVP_CIPHER;
typedef struct evp_cipher_ctx_st EVP_CIPHER_CTX;
typedef struct evp_pkey_st      EVP_PKEY;
typedef struct evp_pkey_ctx_st  EVP_PKEY_CTX;

/* ========================================================================= */
/* Message digest (hash) functions                                           */
/* ========================================================================= */

/** Create a new message digest context. */
EVP_MD_CTX *EVP_MD_CTX_new(void);

/** Free a message digest context. */
void EVP_MD_CTX_free(EVP_MD_CTX *ctx);

/** Initialize a digest operation. */
int EVP_DigestInit(EVP_MD_CTX *ctx, const EVP_MD *type);

/** Initialize a digest operation (extended). */
int EVP_DigestInit_ex(EVP_MD_CTX *ctx, const EVP_MD *type, void *impl);

/** Hash more data. */
int EVP_DigestUpdate(EVP_MD_CTX *ctx, const void *d, size_t cnt);

/** Finalize the digest and retrieve the hash. */
int EVP_DigestFinal(EVP_MD_CTX *ctx, unsigned char *md,
                    unsigned int *s);

/** Finalize the digest (extended). */
int EVP_DigestFinal_ex(EVP_MD_CTX *ctx, unsigned char *md,
                       unsigned int *s);

/** Get the SHA-256 digest method. */
const EVP_MD *EVP_sha256(void);

/** Get the SHA-384 digest method. */
const EVP_MD *EVP_sha384(void);

/** Get the SHA-512 digest method. */
const EVP_MD *EVP_sha512(void);

/** Get the SHA-1 digest method. */
const EVP_MD *EVP_sha1(void);

/** Get the MD5 digest method. */
const EVP_MD *EVP_md5(void);

/** Get the digest output size. */
int EVP_MD_get_size(const EVP_MD *md);

/** Get the digest block size. */
int EVP_MD_get_block_size(const EVP_MD *md);

/* Legacy name */
#define EVP_MD_size(md) EVP_MD_get_size(md)
#define EVP_MD_block_size(md) EVP_MD_get_block_size(md)

/** One-shot digest. */
int EVP_Digest(const void *data, size_t count,
               unsigned char *md, unsigned int *size,
               const EVP_MD *type, void *impl);

/* ========================================================================= */
/* Cipher functions (stubs)                                                  */
/* ========================================================================= */

EVP_CIPHER_CTX *EVP_CIPHER_CTX_new(void);
void EVP_CIPHER_CTX_free(EVP_CIPHER_CTX *ctx);

int EVP_EncryptInit_ex(EVP_CIPHER_CTX *ctx, const EVP_CIPHER *cipher,
                       void *impl, const unsigned char *key,
                       const unsigned char *iv);
int EVP_EncryptUpdate(EVP_CIPHER_CTX *ctx, unsigned char *out,
                      int *outl, const unsigned char *in, int inl);
int EVP_EncryptFinal_ex(EVP_CIPHER_CTX *ctx, unsigned char *out,
                        int *outl);

int EVP_DecryptInit_ex(EVP_CIPHER_CTX *ctx, const EVP_CIPHER *cipher,
                       void *impl, const unsigned char *key,
                       const unsigned char *iv);
int EVP_DecryptUpdate(EVP_CIPHER_CTX *ctx, unsigned char *out,
                      int *outl, const unsigned char *in, int inl);
int EVP_DecryptFinal_ex(EVP_CIPHER_CTX *ctx, unsigned char *out,
                        int *outl);

const EVP_CIPHER *EVP_aes_128_cbc(void);
const EVP_CIPHER *EVP_aes_256_cbc(void);
const EVP_CIPHER *EVP_aes_128_gcm(void);
const EVP_CIPHER *EVP_aes_256_gcm(void);

/* ========================================================================= */
/* PKEY functions (stubs)                                                    */
/* ========================================================================= */

EVP_PKEY *EVP_PKEY_new(void);
void EVP_PKEY_free(EVP_PKEY *pkey);
int EVP_PKEY_get_id(const EVP_PKEY *pkey);

#ifdef __cplusplus
}
#endif

#endif /* _OPENSSL_EVP_H */
