/*
 * VeridianOS libc -- fontconfig.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Fontconfig 2.15.0 compatible implementation.
 * Provides font configuration, pattern matching with default fallback
 * fonts (DejaVu Sans / Noto Sans), object set management, charset
 * and langset support, and FreeType integration stubs.
 */

#include <fontconfig/fontconfig.h>
#include <fontconfig/fcfreetype.h>
#include <string.h>
#include <stdlib.h>
#include <stdarg.h>

/* ========================================================================= */
/* Internal limits                                                           */
/* ========================================================================= */

#define MAX_PATTERNS     256
#define MAX_PATTERN_PROPS 32
#define MAX_FONTSETS      16
#define MAX_OBJECTSETS    16
#define MAX_LANGSETS      32
#define MAX_CHARSETS      32
#define MAX_OBJSET_ITEMS  16

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct fc_prop {
    char        name[32];
    FcType      type;
    union {
        char    str[256];
        int     ival;
        double  dval;
        FcBool  bval;
    } val;
};

struct fc_pattern_internal {
    int              in_use;
    int              ref_count;
    struct fc_prop   props[MAX_PATTERN_PROPS];
    int              nprops;
};

struct fc_fontset_internal {
    int              in_use;
    FcPattern       *fonts[64];
    int              nfont;
    int              sfont;
};

struct fc_objectset_internal {
    int              in_use;
    char             objects[MAX_OBJSET_ITEMS][32];
    int              nobjects;
};

struct fc_langset_internal {
    int              in_use;
    int              ref_count;
    char             langs[16][32];
    int              nlangs;
};

struct fc_charset_internal {
    int              in_use;
    int              ref_count;
    /* Bitmap for codepoints 0-1023 */
    uint32_t         bits[32];
    unsigned int     count;
};

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static int                         g_fc_initialized = 0;
static struct fc_pattern_internal  g_patterns[MAX_PATTERNS];
static struct fc_fontset_internal  g_fontsets[MAX_FONTSETS];
static struct fc_objectset_internal g_objectsets[MAX_OBJECTSETS];
static struct fc_langset_internal  g_langsets[MAX_LANGSETS];
static struct fc_charset_internal  g_charsets[MAX_CHARSETS];

/* Singleton config */
static struct { int initialized; } g_config;

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static struct fc_pattern_internal *alloc_pattern(void)
{
    int i;
    for (i = 0; i < MAX_PATTERNS; i++) {
        if (!g_patterns[i].in_use) {
            memset(&g_patterns[i], 0, sizeof(g_patterns[i]));
            g_patterns[i].in_use    = 1;
            g_patterns[i].ref_count = 1;
            return &g_patterns[i];
        }
    }
    return NULL;
}

static struct fc_prop *find_prop(const struct fc_pattern_internal *pi,
                                  const char *name, int n)
{
    int i, idx = 0;
    for (i = 0; i < pi->nprops; i++) {
        if (strcmp(pi->props[i].name, name) == 0) {
            if (idx == n)
                return (struct fc_prop *)&pi->props[i];
            idx++;
        }
    }
    return NULL;
}

static struct fc_prop *add_prop(struct fc_pattern_internal *pi,
                                 const char *name, FcType type)
{
    if (pi->nprops >= MAX_PATTERN_PROPS)
        return NULL;

    struct fc_prop *p = &pi->props[pi->nprops++];
    strncpy(p->name, name, sizeof(p->name) - 1);
    p->name[sizeof(p->name) - 1] = '\0';
    p->type = type;
    return p;
}

/* ========================================================================= */
/* Init / Config                                                             */
/* ========================================================================= */

FcBool FcInit(void)
{
    g_fc_initialized    = 1;
    g_config.initialized = 1;
    return FcTrue;
}

void FcFini(void)
{
    g_fc_initialized    = 0;
    g_config.initialized = 0;
}

int FcGetVersion(void)
{
    return FC_VERSION;
}

FcConfig *FcConfigGetCurrent(void)
{
    if (!g_fc_initialized)
        FcInit();
    return (FcConfig *)&g_config;
}

FcBool FcConfigUptoDate(FcConfig *config)
{
    (void)config;
    return FcTrue;
}

FcBool FcConfigBuildFonts(FcConfig *config)
{
    (void)config;
    return FcTrue;
}

FcFontSet *FcConfigGetFonts(FcConfig *config, FcSetName set)
{
    (void)config;
    (void)set;
    return NULL;
}

