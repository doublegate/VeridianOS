/*
 * VeridianOS libc -- freetype/fttypes.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 basic type definitions.
 */

#ifndef _FREETYPE_FTTYPES_H
#define _FREETYPE_FTTYPES_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Basic types                                                               */
/* ========================================================================= */

typedef unsigned char   FT_Byte;
typedef signed char     FT_Char;
typedef signed int      FT_Int;
typedef unsigned int    FT_UInt;
typedef signed short    FT_Short;
typedef unsigned short  FT_UShort;
typedef signed long     FT_Long;
typedef unsigned long   FT_ULong;
typedef signed long     FT_Fixed;      /* 16.16 fixed-point */
typedef signed long     FT_F26Dot6;    /* 26.6 fixed-point  */
typedef int             FT_Error;
typedef void           *FT_Pointer;
typedef size_t          FT_Offset;
typedef int32_t         FT_Int32;
typedef uint32_t        FT_UInt32;
typedef char            FT_String;
typedef unsigned char   FT_Bool;

typedef int32_t         FT_F2Dot14;
typedef int16_t         FT_FWord;
typedef uint16_t        FT_UFWord;

typedef int             FT_Pos;

/* ========================================================================= */
/* Generic callback types                                                    */
/* ========================================================================= */

typedef void (*FT_Generic_Finalizer)(void *object);

typedef struct FT_Generic_ {
    void                  *data;
    FT_Generic_Finalizer   finalizer;
} FT_Generic;

/* ========================================================================= */
/* Memory allocation callbacks                                               */
/* ========================================================================= */

typedef void *(*FT_Alloc_Func)(void *memory, long size);
typedef void  (*FT_Free_Func)(void *memory, void *block);
typedef void *(*FT_Realloc_Func)(void *memory, long cur_size,
                                  long new_size, void *block);

typedef struct FT_MemoryRec_ *FT_Memory;

struct FT_MemoryRec_ {
    void            *user;
    FT_Alloc_Func    alloc;
    FT_Free_Func     free;
    FT_Realloc_Func  realloc;
};

/* ========================================================================= */
/* List types                                                                */
/* ========================================================================= */

typedef struct FT_ListNodeRec_ *FT_ListNode;
typedef struct FT_ListRec_     *FT_List;

struct FT_ListNodeRec_ {
    FT_ListNode  prev;
    FT_ListNode  next;
    void        *data;
};

struct FT_ListRec_ {
    FT_ListNode  head;
    FT_ListNode  tail;
};

typedef struct FT_ListRec_ FT_ListRec;

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTTYPES_H */
