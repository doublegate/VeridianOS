/* glibc_shim.c - Provide glibc-specific symbols needed by system libstdc++.a
 *
 * When using a system GCC's libstdc++.a (compiled against glibc) with musl,
 * a handful of glibc-internal symbols are referenced but not present in musl.
 * This shim provides compatible implementations.
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>
#include <unistd.h>
#include <pthread.h>
#include <sys/random.h>

/* --------------------------------------------------------------------------
 * __ctype_b_loc / __ctype_tolower_loc / __ctype_toupper_loc
 *
 * GCC's libstdc++.a (compiled against glibc) uses glibc's locale struct
 * layout to find the ctype character-classification table.  In the
 * std::ctype<char> constructor, when no explicit table is given, it reads
 * offset 0x68 of the __locale_struct returned by _S_get_c_locale().
 * On glibc this is __locale_data.__ctype_b (the ctype bitmask table).
 * On musl the struct is completely different and offset 0x68 is garbage,
 * causing a NULL-pointer crash in KConfigWatcher's string processing.
 *
 * By providing __ctype_b_loc() here, any code that calls it directly will
 * get a valid table.  Additionally, we provide a glibc-layout __locale_struct
 * shim via a wrapped newlocale() so that offset 0x68 contains a valid pointer.
 * -------------------------------------------------------------------------- */
#include <ctype.h>

/* glibc ctype mask bits (from glibc's ctype.h / ctype-info.c) */
#define _GU  0x0001  /* UPPER */
#define _GL  0x0002  /* LOWER */
#define _GA  0x0004  /* ALPHA */
#define _GD  0x0008  /* DIGIT */
#define _GX  0x0010  /* XDIGIT */
#define _GS  0x0020  /* SPACE */
#define _GP  0x0040  /* PRINT */
#define _GG  0x0080  /* GRAPH */
#define _GB  0x0100  /* BLANK (space/tab) */
#define _GC  0x0200  /* CNTRL */
#define _GN  0x0400  /* PUNCT */
#define _GW  0x0800  /* ALNUM */

/*
 * Static ctype table indexed by (char + 128).  384 entries total
 * (indices -128..255).  The first 128 entries handle EOF (-1) and
 * negative chars; entries 128..383 cover 0..255.
 */
