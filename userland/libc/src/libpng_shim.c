/*
 * VeridianOS libc -- libpng_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libpng 1.6.x shim.
 * Implements PNG reading using zlib inflate for IDAT decompression.
 * Parses IHDR, IDAT, IEND chunks and decodes every color type and bit
 * depth, non-interlaced and Adam7, into rows in the file's own pixel
 * format (transforms such as png_set_expand are recorded but not applied).
 * Write functions are stubs.
 */

#include <png.h>
#include <zlib.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct png_struct_def {
    png_error_ptr  error_fn;
    png_error_ptr  warning_fn;
    png_voidp      error_ptr;
    png_rw_ptr     read_fn;
    png_rw_ptr     write_fn;
    png_flush_ptr  flush_fn;
    png_voidp      io_ptr;
    int            mode;          /* 0=read, 1=write */
    /* Read state */
    unsigned char *idat_buf;      /* accumulated IDAT data */
    size_t         idat_size;
    size_t         idat_capacity;
    /* Transform flags */
    int            transforms;
    /* Image header, copied from IHDR by png_read_info */
    int            have_ihdr;
    png_uint_32    width;
    png_uint_32    height;
    int            interlace;
    unsigned int   pixel_bits;    /* bits per pixel: channels * bit depth */
    size_t         rowbytes;
    /* Decoded image (height * rowbytes) and the png_read_row cursor */
    unsigned char *pixels;
    png_uint_32    next_row;
};

struct png_info_def {
    png_uint_32    width;
    png_uint_32    height;
    int            bit_depth;
    int            color_type;
    int            interlace_type;
    int            compression_type;
    int            filter_type;
    png_byte       channels;
    png_size_t     rowbytes;
    int            valid;  /* 1 if IHDR has been read */
};

/* PNG signature */
static const unsigned char png_sig[8] = {
    137, 80, 78, 71, 13, 10, 26, 10
};

/* ========================================================================= */
/* Constructor / destructor                                                  */
/* ========================================================================= */

png_structp png_create_read_struct(png_const_charp user_png_ver,
    png_voidp error_ptr, png_error_ptr error_fn, png_error_ptr warn_fn)
{
    png_structp p;
    (void)user_png_ver;

    p = (png_structp)calloc(1, sizeof(struct png_struct_def));
    if (p == NULL) return NULL;

    p->error_fn = error_fn;
    p->warning_fn = warn_fn;
    p->error_ptr = error_ptr;
    p->mode = 0;  /* read */
    return p;
}

png_structp png_create_write_struct(png_const_charp user_png_ver,
    png_voidp error_ptr, png_error_ptr error_fn, png_error_ptr warn_fn)
{
    png_structp p;
    (void)user_png_ver;

    p = (png_structp)calloc(1, sizeof(struct png_struct_def));
    if (p == NULL) return NULL;

    p->error_fn = error_fn;
    p->warning_fn = warn_fn;
    p->error_ptr = error_ptr;
    p->mode = 1;  /* write */
    return p;
}

png_infop png_create_info_struct(png_const_structrp png_ptr)
{
    png_infop info;
    (void)png_ptr;

    info = (png_infop)calloc(1, sizeof(struct png_info_def));
    return info;
}

void png_destroy_read_struct(png_structpp png_ptr_ptr,
    png_infopp info_ptr_ptr, png_infopp end_info_ptr_ptr)
{
    if (png_ptr_ptr && *png_ptr_ptr) {
        free((*png_ptr_ptr)->idat_buf);
        free((*png_ptr_ptr)->pixels);
        free(*png_ptr_ptr);
        *png_ptr_ptr = NULL;
    }
    if (info_ptr_ptr && *info_ptr_ptr) {
        free(*info_ptr_ptr);
        *info_ptr_ptr = NULL;
    }
    if (end_info_ptr_ptr && *end_info_ptr_ptr) {
        free(*end_info_ptr_ptr);
        *end_info_ptr_ptr = NULL;
    }
}

void png_destroy_write_struct(png_structpp png_ptr_ptr,
                              png_infopp info_ptr_ptr)
{
    if (png_ptr_ptr && *png_ptr_ptr) {
        free(*png_ptr_ptr);
        *png_ptr_ptr = NULL;
    }
    if (info_ptr_ptr && *info_ptr_ptr) {
        free(*info_ptr_ptr);
        *info_ptr_ptr = NULL;
    }
}