FcBool FcConfigAppFontAddFile(FcConfig *config, const FcChar8 *file)
{
    (void)config;
    (void)file;
    return FcTrue;
}

FcBool FcConfigAppFontAddDir(FcConfig *config, const FcChar8 *dir)
{
    (void)config;
    (void)dir;
    return FcTrue;
}

void FcConfigAppFontClear(FcConfig *config)
{
    (void)config;
}

FcBool FcConfigSubstitute(FcConfig *config, FcPattern *p,
                            FcMatchKind kind)
{
    (void)config;
    (void)p;
    (void)kind;
    return FcTrue;
}

/* ========================================================================= */
/* Pattern                                                                   */
/* ========================================================================= */

FcPattern *FcPatternCreate(void)
{
    struct fc_pattern_internal *pi = alloc_pattern();
    return pi ? (FcPattern *)pi : NULL;
}

void FcPatternDestroy(FcPattern *p)
{
    struct fc_pattern_internal *pi = (struct fc_pattern_internal *)p;
    if (!pi) return;
    pi->ref_count--;
    if (pi->ref_count <= 0)
        pi->in_use = 0;
}

FcPattern *FcPatternDuplicate(const FcPattern *p)
{
    const struct fc_pattern_internal *src = (const struct fc_pattern_internal *)p;
    struct fc_pattern_internal *dst;

    if (!src)
        return FcPatternCreate();

    dst = alloc_pattern();
    if (!dst)
        return NULL;

    memcpy(dst->props, src->props, sizeof(src->props));
    dst->nprops = src->nprops;
    return (FcPattern *)dst;
}

FcBool FcPatternAddString(FcPattern *p, const char *object,
                            const FcChar8 *s)
{
    struct fc_pattern_internal *pi = (struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !object)
        return FcFalse;

    prop = add_prop(pi, object, FcTypeString);
    if (!prop)
        return FcFalse;

    if (s)
        strncpy(prop->val.str, (const char *)s, sizeof(prop->val.str) - 1);
    else
        prop->val.str[0] = '\0';

    return FcTrue;
}

FcBool FcPatternAddInteger(FcPattern *p, const char *object, int i)
{
    struct fc_pattern_internal *pi = (struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !object)
        return FcFalse;

    prop = add_prop(pi, object, FcTypeInteger);
    if (!prop)
        return FcFalse;

    prop->val.ival = i;
    return FcTrue;
}

FcBool FcPatternAddDouble(FcPattern *p, const char *object, double d)
{
    struct fc_pattern_internal *pi = (struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !object)
        return FcFalse;

    prop = add_prop(pi, object, FcTypeDouble);
    if (!prop)
        return FcFalse;

    prop->val.dval = d;
    return FcTrue;
}

FcBool FcPatternAddBool(FcPattern *p, const char *object, FcBool b)
{
    struct fc_pattern_internal *pi = (struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !object)
        return FcFalse;

    prop = add_prop(pi, object, FcTypeBool);
    if (!prop)
        return FcFalse;

    prop->val.bval = b;
    return FcTrue;
}

FcBool FcPatternAddCharSet(FcPattern *p, const char *object,
                             const FcCharSet *c)
{
    (void)p;
    (void)object;
    (void)c;
    return FcTrue;
}

FcBool FcPatternAddLangSet(FcPattern *p, const char *object,
                             const FcLangSet *ls)
{
    (void)p;
    (void)object;
    (void)ls;
    return FcTrue;
}

FcResult FcPatternGetString(const FcPattern *p, const char *object,
                              int n, FcChar8 **s)
{
    const struct fc_pattern_internal *pi = (const struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !s)
        return FcResultNoMatch;

    prop = find_prop(pi, object, n);
    if (!prop || prop->type != FcTypeString)
        return FcResultNoMatch;

    *s = (FcChar8 *)prop->val.str;
    return FcResultMatch;
}

FcResult FcPatternGetInteger(const FcPattern *p, const char *object,
                               int n, int *i)
{
    const struct fc_pattern_internal *pi = (const struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !i)
        return FcResultNoMatch;

    prop = find_prop(pi, object, n);
    if (!prop)
        return FcResultNoMatch;

    if (prop->type == FcTypeInteger) {
        *i = prop->val.ival;
        return FcResultMatch;
    }
    if (prop->type == FcTypeDouble) {
        *i = (int)prop->val.dval;
        return FcResultMatch;
    }
    return FcResultTypeMismatch;
}