static const unsigned short _ctype_b_table[384] = {
    /* -128..-1: all zero (control / undefined) */
    [0 ... 127] = 0,
    /* 0x00..0x08: control chars */
    [128 + 0x00] = _GC,
    [128 + 0x01] = _GC,
    [128 + 0x02] = _GC,
    [128 + 0x03] = _GC,
    [128 + 0x04] = _GC,
    [128 + 0x05] = _GC,
    [128 + 0x06] = _GC,
    [128 + 0x07] = _GC,
    [128 + 0x08] = _GC,
    /* 0x09: TAB  (control + space + blank) */
    [128 + 0x09] = _GC | _GS | _GB,
    /* 0x0A..0x0D: LF, VT, FF, CR (control + space) */
    [128 + 0x0A] = _GC | _GS,
    [128 + 0x0B] = _GC | _GS,
    [128 + 0x0C] = _GC | _GS,
    [128 + 0x0D] = _GC | _GS,
    /* 0x0E..0x1F: remaining control chars */
    [128 + 0x0E] = _GC,
    [128 + 0x0F] = _GC,
    [128 + 0x10] = _GC,
    [128 + 0x11] = _GC,
    [128 + 0x12] = _GC,
    [128 + 0x13] = _GC,
    [128 + 0x14] = _GC,
    [128 + 0x15] = _GC,
    [128 + 0x16] = _GC,
    [128 + 0x17] = _GC,
    [128 + 0x18] = _GC,
    [128 + 0x19] = _GC,
    [128 + 0x1A] = _GC,
    [128 + 0x1B] = _GC,
    [128 + 0x1C] = _GC,
    [128 + 0x1D] = _GC,
    [128 + 0x1E] = _GC,
    [128 + 0x1F] = _GC,
    /* 0x20: SPACE (space + print + blank) */
    [128 + 0x20] = _GS | _GP | _GB,
    /* 0x21..0x2F: !"#$%&'()*+,-./ (punct + print + graph) */
    [128 + 0x21] = _GN | _GP | _GG,
    [128 + 0x22] = _GN | _GP | _GG,
    [128 + 0x23] = _GN | _GP | _GG,
    [128 + 0x24] = _GN | _GP | _GG,
    [128 + 0x25] = _GN | _GP | _GG,
    [128 + 0x26] = _GN | _GP | _GG,
    [128 + 0x27] = _GN | _GP | _GG,
    [128 + 0x28] = _GN | _GP | _GG,
    [128 + 0x29] = _GN | _GP | _GG,
    [128 + 0x2A] = _GN | _GP | _GG,
    [128 + 0x2B] = _GN | _GP | _GG,
    [128 + 0x2C] = _GN | _GP | _GG,
    [128 + 0x2D] = _GN | _GP | _GG,
    [128 + 0x2E] = _GN | _GP | _GG,
    [128 + 0x2F] = _GN | _GP | _GG,
    /* 0x30..0x39: digits 0-9 (digit + xdigit + print + graph + alnum) */
    [128 + 0x30] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x31] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x32] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x33] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x34] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x35] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x36] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x37] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x38] = _GD | _GX | _GP | _GG | _GW,
    [128 + 0x39] = _GD | _GX | _GP | _GG | _GW,
    /* 0x3A..0x40: :;<=>?@ (punct + print + graph) */
    [128 + 0x3A] = _GN | _GP | _GG,
    [128 + 0x3B] = _GN | _GP | _GG,
    [128 + 0x3C] = _GN | _GP | _GG,
    [128 + 0x3D] = _GN | _GP | _GG,
    [128 + 0x3E] = _GN | _GP | _GG,
    [128 + 0x3F] = _GN | _GP | _GG,
    [128 + 0x40] = _GN | _GP | _GG,
    /* 0x41..0x46: A-F (upper + alpha + xdigit + print + graph + alnum) */
    [128 + 0x41] = _GU | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x42] = _GU | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x43] = _GU | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x44] = _GU | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x45] = _GU | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x46] = _GU | _GA | _GX | _GP | _GG | _GW,
    /* 0x47..0x5A: G-Z (upper + alpha + print + graph + alnum) */
    [128 + 0x47] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x48] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x49] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x4A] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x4B] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x4C] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x4D] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x4E] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x4F] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x50] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x51] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x52] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x53] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x54] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x55] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x56] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x57] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x58] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x59] = _GU | _GA | _GP | _GG | _GW,
    [128 + 0x5A] = _GU | _GA | _GP | _GG | _GW,
    /* 0x5B..0x60: [\]^_` (punct + print + graph) */
    [128 + 0x5B] = _GN | _GP | _GG,
    [128 + 0x5C] = _GN | _GP | _GG,
    [128 + 0x5D] = _GN | _GP | _GG,
    [128 + 0x5E] = _GN | _GP | _GG,
    [128 + 0x5F] = _GN | _GP | _GG,
    [128 + 0x60] = _GN | _GP | _GG,
    /* 0x61..0x66: a-f (lower + alpha + xdigit + print + graph + alnum) */
    [128 + 0x61] = _GL | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x62] = _GL | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x63] = _GL | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x64] = _GL | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x65] = _GL | _GA | _GX | _GP | _GG | _GW,
    [128 + 0x66] = _GL | _GA | _GX | _GP | _GG | _GW,
    /* 0x67..0x7A: g-z (lower + alpha + print + graph + alnum) */
    [128 + 0x67] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x68] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x69] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x6A] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x6B] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x6C] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x6D] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x6E] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x6F] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x70] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x71] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x72] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x73] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x74] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x75] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x76] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x77] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x78] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x79] = _GL | _GA | _GP | _GG | _GW,
    [128 + 0x7A] = _GL | _GA | _GP | _GG | _GW,
    /* 0x7B..0x7E: {|}~ (punct + print + graph) */
    [128 + 0x7B] = _GN | _GP | _GG,
    [128 + 0x7C] = _GN | _GP | _GG,
    [128 + 0x7D] = _GN | _GP | _GG,
    [128 + 0x7E] = _GN | _GP | _GG,
    /* 0x7F: DEL (control) */
    [128 + 0x7F] = _GC,
    /* 0x80..0xFF: all zero for C locale */
};

/* Pointer into the table at index 128 (the 'char 0' position).
 * Thread-local to match glibc's per-thread locale support. */
static __thread const unsigned short *_ctype_b_ptr = _ctype_b_table + 128;

const unsigned short **__ctype_b_loc(void) {
    return &_ctype_b_ptr;
}