/* ========================================================================= */
/* I/O setup                                                                 */
/* ========================================================================= */

void png_set_read_fn(png_structrp png_ptr, png_voidp io_ptr,
                     png_rw_ptr read_data_fn)
{
    if (png_ptr) {
        png_ptr->io_ptr = io_ptr;
        png_ptr->read_fn = read_data_fn;
    }
}

void png_set_write_fn(png_structrp png_ptr, png_voidp io_ptr,
                      png_rw_ptr write_data_fn,
                      png_flush_ptr output_flush_fn)
{
    if (png_ptr) {
        png_ptr->io_ptr = io_ptr;
        png_ptr->write_fn = write_data_fn;
        png_ptr->flush_fn = output_flush_fn;
    }
}

void png_init_io(png_structrp png_ptr, FILE *fp)
{
    (void)png_ptr;
    (void)fp;
    /* File I/O not implemented -- use png_set_read_fn */
}

png_voidp png_get_io_ptr(png_const_structrp png_ptr)
{
    return png_ptr ? png_ptr->io_ptr : NULL;
}

/* ========================================================================= */
/* Internal chunk reading via custom read function                           */
/* ========================================================================= */

static int read_bytes(png_structrp p, unsigned char *buf, size_t n)
{
    if (p->read_fn == NULL)
        return -1;
    p->read_fn(p, buf, n);
    return 0;
}

static png_uint_32 read_u32(png_structrp p)
{
    unsigned char b[4];
    if (read_bytes(p, b, 4) != 0)
        return 0;
    return ((png_uint_32)b[0] << 24) | ((png_uint_32)b[1] << 16) |
           ((png_uint_32)b[2] << 8) | b[3];
}

/* ========================================================================= */
/* Read info (parse chunks)                                                  */
/* ========================================================================= */

void png_read_info(png_structrp png_ptr, png_inforp info_ptr)
{
    unsigned char sig[8];
    unsigned char chunk_type[4];
    png_uint_32 length;

    if (png_ptr == NULL || info_ptr == NULL || png_ptr->read_fn == NULL)
        return;

    /* Read and verify signature */
    if (read_bytes(png_ptr, sig, 8) != 0)
        return;
    if (memcmp(sig, png_sig, 8) != 0)
        return;

    /* Read chunks until IDAT or IEND */
    for (;;) {
        length = read_u32(png_ptr);
        if (read_bytes(png_ptr, chunk_type, 4) != 0)
            break;

        if (memcmp(chunk_type, "IHDR", 4) == 0) {
            unsigned char ihdr[13];
            if (length >= 13 && read_bytes(png_ptr, ihdr, 13) == 0) {
                info_ptr->width = ((png_uint_32)ihdr[0] << 24) |
                                  ((png_uint_32)ihdr[1] << 16) |
                                  ((png_uint_32)ihdr[2] << 8) | ihdr[3];
                info_ptr->height = ((png_uint_32)ihdr[4] << 24) |
                                   ((png_uint_32)ihdr[5] << 16) |
                                   ((png_uint_32)ihdr[6] << 8) | ihdr[7];
                info_ptr->bit_depth = ihdr[8];
                info_ptr->color_type = ihdr[9];
                info_ptr->compression_type = ihdr[10];
                info_ptr->filter_type = ihdr[11];
                info_ptr->interlace_type = ihdr[12];

                /* Calculate channels and rowbytes */
                switch (info_ptr->color_type) {
                case PNG_COLOR_TYPE_GRAY:       info_ptr->channels = 1; break;
                case PNG_COLOR_TYPE_RGB:        info_ptr->channels = 3; break;
                case PNG_COLOR_TYPE_PALETTE:    info_ptr->channels = 1; break;
                case PNG_COLOR_TYPE_GRAY_ALPHA: info_ptr->channels = 2; break;
                case PNG_COLOR_TYPE_RGB_ALPHA:  info_ptr->channels = 4; break;
                default:                        info_ptr->channels = 1; break;
                }
                /* Sub-byte depths pack several pixels per byte, so round
                 * the row up to whole bytes. */
                info_ptr->rowbytes = ((png_size_t)info_ptr->width *
                                      info_ptr->channels *
                                      (png_size_t)info_ptr->bit_depth + 7) / 8;
                info_ptr->valid = 1;

                png_ptr->have_ihdr = 1;
                png_ptr->width = info_ptr->width;
                png_ptr->height = info_ptr->height;
                png_ptr->interlace = info_ptr->interlace_type;
                png_ptr->pixel_bits = (unsigned int)info_ptr->channels *
                                      (unsigned int)info_ptr->bit_depth;
                png_ptr->rowbytes = info_ptr->rowbytes;

                /* Skip remaining + CRC */
                if (length > 13) {
                    unsigned char skip;
                    png_uint_32 rem = length - 13;
                    while (rem--) read_bytes(png_ptr, &skip, 1);
                }
            }
            /* Skip CRC */
            read_u32(png_ptr);

        } else if (memcmp(chunk_type, "IDAT", 4) == 0) {
            /* Accumulate IDAT data */
            if (length > 0) {
                size_t new_size = png_ptr->idat_size + length;
                if (new_size > png_ptr->idat_capacity) {
                    size_t new_cap = new_size * 2;
                    unsigned char *nb = (unsigned char *)realloc(
                        png_ptr->idat_buf, new_cap);
                    if (nb == NULL) break;
                    png_ptr->idat_buf = nb;
                    png_ptr->idat_capacity = new_cap;
                }
                if (read_bytes(png_ptr, png_ptr->idat_buf + png_ptr->idat_size,
                               length) != 0)
                    break;
                png_ptr->idat_size += length;
            }
            read_u32(png_ptr);  /* CRC */

        } else if (memcmp(chunk_type, "IEND", 4) == 0) {
            break;

        } else {
            /* Skip unknown chunk data + CRC */
            unsigned char skip;
            png_uint_32 rem = length;
            while (rem--) read_bytes(png_ptr, &skip, 1);
            read_u32(png_ptr);  /* CRC */
        }
    }
}

