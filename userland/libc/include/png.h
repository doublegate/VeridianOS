/*
 * VeridianOS libc -- png.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libpng 1.6.x compatible API.
 * PNG image reading and writing.
 */

#ifndef _PNG_H
#define _PNG_H

#include <stddef.h>
#include <stdio.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define PNG_LIBPNG_VER_STRING "1.6.43"
#define PNG_LIBPNG_VER        10643
#define PNG_LIBPNG_VER_MAJOR  1
#define PNG_LIBPNG_VER_MINOR  6
#define PNG_LIBPNG_VER_RELEASE 43

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

typedef unsigned char  png_byte;
typedef unsigned short png_uint_16;
typedef unsigned int   png_uint_32;
typedef int            png_int_32;
typedef size_t         png_size_t;
typedef png_byte      *png_bytep;
typedef png_byte     **png_bytepp;
typedef png_uint_32   *png_uint_32p;
typedef const png_byte *png_const_bytep;
typedef const char    *png_const_charp;
typedef char          *png_charp;
typedef void          *png_voidp;
typedef const void    *png_const_voidp;

/* Opaque structures */
typedef struct png_struct_def  png_struct;
typedef struct png_info_def    png_info;
typedef png_struct *png_structp;
typedef png_struct **png_structpp;
typedef const png_struct *png_const_structp;
typedef png_info   *png_infop;
typedef png_info   **png_infopp;
typedef const png_info *png_const_infop;

typedef png_structp  png_structrp;
typedef png_infop    png_inforp;
typedef const png_struct *png_const_structrp;
typedef const png_info   *png_const_inforp;

/* ========================================================================= */
/* Color types                                                               */
/* ========================================================================= */

#define PNG_COLOR_MASK_PALETTE    1
#define PNG_COLOR_MASK_COLOR      2
#define PNG_COLOR_MASK_ALPHA      4

#define PNG_COLOR_TYPE_GRAY       0
#define PNG_COLOR_TYPE_PALETTE    (PNG_COLOR_MASK_COLOR | PNG_COLOR_MASK_PALETTE)
#define PNG_COLOR_TYPE_RGB        PNG_COLOR_MASK_COLOR
#define PNG_COLOR_TYPE_RGB_ALPHA  (PNG_COLOR_MASK_COLOR | PNG_COLOR_MASK_ALPHA)
#define PNG_COLOR_TYPE_GRAY_ALPHA PNG_COLOR_MASK_ALPHA
#define PNG_COLOR_TYPE_GA         PNG_COLOR_TYPE_GRAY_ALPHA
#define PNG_COLOR_TYPE_RGBA       PNG_COLOR_TYPE_RGB_ALPHA

/* ========================================================================= */
/* Interlace types                                                           */
/* ========================================================================= */

#define PNG_INTERLACE_NONE  0
#define PNG_INTERLACE_ADAM7 1

/* ========================================================================= */
/* Compression/filter types                                                  */
/* ========================================================================= */

#define PNG_COMPRESSION_TYPE_BASE    0
#define PNG_COMPRESSION_TYPE_DEFAULT PNG_COMPRESSION_TYPE_BASE
#define PNG_FILTER_TYPE_BASE         0
#define PNG_FILTER_TYPE_DEFAULT      PNG_FILTER_TYPE_BASE

/* ========================================================================= */
/* Transform flags                                                           */
/* ========================================================================= */

#define PNG_TRANSFORM_IDENTITY       0x0000
#define PNG_TRANSFORM_STRIP_16       0x0001
#define PNG_TRANSFORM_STRIP_ALPHA    0x0002
#define PNG_TRANSFORM_PACKING        0x0004
#define PNG_TRANSFORM_PACKSWAP       0x0008
#define PNG_TRANSFORM_EXPAND         0x0010
#define PNG_TRANSFORM_INVERT_MONO    0x0020
#define PNG_TRANSFORM_SHIFT          0x0040
#define PNG_TRANSFORM_BGR            0x0080
#define PNG_TRANSFORM_SWAP_ALPHA     0x0100
#define PNG_TRANSFORM_SWAP_ENDIAN    0x0200
#define PNG_TRANSFORM_INVERT_ALPHA   0x0400
#define PNG_TRANSFORM_STRIP_FILLER   0x0800
#define PNG_TRANSFORM_EXPAND_16     0x4000

/* ========================================================================= */
/* Callback typedefs                                                         */
/* ========================================================================= */

typedef void (*png_error_ptr)(png_structp, png_const_charp);
typedef void (*png_rw_ptr)(png_structp, png_bytep, png_size_t);
typedef void (*png_flush_ptr)(png_structp);
typedef void (*png_read_status_ptr)(png_structp, png_uint_32, int);
typedef void (*png_write_status_ptr)(png_structp, png_uint_32, int);

/* ========================================================================= */
/* Read API                                                                  */
/* ========================================================================= */

png_structp png_create_read_struct(png_const_charp user_png_ver,
    png_voidp error_ptr, png_error_ptr error_fn, png_error_ptr warn_fn);

