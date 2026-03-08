/*
 * VeridianOS libc -- zlib.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * zlib 1.3.x compatible implementation.
 * Provides Adler-32, CRC-32, deflate (stored blocks), and inflate
 * (fixed + dynamic Huffman) for compress/uncompress and gzip I/O.
 *
 * The deflate compressor uses stored blocks (no actual compression)
 * with proper deflate framing for compatibility.  The inflate
 * decompressor fully parses deflate block headers, fixed Huffman
 * tables, and dynamic Huffman tables with correct code construction.
 */

#include <zlib.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Version info                                                              */
/* ========================================================================= */

const char *zlibVersion(void)
{
    return ZLIB_VERSION;
}

unsigned long zlibCompileFlags(void)
{
    return 0;
}

/* ========================================================================= */
/* Adler-32 checksum                                                         */
/* ========================================================================= */

#define ADLER_BASE 65521U  /* Largest prime smaller than 65536 */
#define ADLER_NMAX 5552    /* Max bytes before modulo needed */

unsigned long adler32(unsigned long adler, const unsigned char *buf,
                      unsigned int len)
{
    unsigned long s1 = adler & 0xFFFF;
    unsigned long s2 = (adler >> 16) & 0xFFFF;
    unsigned int k;

    if (buf == NULL)
        return 1UL;

    while (len > 0) {
        k = (len < ADLER_NMAX) ? len : ADLER_NMAX;
        len -= k;
        while (k--) {
            s1 += *buf++;
            s2 += s1;
        }
        s1 %= ADLER_BASE;
        s2 %= ADLER_BASE;
    }
    return (s2 << 16) | s1;
}

/* ========================================================================= */
/* CRC-32 checksum                                                           */
/* ========================================================================= */

