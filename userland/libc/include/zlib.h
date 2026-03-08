/*
 * VeridianOS libc -- zlib.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * zlib 1.3.x compatible API.
 * Provides deflate/inflate compression, checksums, and gzip file I/O.
 */

#ifndef _ZLIB_H
#define _ZLIB_H

#include <zconf.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define ZLIB_VERSION    "1.3.1"
#define ZLIB_VERNUM     0x1310
#define ZLIB_VER_MAJOR  1
#define ZLIB_VER_MINOR  3
#define ZLIB_VER_REVISION 1
#define ZLIB_VER_SUBREVISION 0

/* ========================================================================= */
/* Return codes                                                              */
/* ========================================================================= */

#define Z_OK            0
#define Z_STREAM_END    1
#define Z_NEED_DICT     2
#define Z_ERRNO        (-1)
#define Z_STREAM_ERROR (-2)
#define Z_DATA_ERROR   (-3)
#define Z_MEM_ERROR    (-4)
#define Z_BUF_ERROR    (-5)
#define Z_VERSION_ERROR (-6)

/* ========================================================================= */
/* Flush values                                                              */
/* ========================================================================= */

#define Z_NO_FLUSH      0
#define Z_PARTIAL_FLUSH 1
#define Z_SYNC_FLUSH    2
#define Z_FULL_FLUSH    3
#define Z_FINISH        4
#define Z_BLOCK         5
#define Z_TREES         6

/* ========================================================================= */
/* Compression levels                                                        */
/* ========================================================================= */

#define Z_NO_COMPRESSION       0
#define Z_BEST_SPEED           1
#define Z_BEST_COMPRESSION     9
#define Z_DEFAULT_COMPRESSION (-1)

/* ========================================================================= */
/* Compression strategies                                                    */
/* ========================================================================= */

#define Z_FILTERED         1
#define Z_HUFFMAN_ONLY     2
#define Z_RLE              3
#define Z_FIXED            4
#define Z_DEFAULT_STRATEGY 0

/* ========================================================================= */
/* Data types                                                                */
/* ========================================================================= */

#define Z_BINARY   0
#define Z_TEXT     1
#define Z_ASCII    Z_TEXT
#define Z_UNKNOWN  2

/* ========================================================================= */
/* Compression method                                                        */
/* ========================================================================= */

#define Z_DEFLATED 8

/* ========================================================================= */
/* Window bits                                                               */
/* ========================================================================= */

#define Z_NULL 0

/* ========================================================================= */
/* z_stream structure                                                        */
/* ========================================================================= */

typedef void *(*alloc_func)(void *opaque, unsigned int items, unsigned int size);
typedef void  (*free_func)(void *opaque, void *address);

typedef struct z_stream_s {
    const unsigned char *next_in;   /* next input byte */
    unsigned int    avail_in;       /* number of bytes available at next_in */
    unsigned long   total_in;       /* total input bytes read so far */

    unsigned char  *next_out;       /* next output byte will go here */
    unsigned int    avail_out;      /* remaining free space at next_out */
    unsigned long   total_out;      /* total output bytes written so far */

    const char     *msg;            /* last error message, NULL if no error */
    void           *state;          /* internal state, not visible to app */

    alloc_func      zalloc;         /* used to allocate internal state */
    free_func       zfree;          /* used to free internal state */
    void           *opaque;         /* private data passed to zalloc/zfree */

    int             data_type;      /* best guess about the data type */
    unsigned long   adler;          /* Adler-32 or CRC-32 value */
    unsigned long   reserved;       /* reserved for future use */
} z_stream;

typedef z_stream *z_streamp;

typedef struct gz_header_s {
    int     text;
    unsigned long time;
    int     xflags;
    int     os;
    unsigned char *extra;
    unsigned int extra_len;
    unsigned int extra_max;
    unsigned char *name;
    unsigned int name_max;
    unsigned char *comment;
    unsigned int comm_max;
    int     hcrc;
    int     done;
} gz_header;

typedef gz_header *gz_headerp;

/* Opaque gzip file handle */
typedef struct gzFile_s *gzFile;

/* ========================================================================= */
/* Checksum functions                                                        */
/* ========================================================================= */

unsigned long adler32(unsigned long adler, const unsigned char *buf,
                      unsigned int len);
unsigned long crc32(unsigned long crc, const unsigned char *buf,
                    unsigned int len);

/* ========================================================================= */
/* Utility functions                                                         */
/* ========================================================================= */

int compress(unsigned char *dest, unsigned long *destLen,
             const unsigned char *source, unsigned long sourceLen);
int compress2(unsigned char *dest, unsigned long *destLen,
              const unsigned char *source, unsigned long sourceLen, int level);
int uncompress(unsigned char *dest, unsigned long *destLen,
               const unsigned char *source, unsigned long sourceLen);
unsigned long compressBound(unsigned long sourceLen);

/* ========================================================================= */
/* Deflate (compression)                                                     */
/* ========================================================================= */

int deflateInit_(z_streamp strm, int level,
                 const char *version, int stream_size);
int deflate(z_streamp strm, int flush);
int deflateEnd(z_streamp strm);
int deflateCopy(z_streamp dest, z_streamp source);
int deflateReset(z_streamp strm);
int deflateParams(z_streamp strm, int level, int strategy);
int deflateBound_f(z_streamp strm, unsigned long sourceLen);

#define deflateInit(strm, level) \
    deflateInit_((strm), (level), ZLIB_VERSION, (int)sizeof(z_stream))

int deflateInit2_(z_streamp strm, int level, int method,
                  int windowBits, int memLevel, int strategy,
                  const char *version, int stream_size);

#define deflateInit2(strm, level, method, windowBits, memLevel, strategy) \
    deflateInit2_((strm), (level), (method), (windowBits), (memLevel), \
                  (strategy), ZLIB_VERSION, (int)sizeof(z_stream))

/* ========================================================================= */
/* Inflate (decompression)                                                   */
/* ========================================================================= */

int inflateInit_(z_streamp strm, const char *version, int stream_size);
int inflate(z_streamp strm, int flush);
int inflateEnd(z_streamp strm);
int inflateReset(z_streamp strm);
int inflateSync(z_streamp strm);

#define inflateInit(strm) \
    inflateInit_((strm), ZLIB_VERSION, (int)sizeof(z_stream))

int inflateInit2_(z_streamp strm, int windowBits,
                  const char *version, int stream_size);

#define inflateInit2(strm, windowBits) \
    inflateInit2_((strm), (windowBits), ZLIB_VERSION, (int)sizeof(z_stream))

/* ========================================================================= */
/* Gzip file I/O                                                             */
/* ========================================================================= */

gzFile gzopen(const char *path, const char *mode);
int    gzread(gzFile file, void *buf, unsigned int len);
int    gzwrite(gzFile file, const void *buf, unsigned int len);
int    gzclose(gzFile file);
int    gzeof(gzFile file);
const char *gzerror(gzFile file, int *errnum);
int    gzflush(gzFile file, int flush);

/* ========================================================================= */
/* Version info                                                              */
/* ========================================================================= */

const char *zlibVersion(void);
unsigned long zlibCompileFlags(void);

#ifdef __cplusplus
}
#endif

#endif /* _ZLIB_H */
