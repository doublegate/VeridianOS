/*
 * VeridianOS libc -- openssl_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OpenSSL 3.3.x shim.
 * Implements SHA-256 and SHA-512 hash functions functionally.
 * SSL/TLS, cipher, BIO, X509, and PEM functions are stubs.
 */

#include <openssl/ssl.h>
#include <openssl/err.h>
#include <openssl/crypto.h>
#include <openssl/evp.h>
#include <openssl/bio.h>
#include <openssl/x509.h>
#include <openssl/pem.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Version and initialization                                                */
/* ========================================================================= */

const char *OpenSSL_version(int type)
{
    switch (type) {
    case OPENSSL_VERSION:   return OPENSSL_VERSION_TEXT;
    case OPENSSL_CFLAGS:    return "compiler: gcc (VeridianOS)";
    case OPENSSL_BUILT_ON:  return "built on: VeridianOS";
    case OPENSSL_PLATFORM:  return "platform: x86_64-veridian";
    case OPENSSL_DIR:       return "/usr/local/ssl";
    case OPENSSL_ENGINES_DIR: return "/usr/local/lib/engines";
    default:                return OPENSSL_VERSION_TEXT;
    }
}

int OPENSSL_init_crypto(uint64_t opts, const void *settings)
{
    (void)opts;
    (void)settings;
    return 1;
}

void *OPENSSL_malloc(size_t num)
{
    return malloc(num);
}

void OPENSSL_free(void *addr)
{
    free(addr);
}

void OPENSSL_cleanse(void *ptr, size_t len)
{
    volatile unsigned char *p = (volatile unsigned char *)ptr;
    while (len--)
        *p++ = 0;
}

int OPENSSL_get_cpu_count(void)
{
    return 1;
}

/* ========================================================================= */
/* Error handling                                                            */
/* ========================================================================= */

static unsigned long last_error = 0;

unsigned long ERR_get_error(void)
{
    unsigned long e = last_error;
    last_error = 0;
    return e;
}

unsigned long ERR_peek_error(void)
{
    return last_error;
}

char *ERR_error_string(unsigned long e, char *buf)
{
    static char static_buf[256];
    char *out = buf ? buf : static_buf;
    if (e == 0) {
        strcpy(out, "no error");
    } else {
        /* Simple numeric representation */
        strcpy(out, "error:");
        /* Append number manually */
        {
            char num[20];
            int i = 0;
            unsigned long v = e;
            if (v == 0) {
                num[i++] = '0';
            } else {
                while (v > 0) {
                    num[i++] = '0' + (char)(v % 10);
                    v /= 10;
                }
            }
            num[i] = '\0';
            /* Reverse */
            {
                int j;
                for (j = 0; j < i / 2; j++) {
                    char tmp = num[j];
                    num[j] = num[i - 1 - j];
                    num[i - 1 - j] = tmp;
                }
            }
            strcat(out, num);
        }
    }
    return out;
}

void ERR_error_string_n(unsigned long e, char *buf, size_t len)
{
    char *s = ERR_error_string(e, NULL);
    if (buf && len > 0) {
        strncpy(buf, s, len - 1);
        buf[len - 1] = '\0';
    }
}

void ERR_clear_error(void)
{
    last_error = 0;
}

void ERR_print_errors_fp(void *fp)
{
    (void)fp;
}

const char *ERR_lib_error_string(unsigned long e)
{
    (void)e;
    return "lib(0)";
}

const char *ERR_reason_error_string(unsigned long e)
{
    (void)e;
    return "reason(0)";
}

/* ========================================================================= */
/* SHA-256 implementation                                                    */
/* ========================================================================= */

#define SHA256_BLOCK_SIZE  64
#define SHA256_DIGEST_SIZE 32
#define SHA512_BLOCK_SIZE  128
#define SHA512_DIGEST_SIZE 64

struct sha256_state {
    uint32_t h[8];
    uint64_t total;
    uint8_t  buf[SHA256_BLOCK_SIZE];
    int      buf_len;
};

static const uint32_t sha256_k[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
};

#define ROR32(x, n) (((x) >> (n)) | ((x) << (32 - (n))))

