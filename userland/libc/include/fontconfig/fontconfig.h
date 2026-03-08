/*
 * VeridianOS libc -- fontconfig/fontconfig.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Fontconfig 2.15.0 API declarations: font configuration, pattern
 * matching, font set management, and object set queries.
 */

#ifndef _FONTCONFIG_FONTCONFIG_H
#define _FONTCONFIG_FONTCONFIG_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define FC_MAJOR     2
#define FC_MINOR     15
#define FC_REVISION  0

#define FC_VERSION   ((FC_MAJOR * 10000) + (FC_MINOR * 100) + FC_REVISION)

#define FcGetVersion FcConfigGetVersion

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

typedef unsigned char  FcChar8;
typedef unsigned short FcChar16;
typedef unsigned int   FcChar32;
typedef int            FcBool;

#define FcTrue   1
#define FcFalse  0

typedef enum _FcResult {
    FcResultMatch,
    FcResultNoMatch,
    FcResultTypeMismatch,
    FcResultNoId,
    FcResultOutOfMemory
} FcResult;

typedef enum _FcMatchKind {
    FcMatchPattern,
    FcMatchFont,
    FcMatchScan,
    FcMatchKindEnd,
    FcMatchKindBegin = FcMatchPattern
} FcMatchKind;

typedef enum _FcSetName {
    FcSetSystem = 0,
    FcSetApplication = 1
} FcSetName;

typedef enum _FcType {
    FcTypeUnknown = -1,
    FcTypeVoid,
    FcTypeInteger,
    FcTypeDouble,
    FcTypeString,
    FcTypeBool,
    FcTypeMatrix,
    FcTypeCharSet,
    FcTypeFTFace,
    FcTypeLangSet,
    FcTypeRange
} FcType;

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

typedef struct _FcConfig    FcConfig;
typedef struct _FcPattern   FcPattern;
typedef struct _FcFontSet   FcFontSet;
typedef struct _FcObjectSet FcObjectSet;
typedef struct _FcLangSet   FcLangSet;
typedef struct _FcCharSet   FcCharSet;
typedef struct _FcBlanks    FcBlanks;
typedef struct _FcStrList   FcStrList;
typedef struct _FcStrSet    FcStrSet;
typedef struct _FcCache     FcCache;
typedef struct _FcRange     FcRange;

/* ========================================================================= */
/* FcFontSet                                                                 */
/* ========================================================================= */

struct _FcFontSet {
    int          nfont;
    int          sfont;
    FcPattern  **fonts;
};

/* ========================================================================= */
/* Property name constants                                                   */
/* ========================================================================= */

#define FC_FAMILY           "family"
#define FC_STYLE            "style"
#define FC_SLANT            "slant"
#define FC_WEIGHT           "weight"
#define FC_SIZE             "size"
#define FC_ASPECT           "aspect"
#define FC_PIXEL_SIZE       "pixelsize"
#define FC_SPACING          "spacing"
#define FC_FOUNDRY          "foundry"
#define FC_ANTIALIAS        "antialias"
#define FC_HINTING          "hinting"
#define FC_HINT_STYLE       "hintstyle"
#define FC_VERTICAL_LAYOUT  "verticallayout"
#define FC_AUTOHINT         "autohint"
#define FC_GLOBAL_ADVANCE   "globaladvance"
#define FC_WIDTH            "width"
#define FC_FILE             "file"
#define FC_INDEX            "index"
#define FC_FT_FACE          "ftface"
#define FC_RASTERIZER       "rasterizer"
#define FC_OUTLINE          "outline"
#define FC_SCALABLE         "scalable"
#define FC_COLOR            "color"
#define FC_VARIABLE         "variable"
#define FC_SCALE            "scale"
#define FC_SYMBOL           "symbol"
#define FC_DPI              "dpi"
#define FC_RGBA             "rgba"
#define FC_MINSPACE         "minspace"
#define FC_SOURCE           "source"
#define FC_CHARSET          "charset"
#define FC_LANG             "lang"
#define FC_FONTVERSION      "fontversion"
#define FC_FULLNAME         "fullname"
#define FC_FAMILYLANG       "familylang"
#define FC_STYLELANG        "stylelang"
#define FC_FULLNAMELANG     "fullnamelang"
#define FC_CAPABILITY       "capability"
#define FC_FONTFORMAT       "fontformat"
#define FC_EMBOLDEN         "embolden"
#define FC_EMBEDDED_BITMAP  "embeddedbitmap"
#define FC_DECORATIVE       "decorative"
#define FC_LCD_FILTER       "lcdfilter"
#define FC_FONT_FEATURES    "fontfeatures"
#define FC_FONT_VARIATIONS  "fontvariations"
#define FC_NAMELANG         "namelang"
#define FC_PRGNAME          "prgname"
#define FC_HASH             "hash"
#define FC_POSTSCRIPT_NAME  "postscriptname"
#define FC_FONT_HAS_HINT    "fonthashint"
#define FC_ORDER            "order"
#define FC_DESKTOP_NAME     "desktop"

