/*
 * VeridianOS libc -- libjpeg_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libjpeg-turbo 3.0.x shim.
 * Provides the JPEG decompression API surface.  Actual DCT decode is
 * stubbed -- the decompressor parses JFIF headers to extract dimensions
 * and color space, then returns gray placeholder data for scanlines.
 * Compression functions are stubs.
 */

#include <jpeglib.h>
#include <jerror.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Error manager                                                             */
/* ========================================================================= */

static void default_error_exit(void *cinfo)
{
    (void)cinfo;
    /* In a real implementation this would longjmp */
}

static void default_emit_message(void *cinfo, int msg_level)
{
    (void)cinfo;
    (void)msg_level;
}

static void default_output_message(void *cinfo)
{
    (void)cinfo;
}

static void default_format_message(void *cinfo, char *buffer)
{
    (void)cinfo;
    if (buffer)
        strcpy(buffer, "jpeg error");
}

static void default_reset_error_mgr(void *cinfo)
{
    struct jpeg_common_struct *c = (struct jpeg_common_struct *)cinfo;
    if (c && c->err) {
        c->err->msg_code = 0;
        c->err->num_warnings = 0;
    }
}

struct jpeg_error_mgr *jpeg_std_error(struct jpeg_error_mgr *err)
{
    if (err == NULL) return NULL;
    memset(err, 0, sizeof(*err));
    err->error_exit = default_error_exit;
    err->emit_message = default_emit_message;
    err->output_message = default_output_message;
    err->format_message = default_format_message;
    err->reset_error_mgr = default_reset_error_mgr;
    return err;
}

/* ========================================================================= */
/* Decompression                                                             */
/* ========================================================================= */

struct jpeg_internal {
    const unsigned char *data;
    unsigned long        data_size;
    unsigned long        data_pos;
};

void jpeg_CreateDecompress(j_decompress_ptr cinfo, int version,
                           size_t structsize)
{
    (void)version;
    (void)structsize;
    if (cinfo == NULL) return;
    cinfo->is_decompressor = 1;
    cinfo->global_state = 0;
    cinfo->_internal = calloc(1, sizeof(struct jpeg_internal));
}

void jpeg_mem_src(j_decompress_ptr cinfo,
                  const unsigned char *inbuffer, unsigned long insize)
{
    struct jpeg_internal *ji;
    if (cinfo == NULL || cinfo->_internal == NULL) return;
    ji = (struct jpeg_internal *)cinfo->_internal;
    ji->data = inbuffer;
    ji->data_size = insize;
    ji->data_pos = 0;
}

void jpeg_stdio_src(j_decompress_ptr cinfo, FILE *infile)
{
    (void)cinfo;
    (void)infile;
    /* File I/O source not implemented */
}

/*
 * Parse JFIF/JPEG markers to extract image dimensions.
 * Looks for SOI (0xFFD8) and SOF0 (0xFFC0) markers.
 */
int jpeg_read_header(j_decompress_ptr cinfo, int require_image)
{
    struct jpeg_internal *ji;
    const unsigned char *d;
    unsigned long sz;

    (void)require_image;

    if (cinfo == NULL || cinfo->_internal == NULL)
        return JPEG_SUSPENDED;

    ji = (struct jpeg_internal *)cinfo->_internal;
    d = ji->data;
    sz = ji->data_size;

    /* Default values */
    cinfo->image_width = 0;
    cinfo->image_height = 0;
    cinfo->num_components = 3;
    cinfo->jpeg_color_space = JCS_YCbCr;
    cinfo->data_precision = 8;

    if (sz < 2 || d[0] != 0xFF || d[1] != 0xD8)
        return JPEG_SUSPENDED;  /* Not a JPEG */

    /* Scan for SOF0 marker (0xFFC0) */
    {
        unsigned long pos = 2;
        while (pos + 4 < sz) {
            if (d[pos] != 0xFF) {
                pos++;
                continue;
            }
            unsigned char marker = d[pos + 1];

            if (marker == 0xC0 || marker == 0xC1 || marker == 0xC2) {
                /* SOFn: Start of Frame */
                if (pos + 9 < sz) {
                    cinfo->data_precision = d[pos + 4];
                    cinfo->image_height = ((unsigned int)d[pos + 5] << 8) |
                                          d[pos + 6];
                    cinfo->image_width = ((unsigned int)d[pos + 7] << 8) |
                                         d[pos + 8];
                    cinfo->num_components = d[pos + 9];
                }
                break;
            }

            if (marker == 0xD9 || marker == 0xDA)
                break;  /* EOI or SOS */

            /* Skip marker segment */
            if (pos + 3 < sz) {
                unsigned int seg_len = ((unsigned int)d[pos + 2] << 8) |
                                       d[pos + 3];
                pos += 2 + seg_len;
            } else {
                break;
            }
        }
    }

    if (cinfo->image_width == 0 || cinfo->image_height == 0)
        return JPEG_SUSPENDED;

    cinfo->global_state = 1;  /* Header read */
    return JPEG_HEADER_OK;
}