static void sha256_transform(struct sha256_state *s, const uint8_t *block)
{
    uint32_t w[64];
    uint32_t a, b, c, d, e, f, g, h;
    int i;

    for (i = 0; i < 16; i++) {
        w[i] = ((uint32_t)block[i * 4] << 24) |
               ((uint32_t)block[i * 4 + 1] << 16) |
               ((uint32_t)block[i * 4 + 2] << 8) |
               ((uint32_t)block[i * 4 + 3]);
    }
    for (i = 16; i < 64; i++) {
        uint32_t s0 = ROR32(w[i - 15], 7) ^ ROR32(w[i - 15], 18) ^
                       (w[i - 15] >> 3);
        uint32_t s1 = ROR32(w[i - 2], 17) ^ ROR32(w[i - 2], 19) ^
                       (w[i - 2] >> 10);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }

    a = s->h[0]; b = s->h[1]; c = s->h[2]; d = s->h[3];
    e = s->h[4]; f = s->h[5]; g = s->h[6]; h = s->h[7];

    for (i = 0; i < 64; i++) {
        uint32_t S1 = ROR32(e, 6) ^ ROR32(e, 11) ^ ROR32(e, 25);
        uint32_t ch = (e & f) ^ (~e & g);
        uint32_t temp1 = h + S1 + ch + sha256_k[i] + w[i];
        uint32_t S0 = ROR32(a, 2) ^ ROR32(a, 13) ^ ROR32(a, 22);
        uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
        uint32_t temp2 = S0 + maj;

        h = g; g = f; f = e;
        e = d + temp1;
        d = c; c = b; b = a;
        a = temp1 + temp2;
    }

    s->h[0] += a; s->h[1] += b; s->h[2] += c; s->h[3] += d;
    s->h[4] += e; s->h[5] += f; s->h[6] += g; s->h[7] += h;
}

static void sha256_init(struct sha256_state *s)
{
    s->h[0] = 0x6a09e667; s->h[1] = 0xbb67ae85;
    s->h[2] = 0x3c6ef372; s->h[3] = 0xa54ff53a;
    s->h[4] = 0x510e527f; s->h[5] = 0x9b05688c;
    s->h[6] = 0x1f83d9ab; s->h[7] = 0x5be0cd19;
    s->total = 0;
    s->buf_len = 0;
}

static void sha256_update(struct sha256_state *s, const uint8_t *data,
                           size_t len)
{
    size_t i;
    for (i = 0; i < len; i++) {
        s->buf[s->buf_len++] = data[i];
        s->total++;
        if (s->buf_len == SHA256_BLOCK_SIZE) {
            sha256_transform(s, s->buf);
            s->buf_len = 0;
        }
    }
}

static void sha256_final(struct sha256_state *s, uint8_t *digest)
{
    uint64_t bits = s->total * 8;
    int i;

    s->buf[s->buf_len++] = 0x80;
    while (s->buf_len != 56) {
        if (s->buf_len == SHA256_BLOCK_SIZE) {
            sha256_transform(s, s->buf);
            s->buf_len = 0;
        }
        s->buf[s->buf_len++] = 0;
    }

    for (i = 7; i >= 0; i--)
        s->buf[s->buf_len++] = (uint8_t)(bits >> (i * 8));

    sha256_transform(s, s->buf);

    for (i = 0; i < 8; i++) {
        digest[i * 4]     = (uint8_t)(s->h[i] >> 24);
        digest[i * 4 + 1] = (uint8_t)(s->h[i] >> 16);
        digest[i * 4 + 2] = (uint8_t)(s->h[i] >> 8);
        digest[i * 4 + 3] = (uint8_t)(s->h[i]);
    }
}

/* ========================================================================= */
/* SHA-512 implementation                                                    */
/* ========================================================================= */

struct sha512_state {
    uint64_t h[8];
    uint64_t total;
    uint8_t  buf[SHA512_BLOCK_SIZE];
    int      buf_len;
};

