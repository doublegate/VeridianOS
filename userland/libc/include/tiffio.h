/*
 * VeridianOS libc -- tiffio.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libtiff 4.6.x compatible API.
 * TIFF image reading and writing.
 */

#ifndef _TIFFIO_H
#define _TIFFIO_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

typedef uint8_t  uint8;
typedef uint16_t uint16;
typedef uint32_t uint32;
typedef uint64_t uint64;

typedef struct tiff TIFF;

/* Tag values */
#define TIFFTAG_IMAGEWIDTH       256
#define TIFFTAG_IMAGELENGTH      257
#define TIFFTAG_BITSPERSAMPLE    258
#define TIFFTAG_COMPRESSION      259
#define TIFFTAG_PHOTOMETRIC      262
#define TIFFTAG_SAMPLESPERPIXEL  277
#define TIFFTAG_ROWSPERSTRIP     278
#define TIFFTAG_PLANARCONFIG     284
#define TIFFTAG_SOFTWARE         305
#define TIFFTAG_TILEWIDTH        322
#define TIFFTAG_TILELENGTH       323

/* Compression types */
#define COMPRESSION_NONE         1
#define COMPRESSION_LZW          5
#define COMPRESSION_JPEG         7
#define COMPRESSION_DEFLATE      32946
#define COMPRESSION_ADOBE_DEFLATE 8

/* Photometric interpretation */
#define PHOTOMETRIC_MINISWHITE   0
#define PHOTOMETRIC_MINISBLACK   1
#define PHOTOMETRIC_RGB          2
#define PHOTOMETRIC_PALETTE      3

/* Planar configuration */
#define PLANARCONFIG_CONTIG      1
#define PLANARCONFIG_SEPARATE    2

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

/** Open a TIFF file. */
TIFF *TIFFOpen(const char *filename, const char *mode);

/** Close a TIFF file. */
void TIFFClose(TIFF *tif);

/** Read an RGBA image. */
int TIFFReadRGBAImage(TIFF *tif, uint32_t width, uint32_t height,
                      uint32_t *raster, int stop);

/** Read an RGBA image (oriented). */
int TIFFReadRGBAImageOriented(TIFF *tif, uint32_t width,
                              uint32_t height, uint32_t *raster,
                              int orientation, int stop);

/** Read a strip of data. */
int TIFFReadEncodedStrip(TIFF *tif, uint32_t strip,
                         void *buf, int size);

/** Write a strip of data. */
int TIFFWriteEncodedStrip(TIFF *tif, uint32_t strip,
                          void *data, int cc);

/** Get a tag value. */
int TIFFGetField(TIFF *tif, uint32_t tag, ...);

/** Set a tag value. */
int TIFFSetField(TIFF *tif, uint32_t tag, ...);

/** Get the number of strips. */
uint32_t TIFFNumberOfStrips(TIFF *tif);

/** Get the size of a strip. */
int TIFFStripSize(TIFF *tif);

/** Get the scanline size. */
int TIFFScanlineSize(TIFF *tif);

/** Read a scanline. */
int TIFFReadScanline(TIFF *tif, void *buf, uint32_t row,
                     uint16_t sample);

/** Write a scanline. */
int TIFFWriteScanline(TIFF *tif, void *buf, uint32_t row,
                      uint16_t sample);

/** Write a directory. */
int TIFFWriteDirectory(TIFF *tif);

/** Flush pending writes. */
int TIFFFlush(TIFF *tif);

/** Get the TIFF version string. */
const char *TIFFGetVersion(void);

/** Check if tiled. */
int TIFFIsTiled(TIFF *tif);

#ifdef __cplusplus
}
#endif

#endif /* _TIFFIO_H */