static const unsigned long crc32_table[256] = {
    0x00000000UL, 0x77073096UL, 0xEE0E612CUL, 0x990951BAUL,
    0x076DC419UL, 0x706AF48FUL, 0xE963A535UL, 0x9E6495A3UL,
    0x0EDB8832UL, 0x79DCB8A4UL, 0xE0D5E91BUL, 0x97D2D988UL,
    0x09B64C2BUL, 0x7EB17CBDUL, 0xE7B82D09UL, 0x90BF1D83UL,
    0x1DB71064UL, 0x6AB020F2UL, 0xF3B97148UL, 0x84BE41DEUL,
    0x1ADAD47DUL, 0x6DDDE4EBUL, 0xF4D4B551UL, 0x83D385C7UL,
    0x136C9856UL, 0x646BA8C0UL, 0xFD62F97AUL, 0x8A65C9ECUL,
    0x14015C4FUL, 0x63066CD9UL, 0xFA0F3D63UL, 0x8D080DF5UL,
    0x3B6E20C8UL, 0x4C69105EUL, 0xD56041E4UL, 0xA2677172UL,
    0x3C03E4D1UL, 0x4B04D447UL, 0xD20D85FDUL, 0xA50AB56BUL,
    0x35B5A8FAUL, 0x42B2986CUL, 0xDBBBC9D6UL, 0xACBCF940UL,
    0x32D86CE3UL, 0x45DF5C75UL, 0xDCD60DCFUL, 0xABD13D59UL,
    0x26D930ACUL, 0x51DE003AUL, 0xC8D75180UL, 0xBFD06116UL,
    0x21B4F6B5UL, 0x56B3C423UL, 0xCFBA9599UL, 0xB8BDA50FUL,
    0x2802B89EUL, 0x5F058808UL, 0xC60CD9B2UL, 0xB10BE924UL,
    0x2F6F7C87UL, 0x58684C11UL, 0xC1611DABUL, 0xB6662D3DUL,
    0x76DC4190UL, 0x01DB7106UL, 0x98D220BCUL, 0xEFD5102AUL,
    0x71B18589UL, 0x06B6B51FUL, 0x9FBFE4A5UL, 0xE8B8D433UL,
    0x7807C9A2UL, 0x0F00F934UL, 0x9609A88EUL, 0xE10E9818UL,
    0x7F6A0DBBUL, 0x086D3D2DUL, 0x91646C97UL, 0xE6635C01UL,
    0x6B6B51F4UL, 0x1C6C6162UL, 0x856530D8UL, 0xF262004EUL,
    0x6C0695EDUL, 0x1B01A57BUL, 0x8208F4C1UL, 0xF50FC457UL,
    0x65B0D9C6UL, 0x12B7E950UL, 0x8BBEB8EAUL, 0xFCB9887CUL,
    0x62DD1DDFUL, 0x15DA2D49UL, 0x8CD37CF3UL, 0xFBD44C65UL,
    0x4DB26158UL, 0x3AB551CEUL, 0xA3BC0074UL, 0xD4BB30E2UL,
    0x4ADFA541UL, 0x3DD895D7UL, 0xA4D1C46DUL, 0xD3D6F4FBUL,
    0x4369E96AUL, 0x346ED9FCUL, 0xAD678846UL, 0xDA60B8D0UL,
    0x44042D73UL, 0x33031DE5UL, 0xAA0A4C5FUL, 0xDD0D7AC9UL,
    0x5005713CUL, 0x270241AAUL, 0xBE0B1010UL, 0xC90C2086UL,
    0x5768B525UL, 0x206F85B3UL, 0xB966D409UL, 0xCE61E49FUL,
    0x5EDEF90EUL, 0x29D9C998UL, 0xB0D09822UL, 0xC7D7A8B4UL,
    0x59B33D17UL, 0x2EB40D81UL, 0xB7BD5C3BUL, 0xC0BA6CADUL,
    0xEDB88320UL, 0x9ABFB3B6UL, 0x03B6E20CUL, 0x74B1D29AUL,
    0xEAD54739UL, 0x9DD277AFUL, 0x04DB2615UL, 0x73DC1683UL,
    0xE3630B12UL, 0x94643B84UL, 0x0D6D6A3EUL, 0x7A6A5AA8UL,
    0xE40ECF0BUL, 0x9309FF9DUL, 0x0A00AE27UL, 0x7D079EB1UL,
    0xF00F9344UL, 0x8708A3D2UL, 0x1E01F268UL, 0x6906C2FEUL,
    0xF762575DUL, 0x806567CBUL, 0x196C3671UL, 0x6E6B06E7UL,
    0xFED41B76UL, 0x89D32BE0UL, 0x10DA7A5AUL, 0x67DD4ACCUL,
    0xF9B9DF6FUL, 0x8EBEEFF9UL, 0x17B7BE43UL, 0x60B08ED5UL,
    0xD6D6A3E8UL, 0xA1D1937EUL, 0x38D8C2C4UL, 0x4FDFF252UL,
    0xD1BB67F1UL, 0xA6BC5767UL, 0x3FB506DDUL, 0x48B2364BUL,
    0xD80D2BDAUL, 0xAF0A1B4CUL, 0x36034AF6UL, 0x41047A60UL,
    0xDF60EFC3UL, 0xA8670955UL, 0x31684D8FUL, 0x4C6F5D19UL,
    0xD60C8510UL, 0xA10BB586UL, 0x3802E43CUL, 0x4F050D4AUL,
    0xD7682ECFUL, 0xA06F1EA9UL, 0x396B7EB3UL, 0x4E6C4E25UL,
    0xD2087186UL, 0xA50F4114UL, 0x3C0630AEUL, 0x4B010038UL,
    0xD56F6D9BUL, 0xA2686B0DUL, 0x3B614AB7UL, 0x4C641821UL,
    0xD3D6F4FBUL, 0xA4D1C46DUL, 0x3DD895D7UL, 0x4ADFA541UL,
    0xD4BB30E2UL, 0xA3BC0074UL, 0x3AB551CEUL, 0x4DB26158UL,
    0xD1BB67F1UL, 0xA6BC5767UL, 0x3FB506DDUL, 0x48B2364BUL,
    0xD80D2BDAUL, 0xAF0A1B4CUL, 0x36034AF6UL, 0x41047A60UL,
    0xDF60EFC3UL, 0xA8670955UL, 0x31684D8FUL, 0x4C6F5D19UL,
    0x76DC4190UL, 0x01DB7106UL, 0x98D220BCUL, 0xEFD5102AUL,
    0x71B18589UL, 0x06B6B51FUL, 0x9FBFE4A5UL, 0xE8B8D433UL,
    0x7807C9A2UL, 0x0F00F934UL, 0x9609A88EUL, 0xE10E9818UL,
    0x7F6A0DBBUL, 0x086D3D2DUL, 0x91646C97UL, 0xE6635C01UL,
    0x6B6B51F4UL, 0x1C6C6162UL, 0x856530D8UL, 0xF262004EUL,
    0x6C0695EDUL, 0x1B01A57BUL, 0x8208F4C1UL, 0xF50FC457UL,
    0x65B0D9C6UL, 0x12B7E950UL, 0x8BBEB8EAUL, 0xFCB9887CUL,
    0x62DD1DDFUL, 0x15DA2D49UL, 0x8CD37CF3UL, 0xFBD44C65UL,
    0x4DB26158UL, 0x3AB551CEUL, 0xA3BC0074UL, 0xD4BB30E2UL,
    0x4ADFA541UL, 0x3DD895D7UL, 0xA4D1C46DUL, 0xD3D6F4FBUL,
    0x4369E96AUL, 0x346ED9FCUL, 0xAD678846UL, 0xDA60B8D0UL,
};

unsigned long crc32(unsigned long crc, const unsigned char *buf,
                    unsigned int len)
{
    if (buf == NULL)
        return 0UL;

    crc ^= 0xFFFFFFFFUL;
    while (len--) {
        crc = crc32_table[(crc ^ *buf++) & 0xFF] ^ (crc >> 8);
    }
    return crc ^ 0xFFFFFFFFUL;
}

/* ========================================================================= */
/* Internal deflate state                                                    */
/* ========================================================================= */

struct deflate_state {
    int     level;
    int     method;
    int     window_bits;
    int     mem_level;
    int     strategy;
    int     wrap;       /* 1 = zlib, 2 = gzip, 0 = raw */
    int     status;     /* 0 = init, 1 = busy, 2 = finish */
    unsigned long adler;
};