static const uint64_t sha512_k[80] = {
    0x428a2f98d728ae22ULL, 0x7137449123ef65cdULL,
    0xb5c0fbcfec4d3b2fULL, 0xe9b5dba58189dbbcULL,
    0x3956c25bf348b538ULL, 0x59f111f1b605d019ULL,
    0x923f82a4af194f9bULL, 0xab1c5ed5da6d8118ULL,
    0xd807aa98a3030242ULL, 0x12835b0145706fbeULL,
    0x243185be4ee4b28cULL, 0x550c7dc3d5ffb4e2ULL,
    0x72be5d74f27b896fULL, 0x80deb1fe3b1696b1ULL,
    0x9bdc06a725c71235ULL, 0xc19bf174cf692694ULL,
    0xe49b69c19ef14ad2ULL, 0xefbe4786384f25e3ULL,
    0x0fc19dc68b8cd5b5ULL, 0x240ca1cc77ac9c65ULL,
    0x2de92c6f592b0275ULL, 0x4a7484aa6ea6e483ULL,
    0x5cb0a9dcbd41fbd4ULL, 0x76f988da831153b5ULL,
    0x983e5152ee66dfabULL, 0xa831c66d2db43210ULL,
    0xb00327c898fb213fULL, 0xbf597fc7beef0ee4ULL,
    0xc6e00bf33da88fc2ULL, 0xd5a79147930aa725ULL,
    0x06ca6351e003826fULL, 0x142929670a0e6e70ULL,
    0x27b70a8546d22ffcULL, 0x2e1b21385c26c926ULL,
    0x4d2c6dfc5ac42aedULL, 0x53380d139d95b3dfULL,
    0x650a73548baf63deULL, 0x766a0abb3c77b2a8ULL,
    0x81c2c92e47edaee6ULL, 0x92722c851482353bULL,
    0xa2bfe8a14cf10364ULL, 0xa81a664bbc423001ULL,
    0xc24b8b70d0f89791ULL, 0xc76c51a30654be30ULL,
    0xd192e819d6ef5218ULL, 0xd69906245565a910ULL,
    0xf40e35855771202aULL, 0x106aa07032bbd1b8ULL,
    0x19a4c116b8d2d0c8ULL, 0x1e376c085141ab53ULL,
    0x2748774cdf8eeb99ULL, 0x34b0bcb5e19b48a8ULL,
    0x391c0cb3c5c95a63ULL, 0x4ed8aa4ae3418acbULL,
    0x5b9cca4f7763e373ULL, 0x682e6ff3d6b2b8a3ULL,
    0x748f82ee5defb2fcULL, 0x78a5636f43172f60ULL,
    0x84c87814a1f0ab72ULL, 0x8cc702081a6439ecULL,
    0x90befffa23631e28ULL, 0xa4506cebde82bde9ULL,
    0xbef9a3f7b2c67915ULL, 0xc67178f2e372532bULL,
    0xca273eceea26619cULL, 0xd186b8c721c0c207ULL,
    0xeada7dd6cde0eb1eULL, 0xf57d4f7fee6ed178ULL,
    0x06f067aa72176fbaULL, 0x0a637dc5a2c898a6ULL,
    0x113f9804bef90daeULL, 0x1b710b35131c471bULL,
    0x28db77f523047d84ULL, 0x32caab7b40c72493ULL,
    0x3c9ebe0a15c9bebcULL, 0x431d67c49c100d4cULL,
    0x4cc5d4becb3e42b6ULL, 0x597f299cfc657e2aULL,
    0x5fcb6fab3ad6faecULL, 0x6c44198c4a475817ULL
};

#define ROR64(x, n) (((x) >> (n)) | ((x) << (64 - (n))))