void png_read_update_info(png_structrp png_ptr, png_inforp info_ptr)
{
    (void)png_ptr;
    (void)info_ptr;
    /* Update rowbytes based on transforms -- no-op for now */
}

/* ========================================================================= */
/* Image reading                                                             */
/* ========================================================================= */

/*
 * Reverse the PNG sub-byte filter for a single row.
 * filter_type is the first byte of each row in the decompressed data.
 *
 * Filter types:
 *   0 = None
 *   1 = Sub  (add left neighbor)
 *   2 = Up   (add above neighbor)
 *   3 = Average
 *   4 = Paeth
 */
static void unfilter_row(unsigned char *row, const unsigned char *prev,
                         size_t rowbytes, int bpp, int filter)
{
    size_t i;

    switch (filter) {
    case 0: /* None */
        break;
    case 1: /* Sub */
        for (i = (size_t)bpp; i < rowbytes; i++)
            row[i] += row[i - bpp];
        break;
    case 2: /* Up */
        if (prev) {
            for (i = 0; i < rowbytes; i++)
                row[i] += prev[i];
        }
        break;
    case 3: /* Average */
        for (i = 0; i < rowbytes; i++) {
            unsigned int a = (i >= (size_t)bpp) ? row[i - bpp] : 0;
            unsigned int b = prev ? prev[i] : 0;
            row[i] += (unsigned char)((a + b) / 2);
        }
        break;
    case 4: /* Paeth */
        for (i = 0; i < rowbytes; i++) {
            int a = (i >= (size_t)bpp) ? row[i - bpp] : 0;
            int b = prev ? prev[i] : 0;
            int c = (prev && i >= (size_t)bpp) ? prev[i - bpp] : 0;
            int p = a + b - c;
            int pa = p - a; if (pa < 0) pa = -pa;
            int pb = p - b; if (pb < 0) pb = -pb;
            int pc = p - c; if (pc < 0) pc = -pc;
            if (pa <= pb && pa <= pc)
                row[i] += (unsigned char)a;
            else if (pb <= pc)
                row[i] += (unsigned char)b;
            else
                row[i] += (unsigned char)c;
        }
        break;
    }
}

/* Adam7 pass origins and steps (PNG spec, section 8.2) */
static const unsigned char adam7_x0[7] = { 0, 4, 0, 2, 0, 1, 0 };
static const unsigned char adam7_y0[7] = { 0, 0, 4, 0, 2, 0, 1 };
static const unsigned char adam7_dx[7] = { 8, 8, 4, 4, 2, 2, 1 };
static const unsigned char adam7_dy[7] = { 8, 8, 8, 4, 4, 2, 2 };