/* tolower/toupper tables (identity for non-letter, case-flip for letters) */
static const int _ctype_tolower_table[384] = {
    [0 ... 127] = 0,
    /* 0..127: identity map, with A-Z -> a-z */
    #define _TL(c) [128 + (c)] = (c)
    _TL(0), _TL(1), _TL(2), _TL(3), _TL(4), _TL(5), _TL(6), _TL(7),
    _TL(8), _TL(9), _TL(10), _TL(11), _TL(12), _TL(13), _TL(14), _TL(15),
    _TL(16), _TL(17), _TL(18), _TL(19), _TL(20), _TL(21), _TL(22), _TL(23),
    _TL(24), _TL(25), _TL(26), _TL(27), _TL(28), _TL(29), _TL(30), _TL(31),
    _TL(32), _TL(33), _TL(34), _TL(35), _TL(36), _TL(37), _TL(38), _TL(39),
    _TL(40), _TL(41), _TL(42), _TL(43), _TL(44), _TL(45), _TL(46), _TL(47),
    _TL(48), _TL(49), _TL(50), _TL(51), _TL(52), _TL(53), _TL(54), _TL(55),
    _TL(56), _TL(57), _TL(58), _TL(59), _TL(60), _TL(61), _TL(62), _TL(63),
    _TL(64),
    /* A-Z -> a-z */
    [128 + 'A'] = 'a', [128 + 'B'] = 'b', [128 + 'C'] = 'c',
    [128 + 'D'] = 'd', [128 + 'E'] = 'e', [128 + 'F'] = 'f',
    [128 + 'G'] = 'g', [128 + 'H'] = 'h', [128 + 'I'] = 'i',
    [128 + 'J'] = 'j', [128 + 'K'] = 'k', [128 + 'L'] = 'l',
    [128 + 'M'] = 'm', [128 + 'N'] = 'n', [128 + 'O'] = 'o',
    [128 + 'P'] = 'p', [128 + 'Q'] = 'q', [128 + 'R'] = 'r',
    [128 + 'S'] = 's', [128 + 'T'] = 't', [128 + 'U'] = 'u',
    [128 + 'V'] = 'v', [128 + 'W'] = 'w', [128 + 'X'] = 'x',
    [128 + 'Y'] = 'y', [128 + 'Z'] = 'z',
    /* [, \, ], ^, _, ` */
    _TL(91), _TL(92), _TL(93), _TL(94), _TL(95), _TL(96),
    /* a-z identity */
    _TL('a'), _TL('b'), _TL('c'), _TL('d'), _TL('e'), _TL('f'),
    _TL('g'), _TL('h'), _TL('i'), _TL('j'), _TL('k'), _TL('l'),
    _TL('m'), _TL('n'), _TL('o'), _TL('p'), _TL('q'), _TL('r'),
    _TL('s'), _TL('t'), _TL('u'), _TL('v'), _TL('w'), _TL('x'),
    _TL('y'), _TL('z'),
    _TL(123), _TL(124), _TL(125), _TL(126), _TL(127),
    #undef _TL
    /* 128..255: identity */
};

static __thread const int *_ctype_tolower_ptr = _ctype_tolower_table + 128;

const int **__ctype_tolower_loc(void) {
    return &_ctype_tolower_ptr;
}