static void sha512_transform(struct sha512_state *s, const uint8_t *block)
{
    uint64_t w[80];
    uint64_t a, b, c, d, e, f, g, h;
    int i;

    for (i = 0; i < 16; i++) {
        w[i] = ((uint64_t)block[i * 8] << 56) |
               ((uint64_t)block[i * 8 + 1] << 48) |
               ((uint64_t)block[i * 8 + 2] << 40) |
               ((uint64_t)block[i * 8 + 3] << 32) |
               ((uint64_t)block[i * 8 + 4] << 24) |
               ((uint64_t)block[i * 8 + 5] << 16) |
               ((uint64_t)block[i * 8 + 6] << 8)  |
               ((uint64_t)block[i * 8 + 7]);
    }
    for (i = 16; i < 80; i++) {
        uint64_t s0 = ROR64(w[i - 15], 1) ^ ROR64(w[i - 15], 8) ^
                       (w[i - 15] >> 7);
        uint64_t s1 = ROR64(w[i - 2], 19) ^ ROR64(w[i - 2], 61) ^
                       (w[i - 2] >> 6);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }

    a = s->h[0]; b = s->h[1]; c = s->h[2]; d = s->h[3];
    e = s->h[4]; f = s->h[5]; g = s->h[6]; h = s->h[7];

    for (i = 0; i < 80; i++) {
        uint64_t S1 = ROR64(e, 14) ^ ROR64(e, 18) ^ ROR64(e, 41);
        uint64_t ch = (e & f) ^ (~e & g);
        uint64_t temp1 = h + S1 + ch + sha512_k[i] + w[i];
        uint64_t S0 = ROR64(a, 28) ^ ROR64(a, 34) ^ ROR64(a, 39);
        uint64_t maj = (a & b) ^ (a & c) ^ (b & c);
        uint64_t temp2 = S0 + maj;

        h = g; g = f; f = e;
        e = d + temp1;
        d = c; c = b; b = a;
        a = temp1 + temp2;
    }

    s->h[0] += a; s->h[1] += b; s->h[2] += c; s->h[3] += d;
    s->h[4] += e; s->h[5] += f; s->h[6] += g; s->h[7] += h;
}

static void sha512_init(struct sha512_state *s)
{
    s->h[0] = 0x6a09e667f3bcc908ULL; s->h[1] = 0xbb67ae8584caa73bULL;
    s->h[2] = 0x3c6ef372fe94f82bULL; s->h[3] = 0xa54ff53a5f1d36f1ULL;
    s->h[4] = 0x510e527fade682d1ULL; s->h[5] = 0x9b05688c2b3e6c1fULL;
    s->h[6] = 0x1f83d9abfb41bd6bULL; s->h[7] = 0x5be0cd19137e2179ULL;
    s->total = 0;
    s->buf_len = 0;
}

static void sha512_update(struct sha512_state *s, const uint8_t *data,
                           size_t len)
{
    size_t i;
    for (i = 0; i < len; i++) {
        s->buf[s->buf_len++] = data[i];
        s->total++;
        if (s->buf_len == SHA512_BLOCK_SIZE) {
            sha512_transform(s, s->buf);
            s->buf_len = 0;
        }
    }
}

static void sha512_final(struct sha512_state *s, uint8_t *digest)
{
    uint64_t bits = s->total * 8;
    int i;

    s->buf[s->buf_len++] = 0x80;
    while (s->buf_len != 112) {
        if (s->buf_len == SHA512_BLOCK_SIZE) {
            sha512_transform(s, s->buf);
            s->buf_len = 0;
        }
        s->buf[s->buf_len++] = 0;
    }

    /* 128-bit length (we only use low 64 bits) */
    for (i = 7; i >= 0; i--)
        s->buf[s->buf_len++] = 0;  /* high 64 bits */
    for (i = 7; i >= 0; i--)
        s->buf[s->buf_len++] = (uint8_t)(bits >> (i * 8));

    sha512_transform(s, s->buf);

    for (i = 0; i < 8; i++) {
        digest[i * 8]     = (uint8_t)(s->h[i] >> 56);
        digest[i * 8 + 1] = (uint8_t)(s->h[i] >> 48);
        digest[i * 8 + 2] = (uint8_t)(s->h[i] >> 40);
        digest[i * 8 + 3] = (uint8_t)(s->h[i] >> 32);
        digest[i * 8 + 4] = (uint8_t)(s->h[i] >> 24);
        digest[i * 8 + 5] = (uint8_t)(s->h[i] >> 16);
        digest[i * 8 + 6] = (uint8_t)(s->h[i] >> 8);
        digest[i * 8 + 7] = (uint8_t)(s->h[i]);
    }
}

/* ========================================================================= */
/* EVP digest API                                                            */
/* ========================================================================= */

#define MD_SHA256 1
#define MD_SHA512 2
#define MD_SHA384 3
#define MD_SHA1   4
#define MD_MD5    5