/* Copy pixel `sx` of `src` to pixel `dx` of `dst`, both pixel_bits wide. */
static void copy_pixel(unsigned char *dst, size_t dx,
                       const unsigned char *src, size_t sx,
                       unsigned int pixel_bits)
{
    if (pixel_bits >= 8) {
        size_t n = pixel_bits / 8;
        memcpy(dst + dx * n, src + sx * n, n);
        return;
    }
    /* 1, 2 or 4 bits, packed most significant first */
    size_t sbit = sx * pixel_bits, dbit = dx * pixel_bits;
    unsigned int mask = (1u << pixel_bits) - 1;
    unsigned int v = (src[sbit / 8] >> (8 - pixel_bits - sbit % 8)) & mask;
    unsigned int shift = 8 - pixel_bits - (unsigned int)(dbit % 8);
    dst[dbit / 8] = (unsigned char)((dst[dbit / 8] & ~(mask << shift)) |
                                    (v << shift));
}

/*
 * Inflate the IDAT stream, undo the per-row filters and (for Adam7)
 * scatter the seven passes into png_ptr->pixels. Returns 0 on success.
 * Every size comes from IHDR, and the stream must inflate to exactly the
 * size the header implies.
 */
static int decode_image(png_structrp png_ptr)
{
    size_t rb = png_ptr->rowbytes, raw_size = 0, pos = 0;
    unsigned int bits = png_ptr->pixel_bits;
    size_t bpp = bits >= 8 ? bits / 8 : 1;  /* filter byte distance */
    png_uint_32 w = png_ptr->width, h = png_ptr->height;
    int passes = png_ptr->interlace == PNG_INTERLACE_ADAM7 ? 7 : 1;
    unsigned char *raw;
    z_stream strm;
    int ret;

    if (png_ptr->pixels)
        return 0;
    if (!png_ptr->have_ihdr || w == 0 || h == 0 || bits == 0 || bits > 64 ||
        png_ptr->idat_buf == NULL || png_ptr->idat_size == 0 ||
        rb > ((size_t)1 << 28) / h)
        return -1;

    /* Raw size: per pass, rows * (1 filter byte + packed row bytes) */
    for (int p = 0; p < passes; p++) {
        size_t pw = passes == 1 ? w
                  : (w + adam7_dx[p] - 1 - adam7_x0[p]) / adam7_dx[p];
        size_t ph = passes == 1 ? h
                  : (h + adam7_dy[p] - 1 - adam7_y0[p]) / adam7_dy[p];
        if (w <= adam7_x0[p] || h <= adam7_y0[p])
            pw = ph = 0;
        if (pw && ph)
            raw_size += ph * (1 + (pw * bits + 7) / 8);
    }

    png_ptr->pixels = (unsigned char *)calloc(h, rb);
    raw = (unsigned char *)malloc(raw_size);
    if (png_ptr->pixels == NULL || raw == NULL)
        goto fail;

    memset(&strm, 0, sizeof(strm));
    strm.next_in = png_ptr->idat_buf;
    strm.avail_in = (unsigned int)png_ptr->idat_size;
    strm.next_out = raw;
    strm.avail_out = (unsigned int)raw_size;
    if (inflateInit(&strm) != Z_OK)
        goto fail;
    ret = inflate(&strm, Z_FINISH);
    inflateEnd(&strm);
    if ((ret != Z_STREAM_END && ret != Z_OK) || strm.total_out != raw_size)
        goto fail;

    for (int p = 0; p < passes; p++) {
        size_t pw, ph, prb;
        if (passes == 1) {
            pw = w;
            ph = h;
        } else {
            if (w <= adam7_x0[p] || h <= adam7_y0[p])
                continue;
            pw = (w + adam7_dx[p] - 1 - adam7_x0[p]) / adam7_dx[p];
            ph = (h + adam7_dy[p] - 1 - adam7_y0[p]) / adam7_dy[p];
        }
        prb = (pw * bits + 7) / 8;
        const unsigned char *prev = NULL;
        for (size_t y = 0; y < ph; y++) {
            int filter = raw[pos];
            unsigned char *row = raw + pos + 1;
            if (filter > 4)
                goto fail;
            unfilter_row(row, prev, prb, (int)bpp, filter);
            if (passes == 1) {
                memcpy(png_ptr->pixels + y * rb, row, rb);
            } else {
                size_t dy = adam7_y0[p] + y * adam7_dy[p];
                for (size_t x = 0; x < pw; x++)
                    copy_pixel(png_ptr->pixels + dy * rb,
                               adam7_x0[p] + x * adam7_dx[p], row, x, bits);
            }
            prev = row;
            pos += 1 + prb;
        }
    }

    free(raw);
    return 0;

fail:
    free(raw);
    free(png_ptr->pixels);
    png_ptr->pixels = NULL;
    return -1;
}