FcResult FcPatternGetDouble(const FcPattern *p, const char *object,
                              int n, double *d)
{
    const struct fc_pattern_internal *pi = (const struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !d)
        return FcResultNoMatch;

    prop = find_prop(pi, object, n);
    if (!prop)
        return FcResultNoMatch;

    if (prop->type == FcTypeDouble) {
        *d = prop->val.dval;
        return FcResultMatch;
    }
    if (prop->type == FcTypeInteger) {
        *d = (double)prop->val.ival;
        return FcResultMatch;
    }
    return FcResultTypeMismatch;
}

FcResult FcPatternGetBool(const FcPattern *p, const char *object,
                            int n, FcBool *b)
{
    const struct fc_pattern_internal *pi = (const struct fc_pattern_internal *)p;
    struct fc_prop *prop;

    if (!pi || !b)
        return FcResultNoMatch;

    prop = find_prop(pi, object, n);
    if (!prop || prop->type != FcTypeBool)
        return FcResultNoMatch;

    *b = prop->val.bval;
    return FcResultMatch;
}

FcResult FcPatternGetCharSet(const FcPattern *p, const char *object,
                               int n, FcCharSet **c)
{
    (void)p;
    (void)object;
    (void)n;
    (void)c;
    return FcResultNoMatch;
}

FcResult FcPatternGetLangSet(const FcPattern *p, const char *object,
                               int n, FcLangSet **ls)
{
    (void)p;
    (void)object;
    (void)n;
    (void)ls;
    return FcResultNoMatch;
}

FcBool FcPatternDel(FcPattern *p, const char *object)
{
    (void)p;
    (void)object;
    return FcTrue;
}

FcBool FcPatternRemove(FcPattern *p, const char *object, int i)
{
    (void)p;
    (void)object;
    (void)i;
    return FcTrue;
}

FcPattern *FcNameParse(const FcChar8 *name)
{
    FcPattern *pat = FcPatternCreate();
    if (pat && name)
        FcPatternAddString(pat, FC_FAMILY, name);
    return pat;
}

FcChar8 *FcNameUnparse(FcPattern *pat)
{
    FcChar8 *s = NULL;
    FcResult result;

    if (!pat)
        return (FcChar8 *)strdup("sans-serif");

    result = FcPatternGetString(pat, FC_FAMILY, 0, &s);
    if (result == FcResultMatch && s)
        return (FcChar8 *)strdup((const char *)s);

    return (FcChar8 *)strdup("sans-serif");
}

void FcPatternPrint(const FcPattern *p) { (void)p; }

FcBool FcPatternEqual(const FcPattern *pa, const FcPattern *pb)
{
    (void)pa;
    (void)pb;
    return FcFalse;
}

FcBool FcPatternEqualSubset(const FcPattern *pa, const FcPattern *pb,
                               const FcObjectSet *os)
{
    (void)pa;
    (void)pb;
    (void)os;
    return FcFalse;
}

FcChar32 FcPatternHash(const FcPattern *p)
{
    (void)p;
    return 0;
}

/* ========================================================================= */
/* Default substitute                                                        */
/* ========================================================================= */

void FcDefaultSubstitute(FcPattern *pattern)
{
    struct fc_pattern_internal *pi = (struct fc_pattern_internal *)pattern;
    FcChar8 *s;
    int ival;
    double dval;
    FcBool bval;

    if (!pi) return;

    /* Add defaults for missing properties */
    if (FcPatternGetString(pattern, FC_FAMILY, 0, &s) != FcResultMatch)
        FcPatternAddString(pattern, FC_FAMILY, (const FcChar8 *)"sans-serif");

    if (FcPatternGetInteger(pattern, FC_SLANT, 0, &ival) != FcResultMatch)
        FcPatternAddInteger(pattern, FC_SLANT, FC_SLANT_ROMAN);

    if (FcPatternGetInteger(pattern, FC_WEIGHT, 0, &ival) != FcResultMatch)
        FcPatternAddInteger(pattern, FC_WEIGHT, FC_WEIGHT_REGULAR);

    if (FcPatternGetDouble(pattern, FC_SIZE, 0, &dval) != FcResultMatch)
        FcPatternAddDouble(pattern, FC_SIZE, 12.0);

    if (FcPatternGetDouble(pattern, FC_PIXEL_SIZE, 0, &dval) != FcResultMatch)
        FcPatternAddDouble(pattern, FC_PIXEL_SIZE, 16.0);

    if (FcPatternGetBool(pattern, FC_ANTIALIAS, 0, &bval) != FcResultMatch)
        FcPatternAddBool(pattern, FC_ANTIALIAS, FcTrue);

    if (FcPatternGetBool(pattern, FC_HINTING, 0, &bval) != FcResultMatch)
        FcPatternAddBool(pattern, FC_HINTING, FcTrue);

    if (FcPatternGetInteger(pattern, FC_HINT_STYLE, 0, &ival) != FcResultMatch)
        FcPatternAddInteger(pattern, FC_HINT_STYLE, FC_HINT_SLIGHT);

    if (FcPatternGetInteger(pattern, FC_RGBA, 0, &ival) != FcResultMatch)
        FcPatternAddInteger(pattern, FC_RGBA, FC_RGBA_UNKNOWN);
}