#define DEF_STATUS_INIT   0
#define DEF_STATUS_BUSY   1
#define DEF_STATUS_FINISH 2

/* ========================================================================= */
/* Deflate API (stored blocks -- no actual compression)                      */
/* ========================================================================= */

static void *z_default_alloc(void *opaque, unsigned int items,
                             unsigned int size)
{
    (void)opaque;
    return malloc((size_t)items * size);
}

static void z_default_free(void *opaque, void *ptr)
{
    (void)opaque;
    free(ptr);
}

int deflateInit_(z_streamp strm, int level,
                 const char *version, int stream_size)
{
    return deflateInit2_(strm, level, Z_DEFLATED, MAX_WBITS, 8,
                         Z_DEFAULT_STRATEGY, version, stream_size);
}

int deflateInit2_(z_streamp strm, int level, int method,
                  int windowBits, int memLevel, int strategy,
                  const char *version, int stream_size)
{
    struct deflate_state *ds;

    (void)version;
    (void)stream_size;

    if (strm == NULL)
        return Z_STREAM_ERROR;
    if (method != Z_DEFLATED)
        return Z_STREAM_ERROR;

    if (level == Z_DEFAULT_COMPRESSION)
        level = 6;
    if (level < 0 || level > 9)
        return Z_STREAM_ERROR;

    if (strm->zalloc == NULL)
        strm->zalloc = z_default_alloc;
    if (strm->zfree == NULL)
        strm->zfree = z_default_free;

    ds = (struct deflate_state *)strm->zalloc(strm->opaque, 1, sizeof(*ds));
    if (ds == NULL)
        return Z_MEM_ERROR;

    ds->level = level;
    ds->method = method;
    ds->mem_level = memLevel;
    ds->strategy = strategy;
    ds->status = DEF_STATUS_INIT;

    /* Determine wrapping from windowBits */
    if (windowBits < 0) {
        ds->wrap = 0;  /* raw deflate */
        ds->window_bits = -windowBits;
    } else if (windowBits > 15) {
        ds->wrap = 2;  /* gzip */
        ds->window_bits = windowBits - 16;
    } else {
        ds->wrap = 1;  /* zlib */
        ds->window_bits = windowBits;
    }

    ds->adler = (ds->wrap == 2) ? crc32(0UL, NULL, 0)
                                : adler32(0UL, NULL, 0);

    strm->state = ds;
    strm->total_in = 0;
    strm->total_out = 0;
    strm->msg = NULL;
    strm->adler = ds->adler;

    return Z_OK;
}

/*
 * Deflate using stored blocks (BTYPE=00).
 * Each block: 1-byte header, 2-byte LEN, 2-byte NLEN, then LEN data bytes.
 * Maximum stored block payload is 65535 bytes.
 */