void png_read_image(png_structrp png_ptr, png_bytepp image)
{
    if (png_ptr == NULL || image == NULL || decode_image(png_ptr) != 0)
        return;
    for (png_uint_32 y = 0; y < png_ptr->height; y++) {
        if (image[y])
            memcpy(image[y], png_ptr->pixels + (size_t)y * png_ptr->rowbytes,
                   png_ptr->rowbytes);
    }
    png_ptr->next_row = png_ptr->height;
}

void png_read_end(png_structrp png_ptr, png_inforp info_ptr)
{
    (void)png_ptr;
    (void)info_ptr;
}

/* Rows come out of the decoded image in order; an Adam7 image is fully
 * assembled first, so each row is final (libpng's display_row semantics). */
void png_read_row(png_structrp png_ptr, png_bytep row,
                  png_bytep display_row)
{
    if (png_ptr == NULL || png_ptr->next_row >= png_ptr->height ||
        decode_image(png_ptr) != 0)
        return;
    const unsigned char *src =
        png_ptr->pixels + (size_t)png_ptr->next_row * png_ptr->rowbytes;
    if (row)
        memcpy(row, src, png_ptr->rowbytes);
    if (display_row)
        memcpy(display_row, src, png_ptr->rowbytes);
    png_ptr->next_row++;
}

void png_read_rows(png_structrp png_ptr, png_bytepp row,
                   png_bytepp display_row, png_uint_32 num_rows)
{
    for (png_uint_32 i = 0; i < num_rows; i++)
        png_read_row(png_ptr, row ? row[i] : NULL,
                     display_row ? display_row[i] : NULL);
}

/* ========================================================================= */
/* Info access                                                               */
/* ========================================================================= */

png_uint_32 png_get_IHDR(png_const_structrp png_ptr,
    png_const_inforp info_ptr,
    png_uint_32 *width, png_uint_32 *height,
    int *bit_depth, int *color_type,
    int *interlace_method, int *compression_method,
    int *filter_method)
{
    (void)png_ptr;
    if (info_ptr == NULL || !info_ptr->valid) return 0;
    if (width) *width = info_ptr->width;
    if (height) *height = info_ptr->height;
    if (bit_depth) *bit_depth = info_ptr->bit_depth;
    if (color_type) *color_type = info_ptr->color_type;
    if (interlace_method) *interlace_method = info_ptr->interlace_type;
    if (compression_method) *compression_method = info_ptr->compression_type;
    if (filter_method) *filter_method = info_ptr->filter_type;
    return 1;
}

void png_set_IHDR(png_const_structrp png_ptr, png_inforp info_ptr,
    png_uint_32 width, png_uint_32 height,
    int bit_depth, int color_type,
    int interlace_method, int compression_method,
    int filter_method)
{
    (void)png_ptr;
    if (info_ptr == NULL) return;
    info_ptr->width = width;
    info_ptr->height = height;
    info_ptr->bit_depth = bit_depth;
    info_ptr->color_type = color_type;
    info_ptr->interlace_type = interlace_method;
    info_ptr->compression_type = compression_method;
    info_ptr->filter_type = filter_method;
    info_ptr->valid = 1;
}

png_uint_32 png_get_image_width(png_const_structrp png_ptr,
    png_const_inforp info_ptr)
{
    (void)png_ptr;
    return info_ptr ? info_ptr->width : 0;
}

png_uint_32 png_get_image_height(png_const_structrp png_ptr,
    png_const_inforp info_ptr)
{
    (void)png_ptr;
    return info_ptr ? info_ptr->height : 0;
}

png_byte png_get_color_type(png_const_structrp png_ptr,
    png_const_inforp info_ptr)
{
    (void)png_ptr;
    return info_ptr ? (png_byte)info_ptr->color_type : 0;
}

png_byte png_get_bit_depth(png_const_structrp png_ptr,
    png_const_inforp info_ptr)
{
    (void)png_ptr;
    return info_ptr ? (png_byte)info_ptr->bit_depth : 0;
}

