/*
 * VeridianOS libc -- jerror.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libjpeg-turbo error code definitions.
 */

#ifndef _JERROR_H
#define _JERROR_H

/* Error message codes (subset) */
#define JERR_BAD_ALIGN_TYPE       1
#define JERR_BAD_ALLOC_CHUNK      2
#define JERR_BAD_BUFFER_MODE      3
#define JERR_BAD_COMPONENT_ID     4
#define JERR_BAD_CROP_SPEC        5
#define JERR_BAD_IN_COLORSPACE    6
#define JERR_BAD_J_COLORSPACE     7
#define JERR_BAD_LENGTH           8
#define JERR_BAD_MCU_SIZE         9
#define JERR_BAD_POOL_ID         10
#define JERR_BAD_PRECISION       11
#define JERR_BAD_SAMPLING        12
#define JERR_BAD_STATE           13
#define JERR_BAD_VIRTUAL_ACCESS  14
#define JERR_BUFFER_SIZE         15
#define JERR_CANT_SUSPEND        16
#define JERR_CCIR601_NOTIMPL     17
#define JERR_COMPONENT_COUNT     18
#define JERR_CONVERSION_NOTIMPL  19
#define JERR_DAC_INDEX           20
#define JERR_DAC_VALUE           21
#define JERR_DHT_INDEX           22
#define JERR_DQT_INDEX           23
#define JERR_EMPTY_IMAGE         24
#define JERR_FILE_READ           25
#define JERR_FILE_WRITE          26
#define JERR_FRACT_SAMPLE_NOTIMPL 27
#define JERR_HUFF_CLEN_OVERFLOW  28
#define JERR_HUFF_MISSING_CODE   29
#define JERR_INPUT_EMPTY         30
#define JERR_INPUT_EOF           31
#define JERR_MISSING_DATA        32
#define JERR_NO_BACKING_STORE    33
#define JERR_NO_HUFF_TABLE       34
#define JERR_NO_IMAGE            35
#define JERR_NO_QUANT_TABLE      36
#define JERR_NO_SOI              37
#define JERR_OUT_OF_MEMORY       38
#define JERR_SOF_DUPLICATE       39
#define JERR_SOF_NO_SOS          40
#define JERR_SOF_UNSUPPORTED     41
#define JERR_SOI_DUPLICATE       42
#define JERR_SOS_NO_SOF          43
#define JERR_TOO_LITTLE_DATA     44
#define JERR_UNKNOWN_MARKER      45
#define JERR_WIDTH_OVERFLOW      46

/* Trace message codes */
#define JTRC_16BIT_TABLES        100
#define JTRC_ADOBE               101
#define JTRC_APP0                102
#define JTRC_APP14               103
#define JTRC_DAC                 104
#define JTRC_DHT                 105
#define JTRC_DQT                 106
#define JTRC_DRI                 107
#define JTRC_EOI                 108
#define JTRC_HUFFBITS            109
#define JTRC_JFIF                110

/* Warning codes */
#define JWRN_ADOBE_XFORM         200
#define JWRN_BOGUS_PROGRESSION   201
#define JWRN_EXTRANEOUS_DATA     202
#define JWRN_HIT_MARKER          203
#define JWRN_HUFF_BAD_CODE       204
#define JWRN_JFIF_MAJOR          205
#define JWRN_NOT_SEQUENTIAL      206

#endif /* _JERROR_H */