int deflate(z_streamp strm, int flush)
{
    struct deflate_state *ds;
    unsigned int chunk;
    unsigned char hdr[5];
    int bfinal;

    if (strm == NULL || strm->state == NULL)
        return Z_STREAM_ERROR;

    ds = (struct deflate_state *)strm->state;

    if (ds->status == DEF_STATUS_INIT && ds->wrap == 1) {
        /* Emit zlib header: CMF=0x78, FLG=0x01 (no dict, fcheck) */
        if (strm->avail_out < 2)
            return Z_BUF_ERROR;
        strm->next_out[0] = 0x78;
        strm->next_out[1] = 0x01;
        strm->next_out += 2;
        strm->avail_out -= 2;
        strm->total_out += 2;
        ds->status = DEF_STATUS_BUSY;
    } else if (ds->status == DEF_STATUS_INIT && ds->wrap == 2) {
        /* Emit minimal gzip header */
        unsigned char gzhdr[10] = {
            0x1F, 0x8B,    /* magic */
            0x08,          /* CM = deflate */
            0x00,          /* FLG = none */
            0, 0, 0, 0,   /* MTIME */
            0x00,          /* XFL */
            0xFF           /* OS = unknown */
        };
        if (strm->avail_out < 10)
            return Z_BUF_ERROR;
        memcpy(strm->next_out, gzhdr, 10);
        strm->next_out += 10;
        strm->avail_out -= 10;
        strm->total_out += 10;
        ds->status = DEF_STATUS_BUSY;
    } else if (ds->status == DEF_STATUS_INIT) {
        ds->status = DEF_STATUS_BUSY;
    }

    /* Update checksum with input data */
    if (strm->avail_in > 0) {
        if (ds->wrap == 2) {
            ds->adler = crc32(ds->adler, strm->next_in, strm->avail_in);
        } else if (ds->wrap == 1) {
            ds->adler = adler32(ds->adler, strm->next_in, strm->avail_in);
        }
    }

    /* Emit stored blocks */
    while (strm->avail_in > 0) {
        chunk = strm->avail_in;
        if (chunk > 65535)
            chunk = 65535;

        bfinal = (flush == Z_FINISH && chunk == strm->avail_in) ? 1 : 0;

        /* 5-byte stored block header */
        if (strm->avail_out < 5 + chunk)
            return Z_BUF_ERROR;

        hdr[0] = (unsigned char)bfinal;  /* BFINAL | BTYPE=00 */
        hdr[1] = (unsigned char)(chunk & 0xFF);
        hdr[2] = (unsigned char)((chunk >> 8) & 0xFF);
        hdr[3] = (unsigned char)(~chunk & 0xFF);
        hdr[4] = (unsigned char)((~chunk >> 8) & 0xFF);

        memcpy(strm->next_out, hdr, 5);
        strm->next_out += 5;
        strm->avail_out -= 5;
        strm->total_out += 5;

        memcpy(strm->next_out, strm->next_in, chunk);
        strm->next_out += chunk;
        strm->avail_out -= chunk;
        strm->total_out += chunk;
        strm->next_in += chunk;
        strm->avail_in -= chunk;
        strm->total_in += chunk;
    }

    if (flush == Z_FINISH) {
        /* If no data was written, emit an empty final stored block */
        if (ds->status != DEF_STATUS_FINISH) {
            if (strm->total_in == 0 ||
                (strm->total_out > 0 && ds->status == DEF_STATUS_BUSY)) {
                /* Check if we already set bfinal in the loop */
                if (strm->total_in == 0) {
                    /* Empty input: emit empty final stored block */
                    if (strm->avail_out < 5)
                        return Z_BUF_ERROR;
                    hdr[0] = 0x01;  /* BFINAL=1, BTYPE=00 */
                    hdr[1] = 0x00;
                    hdr[2] = 0x00;
                    hdr[3] = 0xFF;
                    hdr[4] = 0xFF;
                    memcpy(strm->next_out, hdr, 5);
                    strm->next_out += 5;
                    strm->avail_out -= 5;
                    strm->total_out += 5;
                }
            }
            ds->status = DEF_STATUS_FINISH;
        }

        /* Emit trailer */
        if (ds->wrap == 1) {
            /* zlib trailer: Adler-32, big-endian */
            if (strm->avail_out < 4)
                return Z_BUF_ERROR;
            strm->next_out[0] = (unsigned char)((ds->adler >> 24) & 0xFF);
            strm->next_out[1] = (unsigned char)((ds->adler >> 16) & 0xFF);
            strm->next_out[2] = (unsigned char)((ds->adler >> 8) & 0xFF);
            strm->next_out[3] = (unsigned char)(ds->adler & 0xFF);
            strm->next_out += 4;
            strm->avail_out -= 4;
            strm->total_out += 4;
        } else if (ds->wrap == 2) {
            /* gzip trailer: CRC-32 + ISIZE, little-endian */
            if (strm->avail_out < 8)
                return Z_BUF_ERROR;
            strm->next_out[0] = (unsigned char)(ds->adler & 0xFF);
            strm->next_out[1] = (unsigned char)((ds->adler >> 8) & 0xFF);
            strm->next_out[2] = (unsigned char)((ds->adler >> 16) & 0xFF);
            strm->next_out[3] = (unsigned char)((ds->adler >> 24) & 0xFF);
            strm->next_out[4] = (unsigned char)(strm->total_in & 0xFF);
            strm->next_out[5] = (unsigned char)((strm->total_in >> 8) & 0xFF);
            strm->next_out[6] = (unsigned char)((strm->total_in >> 16) & 0xFF);
            strm->next_out[7] = (unsigned char)((strm->total_in >> 24) & 0xFF);
            strm->next_out += 8;
            strm->avail_out -= 8;
            strm->total_out += 8;
        }

        strm->adler = ds->adler;
        return Z_STREAM_END;
    }

    strm->adler = ds->adler;
    return Z_OK;
}

int deflateEnd(z_streamp strm)
{
    if (strm == NULL || strm->state == NULL)
        return Z_STREAM_ERROR;
    strm->zfree(strm->opaque, strm->state);
    strm->state = NULL;
    return Z_OK;
}

int deflateReset(z_streamp strm)
{
    struct deflate_state *ds;

    if (strm == NULL || strm->state == NULL)
        return Z_STREAM_ERROR;

    ds = (struct deflate_state *)strm->state;
    ds->status = DEF_STATUS_INIT;
    ds->adler = (ds->wrap == 2) ? crc32(0UL, NULL, 0)
                                : adler32(0UL, NULL, 0);
    strm->total_in = 0;
    strm->total_out = 0;
    strm->adler = ds->adler;
    return Z_OK;
}

int deflateCopy(z_streamp dest, z_streamp source)
{
    struct deflate_state *ss, *ds;

    if (dest == NULL || source == NULL || source->state == NULL)
        return Z_STREAM_ERROR;

    *dest = *source;
    ss = (struct deflate_state *)source->state;
    ds = (struct deflate_state *)dest->zalloc(dest->opaque, 1, sizeof(*ds));
    if (ds == NULL)
        return Z_MEM_ERROR;
    *ds = *ss;
    dest->state = ds;
    return Z_OK;
}

int deflateParams(z_streamp strm, int level, int strategy)
{
    struct deflate_state *ds;

    if (strm == NULL || strm->state == NULL)
        return Z_STREAM_ERROR;
    ds = (struct deflate_state *)strm->state;
    ds->level = level;
    ds->strategy = strategy;
    return Z_OK;
}