/* ========================================================================= */
/* Font matching                                                             */
/* ========================================================================= */

FcPattern *FcFontMatch(FcConfig *config, FcPattern *p, FcResult *result)
{
    FcPattern *match;
    FcChar8 *family = NULL;

    (void)config;

    match = FcPatternDuplicate(p);
    if (!match) {
        if (result) *result = FcResultOutOfMemory;
        return NULL;
    }

    /* Resolve family name to a default font file */
    if (FcPatternGetString(p, FC_FAMILY, 0, &family) != FcResultMatch)
        family = NULL;

    /* Add file path based on requested family */
    if (family) {
        if (strstr((const char *)family, "Mono") ||
            strstr((const char *)family, "mono") ||
            strstr((const char *)family, "Courier")) {
            FcPatternAddString(match, FC_FILE,
                (const FcChar8 *)"/usr/share/fonts/dejavu/DejaVuSansMono.ttf");
            FcPatternAddString(match, FC_FAMILY,
                (const FcChar8 *)"DejaVu Sans Mono");
        } else if (strstr((const char *)family, "Serif") ||
                   strstr((const char *)family, "serif") ||
                   strstr((const char *)family, "Times")) {
            FcPatternAddString(match, FC_FILE,
                (const FcChar8 *)"/usr/share/fonts/dejavu/DejaVuSerif.ttf");
            FcPatternAddString(match, FC_FAMILY,
                (const FcChar8 *)"DejaVu Serif");
        } else {
            FcPatternAddString(match, FC_FILE,
                (const FcChar8 *)"/usr/share/fonts/dejavu/DejaVuSans.ttf");
            FcPatternAddString(match, FC_FAMILY,
                (const FcChar8 *)"DejaVu Sans");
        }
    } else {
        FcPatternAddString(match, FC_FILE,
            (const FcChar8 *)"/usr/share/fonts/dejavu/DejaVuSans.ttf");
        FcPatternAddString(match, FC_FAMILY,
            (const FcChar8 *)"DejaVu Sans");
    }

    FcPatternAddString(match, FC_STYLE, (const FcChar8 *)"Regular");
    FcPatternAddInteger(match, FC_INDEX, 0);
    FcPatternAddBool(match, FC_SCALABLE, FcTrue);
    FcPatternAddBool(match, FC_OUTLINE, FcTrue);
    FcPatternAddString(match, FC_FONTFORMAT,
        (const FcChar8 *)"TrueType");

    if (result) *result = FcResultMatch;
    return match;
}

FcFontSet *FcFontSort(FcConfig *config, FcPattern *p, FcBool trim,
                         FcCharSet **csp, FcResult *result)
{
    FcFontSet *fs;
    FcPattern *match;

    (void)config;
    (void)trim;
    if (csp) *csp = NULL;

    fs = FcFontSetCreate();
    if (!fs) {
        if (result) *result = FcResultOutOfMemory;
        return NULL;
    }

    match = FcFontMatch(NULL, p, result);
    if (match)
        FcFontSetAdd(fs, match);

    return fs;
}

FcFontSet *FcFontList(FcConfig *config, FcPattern *p, FcObjectSet *os)
{
    FcFontSet *fs;
    FcResult result;
    FcPattern *match;

    (void)config;
    (void)os;

    fs = FcFontSetCreate();
    if (!fs) return NULL;

    match = FcFontMatch(NULL, p, &result);
    if (match)
        FcFontSetAdd(fs, match);

    return fs;
}

/* ========================================================================= */
/* FcFontSet                                                                 */
/* ========================================================================= */