png_size_t png_get_rowbytes(png_const_structrp png_ptr,
    png_const_inforp info_ptr)
{
    (void)png_ptr;
    return info_ptr ? info_ptr->rowbytes : 0;
}

png_byte png_get_channels(png_const_structrp png_ptr,
    png_const_inforp info_ptr)
{
    (void)png_ptr;
    return info_ptr ? info_ptr->channels : 0;
}

/* ========================================================================= */
/* Transforms (stubs / simple flags)                                         */
/* ========================================================================= */

void png_set_expand(png_structrp png_ptr)
{
    if (png_ptr) png_ptr->transforms |= PNG_TRANSFORM_EXPAND;
}

void png_set_expand_gray_1_2_4_to_8(png_structrp png_ptr)
{
    (void)png_ptr;
}

void png_set_palette_to_rgb(png_structrp png_ptr)
{
    (void)png_ptr;
}

void png_set_tRNS_to_alpha(png_structrp png_ptr)
{
    (void)png_ptr;
}

void png_set_gray_to_rgb(png_structrp png_ptr)
{
    (void)png_ptr;
}

void png_set_strip_16(png_structrp png_ptr)
{
    if (png_ptr) png_ptr->transforms |= PNG_TRANSFORM_STRIP_16;
}

void png_set_strip_alpha(png_structrp png_ptr)
{
    if (png_ptr) png_ptr->transforms |= PNG_TRANSFORM_STRIP_ALPHA;
}

void png_set_bgr(png_structrp png_ptr)
{
    if (png_ptr) png_ptr->transforms |= PNG_TRANSFORM_BGR;
}

void png_set_swap_alpha(png_structrp png_ptr)
{
    if (png_ptr) png_ptr->transforms |= PNG_TRANSFORM_SWAP_ALPHA;
}

void png_set_filler(png_structrp png_ptr, png_uint_32 filler, int flags)
{
    (void)png_ptr; (void)filler; (void)flags;
}

void png_set_add_alpha(png_structrp png_ptr, png_uint_32 filler, int flags)
{
    (void)png_ptr; (void)filler; (void)flags;
}

/* ========================================================================= */
/* Write stubs                                                               */
/* ========================================================================= */

void png_write_info(png_structrp png_ptr, png_const_inforp info_ptr)
{
    (void)png_ptr; (void)info_ptr;
}

void png_write_row(png_structrp png_ptr, png_const_bytep row)
{
    (void)png_ptr; (void)row;
}

void png_write_rows(png_structrp png_ptr, png_bytepp row,
                    png_uint_32 num_rows)
{
    (void)png_ptr; (void)row; (void)num_rows;
}

void png_write_image(png_structrp png_ptr, png_bytepp image)
{
    (void)png_ptr; (void)image;
}

void png_write_end(png_structrp png_ptr, png_inforp info_ptr)
{
    (void)png_ptr; (void)info_ptr;
}

/* ========================================================================= */
/* Error handling                                                            */
/* ========================================================================= */

void png_error(png_const_structrp png_ptr, png_const_charp error_message)
{
    if (png_ptr && png_ptr->error_fn)
        png_ptr->error_fn((png_structp)png_ptr, error_message);
}

void png_warning(png_const_structrp png_ptr, png_const_charp warning_message)
{
    if (png_ptr && png_ptr->warning_fn)
        png_ptr->warning_fn((png_structp)png_ptr, warning_message);
}

void png_set_error_fn(png_structrp png_ptr, png_voidp error_ptr,
                      png_error_ptr error_fn, png_error_ptr warning_fn)
{
    if (png_ptr) {
        png_ptr->error_ptr = error_ptr;
        png_ptr->error_fn = error_fn;
        png_ptr->warning_fn = warning_fn;
    }
}

png_voidp png_get_error_ptr(png_const_structrp png_ptr)
{
    return png_ptr ? png_ptr->error_ptr : NULL;
}

/* ========================================================================= */
/* Utility                                                                   */
/* ========================================================================= */

png_uint_32 png_access_version_number(void)
{
    return PNG_LIBPNG_VER;
}

int png_sig_cmp(png_const_bytep sig, png_size_t start,
                png_size_t num_to_check)
{
    if (sig == NULL || start >= 8 || num_to_check == 0)
        return -1;

    if (start + num_to_check > 8)
        num_to_check = 8 - start;

    return memcmp(sig + start, png_sig + start, num_to_check);
}