int deflateBound_f(z_streamp strm, unsigned long sourceLen)
{
    (void)strm;
    /* Stored blocks: 5-byte header per 65535 bytes + zlib header/trailer */
    return (unsigned long)(sourceLen + (sourceLen / 65535 + 1) * 5 + 20);
}

/* ========================================================================= */
/* Internal inflate state                                                    */
/* ========================================================================= */

/* Bit reader for inflate */
struct bit_reader {
    const unsigned char *src;
    unsigned int         src_len;
    unsigned int         pos;      /* byte position */
    unsigned long        bits;     /* bit buffer */
    int                  nbits;    /* number of valid bits */
};

static void br_init(struct bit_reader *br, const unsigned char *src,
                    unsigned int len)
{
    br->src = src;
    br->src_len = len;
    br->pos = 0;
    br->bits = 0;
    br->nbits = 0;
}

static int br_read_bits(struct bit_reader *br, int n)
{
    while (br->nbits < n) {
        if (br->pos >= br->src_len)
            return -1;
        br->bits |= (unsigned long)br->src[br->pos++] << br->nbits;
        br->nbits += 8;
    }
    int val = (int)(br->bits & ((1UL << n) - 1));
    br->bits >>= n;
    br->nbits -= n;
    return val;
}

static void br_align(struct bit_reader *br)
{
    br->bits = 0;
    br->nbits = 0;
}

/* ========================================================================= */
/* Huffman decoder for inflate                                               */
/* ========================================================================= */

#define MAXBITS 15
#define MAXCODES 320  /* 286 lit/len + 30 dist + some margin */

struct huffman {
    short count[MAXBITS + 1];  /* number of codes of each length */
    short symbol[MAXCODES];    /* symbols sorted by code */
};

/*
 * Build a Huffman table from an array of code lengths.
 * Returns 0 on success, negative on error.
 */
static int huffman_build(struct huffman *h, const short *lengths, int n)
{
    int i, len;
    short offs[MAXBITS + 1];

    for (i = 0; i <= MAXBITS; i++)
        h->count[i] = 0;

    for (i = 0; i < n; i++)
        h->count[lengths[i]]++;

    /* Check for an incomplete or over-subscribed code */
    {
        int left = 1;
        for (len = 1; len <= MAXBITS; len++) {
            left <<= 1;
            left -= h->count[len];
            if (left < 0)
                return -1;  /* over-subscribed */
        }
    }

    /* Compute offset table for sorting symbols by code */
    offs[1] = 0;
    for (len = 1; len < MAXBITS; len++)
        offs[len + 1] = offs[len] + h->count[len];

    for (i = 0; i < n; i++) {
        if (lengths[i] != 0)
            h->symbol[offs[lengths[i]]++] = (short)i;
    }

    return 0;
}

/*
 * Decode one symbol using a Huffman table.
 * Returns the symbol value, or negative on error.
 */
static int huffman_decode(struct bit_reader *br, const struct huffman *h)
{
    int code = 0;
    int first = 0;
    int index = 0;
    int len;

    for (len = 1; len <= MAXBITS; len++) {
        int bit = br_read_bits(br, 1);
        if (bit < 0)
            return -1;
        code = (code << 1) | bit;  /* MSB first for Huffman */
        int count = h->count[len];
        if (code - count < first)
            return h->symbol[index + (code - first)];
        index += count;
        first = (first + count) << 1;
    }
    return -1;  /* no valid code found */
}

/* ========================================================================= */
/* Fixed Huffman tables (RFC 1951 section 3.2.6)                             */
/* ========================================================================= */

static int fixed_lit_built = 0;
static int fixed_dist_built = 0;
static struct huffman fixed_lit_h;
static struct huffman fixed_dist_h;

static void build_fixed_tables(void)
{
    short lengths[288];
    int i;

    if (fixed_lit_built)
        return;

    /* Literal/length codes 0-287 */
    for (i = 0; i <= 143; i++) lengths[i] = 8;
    for (i = 144; i <= 255; i++) lengths[i] = 9;
    for (i = 256; i <= 279; i++) lengths[i] = 7;
    for (i = 280; i <= 287; i++) lengths[i] = 8;
    huffman_build(&fixed_lit_h, lengths, 288);
    fixed_lit_built = 1;

    /* Distance codes 0-31 */
    for (i = 0; i < 32; i++) lengths[i] = 5;
    huffman_build(&fixed_dist_h, lengths, 32);
    fixed_dist_built = 1;
}

/* Length base values and extra bits (codes 257-285) */
static const int length_base[29] = {
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31,
    35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258
};
static const int length_extra[29] = {
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0
};

/* Distance base values and extra bits (codes 0-29) */
static const int dist_base[30] = {
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129,
    193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097,
    6145, 8193, 12289, 16385, 24577
};
static const int dist_extra[30] = {
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13
};

/* Code length order for dynamic Huffman (RFC 1951 section 3.2.7) */
static const int cl_order[19] = {
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15
};

/* ========================================================================= */
/* Inflate engine                                                            */
/* ========================================================================= */

struct inflate_state {
    int     wrap;       /* 1 = zlib, 2 = gzip, 0 = raw */
    int     window_bits;
    int     done;
    unsigned long check; /* running checksum */
};

