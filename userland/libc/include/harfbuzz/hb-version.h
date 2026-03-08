/*
 * VeridianOS libc -- harfbuzz/hb-version.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz version information.
 */

#ifndef _HARFBUZZ_HB_VERSION_H
#define _HARFBUZZ_HB_VERSION_H

#include <harfbuzz/hb-common.h>

#ifdef __cplusplus
extern "C" {
#endif

#define HB_VERSION_MAJOR  9
#define HB_VERSION_MINOR  0
#define HB_VERSION_MICRO  0

#define HB_VERSION_STRING "9.0.0"

#define HB_VERSION_ATLEAST(major,minor,micro) \
    ((major) * 10000 + (minor) * 100 + (micro) <= \
     HB_VERSION_MAJOR * 10000 + HB_VERSION_MINOR * 100 + HB_VERSION_MICRO)

void          hb_version(unsigned int *major, unsigned int *minor,
                           unsigned int *micro);
const char   *hb_version_string(void);
hb_bool_t     hb_version_atleast(unsigned int major, unsigned int minor,
                                    unsigned int micro);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_VERSION_H */
