/*
 * VeridianOS libc -- freetype/ftmodapi.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 module management API.
 */

#ifndef _FREETYPE_FTMODAPI_H
#define _FREETYPE_FTMODAPI_H

#include <freetype/freetype.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Module types                                                              */
/* ========================================================================= */

typedef struct FT_ModuleRec_   *FT_Module;
typedef struct FT_RendererRec_ *FT_Renderer;

/* Module flags */
#define FT_MODULE_FONT_DRIVER      1
#define FT_MODULE_RENDERER         2
#define FT_MODULE_HINTER           4
#define FT_MODULE_STYLER           8
#define FT_MODULE_DRIVER_SCALABLE  0x100
#define FT_MODULE_DRIVER_NO_OUTLINES 0x200
#define FT_MODULE_DRIVER_HAS_HINTER 0x400

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

FT_Error  FT_Add_Module(FT_Library library, const void *clazz);
FT_Module FT_Get_Module(FT_Library library, const char *module_name);
FT_Error  FT_Remove_Module(FT_Library library, FT_Module module);
FT_Error  FT_Property_Set(FT_Library library, const FT_String *module_name,
                            const FT_String *property_name,
                            const void *value);
FT_Error  FT_Property_Get(FT_Library library, const FT_String *module_name,
                            const FT_String *property_name, void *value);
void      FT_Set_Default_Properties(FT_Library library);
FT_Error  FT_Set_Renderer(FT_Library library, FT_Renderer renderer,
                            FT_UInt num_params, FT_Parameter *parameters);
FT_Renderer FT_Get_Renderer(FT_Library library, FT_Glyph_Format format);

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTMODAPI_H */