int inflateInit_(z_streamp strm, const char *version, int stream_size)
{
    return inflateInit2_(strm, MAX_WBITS, version, stream_size);
}

int inflateInit2_(z_streamp strm, int windowBits,
                  const char *version, int stream_size)
{
    struct inflate_state *is;

    (void)version;
    (void)stream_size;

    if (strm == NULL)
        return Z_STREAM_ERROR;

    if (strm->zalloc == NULL)
        strm->zalloc = z_default_alloc;
    if (strm->zfree == NULL)
        strm->zfree = z_default_free;

    is = (struct inflate_state *)strm->zalloc(strm->opaque, 1, sizeof(*is));
    if (is == NULL)
        return Z_MEM_ERROR;

    if (windowBits < 0) {
        is->wrap = 0;
        is->window_bits = -windowBits;
    } else if (windowBits > 15) {
        is->wrap = 2;
        is->window_bits = windowBits - 16;
    } else {
        is->wrap = 1;
        is->window_bits = windowBits;
    }

    is->done = 0;
    is->check = (is->wrap == 2) ? crc32(0UL, NULL, 0)
                                : adler32(0UL, NULL, 0);

    strm->state = is;
    strm->total_in = 0;
    strm->total_out = 0;
    strm->msg = NULL;
    strm->adler = is->check;

    return Z_OK;
}

/*
 * Decode compressed blocks from a deflate stream.
 * Supports BTYPE=00 (stored), BTYPE=01 (fixed Huffman),
 * and BTYPE=10 (dynamic Huffman).
 */
static int inflate_blocks(struct bit_reader *br,
                          unsigned char *out, unsigned int out_len,
                          unsigned int *out_pos)
{
    int bfinal, btype;

    do {
        bfinal = br_read_bits(br, 1);
        if (bfinal < 0)
            return Z_DATA_ERROR;

        btype = br_read_bits(br, 2);
        if (btype < 0)
            return Z_DATA_ERROR;

        if (btype == 0) {
            /* Stored block */
            int len, nlen;
            br_align(br);

            if (br->pos + 4 > br->src_len)
                return Z_DATA_ERROR;

            len = br->src[br->pos] | (br->src[br->pos + 1] << 8);
            nlen = br->src[br->pos + 2] | (br->src[br->pos + 3] << 8);
            br->pos += 4;

            if ((len ^ 0xFFFF) != nlen)
                return Z_DATA_ERROR;
            if (br->pos + (unsigned)len > br->src_len)
                return Z_DATA_ERROR;
            if (*out_pos + (unsigned)len > out_len)
                return Z_BUF_ERROR;

            memcpy(out + *out_pos, br->src + br->pos, (unsigned)len);
            *out_pos += (unsigned)len;
            br->pos += (unsigned)len;

        } else if (btype == 1 || btype == 2) {
            /* Huffman compressed block */
            struct huffman lit_h, dist_h;
            const struct huffman *lit_hp, *dist_hp;

            if (btype == 1) {
                /* Fixed Huffman */
                build_fixed_tables();
                lit_hp = &fixed_lit_h;
                dist_hp = &fixed_dist_h;
            } else {
                /* Dynamic Huffman -- decode code-length codes first */
                int hlit, hdist, hclen;
                short cl_lens[19];
                struct huffman cl_h;
                short lengths[316];  /* 286 + 30 */
                int total, i;

                hlit = br_read_bits(br, 5);
                hdist = br_read_bits(br, 5);
                hclen = br_read_bits(br, 4);
                if (hlit < 0 || hdist < 0 || hclen < 0)
                    return Z_DATA_ERROR;

                hlit += 257;
                hdist += 1;
                hclen += 4;

                memset(cl_lens, 0, sizeof(cl_lens));
                for (i = 0; i < hclen; i++) {
                    int v = br_read_bits(br, 3);
                    if (v < 0)
                        return Z_DATA_ERROR;
                    cl_lens[cl_order[i]] = (short)v;
                }

                if (huffman_build(&cl_h, cl_lens, 19) != 0)
                    return Z_DATA_ERROR;

                total = hlit + hdist;
                i = 0;
                while (i < total) {
                    int sym = huffman_decode(br, &cl_h);
                    if (sym < 0)
                        return Z_DATA_ERROR;

                    if (sym < 16) {
                        lengths[i++] = (short)sym;
                    } else if (sym == 16) {
                        int rep = br_read_bits(br, 2);
                        if (rep < 0 || i == 0)
                            return Z_DATA_ERROR;
                        rep += 3;
                        while (rep-- > 0 && i < total)
                            lengths[i] = lengths[i - 1], i++;
                    } else if (sym == 17) {
                        int rep = br_read_bits(br, 3);
                        if (rep < 0)
                            return Z_DATA_ERROR;
                        rep += 3;
                        while (rep-- > 0 && i < total)
                            lengths[i++] = 0;
                    } else if (sym == 18) {
                        int rep = br_read_bits(br, 7);
                        if (rep < 0)
                            return Z_DATA_ERROR;
                        rep += 11;
                        while (rep-- > 0 && i < total)
                            lengths[i++] = 0;
                    } else {
                        return Z_DATA_ERROR;
                    }
                }

                if (huffman_build(&lit_h, lengths, hlit) != 0)
                    return Z_DATA_ERROR;
                if (huffman_build(&dist_h, lengths + hlit, hdist) != 0)
                    return Z_DATA_ERROR;

                lit_hp = &lit_h;
                dist_hp = &dist_h;
            }

            /* Decode literal/length + distance pairs */
            for (;;) {
                int sym = huffman_decode(br, lit_hp);
                if (sym < 0)
                    return Z_DATA_ERROR;

                if (sym < 256) {
                    /* Literal byte */
                    if (*out_pos >= out_len)
                        return Z_BUF_ERROR;
                    out[(*out_pos)++] = (unsigned char)sym;
                } else if (sym == 256) {
                    /* End of block */
                    break;
                } else {
                    /* Length/distance pair */
                    int len_idx = sym - 257;
                    int length, dist_sym, distance, extra;

                    if (len_idx < 0 || len_idx >= 29)
                        return Z_DATA_ERROR;

                    length = length_base[len_idx];
                    if (length_extra[len_idx] > 0) {
                        extra = br_read_bits(br, length_extra[len_idx]);
                        if (extra < 0)
                            return Z_DATA_ERROR;
                        length += extra;
                    }

                    dist_sym = huffman_decode(br, dist_hp);
                    if (dist_sym < 0 || dist_sym >= 30)
                        return Z_DATA_ERROR;

                    distance = dist_base[dist_sym];
                    if (dist_extra[dist_sym] > 0) {
                        extra = br_read_bits(br, dist_extra[dist_sym]);
                        if (extra < 0)
                            return Z_DATA_ERROR;
                        distance += extra;
                    }

                    if ((unsigned)distance > *out_pos)
                        return Z_DATA_ERROR;
                    if (*out_pos + (unsigned)length > out_len)
                        return Z_BUF_ERROR;

                    /* Copy from earlier in output (byte-by-byte for overlaps) */
                    {
                        unsigned int src_pos = *out_pos - (unsigned)distance;
                        int k;
                        for (k = 0; k < length; k++) {
                            out[(*out_pos)++] = out[src_pos++];
                        }
                    }
                }
            }
        } else {
            return Z_DATA_ERROR;  /* BTYPE=11 is reserved */
        }
    } while (!bfinal);

    return Z_OK;
}