struct evp_md_st {
    int type;
    int digest_size;
    int block_size;
};

static struct evp_md_st md_sha256 = { MD_SHA256, 32, 64 };
static struct evp_md_st md_sha384 = { MD_SHA384, 48, 128 };
static struct evp_md_st md_sha512 = { MD_SHA512, 64, 128 };
static struct evp_md_st md_sha1   = { MD_SHA1,   20, 64 };
static struct evp_md_st md_md5    = { MD_MD5,    16, 64 };

struct evp_md_ctx_st {
    const EVP_MD *md;
    union {
        struct sha256_state sha256;
        struct sha512_state sha512;
    } state;
};

const EVP_MD *EVP_sha256(void) { return &md_sha256; }
const EVP_MD *EVP_sha384(void) { return &md_sha384; }
const EVP_MD *EVP_sha512(void) { return &md_sha512; }
const EVP_MD *EVP_sha1(void)   { return &md_sha1; }
const EVP_MD *EVP_md5(void)    { return &md_md5; }

int EVP_MD_get_size(const EVP_MD *md)
{
    return md ? md->digest_size : 0;
}

int EVP_MD_get_block_size(const EVP_MD *md)
{
    return md ? md->block_size : 0;
}

EVP_MD_CTX *EVP_MD_CTX_new(void)
{
    EVP_MD_CTX *ctx = (EVP_MD_CTX *)calloc(1, sizeof(*ctx));
    return ctx;
}

void EVP_MD_CTX_free(EVP_MD_CTX *ctx)
{
    if (ctx) {
        OPENSSL_cleanse(ctx, sizeof(*ctx));
        free(ctx);
    }
}

int EVP_DigestInit(EVP_MD_CTX *ctx, const EVP_MD *type)
{
    return EVP_DigestInit_ex(ctx, type, NULL);
}

int EVP_DigestInit_ex(EVP_MD_CTX *ctx, const EVP_MD *type, void *impl)
{
    (void)impl;
    if (ctx == NULL || type == NULL)
        return 0;

    ctx->md = type;
    switch (type->type) {
    case MD_SHA256:
        sha256_init(&ctx->state.sha256);
        break;
    case MD_SHA512:
    case MD_SHA384:
        sha512_init(&ctx->state.sha512);
        if (type->type == MD_SHA384) {
            /* SHA-384 uses different initial values */
            ctx->state.sha512.h[0] = 0xcbbb9d5dc1059ed8ULL;
            ctx->state.sha512.h[1] = 0x629a292a367cd507ULL;
            ctx->state.sha512.h[2] = 0x9159015a3070dd17ULL;
            ctx->state.sha512.h[3] = 0x152fecd8f70e5939ULL;
            ctx->state.sha512.h[4] = 0x67332667ffc00b31ULL;
            ctx->state.sha512.h[5] = 0x8eb44a8768581511ULL;
            ctx->state.sha512.h[6] = 0xdb0c2e0d64f98fa7ULL;
            ctx->state.sha512.h[7] = 0x47b5481dbefa4fa4ULL;
        }
        break;
    default:
        /* SHA-1 and MD5 not fully implemented; init as no-op */
        memset(&ctx->state, 0, sizeof(ctx->state));
        break;
    }
    return 1;
}

int EVP_DigestUpdate(EVP_MD_CTX *ctx, const void *d, size_t cnt)
{
    if (ctx == NULL || ctx->md == NULL)
        return 0;

    switch (ctx->md->type) {
    case MD_SHA256:
        sha256_update(&ctx->state.sha256, (const uint8_t *)d, cnt);
        break;
    case MD_SHA512:
    case MD_SHA384:
        sha512_update(&ctx->state.sha512, (const uint8_t *)d, cnt);
        break;
    default:
        break;
    }
    return 1;
}

int EVP_DigestFinal(EVP_MD_CTX *ctx, unsigned char *md,
                    unsigned int *s)
{
    return EVP_DigestFinal_ex(ctx, md, s);
}