int jpeg_start_decompress(j_decompress_ptr cinfo)
{
    if (cinfo == NULL) return 0;

    /* Set output parameters */
    cinfo->out_color_space = JCS_RGB;
    cinfo->output_width = cinfo->image_width;
    cinfo->output_height = cinfo->image_height;
    cinfo->output_components = 3;
    cinfo->out_color_components = 3;
    cinfo->output_scanline = 0;
    cinfo->rec_outbuf_height = 1;
    cinfo->global_state = 2;  /* Decompressing */

    return 1;
}

JDIMENSION jpeg_read_scanlines(j_decompress_ptr cinfo,
                               JSAMPARRAY scanlines,
                               JDIMENSION max_lines)
{
    JDIMENSION lines_read = 0;
    JDIMENSION i;

    if (cinfo == NULL || scanlines == NULL)
        return 0;

    for (i = 0; i < max_lines; i++) {
        if (cinfo->output_scanline >= cinfo->output_height)
            break;

        /* Fill with gray placeholder (128,128,128) */
        if (scanlines[i] != NULL) {
            memset(scanlines[i], 128,
                   (size_t)cinfo->output_width * (size_t)cinfo->output_components);
        }

        cinfo->output_scanline++;
        lines_read++;
    }

    return lines_read;
}

int jpeg_finish_decompress(j_decompress_ptr cinfo)
{
    if (cinfo) cinfo->global_state = 3;
    return 1;
}

void jpeg_destroy_decompress(j_decompress_ptr cinfo)
{
    if (cinfo == NULL) return;
    free(cinfo->_internal);
    cinfo->_internal = NULL;
    cinfo->global_state = 0;
}

/* ========================================================================= */
/* Compression stubs                                                         */
/* ========================================================================= */

void jpeg_CreateCompress(j_compress_ptr cinfo, int version,
                         size_t structsize)
{
    (void)version;
    (void)structsize;
    if (cinfo == NULL) return;
    cinfo->is_decompressor = 0;
    cinfo->global_state = 0;
}

void jpeg_set_defaults(j_compress_ptr cinfo) { (void)cinfo; }

void jpeg_set_quality(j_compress_ptr cinfo, int quality,
                      int force_baseline)
{
    if (cinfo) cinfo->quality = quality;
    (void)force_baseline;
}

void jpeg_start_compress(j_compress_ptr cinfo, int write_all_tables)
{
    (void)cinfo; (void)write_all_tables;
}

JDIMENSION jpeg_write_scanlines(j_compress_ptr cinfo,
                                JSAMPARRAY scanlines,
                                JDIMENSION num_lines)
{
    (void)cinfo; (void)scanlines;
    return num_lines;  /* Pretend we wrote them */
}

void jpeg_finish_compress(j_compress_ptr cinfo) { (void)cinfo; }

void jpeg_destroy_compress(j_compress_ptr cinfo)
{
    if (cinfo) cinfo->global_state = 0;
}

void jpeg_stdio_dest(j_compress_ptr cinfo, FILE *outfile)
{
    (void)cinfo; (void)outfile;
}

void jpeg_mem_dest(j_compress_ptr cinfo,
                   unsigned char **outbuffer, unsigned long *outsize)
{
    (void)cinfo; (void)outbuffer; (void)outsize;
}

/* ========================================================================= */
/* Common                                                                    */
/* ========================================================================= */

void jpeg_destroy(j_common_ptr cinfo)
{
    if (cinfo == NULL) return;
    if (cinfo->is_decompressor)
        jpeg_destroy_decompress((j_decompress_ptr)cinfo);
    else
        jpeg_destroy_compress((j_compress_ptr)cinfo);
}

void jpeg_abort(j_common_ptr cinfo)
{
    if (cinfo) cinfo->global_state = 0;
}