FcFontSet *FcFontSetCreate(void)
{
    int i;
    for (i = 0; i < MAX_FONTSETS; i++) {
        if (!g_fontsets[i].in_use) {
            memset(&g_fontsets[i], 0, sizeof(g_fontsets[i]));
            g_fontsets[i].in_use = 1;
            g_fontsets[i].sfont  = 64;

            /* Return a pointer to a FcFontSet-compatible struct.
             * The first 3 fields match FcFontSet layout. */
            return (FcFontSet *)&g_fontsets[i].nfont;
        }
    }
    return NULL;
}

void FcFontSetDestroy(FcFontSet *fs)
{
    int i;

    if (!fs) return;

    /* Free patterns in set */
    for (i = 0; i < fs->nfont; i++) {
        if (fs->fonts[i])
            FcPatternDestroy(fs->fonts[i]);
    }

    /* Find internal and release */
    struct fc_fontset_internal *fi =
        (struct fc_fontset_internal *)((char *)fs - offsetof(struct fc_fontset_internal, nfont));
    fi->in_use = 0;
}

FcBool FcFontSetAdd(FcFontSet *fs, FcPattern *font)
{
    if (!fs || !font || fs->nfont >= fs->sfont)
        return FcFalse;

    fs->fonts[fs->nfont++] = font;
    return FcTrue;
}

/* ========================================================================= */
/* ObjectSet                                                                 */
/* ========================================================================= */

FcObjectSet *FcObjectSetCreate(void)
{
    int i;
    for (i = 0; i < MAX_OBJECTSETS; i++) {
        if (!g_objectsets[i].in_use) {
            memset(&g_objectsets[i], 0, sizeof(g_objectsets[i]));
            g_objectsets[i].in_use = 1;
            return (FcObjectSet *)&g_objectsets[i];
        }
    }
    return NULL;
}

FcBool FcObjectSetAdd(FcObjectSet *os, const char *object)
{
    struct fc_objectset_internal *oi = (struct fc_objectset_internal *)os;

    if (!oi || oi->nobjects >= MAX_OBJSET_ITEMS)
        return FcFalse;

    strncpy(oi->objects[oi->nobjects], object,
            sizeof(oi->objects[0]) - 1);
    oi->nobjects++;
    return FcTrue;
}

void FcObjectSetDestroy(FcObjectSet *os)
{
    struct fc_objectset_internal *oi = (struct fc_objectset_internal *)os;
    if (oi) oi->in_use = 0;
}

FcObjectSet *FcObjectSetBuild(const char *first, ...)
{
    FcObjectSet *os = FcObjectSetCreate();
    va_list ap;
    const char *s;

    if (!os) return NULL;

    FcObjectSetAdd(os, first);

    va_start(ap, first);
    while ((s = va_arg(ap, const char *)) != NULL)
        FcObjectSetAdd(os, s);
    va_end(ap);

    return os;
}

/* ========================================================================= */
/* LangSet                                                                   */
/* ========================================================================= */

FcLangSet *FcLangSetCreate(void)
{
    int i;
    for (i = 0; i < MAX_LANGSETS; i++) {
        if (!g_langsets[i].in_use) {
            memset(&g_langsets[i], 0, sizeof(g_langsets[i]));
            g_langsets[i].in_use = 1;
            g_langsets[i].ref_count = 1;
            return (FcLangSet *)&g_langsets[i];
        }
    }
    return NULL;
}

void FcLangSetDestroy(FcLangSet *ls)
{
    struct fc_langset_internal *li = (struct fc_langset_internal *)ls;
    if (!li) return;
    if (--li->ref_count <= 0)
        li->in_use = 0;
}

FcLangSet *FcLangSetCopy(const FcLangSet *ls)
{
    const struct fc_langset_internal *src = (const struct fc_langset_internal *)ls;
    FcLangSet *dst = FcLangSetCreate();
    struct fc_langset_internal *di = (struct fc_langset_internal *)dst;

    if (di && src) {
        memcpy(di->langs, src->langs, sizeof(src->langs));
        di->nlangs = src->nlangs;
    }
    return dst;
}

FcBool FcLangSetAdd(FcLangSet *ls, const FcChar8 *lang)
{
    struct fc_langset_internal *li = (struct fc_langset_internal *)ls;

    if (!li || li->nlangs >= 16)
        return FcFalse;

    strncpy(li->langs[li->nlangs], (const char *)lang,
            sizeof(li->langs[0]) - 1);
    li->nlangs++;
    return FcTrue;
}