static const int _ctype_toupper_table[384] = {
    [0 ... 127] = 0,
    /* identity for most, a-z -> A-Z */
    #define _TU(c) [128 + (c)] = (c)
    _TU(0), _TU(1), _TU(2), _TU(3), _TU(4), _TU(5), _TU(6), _TU(7),
    _TU(8), _TU(9), _TU(10), _TU(11), _TU(12), _TU(13), _TU(14), _TU(15),
    _TU(16), _TU(17), _TU(18), _TU(19), _TU(20), _TU(21), _TU(22), _TU(23),
    _TU(24), _TU(25), _TU(26), _TU(27), _TU(28), _TU(29), _TU(30), _TU(31),
    _TU(32), _TU(33), _TU(34), _TU(35), _TU(36), _TU(37), _TU(38), _TU(39),
    _TU(40), _TU(41), _TU(42), _TU(43), _TU(44), _TU(45), _TU(46), _TU(47),
    _TU(48), _TU(49), _TU(50), _TU(51), _TU(52), _TU(53), _TU(54), _TU(55),
    _TU(56), _TU(57), _TU(58), _TU(59), _TU(60), _TU(61), _TU(62), _TU(63),
    _TU(64),
    /* A-Z identity */
    _TU('A'), _TU('B'), _TU('C'), _TU('D'), _TU('E'), _TU('F'),
    _TU('G'), _TU('H'), _TU('I'), _TU('J'), _TU('K'), _TU('L'),
    _TU('M'), _TU('N'), _TU('O'), _TU('P'), _TU('Q'), _TU('R'),
    _TU('S'), _TU('T'), _TU('U'), _TU('V'), _TU('W'), _TU('X'),
    _TU('Y'), _TU('Z'),
    _TU(91), _TU(92), _TU(93), _TU(94), _TU(95), _TU(96),
    /* a-z -> A-Z */
    [128 + 'a'] = 'A', [128 + 'b'] = 'B', [128 + 'c'] = 'C',
    [128 + 'd'] = 'D', [128 + 'e'] = 'E', [128 + 'f'] = 'F',
    [128 + 'g'] = 'G', [128 + 'h'] = 'H', [128 + 'i'] = 'I',
    [128 + 'j'] = 'J', [128 + 'k'] = 'K', [128 + 'l'] = 'L',
    [128 + 'm'] = 'M', [128 + 'n'] = 'N', [128 + 'o'] = 'O',
    [128 + 'p'] = 'P', [128 + 'q'] = 'Q', [128 + 'r'] = 'R',
    [128 + 's'] = 'S', [128 + 't'] = 'T', [128 + 'u'] = 'U',
    [128 + 'v'] = 'V', [128 + 'w'] = 'W', [128 + 'x'] = 'X',
    [128 + 'y'] = 'Y', [128 + 'z'] = 'Z',
    _TU(123), _TU(124), _TU(125), _TU(126), _TU(127),
    #undef _TU
};

static __thread const int *_ctype_toupper_ptr = _ctype_toupper_table + 128;

const int **__ctype_toupper_loc(void) {
    return &_ctype_toupper_ptr;
}

/* --------------------------------------------------------------------------
 * __newlocale / newlocale override
 *
 * libstdc++ (compiled against glibc) expects the locale_t returned by
 * newlocale() to be a glibc __locale_struct, which has:
 *   offset 0x68: const unsigned short *__ctype_b     (classification table)
 *   offset 0x70: const int *__ctype_tolower           (tolower table)
 *   offset 0x78: const int *__ctype_toupper           (toupper table)
 *
 * musl's locale_t is a completely different struct (6 pointers = 48 bytes).
 * When libstdc++ reads offset 0x68, it gets garbage from .rodata strings
 * that happen to follow musl's __c_locale in memory, causing a segfault.
 *
 * We provide a fake glibc-layout locale struct with the ctype tables at
 * the right offsets.  Since this binary only uses the "C" locale, a single
 * static instance suffices.
 * -------------------------------------------------------------------------- */
#include <locale.h>

/* Fake glibc __locale_struct.  Must be at least 0x80 bytes.
 * Layout: 13 __locale_data pointers (0x00-0x67), then 3 ctype pointers. */
static const struct {
    const void *__locales[13];         /* 0x00 - 0x67: dummy locale data ptrs */
    const unsigned short *__ctype_b;   /* 0x68: classification table */
    const int *__ctype_tolower;        /* 0x70: tolower table */
    const int *__ctype_toupper;        /* 0x78: toupper table */
} _glibc_c_locale = {
    .__locales = { 0 },
    .__ctype_b       = _ctype_b_table + 128,
    .__ctype_tolower  = _ctype_tolower_table + 128,
    .__ctype_toupper  = _ctype_toupper_table + 128,
};

/* Override musl's __newlocale.  For a static musl binary, all locale
 * operations go through this entry point.  We always return our glibc-
 * compatible C locale struct.  The mask and name arguments are ignored
 * because kwin only ever uses the "C" locale in this context. */
struct __locale_struct *__newlocale(int mask, const char *name,
                                    struct __locale_struct *base) {
    (void)mask; (void)name; (void)base;
    return (struct __locale_struct *)(void *)&_glibc_c_locale;
}

/* Public alias -- musl declares newlocale with locale_t = struct __locale_struct* */
struct __locale_struct *newlocale(int mask, const char *name,
                                  struct __locale_struct *base) {
    return __newlocale(mask, name, base);
}

