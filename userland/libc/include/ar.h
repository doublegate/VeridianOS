/*
 * VeridianOS C Library -- <ar.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Archive file format definitions (ar(1) compatible).
 */

#ifndef _AR_H
#define _AR_H

#define ARMAG   "!<arch>\n"
#define SARMAG  8
#define ARFMAG  "`\n"

struct ar_hdr {
    char ar_name[16];   /* Member file name, terminated with '/' */
    char ar_date[12];   /* File modification timestamp (decimal) */
    char ar_uid[6];     /* Owner UID (decimal) */
    char ar_gid[6];     /* Owner GID (decimal) */
    char ar_mode[8];    /* File mode (octal) */
    char ar_size[10];   /* File size (decimal) */
    char ar_fmag[2];    /* Always ARFMAG */
};

#endif /* _AR_H */