FcBool FcLangSetDel(FcLangSet *ls, const FcChar8 *lang)
{
    (void)ls;
    (void)lang;
    return FcTrue;
}

FcLangResult FcLangSetHasLang(const FcLangSet *ls, const FcChar8 *lang)
{
    const struct fc_langset_internal *li = (const struct fc_langset_internal *)ls;
    int i;

    if (!li || !lang)
        return FcLangDifferentLang;

    for (i = 0; i < li->nlangs; i++) {
        if (strcmp(li->langs[i], (const char *)lang) == 0)
            return FcLangEqual;
    }
    return FcLangDifferentLang;
}

FcBool FcLangSetEqual(const FcLangSet *lsa, const FcLangSet *lsb)
{
    (void)lsa;
    (void)lsb;
    return FcFalse;
}

FcChar32 FcLangSetHash(const FcLangSet *ls)
{
    (void)ls;
    return 0;
}

FcStrSet *FcLangSetGetLangs(const FcLangSet *ls) { (void)ls; return NULL; }
FcLangSet *FcLangSetUnion(const FcLangSet *a, const FcLangSet *b) {
    (void)a; (void)b; return FcLangSetCreate();
}
FcLangSet *FcLangSetSubtract(const FcLangSet *a, const FcLangSet *b) {
    (void)a; (void)b; return FcLangSetCreate();
}

/* ========================================================================= */
/* CharSet                                                                   */
/* ========================================================================= */

FcCharSet *FcCharSetCreate(void)
{
    int i;
    for (i = 0; i < MAX_CHARSETS; i++) {
        if (!g_charsets[i].in_use) {
            memset(&g_charsets[i], 0, sizeof(g_charsets[i]));
            g_charsets[i].in_use = 1;
            g_charsets[i].ref_count = 1;
            return (FcCharSet *)&g_charsets[i];
        }
    }
    return NULL;
}

void FcCharSetDestroy(FcCharSet *fcs)
{
    struct fc_charset_internal *ci = (struct fc_charset_internal *)fcs;
    if (!ci) return;
    if (--ci->ref_count <= 0)
        ci->in_use = 0;
}

FcBool FcCharSetAddChar(FcCharSet *fcs, FcChar32 ucs4)
{
    struct fc_charset_internal *ci = (struct fc_charset_internal *)fcs;
    if (!ci || ucs4 >= 1024) return FcFalse;
    if (!((ci->bits[ucs4 / 32] >> (ucs4 % 32)) & 1)) {
        ci->bits[ucs4 / 32] |= (1u << (ucs4 % 32));
        ci->count++;
    }
    return FcTrue;
}

FcBool FcCharSetDelChar(FcCharSet *fcs, FcChar32 ucs4)
{
    struct fc_charset_internal *ci = (struct fc_charset_internal *)fcs;
    if (!ci || ucs4 >= 1024) return FcFalse;
    if ((ci->bits[ucs4 / 32] >> (ucs4 % 32)) & 1) {
        ci->bits[ucs4 / 32] &= ~(1u << (ucs4 % 32));
        ci->count--;
    }
    return FcTrue;
}

FcCharSet *FcCharSetCopy(FcCharSet *src)
{
    struct fc_charset_internal *si = (struct fc_charset_internal *)src;
    if (si) si->ref_count++;
    return src;
}

FcBool FcCharSetEqual(const FcCharSet *a, const FcCharSet *b)
{
    (void)a; (void)b;
    return FcFalse;
}

FcCharSet *FcCharSetIntersect(const FcCharSet *a, const FcCharSet *b)
{
    (void)a; (void)b;
    return FcCharSetCreate();
}

FcCharSet *FcCharSetUnion(const FcCharSet *a, const FcCharSet *b)
{
    (void)a; (void)b;
    return FcCharSetCreate();
}

FcCharSet *FcCharSetSubtract(const FcCharSet *a, const FcCharSet *b)
{
    (void)a; (void)b;
    return FcCharSetCreate();
}

FcBool FcCharSetHasChar(const FcCharSet *fcs, FcChar32 ucs4)
{
    const struct fc_charset_internal *ci = (const struct fc_charset_internal *)fcs;
    if (!ci || ucs4 >= 1024) return FcFalse;
    return (ci->bits[ucs4 / 32] >> (ucs4 % 32)) & 1;
}

FcChar32 FcCharSetCount(const FcCharSet *a)
{
    const struct fc_charset_internal *ci = (const struct fc_charset_internal *)a;
    return ci ? ci->count : 0;
}

