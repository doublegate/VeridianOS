/*
 * VeridianOS libc -- jpeglib.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libjpeg-turbo 3.0.x compatible API.
 * JPEG compression and decompression.
 */

#ifndef _JPEGLIB_H
#define _JPEGLIB_H

#include "jconfig.h"
#include "jmorecfg.h"
#include <stddef.h>
#include <stdio.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define JPEG_LIB_VERSION  62
#define LIBJPEG_TURBO_VERSION_NUMBER 3000000

/* ========================================================================= */
/* Color spaces                                                              */
/* ========================================================================= */

typedef enum {
    JCS_UNKNOWN,
    JCS_GRAYSCALE,
    JCS_RGB,
    JCS_YCbCr,
    JCS_CMYK,
    JCS_YCCK,
    JCS_EXT_RGB,
    JCS_EXT_RGBX,
    JCS_EXT_BGR,
    JCS_EXT_BGRX,
    JCS_EXT_XBGR,
    JCS_EXT_XRGB,
    JCS_EXT_RGBA,
    JCS_EXT_BGRA,
    JCS_EXT_ABGR,
    JCS_EXT_ARGB,
    JCS_RGB565
} J_COLOR_SPACE;

typedef enum {
    JDCT_ISLOW,
    JDCT_IFAST,
    JDCT_FLOAT
} J_DCT_METHOD;

typedef enum {
    JDITHER_NONE,
    JDITHER_ORDERED,
    JDITHER_FS
} J_DITHER_MODE;

/* ========================================================================= */
/* Error manager                                                             */
/* ========================================================================= */

struct jpeg_error_mgr {
    void (*error_exit)(void *cinfo);
    void (*emit_message)(void *cinfo, int msg_level);
    void (*output_message)(void *cinfo);
    void (*format_message)(void *cinfo, char *buffer);
    void (*reset_error_mgr)(void *cinfo);
    int msg_code;
    char msg_parm_s[80];
    int msg_parm_i[8];
    int trace_level;
    long num_warnings;
    const char *const *jpeg_message_table;
    int last_jpeg_message;
    const char *const *addon_message_table;
    int first_addon_message;
    int last_addon_message;
};

/* ========================================================================= */
/* Source/destination managers                                                */
/* ========================================================================= */

struct jpeg_source_mgr {
    const JOCTET *next_input_byte;
    size_t bytes_in_buffer;
    void (*init_source)(void *cinfo);
    int (*fill_input_buffer)(void *cinfo);
    void (*skip_input_data)(void *cinfo, long num_bytes);
    int (*resync_to_restart)(void *cinfo, int desired);
    void (*term_source)(void *cinfo);
};

struct jpeg_destination_mgr {
    JOCTET *next_output_byte;
    size_t free_in_buffer;
    void (*init_destination)(void *cinfo);
    int (*empty_output_buffer)(void *cinfo);
    void (*term_destination)(void *cinfo);
};

/* ========================================================================= */
/* Compress/decompress structures                                            */
/* ========================================================================= */

struct jpeg_common_struct {
    struct jpeg_error_mgr *err;
    void *mem;
    void *progress;
    void *client_data;
    int is_decompressor;
    int global_state;
};

struct jpeg_compress_struct {
    struct jpeg_error_mgr *err;
    void *mem;
    void *progress;
    void *client_data;
    int is_decompressor;
    int global_state;
    struct jpeg_destination_mgr *dest;
    JDIMENSION image_width;
    JDIMENSION image_height;
    int input_components;
    J_COLOR_SPACE in_color_space;
    int data_precision;
    int num_components;
    J_COLOR_SPACE jpeg_color_space;
    int quality;
    /* Internal fields omitted */
    void *_internal;
};