/* glibc fortified I/O - just forward to standard versions */
int __sprintf_chk(char *s, int flag, size_t slen, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vsprintf(s, fmt, ap);
    va_end(ap);
    return ret;
}

int __fprintf_chk(FILE *stream, int flag, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vfprintf(stream, fmt, ap);
    va_end(ap);
    return ret;
}

ssize_t __read_chk(int fd, void *buf, size_t nbytes, size_t buflen) {
    return read(fd, buf, nbytes);
}

/* C23 strtoul - forward to standard strtoul */
unsigned long __isoc23_strtoul(const char *nptr, char **endptr, int base) {
    return strtoul(nptr, endptr, base);
}

/* glibc single-threaded optimization flag - always say multi-threaded (safe) */
char __libc_single_threaded = 0;

/* arc4random - use getrandom() as backend */
unsigned int arc4random(void) {
    unsigned int val;
    if (getrandom(&val, sizeof(val), 0) != sizeof(val)) {
        /* fallback: read from /dev/urandom */
        FILE *f = fopen("/dev/urandom", "r");
        if (f) {
            fread(&val, sizeof(val), 1, f);
            fclose(f);
        }
    }
    return val;
}

/* _dl_find_object - glibc dynamic linker function used by libgcc_eh for
 * exception handling. In static musl builds, return failure (-1) to fall
 * back to dl_iterate_phdr-based unwinding. */
struct dl_find_object;
int _dl_find_object(void *address, struct dl_find_object *result) {
    return -1;  /* not found - triggers fallback path */
}

/* __dso_handle - required for C++ static destructors in shared libraries.
 * GCC's crtbeginS.o normally provides this, but our nostdlib linking skips it. */
void *__dso_handle __attribute__((visibility("hidden"))) = &__dso_handle;

/* glibc FORTIFY_SOURCE functions - forward to standard versions */
#include <string.h>
#include <wchar.h>

void *__memcpy_chk(void *dest, const void *src, size_t len, size_t destlen) {
    return memcpy(dest, src, len);
}

void *__memmove_chk(void *dest, const void *src, size_t len, size_t destlen) {
    return memmove(dest, src, len);
}

void *__memset_chk(void *s, int c, size_t n, size_t slen) {
    return memset(s, c, n);
}

char *__strcpy_chk(char *dest, const char *src, size_t destlen) {
    return strcpy(dest, src);
}

char *__strcat_chk(char *dest, const char *src, size_t destlen) {
    return strcat(dest, src);
}

char *__stpcpy_chk(char *dest, const char *src, size_t destlen) {
    return stpcpy(dest, src);
}

int __snprintf_chk(char *s, size_t maxlen, int flag, size_t slen,
                   const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vsnprintf(s, maxlen, fmt, ap);
    va_end(ap);
    return ret;
}

size_t __mbsrtowcs_chk(wchar_t *dest, const char **src, size_t len,
                        mbstate_t *ps, size_t destlen) {
    return mbsrtowcs(dest, src, len, ps);
}

wchar_t *__wmemcpy_chk(wchar_t *dest, const wchar_t *src, size_t n,
                        size_t destlen) {
    return wmemcpy(dest, src, n);
}

wchar_t *__wmemmove_chk(wchar_t *dest, const wchar_t *src, size_t n,
                         size_t destlen) {
    return wmemmove(dest, src, n);
}

wchar_t *__wmemset_chk(wchar_t *dest, wchar_t c, size_t n, size_t destlen) {
    return wmemset(dest, c, n);
}

wchar_t *__wcscpy_chk(wchar_t *dest, const wchar_t *src, size_t destlen) {
    return wcscpy(dest, src);
}

wchar_t *__wcscat_chk(wchar_t *dest, const wchar_t *src, size_t destlen) {
    return wcscat(dest, src);
}

size_t __wcrtomb_chk(char *s, wchar_t wc, mbstate_t *ps, size_t buflen) {
    return wcrtomb(s, wc, ps);
}

size_t __mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps) {
    return mbrtowc(pwc, s, n, ps);
}

int __swprintf_chk(wchar_t *s, size_t maxlen, int flag, size_t slen,
                   const wchar_t *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vswprintf(s, maxlen, fmt, ap);
    va_end(ap);
    return ret;
}