int EVP_DigestFinal_ex(EVP_MD_CTX *ctx, unsigned char *md,
                       unsigned int *s)
{
    if (ctx == NULL || ctx->md == NULL || md == NULL)
        return 0;

    switch (ctx->md->type) {
    case MD_SHA256:
        sha256_final(&ctx->state.sha256, md);
        if (s) *s = 32;
        break;
    case MD_SHA512:
        sha512_final(&ctx->state.sha512, md);
        if (s) *s = 64;
        break;
    case MD_SHA384: {
        uint8_t full[64];
        sha512_final(&ctx->state.sha512, full);
        memcpy(md, full, 48);
        OPENSSL_cleanse(full, sizeof(full));
        if (s) *s = 48;
        break;
    }
    default:
        memset(md, 0, (size_t)ctx->md->digest_size);
        if (s) *s = (unsigned int)ctx->md->digest_size;
        break;
    }
    return 1;
}

int EVP_Digest(const void *data, size_t count,
               unsigned char *md, unsigned int *size,
               const EVP_MD *type, void *impl)
{
    EVP_MD_CTX *ctx = EVP_MD_CTX_new();
    int ret;
    (void)impl;

    if (ctx == NULL)
        return 0;

    ret = EVP_DigestInit_ex(ctx, type, NULL) &&
          EVP_DigestUpdate(ctx, data, count) &&
          EVP_DigestFinal_ex(ctx, md, size);

    EVP_MD_CTX_free(ctx);
    return ret;
}

/* ========================================================================= */
/* EVP cipher stubs                                                          */
/* ========================================================================= */

struct evp_cipher_st { int type; };
struct evp_cipher_ctx_st { int dummy; };

EVP_CIPHER_CTX *EVP_CIPHER_CTX_new(void)
{
    return (EVP_CIPHER_CTX *)calloc(1, sizeof(EVP_CIPHER_CTX));
}

void EVP_CIPHER_CTX_free(EVP_CIPHER_CTX *ctx) { free(ctx); }

int EVP_EncryptInit_ex(EVP_CIPHER_CTX *ctx, const EVP_CIPHER *cipher,
                       void *impl, const unsigned char *key,
                       const unsigned char *iv)
{
    (void)ctx; (void)cipher; (void)impl; (void)key; (void)iv;
    return 0;  /* Not implemented */
}

int EVP_EncryptUpdate(EVP_CIPHER_CTX *ctx, unsigned char *out,
                      int *outl, const unsigned char *in, int inl)
{
    (void)ctx; (void)out; (void)outl; (void)in; (void)inl;
    return 0;
}

int EVP_EncryptFinal_ex(EVP_CIPHER_CTX *ctx, unsigned char *out,
                        int *outl)
{
    (void)ctx; (void)out; (void)outl;
    return 0;
}

int EVP_DecryptInit_ex(EVP_CIPHER_CTX *ctx, const EVP_CIPHER *cipher,
                       void *impl, const unsigned char *key,
                       const unsigned char *iv)
{
    (void)ctx; (void)cipher; (void)impl; (void)key; (void)iv;
    return 0;
}

int EVP_DecryptUpdate(EVP_CIPHER_CTX *ctx, unsigned char *out,
                      int *outl, const unsigned char *in, int inl)
{
    (void)ctx; (void)out; (void)outl; (void)in; (void)inl;
    return 0;
}

int EVP_DecryptFinal_ex(EVP_CIPHER_CTX *ctx, unsigned char *out,
                        int *outl)
{
    (void)ctx; (void)out; (void)outl;
    return 0;
}

static struct evp_cipher_st cipher_aes128cbc = { 1 };
static struct evp_cipher_st cipher_aes256cbc = { 2 };
static struct evp_cipher_st cipher_aes128gcm = { 3 };
static struct evp_cipher_st cipher_aes256gcm = { 4 };

const EVP_CIPHER *EVP_aes_128_cbc(void) { return &cipher_aes128cbc; }
const EVP_CIPHER *EVP_aes_256_cbc(void) { return &cipher_aes256cbc; }
const EVP_CIPHER *EVP_aes_128_gcm(void) { return &cipher_aes128gcm; }
const EVP_CIPHER *EVP_aes_256_gcm(void) { return &cipher_aes256gcm; }

/* ========================================================================= */
/* PKEY stubs                                                                */
/* ========================================================================= */