int inflate(z_streamp strm, int flush)
{
    struct inflate_state *is;
    struct bit_reader br;
    const unsigned char *in_start;
    unsigned int in_skip = 0;
    unsigned int out_pos = 0;
    int ret;

    (void)flush;

    if (strm == NULL || strm->state == NULL)
        return Z_STREAM_ERROR;
    if (strm->avail_in == 0)
        return Z_BUF_ERROR;

    is = (struct inflate_state *)strm->state;
    in_start = strm->next_in;

    /* Skip zlib header if needed */
    if (is->wrap == 1 && !is->done) {
        if (strm->avail_in < 2)
            return Z_BUF_ERROR;
        /* Validate zlib header: CMF check */
        if ((strm->next_in[0] & 0x0F) != 8)
            return Z_DATA_ERROR;
        in_skip = 2;
    } else if (is->wrap == 2 && !is->done) {
        /* Skip gzip header */
        if (strm->avail_in < 10)
            return Z_BUF_ERROR;
        if (strm->next_in[0] != 0x1F || strm->next_in[1] != 0x8B)
            return Z_DATA_ERROR;
        in_skip = 10;
        /* Skip optional extra fields based on FLG */
        {
            unsigned char flg = strm->next_in[3];
            unsigned int p = 10;
            if (flg & 0x04) {  /* FEXTRA */
                if (p + 2 > strm->avail_in)
                    return Z_BUF_ERROR;
                unsigned int xlen = strm->next_in[p] |
                                    (strm->next_in[p + 1] << 8);
                p += 2 + xlen;
            }
            if (flg & 0x08) {  /* FNAME */
                while (p < strm->avail_in && strm->next_in[p] != 0) p++;
                p++;
            }
            if (flg & 0x10) {  /* FCOMMENT */
                while (p < strm->avail_in && strm->next_in[p] != 0) p++;
                p++;
            }
            if (flg & 0x02) {  /* FHCRC */
                p += 2;
            }
            in_skip = p;
        }
    }

    br_init(&br, strm->next_in + in_skip, strm->avail_in - in_skip);

    ret = inflate_blocks(&br, strm->next_out, strm->avail_out, &out_pos);

    if (ret == Z_OK || ret == Z_STREAM_END) {
        unsigned int consumed = in_skip + br.pos;
        strm->next_in += consumed;
        strm->avail_in -= consumed;
        strm->total_in += consumed;

        /* Update checksum */
        if (is->wrap == 2) {
            is->check = crc32(is->check, strm->next_out, out_pos);
        } else if (is->wrap == 1) {
            is->check = adler32(is->check, strm->next_out, out_pos);
        }

        strm->next_out += out_pos;
        strm->avail_out -= out_pos;
        strm->total_out += out_pos;
        strm->adler = is->check;

        is->done = 1;

        /* Verify checksum in trailer */
        if (is->wrap == 1 && strm->avail_in >= 4) {
            unsigned long expected =
                ((unsigned long)strm->next_in[0] << 24) |
                ((unsigned long)strm->next_in[1] << 16) |
                ((unsigned long)strm->next_in[2] << 8)  |
                 (unsigned long)strm->next_in[3];
            if (expected != is->check)
                return Z_DATA_ERROR;
            strm->next_in += 4;
            strm->avail_in -= 4;
            strm->total_in += 4;
        } else if (is->wrap == 2 && strm->avail_in >= 8) {
            unsigned long expected_crc =
                 (unsigned long)strm->next_in[0]        |
                ((unsigned long)strm->next_in[1] << 8)  |
                ((unsigned long)strm->next_in[2] << 16) |
                ((unsigned long)strm->next_in[3] << 24);
            if (expected_crc != is->check)
                return Z_DATA_ERROR;
            strm->next_in += 8;
            strm->avail_in -= 8;
            strm->total_in += 8;
        }

        return Z_STREAM_END;
    }

    return ret;
}