/* fseeko64/ftello64 - musl's fseeko/ftello are already 64-bit on 64-bit systems */
int fseeko64(FILE *stream, long long offset, int whence) {
    return fseeko(stream, (off_t)offset, whence);
}

long long ftello64(FILE *stream) {
    return (long long)ftello(stream);
}

/* __cxa_thread_atexit_impl - thread-local destructor registration.
 * musl provides __cxa_thread_atexit but GCC's libstdc++ references _impl. */
int __cxa_thread_atexit_impl(void (*func)(void *), void *obj, void *dso_handle) {
    /* Forward to musl's __cxa_thread_atexit */
    extern int __cxa_thread_atexit(void (*)(void *), void *, void *);
    return __cxa_thread_atexit(func, obj, dso_handle);
}

/* pthread_*_clock* - glibc extensions for clock-specific pthread operations.
 * GCC 15 libstdc++ references these when _GLIBCXX_USE_PTHREAD_COND_CLOCKWAIT
 * is set. Provide fallback implementations using standard POSIX equivalents. */
#include <pthread.h>
#include <time.h>

int pthread_cond_clockwait(pthread_cond_t *cond,
                           pthread_mutex_t *mutex,
                           clockid_t clock_id,
                           const struct timespec *abstime) {
    (void)clock_id;
    return pthread_cond_timedwait(cond, mutex, abstime);
}

int pthread_mutex_clocklock(pthread_mutex_t *mutex,
                            clockid_t clock_id,
                            const struct timespec *abstime) {
    (void)clock_id;
    return pthread_mutex_timedlock(mutex, abstime);
}

int pthread_rwlock_clockwrlock(pthread_rwlock_t *rwlock,
                               clockid_t clock_id,
                               const struct timespec *abstime) {
    (void)clock_id;
    return pthread_rwlock_timedwrlock(rwlock, abstime);
}

int pthread_rwlock_clockrdlock(pthread_rwlock_t *rwlock,
                               clockid_t clock_id,
                               const struct timespec *abstime) {
    (void)clock_id;
    return pthread_rwlock_timedrdlock(rwlock, abstime);
}

/* --------------------------------------------------------------------------
 * libseat shim -- minimal seat management for DRM device access
 *
 * KWin 6.x can optionally use libseat for DRM device management.  On
 * VeridianOS there is no seatd or logind, so we provide a minimal
 * implementation whose open_device() calls open() directly on the device
 * path.  This allows KWin's LibSeatSession to function without a real
 * seat management daemon.
 *
 * If kwin is compiled WITHOUT libseat, it falls back to its own
 * NoopSession (which we also patch in build-kwin.sh to call open()).
 * -------------------------------------------------------------------------- */
#include <fcntl.h>
#include <errno.h>

struct libseat;
struct libseat_seat_listener {
    void (*enable_seat)(struct libseat *seat, void *data);
    void (*disable_seat)(struct libseat *seat, void *data);
};

/* Internal state for our minimal seat implementation. */
struct libseat_shim {
    int dummy;
    void *user_data;
};

static struct libseat_shim _seat_shim;

struct libseat *libseat_open_seat(const struct libseat_seat_listener *listener,
                                   void *data) {
    (void)listener;
    _seat_shim.user_data = data;
    /* Enable seat immediately so KWin knows the session is active. */
    if (listener && listener->enable_seat)
        listener->enable_seat((struct libseat *)&_seat_shim, data);
    return (struct libseat *)&_seat_shim;
}

int libseat_open_device(struct libseat *seat, const char *path, int *fd) {
    (void)seat;
    int f = open(path, O_RDWR | O_CLOEXEC);
    if (f < 0)
        return -1;
    *fd = f;
    return f;
}

int libseat_close_device(struct libseat *seat, int device_id) {
    (void)seat;
    close(device_id);
    return 0;
}

int libseat_disable_seat(struct libseat *seat) {
    (void)seat;
    return 0;
}

int libseat_dispatch(struct libseat *seat, int timeout) {
    (void)seat; (void)timeout;
    return 0;
}

int libseat_get_fd(struct libseat *seat) {
    (void)seat;
    return -1;  /* no event fd needed -- seat is always active */
}

const char *libseat_seat_name(struct libseat *seat) {
    (void)seat;
    return "seat0";
}

int libseat_switch_session(struct libseat *seat, int session) {
    (void)seat; (void)session;
    return 0;
}

void libseat_close_seat(struct libseat *seat) {
    (void)seat;
}