struct evp_pkey_st { int type; };

EVP_PKEY *EVP_PKEY_new(void)
{
    return (EVP_PKEY *)calloc(1, sizeof(EVP_PKEY));
}

void EVP_PKEY_free(EVP_PKEY *pkey) { free(pkey); }

int EVP_PKEY_get_id(const EVP_PKEY *pkey)
{
    return pkey ? pkey->type : 0;
}

/* ========================================================================= */
/* SSL/TLS stubs                                                             */
/* ========================================================================= */

struct ssl_method_st { int version; };
struct ssl_ctx_st { const SSL_METHOD *method; long options; };
struct ssl_st { SSL_CTX *ctx; int fd; int error; };

static struct ssl_method_st tls_method_st   = { TLS1_3_VERSION };
static struct ssl_method_st tls_client_st   = { TLS1_3_VERSION };
static struct ssl_method_st tls_server_st   = { TLS1_3_VERSION };

const SSL_METHOD *TLS_method(void)        { return &tls_method_st; }
const SSL_METHOD *TLS_client_method(void) { return &tls_client_st; }
const SSL_METHOD *TLS_server_method(void) { return &tls_server_st; }

SSL_CTX *SSL_CTX_new(const SSL_METHOD *method)
{
    SSL_CTX *ctx = (SSL_CTX *)calloc(1, sizeof(*ctx));
    if (ctx) ctx->method = method;
    return ctx;
}

void SSL_CTX_free(SSL_CTX *ctx) { free(ctx); }

int SSL_CTX_set_min_proto_version(SSL_CTX *ctx, int version)
{
    (void)ctx; (void)version; return 1;
}

int SSL_CTX_set_max_proto_version(SSL_CTX *ctx, int version)
{
    (void)ctx; (void)version; return 1;
}

int SSL_CTX_use_certificate_file(SSL_CTX *ctx, const char *file, int type)
{
    (void)ctx; (void)file; (void)type; return 0;
}

int SSL_CTX_use_PrivateKey_file(SSL_CTX *ctx, const char *file, int type)
{
    (void)ctx; (void)file; (void)type; return 0;
}

int SSL_CTX_check_private_key(const SSL_CTX *ctx)
{
    (void)ctx; return 0;
}

int SSL_CTX_load_verify_locations(SSL_CTX *ctx, const char *CAfile,
                                  const char *CApath)
{
    (void)ctx; (void)CAfile; (void)CApath; return 0;
}

long SSL_CTX_set_options(SSL_CTX *ctx, long options)
{
    if (ctx) ctx->options |= options;
    return ctx ? ctx->options : 0;
}

long SSL_CTX_clear_options(SSL_CTX *ctx, long options)
{
    if (ctx) ctx->options &= ~options;
    return ctx ? ctx->options : 0;
}

SSL *SSL_new(SSL_CTX *ctx)
{
    SSL *ssl = (SSL *)calloc(1, sizeof(*ssl));
    if (ssl) ssl->ctx = ctx;
    return ssl;
}

void SSL_free(SSL *ssl) { free(ssl); }

int SSL_set_fd(SSL *ssl, int fd)
{
    if (ssl) ssl->fd = fd;
    return ssl ? 1 : 0;
}

int SSL_connect(SSL *ssl)
{
    (void)ssl;
    last_error = 1;
    return -1;  /* TLS not implemented */
}

int SSL_accept(SSL *ssl) { (void)ssl; return -1; }
int SSL_read(SSL *ssl, void *buf, int num)
{
    (void)ssl; (void)buf; (void)num; return -1;
}

int SSL_write(SSL *ssl, const void *buf, int num)
{
    (void)ssl; (void)buf; (void)num; return -1;
}

int SSL_shutdown(SSL *ssl) { (void)ssl; return 0; }

int SSL_get_error(const SSL *ssl, int ret)
{
    (void)ssl; (void)ret;
    return SSL_ERROR_SSL;
}

long SSL_get_verify_result(const SSL *ssl)
{
    (void)ssl; return X509_V_OK;
}

void *SSL_get_peer_certificate(const SSL *ssl)
{
    (void)ssl; return NULL;
}

