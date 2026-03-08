/*
 * VeridianOS libc -- freetype/fterrors.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 error codes.
 */

#ifndef _FREETYPE_FTERRORS_H
#define _FREETYPE_FTERRORS_H

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Error codes                                                               */
/* ========================================================================= */

#define FT_Err_Ok                           0x00
#define FT_Err_Cannot_Open_Resource         0x01
#define FT_Err_Unknown_File_Format          0x02
#define FT_Err_Invalid_File_Format          0x03
#define FT_Err_Invalid_Version              0x04
#define FT_Err_Lower_Module_Version         0x05
#define FT_Err_Invalid_Argument             0x06
#define FT_Err_Unimplemented_Feature        0x07
#define FT_Err_Invalid_Table                0x08
#define FT_Err_Invalid_Offset               0x09
#define FT_Err_Array_Too_Large              0x0A
#define FT_Err_Missing_Module               0x0B
#define FT_Err_Missing_Property             0x0C

/* Glyph/character errors */
#define FT_Err_Invalid_Glyph_Index          0x10
#define FT_Err_Invalid_Character_Code       0x11
#define FT_Err_Invalid_Glyph_Format         0x12
#define FT_Err_Cannot_Render_Glyph          0x13
#define FT_Err_Invalid_Outline              0x14
#define FT_Err_Invalid_Composite            0x15
#define FT_Err_Too_Many_Hints               0x16
#define FT_Err_Invalid_Pixel_Size           0x17
#define FT_Err_Invalid_SVG_Document         0x18

/* Handle errors */
#define FT_Err_Invalid_Handle               0x20
#define FT_Err_Invalid_Library_Handle       0x21
#define FT_Err_Invalid_Driver_Handle        0x22
#define FT_Err_Invalid_Face_Handle          0x23
#define FT_Err_Invalid_Size_Handle          0x24
#define FT_Err_Invalid_Slot_Handle          0x25
#define FT_Err_Invalid_CharMap_Handle       0x26
#define FT_Err_Invalid_Cache_Handle         0x27
#define FT_Err_Invalid_Stream_Handle        0x28

/* Driver errors */
#define FT_Err_Too_Many_Drivers             0x30
#define FT_Err_Too_Many_Extensions          0x31

/* Memory errors */
#define FT_Err_Out_Of_Memory                0x40
#define FT_Err_Unlisted_Object              0x41

/* Stream errors */
#define FT_Err_Cannot_Open_Stream           0x51
#define FT_Err_Invalid_Stream_Seek          0x52
#define FT_Err_Invalid_Stream_Skip          0x53
#define FT_Err_Invalid_Stream_Read          0x54
#define FT_Err_Invalid_Stream_Operation     0x55
#define FT_Err_Invalid_Frame_Operation      0x56
#define FT_Err_Nested_Frame_Access          0x57
#define FT_Err_Invalid_Frame_Read           0x58

/* Raster errors */
#define FT_Err_Raster_Uninitialized         0x60
#define FT_Err_Raster_Corrupted             0x61
#define FT_Err_Raster_Overflow              0x62
#define FT_Err_Raster_Negative_Height       0x63

/* Cache errors */
#define FT_Err_Too_Many_Caches              0x70

/* TrueType / SFNT errors */
#define FT_Err_Invalid_Opcode               0x80
#define FT_Err_Too_Few_Arguments            0x81
#define FT_Err_Stack_Overflow               0x82
#define FT_Err_Code_Overflow                0x83
#define FT_Err_Bad_Argument                 0x84
#define FT_Err_Divide_By_Zero              0x85
#define FT_Err_Invalid_Reference            0x86
#define FT_Err_Debug_OpCode                 0x87
#define FT_Err_ENDF_In_Exec_Stream         0x88
#define FT_Err_Nested_DEFS                  0x89
#define FT_Err_Invalid_CodeRange            0x8A
#define FT_Err_Execution_Too_Long           0x8B
#define FT_Err_Too_Many_Function_Defs       0x8C
#define FT_Err_Too_Many_Instruction_Defs    0x8D
#define FT_Err_Table_Missing                0x8E
#define FT_Err_Horiz_Header_Missing         0x8F
#define FT_Err_Locations_Missing            0x90
#define FT_Err_Name_Table_Missing           0x91
#define FT_Err_CMap_Table_Missing           0x92
#define FT_Err_Hmtx_Table_Missing           0x93
#define FT_Err_Post_Table_Missing           0x94
#define FT_Err_Invalid_Horiz_Metrics        0x95
#define FT_Err_Invalid_CharMap_Format        0x96
#define FT_Err_Invalid_PPem                 0x97
#define FT_Err_Invalid_Vert_Metrics         0x98
#define FT_Err_Could_Not_Find_Context       0x99

/* CFF / Type 1 errors */
#define FT_Err_Syntax_Error                 0xA0
#define FT_Err_Stack_Underflow              0xA1
#define FT_Err_Ignore                       0xA2
#define FT_Err_No_Unicode_Glyph_Name        0xA3
#define FT_Err_Glyph_Too_Big               0xA4

/* BDF errors */
#define FT_Err_Missing_Startfont_Field      0xB0
#define FT_Err_Missing_Font_Field           0xB1
#define FT_Err_Missing_Size_Field           0xB2
#define FT_Err_Missing_Fontboundingbox_Field 0xB3
#define FT_Err_Missing_Chars_Field          0xB4
#define FT_Err_Missing_Startchar_Field      0xB5
#define FT_Err_Missing_Encoding_Field       0xB6
#define FT_Err_Missing_Bbx_Field            0xB7
#define FT_Err_Bbx_Too_Big                  0xB8
#define FT_Err_Corrupted_Font_Header        0xB9
#define FT_Err_Corrupted_Font_Glyphs        0xBA

#define FT_Err_Max                          0xFF

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTERRORS_H */