png_infop png_create_info_struct(png_const_structrp png_ptr);

void png_destroy_read_struct(png_structpp png_ptr_ptr,
    png_infopp info_ptr_ptr, png_infopp end_info_ptr_ptr);

void png_set_read_fn(png_structrp png_ptr, png_voidp io_ptr,
                     png_rw_ptr read_data_fn);

void png_init_io(png_structrp png_ptr, FILE *fp);

void png_read_info(png_structrp png_ptr, png_inforp info_ptr);

void png_read_image(png_structrp png_ptr, png_bytepp image);

void png_read_end(png_structrp png_ptr, png_inforp info_ptr);

void png_read_update_info(png_structrp png_ptr, png_inforp info_ptr);

void png_read_row(png_structrp png_ptr, png_bytep row,
                  png_bytep display_row);

void png_read_rows(png_structrp png_ptr, png_bytepp row,
                   png_bytepp display_row, png_uint_32 num_rows);

/* ========================================================================= */
/* Write API                                                                 */
/* ========================================================================= */

png_structp png_create_write_struct(png_const_charp user_png_ver,
    png_voidp error_ptr, png_error_ptr error_fn, png_error_ptr warn_fn);

void png_destroy_write_struct(png_structpp png_ptr_ptr,
                              png_infopp info_ptr_ptr);

void png_set_write_fn(png_structrp png_ptr, png_voidp io_ptr,
                      png_rw_ptr write_data_fn,
                      png_flush_ptr output_flush_fn);

void png_write_info(png_structrp png_ptr, png_const_inforp info_ptr);

void png_write_row(png_structrp png_ptr, png_const_bytep row);

void png_write_rows(png_structrp png_ptr, png_bytepp row,
                    png_uint_32 num_rows);

void png_write_image(png_structrp png_ptr, png_bytepp image);

void png_write_end(png_structrp png_ptr, png_inforp info_ptr);

/* ========================================================================= */
/* Info access functions                                                     */
/* ========================================================================= */

png_uint_32 png_get_IHDR(png_const_structrp png_ptr,
    png_const_inforp info_ptr,
    png_uint_32 *width, png_uint_32 *height,
    int *bit_depth, int *color_type,
    int *interlace_method, int *compression_method,
    int *filter_method);

void png_set_IHDR(png_const_structrp png_ptr, png_inforp info_ptr,
    png_uint_32 width, png_uint_32 height,
    int bit_depth, int color_type,
    int interlace_method, int compression_method,
    int filter_method);

png_uint_32 png_get_image_width(png_const_structrp png_ptr,
    png_const_inforp info_ptr);

png_uint_32 png_get_image_height(png_const_structrp png_ptr,
    png_const_inforp info_ptr);

png_byte png_get_color_type(png_const_structrp png_ptr,
    png_const_inforp info_ptr);

png_byte png_get_bit_depth(png_const_structrp png_ptr,
    png_const_inforp info_ptr);

png_size_t png_get_rowbytes(png_const_structrp png_ptr,
    png_const_inforp info_ptr);

png_byte png_get_channels(png_const_structrp png_ptr,
    png_const_inforp info_ptr);

/* ========================================================================= */
/* Transform functions                                                       */
/* ========================================================================= */

void png_set_expand(png_structrp png_ptr);
void png_set_expand_gray_1_2_4_to_8(png_structrp png_ptr);
void png_set_palette_to_rgb(png_structrp png_ptr);
void png_set_tRNS_to_alpha(png_structrp png_ptr);
void png_set_gray_to_rgb(png_structrp png_ptr);
void png_set_strip_16(png_structrp png_ptr);
void png_set_strip_alpha(png_structrp png_ptr);
void png_set_bgr(png_structrp png_ptr);
void png_set_swap_alpha(png_structrp png_ptr);
void png_set_filler(png_structrp png_ptr, png_uint_32 filler,
                    int flags);
void png_set_add_alpha(png_structrp png_ptr, png_uint_32 filler,
                       int flags);

#define PNG_FILLER_BEFORE 0
#define PNG_FILLER_AFTER  1

/* ========================================================================= */
/* Error handling                                                            */
/* ========================================================================= */

void png_error(png_const_structrp png_ptr, png_const_charp error_message);
void png_warning(png_const_structrp png_ptr, png_const_charp warning_message);

/* ========================================================================= */
/* Utility                                                                   */
/* ========================================================================= */

png_uint_32 png_access_version_number(void);
int png_sig_cmp(png_const_bytep sig, png_size_t start,
                png_size_t num_to_check);

/* User data pointer */
void png_set_error_fn(png_structrp png_ptr, png_voidp error_ptr,
                      png_error_ptr error_fn, png_error_ptr warning_fn);
png_voidp png_get_error_ptr(png_const_structrp png_ptr);
png_voidp png_get_io_ptr(png_const_structrp png_ptr);

#ifdef __cplusplus
}
#endif

#endif /* _PNG_H */