/* ========================================================================= */
/* Weight constants                                                          */
/* ========================================================================= */

#define FC_WEIGHT_THIN          0
#define FC_WEIGHT_EXTRALIGHT    40
#define FC_WEIGHT_ULTRALIGHT    FC_WEIGHT_EXTRALIGHT
#define FC_WEIGHT_LIGHT         50
#define FC_WEIGHT_DEMILIGHT     55
#define FC_WEIGHT_SEMILIGHT     FC_WEIGHT_DEMILIGHT
#define FC_WEIGHT_BOOK          75
#define FC_WEIGHT_REGULAR       80
#define FC_WEIGHT_NORMAL        FC_WEIGHT_REGULAR
#define FC_WEIGHT_MEDIUM        100
#define FC_WEIGHT_DEMIBOLD      180
#define FC_WEIGHT_SEMIBOLD      FC_WEIGHT_DEMIBOLD
#define FC_WEIGHT_BOLD          200
#define FC_WEIGHT_EXTRABOLD     205
#define FC_WEIGHT_ULTRABOLD     FC_WEIGHT_EXTRABOLD
#define FC_WEIGHT_BLACK         210
#define FC_WEIGHT_HEAVY         FC_WEIGHT_BLACK
#define FC_WEIGHT_EXTRA_BLACK   215
#define FC_WEIGHT_ULTRA_BLACK   FC_WEIGHT_EXTRA_BLACK

/* ========================================================================= */
/* Slant constants                                                           */
/* ========================================================================= */

#define FC_SLANT_ROMAN     0
#define FC_SLANT_ITALIC    100
#define FC_SLANT_OBLIQUE   110

/* ========================================================================= */
/* Width constants                                                           */
/* ========================================================================= */

#define FC_WIDTH_ULTRACONDENSED    50
#define FC_WIDTH_EXTRACONDENSED    63
#define FC_WIDTH_CONDENSED         75
#define FC_WIDTH_SEMICONDENSED     87
#define FC_WIDTH_NORMAL            100
#define FC_WIDTH_SEMIEXPANDED      113
#define FC_WIDTH_EXPANDED          125
#define FC_WIDTH_EXTRAEXPANDED     150
#define FC_WIDTH_ULTRAEXPANDED     200

/* ========================================================================= */
/* Spacing constants                                                         */
/* ========================================================================= */

#define FC_PROPORTIONAL    0
#define FC_DUAL            90
#define FC_MONO            100
#define FC_CHARCELL        110

/* ========================================================================= */
/* Hint style constants                                                      */
/* ========================================================================= */

#define FC_HINT_NONE       0
#define FC_HINT_SLIGHT     1
#define FC_HINT_MEDIUM     2
#define FC_HINT_FULL       3

/* ========================================================================= */
/* RGBA constants                                                            */
/* ========================================================================= */

#define FC_RGBA_UNKNOWN    0
#define FC_RGBA_RGB        1
#define FC_RGBA_BGR        2
#define FC_RGBA_VRGB       3
#define FC_RGBA_VBGR       4
#define FC_RGBA_NONE       5

/* ========================================================================= */
/* LCD filter constants                                                      */
/* ========================================================================= */

#define FC_LCD_NONE        0
#define FC_LCD_DEFAULT     1
#define FC_LCD_LIGHT       2
#define FC_LCD_LEGACY      3

/* ========================================================================= */
/* LangResult                                                                */
/* ========================================================================= */

typedef enum _FcLangResult {
    FcLangEqual           = 0,
    FcLangDifferentCountry = 1,
    FcLangDifferentTerritory = 1,
    FcLangDifferentLang   = 2
} FcLangResult;

/* ========================================================================= */
/* Matrix                                                                    */
/* ========================================================================= */

typedef struct _FcMatrix {
    double  xx, xy, yx, yy;
} FcMatrix;

#define FcMatrixInit(m) ((m)->xx = (m)->yy = 1, (m)->xy = (m)->yx = 0)

/* ========================================================================= */
/* Init / Config API                                                         */
/* ========================================================================= */