struct jpeg_decompress_struct {
    struct jpeg_error_mgr *err;
    void *mem;
    void *progress;
    void *client_data;
    int is_decompressor;
    int global_state;
    struct jpeg_source_mgr *src;
    JDIMENSION image_width;
    JDIMENSION image_height;
    int num_components;
    J_COLOR_SPACE jpeg_color_space;
    J_COLOR_SPACE out_color_space;
    unsigned int scale_num;
    unsigned int scale_denom;
    int output_gamma;
    int buffered_image;
    int raw_data_out;
    J_DCT_METHOD dct_method;
    int do_fancy_upsampling;
    int do_block_smoothing;
    int quantize_colors;
    J_DITHER_MODE dither_mode;
    int two_pass_quantize;
    int desired_number_of_colors;
    int enable_1pass_quant;
    int enable_external_quant;
    int enable_2pass_quant;
    JDIMENSION output_width;
    JDIMENSION output_height;
    int out_color_components;
    int output_components;
    int rec_outbuf_height;
    int actual_number_of_colors;
    JSAMPLE **colormap;
    JDIMENSION output_scanline;
    int input_scan_number;
    JDIMENSION input_iMCU_row;
    int output_scan_number;
    JDIMENSION output_iMCU_row;
    int data_precision;
    /* Internal fields omitted */
    void *_internal;
};

typedef struct jpeg_compress_struct   *j_compress_ptr;
typedef struct jpeg_decompress_struct *j_decompress_ptr;
typedef struct jpeg_common_struct     *j_common_ptr;

typedef JSAMPLE *JSAMPROW;
typedef JSAMPROW *JSAMPARRAY;

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

/* Error manager */
struct jpeg_error_mgr *jpeg_std_error(struct jpeg_error_mgr *err);

/* Decompression */
void jpeg_CreateDecompress(j_decompress_ptr cinfo, int version,
                           size_t structsize);
int jpeg_read_header(j_decompress_ptr cinfo, int require_image);
int jpeg_start_decompress(j_decompress_ptr cinfo);
JDIMENSION jpeg_read_scanlines(j_decompress_ptr cinfo,
                               JSAMPARRAY scanlines,
                               JDIMENSION max_lines);
int jpeg_finish_decompress(j_decompress_ptr cinfo);
void jpeg_destroy_decompress(j_decompress_ptr cinfo);
void jpeg_stdio_src(j_decompress_ptr cinfo, FILE *infile);

/* Compression */
void jpeg_CreateCompress(j_compress_ptr cinfo, int version,
                         size_t structsize);
void jpeg_set_defaults(j_compress_ptr cinfo);
void jpeg_set_quality(j_compress_ptr cinfo, int quality,
                      int force_baseline);
void jpeg_start_compress(j_compress_ptr cinfo, int write_all_tables);
JDIMENSION jpeg_write_scanlines(j_compress_ptr cinfo,
                                JSAMPARRAY scanlines,
                                JDIMENSION num_lines);
void jpeg_finish_compress(j_compress_ptr cinfo);
void jpeg_destroy_compress(j_compress_ptr cinfo);
void jpeg_stdio_dest(j_compress_ptr cinfo, FILE *outfile);

/* Memory source/dest */
void jpeg_mem_src(j_decompress_ptr cinfo,
                  const unsigned char *inbuffer, unsigned long insize);
void jpeg_mem_dest(j_compress_ptr cinfo,
                   unsigned char **outbuffer, unsigned long *outsize);

/* Common */
void jpeg_destroy(j_common_ptr cinfo);
void jpeg_abort(j_common_ptr cinfo);

/* Convenience macros */
#define jpeg_create_decompress(cinfo) \
    jpeg_CreateDecompress((cinfo), JPEG_LIB_VERSION, \
        (size_t)sizeof(struct jpeg_decompress_struct))

#define jpeg_create_compress(cinfo) \
    jpeg_CreateCompress((cinfo), JPEG_LIB_VERSION, \
        (size_t)sizeof(struct jpeg_compress_struct))

/* Header return codes */
#define JPEG_SUSPENDED       0
#define JPEG_HEADER_OK       1
#define JPEG_HEADER_TABLES_ONLY 2

#ifdef __cplusplus
}
#endif

#endif /* _JPEGLIB_H */