FcChar32 FcCharSetIntersectCount(const FcCharSet *a, const FcCharSet *b)
{
    (void)a; (void)b;
    return 0;
}

FcBool FcCharSetIsSubset(const FcCharSet *a, const FcCharSet *b)
{
    (void)a; (void)b;
    return FcFalse;
}

FcChar32 FcCharSetFirstPage(const FcCharSet *a, FcChar32 map[8],
                              FcChar32 *next)
{
    (void)a;
    memset(map, 0, sizeof(FcChar32) * 8);
    if (next) *next = 0;
    return 0;
}

FcChar32 FcCharSetNextPage(const FcCharSet *a, FcChar32 map[8],
                             FcChar32 *next)
{
    (void)a;
    memset(map, 0, sizeof(FcChar32) * 8);
    if (next) *next = 0;
    return (FcChar32)-1;  /* FC_CHARSET_DONE */
}

/* ========================================================================= */
/* String utilities                                                          */
/* ========================================================================= */

FcChar8 *FcStrCopy(const FcChar8 *s)
{
    if (!s) return NULL;
    return (FcChar8 *)strdup((const char *)s);
}

FcChar8 *FcStrCopyFilename(const FcChar8 *s)
{
    return FcStrCopy(s);
}

int FcStrCmpIgnoreCase(const FcChar8 *s1, const FcChar8 *s2)
{
    if (!s1 || !s2) return s1 ? 1 : (s2 ? -1 : 0);

    while (*s1 && *s2) {
        unsigned char c1 = *s1, c2 = *s2;
        if (c1 >= 'A' && c1 <= 'Z') c1 += 32;
        if (c2 >= 'A' && c2 <= 'Z') c2 += 32;
        if (c1 != c2) return c1 - c2;
        s1++;
        s2++;
    }
    return *s1 - *s2;
}

int FcStrCmp(const FcChar8 *s1, const FcChar8 *s2)
{
    return strcmp((const char *)s1, (const char *)s2);
}

FcChar8 *FcStrDowncase(const FcChar8 *s)
{
    FcChar8 *r, *p;
    if (!s) return NULL;
    r = FcStrCopy(s);
    if (!r) return NULL;
    for (p = r; *p; p++)
        if (*p >= 'A' && *p <= 'Z') *p += 32;
    return r;
}

int FcUtf8ToUcs4(const FcChar8 *src, FcChar32 *dst, int len)
{
    if (!src || len <= 0) return 0;

    unsigned char c = src[0];
    if (c < 0x80) {
        *dst = c;
        return 1;
    }
    if ((c & 0xE0) == 0xC0 && len >= 2) {
        *dst = ((FcChar32)(c & 0x1F) << 6) | (src[1] & 0x3F);
        return 2;
    }
    if ((c & 0xF0) == 0xE0 && len >= 3) {
        *dst = ((FcChar32)(c & 0x0F) << 12) |
               ((FcChar32)(src[1] & 0x3F) << 6) | (src[2] & 0x3F);
        return 3;
    }
    if ((c & 0xF8) == 0xF0 && len >= 4) {
        *dst = ((FcChar32)(c & 0x07) << 18) |
               ((FcChar32)(src[1] & 0x3F) << 12) |
               ((FcChar32)(src[2] & 0x3F) << 6) | (src[3] & 0x3F);
        return 4;
    }
    *dst = 0xFFFD;
    return 1;
}

int FcUtf8Len(const FcChar8 *src, int len, int *nchar, int *wchar)
{
    int nc = 0, wc = 1;
    int i = 0;

    while (i < len) {
        FcChar32 ch;
        int clen = FcUtf8ToUcs4(src + i, &ch, len - i);
        if (clen <= 0) break;
        nc++;
        if (ch > 0xFFFF) wc = 4;
        else if (ch > 0xFF && wc < 4) wc = 2;
        i += clen;
    }
    if (nchar) *nchar = nc;
    if (wchar) *wchar = wc;
    return i == len;
}