FcBool      FcInit(void);
void        FcFini(void);
int         FcGetVersion(void);

FcConfig   *FcConfigGetCurrent(void);
FcBool      FcConfigUptoDate(FcConfig *config);
FcBool      FcConfigBuildFonts(FcConfig *config);
FcFontSet  *FcConfigGetFonts(FcConfig *config, FcSetName set);
FcBool      FcConfigAppFontAddFile(FcConfig *config, const FcChar8 *file);
FcBool      FcConfigAppFontAddDir(FcConfig *config, const FcChar8 *dir);
void        FcConfigAppFontClear(FcConfig *config);
FcBool      FcConfigSubstitute(FcConfig *config, FcPattern *p,
                                FcMatchKind kind);

/* ========================================================================= */
/* Pattern API                                                               */
/* ========================================================================= */

FcPattern  *FcPatternCreate(void);
void        FcPatternDestroy(FcPattern *p);
FcPattern  *FcPatternDuplicate(const FcPattern *p);

FcBool      FcPatternAddString(FcPattern *p, const char *object,
                                const FcChar8 *s);
FcBool      FcPatternAddInteger(FcPattern *p, const char *object, int i);
FcBool      FcPatternAddDouble(FcPattern *p, const char *object, double d);
FcBool      FcPatternAddBool(FcPattern *p, const char *object, FcBool b);
FcBool      FcPatternAddCharSet(FcPattern *p, const char *object,
                                 const FcCharSet *c);
FcBool      FcPatternAddLangSet(FcPattern *p, const char *object,
                                 const FcLangSet *ls);

FcResult    FcPatternGetString(const FcPattern *p, const char *object,
                                int n, FcChar8 **s);
FcResult    FcPatternGetInteger(const FcPattern *p, const char *object,
                                 int n, int *i);
FcResult    FcPatternGetDouble(const FcPattern *p, const char *object,
                                int n, double *d);
FcResult    FcPatternGetBool(const FcPattern *p, const char *object,
                              int n, FcBool *b);
FcResult    FcPatternGetCharSet(const FcPattern *p, const char *object,
                                 int n, FcCharSet **c);
FcResult    FcPatternGetLangSet(const FcPattern *p, const char *object,
                                 int n, FcLangSet **ls);

FcBool      FcPatternDel(FcPattern *p, const char *object);
FcBool      FcPatternRemove(FcPattern *p, const char *object, int i);

FcPattern  *FcNameParse(const FcChar8 *name);
FcChar8    *FcNameUnparse(FcPattern *pat);

void        FcPatternPrint(const FcPattern *p);
FcBool      FcPatternEqual(const FcPattern *pa, const FcPattern *pb);
FcBool      FcPatternEqualSubset(const FcPattern *pa, const FcPattern *pb,
                                   const FcObjectSet *os);
FcChar32    FcPatternHash(const FcPattern *p);

/* ========================================================================= */
/* Default substitute                                                        */
/* ========================================================================= */

void        FcDefaultSubstitute(FcPattern *pattern);

/* ========================================================================= */
/* Font matching                                                             */
/* ========================================================================= */

FcPattern  *FcFontMatch(FcConfig *config, FcPattern *p, FcResult *result);
FcFontSet  *FcFontSort(FcConfig *config, FcPattern *p, FcBool trim,
                         FcCharSet **csp, FcResult *result);
FcFontSet  *FcFontList(FcConfig *config, FcPattern *p, FcObjectSet *os);

/* ========================================================================= */
/* FcFontSet API                                                             */
/* ========================================================================= */

FcFontSet  *FcFontSetCreate(void);
void        FcFontSetDestroy(FcFontSet *fs);
FcBool      FcFontSetAdd(FcFontSet *fs, FcPattern *font);

/* ========================================================================= */
/* ObjectSet API                                                             */
/* ========================================================================= */

FcObjectSet *FcObjectSetCreate(void);
FcBool       FcObjectSetAdd(FcObjectSet *os, const char *object);
void         FcObjectSetDestroy(FcObjectSet *os);

FcObjectSet *FcObjectSetBuild(const char *first, ...);

/* ========================================================================= */
/* LangSet API                                                               */
/* ========================================================================= */

