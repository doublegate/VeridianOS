/*
 * VeridianOS libc -- libtiff_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libtiff 4.6.x stubs.
 * Provides the TIFF API surface.  All I/O functions return error
 * since TIFF support requires filesystem integration.
 */

#include <tiffio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>

/* ========================================================================= */
/* Internal structure                                                        */
/* ========================================================================= */

struct tiff {
    int    fd;
    int    mode;
    uint32_t width;
    uint32_t height;
    uint16_t bps;
    uint16_t spp;
    uint16_t compression;
    uint16_t photometric;
    uint16_t planar;
    uint32_t rowsperstrip;
};

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

TIFF *TIFFOpen(const char *filename, const char *mode)
{
    (void)filename;
    (void)mode;
    /* File I/O not available */
    return NULL;
}

void TIFFClose(TIFF *tif)
{
    free(tif);
}

int TIFFReadRGBAImage(TIFF *tif, uint32_t width, uint32_t height,
                      uint32_t *raster, int stop)
{
    (void)tif; (void)width; (void)height; (void)raster; (void)stop;
    return 0;
}

int TIFFReadRGBAImageOriented(TIFF *tif, uint32_t width,
                              uint32_t height, uint32_t *raster,
                              int orientation, int stop)
{
    (void)tif; (void)width; (void)height; (void)raster;
    (void)orientation; (void)stop;
    return 0;
}

int TIFFReadEncodedStrip(TIFF *tif, uint32_t strip,
                         void *buf, int size)
{
    (void)tif; (void)strip; (void)buf; (void)size;
    return -1;
}

int TIFFWriteEncodedStrip(TIFF *tif, uint32_t strip,
                          void *data, int cc)
{
    (void)tif; (void)strip; (void)data; (void)cc;
    return -1;
}

int TIFFGetField(TIFF *tif, uint32_t tag, ...)
{
    va_list ap;
    (void)tif;

    va_start(ap, tag);
    /* Return default values for common tags */
    switch (tag) {
    case TIFFTAG_IMAGEWIDTH: {
        uint32_t *p = va_arg(ap, uint32_t *);
        if (p && tif) *p = tif->width;
        break;
    }
    case TIFFTAG_IMAGELENGTH: {
        uint32_t *p = va_arg(ap, uint32_t *);
        if (p && tif) *p = tif->height;
        break;
    }
    default:
        break;
    }
    va_end(ap);
    return 0;
}

int TIFFSetField(TIFF *tif, uint32_t tag, ...)
{
    (void)tif; (void)tag;
    return 0;
}

uint32_t TIFFNumberOfStrips(TIFF *tif)
{
    (void)tif;
    return 0;
}

int TIFFStripSize(TIFF *tif)
{
    (void)tif;
    return 0;
}

int TIFFScanlineSize(TIFF *tif)
{
    (void)tif;
    return 0;
}

int TIFFReadScanline(TIFF *tif, void *buf, uint32_t row,
                     uint16_t sample)
{
    (void)tif; (void)buf; (void)row; (void)sample;
    return -1;
}

int TIFFWriteScanline(TIFF *tif, void *buf, uint32_t row,
                      uint16_t sample)
{
    (void)tif; (void)buf; (void)row; (void)sample;
    return -1;
}

int TIFFWriteDirectory(TIFF *tif) { (void)tif; return 0; }
int TIFFFlush(TIFF *tif) { (void)tif; return 0; }

const char *TIFFGetVersion(void)
{
    return "LIBTIFF, Version 4.6.0 (VeridianOS)";
}

int TIFFIsTiled(TIFF *tif) { (void)tif; return 0; }