/* String set/list stubs */
FcStrSet *FcStrSetCreate(void) { return NULL; }
FcBool FcStrSetMember(FcStrSet *s, const FcChar8 *str) { (void)s; (void)str; return FcFalse; }
FcBool FcStrSetEqual(FcStrSet *a, FcStrSet *b) { (void)a; (void)b; return FcFalse; }
FcBool FcStrSetAdd(FcStrSet *s, const FcChar8 *str) { (void)s; (void)str; return FcFalse; }
FcBool FcStrSetAddFilename(FcStrSet *s, const FcChar8 *str) { (void)s; (void)str; return FcFalse; }
FcBool FcStrSetDel(FcStrSet *s, const FcChar8 *str) { (void)s; (void)str; return FcFalse; }
void FcStrSetDestroy(FcStrSet *s) { (void)s; }
FcStrList *FcStrListCreate(FcStrSet *s) { (void)s; return NULL; }
void FcStrListFirst(FcStrList *l) { (void)l; }
FcChar8 *FcStrListNext(FcStrList *l) { (void)l; return NULL; }
void FcStrListDone(FcStrList *l) { (void)l; }

/* ========================================================================= */
/* Blanks                                                                    */
/* ========================================================================= */

FcBlanks *FcBlanksCreate(void) { return NULL; }
void FcBlanksDestroy(FcBlanks *b) { (void)b; }
FcBool FcBlanksAdd(FcBlanks *b, FcChar32 ucs4) { (void)b; (void)ucs4; return FcFalse; }
FcBool FcBlanksIsMember(FcBlanks *b, FcChar32 ucs4) { (void)b; (void)ucs4; return FcFalse; }

/* ========================================================================= */
/* Cache                                                                     */
/* ========================================================================= */

FcBool FcDirCacheValid(const FcChar8 *dir) { (void)dir; return FcTrue; }

/* ========================================================================= */
/* Weight conversion                                                         */
/* ========================================================================= */

double FcWeightFromOpenType(int ot_weight)
{
    return FcWeightFromOpenTypeDouble((double)ot_weight);
}

double FcWeightFromOpenTypeDouble(double ot_weight)
{
    /* Approximate mapping from OS/2 usWeightClass to FC weight */
    if (ot_weight <= 100) return FC_WEIGHT_THIN;
    if (ot_weight <= 200) return FC_WEIGHT_EXTRALIGHT;
    if (ot_weight <= 300) return FC_WEIGHT_LIGHT;
    if (ot_weight <= 400) return FC_WEIGHT_REGULAR;
    if (ot_weight <= 500) return FC_WEIGHT_MEDIUM;
    if (ot_weight <= 600) return FC_WEIGHT_SEMIBOLD;
    if (ot_weight <= 700) return FC_WEIGHT_BOLD;
    if (ot_weight <= 800) return FC_WEIGHT_EXTRABOLD;
    return FC_WEIGHT_BLACK;
}

int FcWeightToOpenType(int fc_weight)
{
    return (int)FcWeightToOpenTypeDouble((double)fc_weight);
}

double FcWeightToOpenTypeDouble(double fc_weight)
{
    if (fc_weight <= FC_WEIGHT_THIN) return 100;
    if (fc_weight <= FC_WEIGHT_EXTRALIGHT) return 200;
    if (fc_weight <= FC_WEIGHT_LIGHT) return 300;
    if (fc_weight <= FC_WEIGHT_REGULAR) return 400;
    if (fc_weight <= FC_WEIGHT_MEDIUM) return 500;
    if (fc_weight <= FC_WEIGHT_SEMIBOLD) return 600;
    if (fc_weight <= FC_WEIGHT_BOLD) return 700;
    if (fc_weight <= FC_WEIGHT_EXTRABOLD) return 800;
    return 900;
}

/* ========================================================================= */
/* FreeType integration                                                      */
/* ========================================================================= */

FT_UInt FcFreeTypeCharIndex(FT_Face face, FcChar32 ucs4)
{
    return FT_Get_Char_Index(face, (FT_ULong)ucs4);
}

FcCharSet *FcFreeTypeCharSetAndSpacing(FT_Face face, FcBlanks *blanks,
                                          int *spacing)
{
    (void)face;
    (void)blanks;
    if (spacing) *spacing = FC_PROPORTIONAL;
    return FcCharSetCreate();
}

FcCharSet *FcFreeTypeCharSet(FT_Face face, FcBlanks *blanks)
{
    return FcFreeTypeCharSetAndSpacing(face, blanks, NULL);
}

FcResult FcPatternGetFTFace(const FcPattern *p, const char *object,
                               int n, FT_Face *f)
{
    (void)p;
    (void)object;
    (void)n;
    if (f) *f = NULL;
    return FcResultNoMatch;
}

FcBool FcPatternAddFTFace(FcPattern *p, const char *object,
                             const FT_Face f)
{
    (void)p;
    (void)object;
    (void)f;
    return FcTrue;
}
