/*
 * VeridianOS libc -- fontconfig/fcfreetype.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Fontconfig FreeType integration.
 */

#ifndef _FONTCONFIG_FCFREETYPE_H
#define _FONTCONFIG_FCFREETYPE_H

#include <fontconfig/fontconfig.h>
#include <ft2build.h>
#include <freetype/freetype.h>

#ifdef __cplusplus
extern "C" {
#endif

FT_UInt     FcFreeTypeCharIndex(FT_Face face, FcChar32 ucs4);
FcCharSet  *FcFreeTypeCharSetAndSpacing(FT_Face face, FcBlanks *blanks,
                                          int *spacing);
FcCharSet  *FcFreeTypeCharSet(FT_Face face, FcBlanks *blanks);
FcResult    FcPatternGetFTFace(const FcPattern *p, const char *object,
                                 int n, FT_Face *f);
FcBool      FcPatternAddFTFace(FcPattern *p, const char *object,
                                 const FT_Face f);

#ifdef __cplusplus
}
#endif

#endif /* _FONTCONFIG_FCFREETYPE_H */