FcLangSet  *FcLangSetCreate(void);
void        FcLangSetDestroy(FcLangSet *ls);
FcLangSet  *FcLangSetCopy(const FcLangSet *ls);
FcBool      FcLangSetAdd(FcLangSet *ls, const FcChar8 *lang);
FcBool      FcLangSetDel(FcLangSet *ls, const FcChar8 *lang);
FcLangResult FcLangSetHasLang(const FcLangSet *ls, const FcChar8 *lang);
FcBool      FcLangSetEqual(const FcLangSet *lsa, const FcLangSet *lsb);
FcChar32    FcLangSetHash(const FcLangSet *ls);
FcStrSet   *FcLangSetGetLangs(const FcLangSet *ls);
FcLangSet  *FcLangSetUnion(const FcLangSet *a, const FcLangSet *b);
FcLangSet  *FcLangSetSubtract(const FcLangSet *a, const FcLangSet *b);

/* ========================================================================= */
/* CharSet API                                                               */
/* ========================================================================= */

FcCharSet  *FcCharSetCreate(void);
void        FcCharSetDestroy(FcCharSet *fcs);
FcBool      FcCharSetAddChar(FcCharSet *fcs, FcChar32 ucs4);
FcBool      FcCharSetDelChar(FcCharSet *fcs, FcChar32 ucs4);
FcCharSet  *FcCharSetCopy(FcCharSet *src);
FcBool      FcCharSetEqual(const FcCharSet *a, const FcCharSet *b);
FcCharSet  *FcCharSetIntersect(const FcCharSet *a, const FcCharSet *b);
FcCharSet  *FcCharSetUnion(const FcCharSet *a, const FcCharSet *b);
FcCharSet  *FcCharSetSubtract(const FcCharSet *a, const FcCharSet *b);
FcBool      FcCharSetHasChar(const FcCharSet *fcs, FcChar32 ucs4);
FcChar32    FcCharSetCount(const FcCharSet *a);
FcChar32    FcCharSetIntersectCount(const FcCharSet *a, const FcCharSet *b);
FcBool      FcCharSetIsSubset(const FcCharSet *a, const FcCharSet *b);
FcChar32    FcCharSetFirstPage(const FcCharSet *a, FcChar32 map[8],
                                FcChar32 *next);
FcChar32    FcCharSetNextPage(const FcCharSet *a, FcChar32 map[8],
                               FcChar32 *next);

/* ========================================================================= */
/* String utilities                                                          */
/* ========================================================================= */

FcChar8    *FcStrCopy(const FcChar8 *s);
FcChar8    *FcStrCopyFilename(const FcChar8 *s);
int         FcStrCmpIgnoreCase(const FcChar8 *s1, const FcChar8 *s2);
int         FcStrCmp(const FcChar8 *s1, const FcChar8 *s2);
FcChar8    *FcStrDowncase(const FcChar8 *s);
int         FcUtf8ToUcs4(const FcChar8 *src, FcChar32 *dst, int len);
int         FcUtf8Len(const FcChar8 *src, int len, int *nchar, int *wchar);

FcStrSet   *FcStrSetCreate(void);
FcBool      FcStrSetMember(FcStrSet *set, const FcChar8 *s);
FcBool      FcStrSetEqual(FcStrSet *sa, FcStrSet *sb);
FcBool      FcStrSetAdd(FcStrSet *set, const FcChar8 *s);
FcBool      FcStrSetAddFilename(FcStrSet *set, const FcChar8 *s);
FcBool      FcStrSetDel(FcStrSet *set, const FcChar8 *s);
void        FcStrSetDestroy(FcStrSet *set);
FcStrList  *FcStrListCreate(FcStrSet *set);
void        FcStrListFirst(FcStrList *list);
FcChar8    *FcStrListNext(FcStrList *list);
void        FcStrListDone(FcStrList *list);

/* ========================================================================= */
/* Blanks                                                                    */
/* ========================================================================= */

FcBlanks   *FcBlanksCreate(void);
void        FcBlanksDestroy(FcBlanks *b);
FcBool      FcBlanksAdd(FcBlanks *b, FcChar32 ucs4);
FcBool      FcBlanksIsMember(FcBlanks *b, FcChar32 ucs4);

/* ========================================================================= */
/* Cache                                                                     */
/* ========================================================================= */

FcBool      FcDirCacheValid(const FcChar8 *dir);

/* ========================================================================= */
/* Weight conversion (Fontconfig <-> OS/2)                                   */
/* ========================================================================= */

double      FcWeightFromOpenType(int ot_weight);
double      FcWeightFromOpenTypeDouble(double ot_weight);
int         FcWeightToOpenType(int fc_weight);
double      FcWeightToOpenTypeDouble(double fc_weight);

#ifdef __cplusplus
}
#endif

#endif /* _FONTCONFIG_FONTCONFIG_H */