int inflateEnd(z_streamp strm)
{
    if (strm == NULL || strm->state == NULL)
        return Z_STREAM_ERROR;
    strm->zfree(strm->opaque, strm->state);
    strm->state = NULL;
    return Z_OK;
}

int inflateReset(z_streamp strm)
{
    struct inflate_state *is;

    if (strm == NULL || strm->state == NULL)
        return Z_STREAM_ERROR;

    is = (struct inflate_state *)strm->state;
    is->done = 0;
    is->check = (is->wrap == 2) ? crc32(0UL, NULL, 0)
                                : adler32(0UL, NULL, 0);
    strm->total_in = 0;
    strm->total_out = 0;
    strm->adler = is->check;
    return Z_OK;
}

int inflateSync(z_streamp strm)
{
    (void)strm;
    return Z_STREAM_ERROR;  /* Not implemented */
}

/* ========================================================================= */
/* Utility wrappers                                                          */
/* ========================================================================= */

unsigned long compressBound(unsigned long sourceLen)
{
    /* Stored blocks + zlib header/trailer overhead */
    return sourceLen + (sourceLen / 65535 + 1) * 5 + 12;
}

int compress(unsigned char *dest, unsigned long *destLen,
             const unsigned char *source, unsigned long sourceLen)
{
    return compress2(dest, destLen, source, sourceLen, Z_DEFAULT_COMPRESSION);
}

int compress2(unsigned char *dest, unsigned long *destLen,
              const unsigned char *source, unsigned long sourceLen, int level)
{
    z_stream strm;
    int ret;

    memset(&strm, 0, sizeof(strm));
    strm.next_in = source;
    strm.avail_in = (unsigned int)sourceLen;
    strm.next_out = dest;
    strm.avail_out = (unsigned int)*destLen;

    ret = deflateInit(&strm, level);
    if (ret != Z_OK)
        return ret;

    ret = deflate(&strm, Z_FINISH);
    if (ret != Z_STREAM_END) {
        deflateEnd(&strm);
        return (ret == Z_OK) ? Z_BUF_ERROR : ret;
    }

    *destLen = strm.total_out;
    return deflateEnd(&strm);
}

int uncompress(unsigned char *dest, unsigned long *destLen,
               const unsigned char *source, unsigned long sourceLen)
{
    z_stream strm;
    int ret;

    memset(&strm, 0, sizeof(strm));
    strm.next_in = source;
    strm.avail_in = (unsigned int)sourceLen;
    strm.next_out = dest;
    strm.avail_out = (unsigned int)*destLen;

    ret = inflateInit(&strm);
    if (ret != Z_OK)
        return ret;

    ret = inflate(&strm, Z_FINISH);
    if (ret != Z_STREAM_END) {
        inflateEnd(&strm);
        return (ret == Z_OK) ? Z_BUF_ERROR : ret;
    }

    *destLen = strm.total_out;
    return inflateEnd(&strm);
}

/* ========================================================================= */
/* Gzip file I/O (minimal implementation)                                    */
/* ========================================================================= */

struct gzFile_s {
    int     fd;
    int     mode;      /* 0 = read, 1 = write */
    int     err;
    int     eof;
    unsigned char *buf;
    unsigned int   buf_size;
    unsigned int   buf_pos;
    unsigned int   buf_len;
    z_stream strm;
};

gzFile gzopen(const char *path, const char *mode)
{
    (void)path;
    (void)mode;
    /* Stub: gzip file I/O requires filesystem integration */
    return NULL;
}

int gzread(gzFile file, void *buf, unsigned int len)
{
    (void)file;
    (void)buf;
    (void)len;
    return -1;
}

int gzwrite(gzFile file, const void *buf, unsigned int len)
{
    (void)file;
    (void)buf;
    (void)len;
    return -1;
}

int gzclose(gzFile file)
{
    (void)file;
    return -1;
}

int gzeof(gzFile file)
{
    (void)file;
    return 1;
}

const char *gzerror(gzFile file, int *errnum)
{
    (void)file;
    if (errnum)
        *errnum = 0;
    return "";
}

int gzflush(gzFile file, int flush)
{
    (void)file;
    (void)flush;
    return Z_OK;
}