const char *SSL_get_version(const SSL *ssl)
{
    (void)ssl; return "TLSv1.3";
}

const char *SSL_get_cipher_name(const SSL *ssl)
{
    (void)ssl; return "(NONE)";
}

int SSL_library_init(void) { return 1; }
void SSL_load_error_strings(void) { }

int OPENSSL_init_ssl(uint64_t opts, const void *settings)
{
    (void)opts; (void)settings; return 1;
}

/* ========================================================================= */
/* BIO stubs                                                                 */
/* ========================================================================= */

struct bio_st { int type; };
struct bio_method_st { int type; };

static struct bio_method_st bio_s_mem_m    = { 1 };
static struct bio_method_st bio_s_file_m   = { 2 };
static struct bio_method_st bio_s_socket_m = { 3 };

BIO *BIO_new(const BIO_METHOD *type) { (void)type; return NULL; }
int BIO_free(BIO *a) { (void)a; return 1; }
void BIO_free_all(BIO *a) { (void)a; }
int BIO_read(BIO *b, void *data, int dlen) { (void)b; (void)data; (void)dlen; return -1; }
int BIO_write(BIO *b, const void *data, int dlen) { (void)b; (void)data; (void)dlen; return -1; }
int BIO_printf(BIO *bio, const char *format, ...) { (void)bio; (void)format; return 0; }
const BIO_METHOD *BIO_s_mem(void) { return &bio_s_mem_m; }
const BIO_METHOD *BIO_s_file(void) { return &bio_s_file_m; }
const BIO_METHOD *BIO_s_socket(void) { return &bio_s_socket_m; }
BIO *BIO_new_file(const char *filename, const char *mode) { (void)filename; (void)mode; return NULL; }
BIO *BIO_new_mem_buf(const void *buf, int len) { (void)buf; (void)len; return NULL; }
BIO *BIO_push(BIO *b, BIO *next) { (void)b; (void)next; return b; }
BIO *BIO_pop(BIO *b) { (void)b; return NULL; }
int BIO_flush(BIO *b) { (void)b; return 1; }

/* ========================================================================= */
/* X509 stubs                                                                */
/* ========================================================================= */

struct x509_st { int dummy; };
struct x509_name_st { char oneline[256]; };
struct x509_store_st { int dummy; };
struct x509_store_ctx_st { int dummy; };

void X509_free(X509 *a) { free(a); }

X509_NAME *X509_get_subject_name(const X509 *a) { (void)a; return NULL; }
X509_NAME *X509_get_issuer_name(const X509 *a) { (void)a; return NULL; }

char *X509_NAME_oneline(const X509_NAME *a, char *buf, int size)
{
    (void)a;
    if (buf && size > 0) {
        strncpy(buf, "/CN=unknown", (size_t)size - 1);
        buf[size - 1] = '\0';
        return buf;
    }
    return NULL;
}

long X509_get_version(const X509 *x) { (void)x; return 2; /* v3 */ }
int X509_verify_cert(X509_STORE_CTX *ctx) { (void)ctx; return 0; }

X509_STORE *X509_STORE_new(void)
{
    return (X509_STORE *)calloc(1, sizeof(X509_STORE));
}

void X509_STORE_free(X509_STORE *v) { free(v); }

/* ========================================================================= */
/* PEM stubs                                                                 */
/* ========================================================================= */

X509 *PEM_read_X509(FILE *fp, X509 **x, pem_password_cb *cb, void *u)
{
    (void)fp; (void)x; (void)cb; (void)u; return NULL;
}

EVP_PKEY *PEM_read_PrivateKey(FILE *fp, EVP_PKEY **x,
                               pem_password_cb *cb, void *u)
{
    (void)fp; (void)x; (void)cb; (void)u; return NULL;
}

int PEM_write_X509(FILE *fp, const X509 *x)
{
    (void)fp; (void)x; return 0;
}

int PEM_write_PrivateKey(FILE *fp, const EVP_PKEY *x,
                         const EVP_CIPHER *enc,
                         const unsigned char *kstr, int klen,
                         pem_password_cb *cb, void *u)
{
    (void)fp; (void)x; (void)enc; (void)kstr; (void)klen;
    (void)cb; (void)u;
    return 0;
}
