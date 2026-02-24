/*
 * VeridianOS libc -- regex.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX Basic and Extended Regular Expression engine.
 * Implements regcomp(), regexec(), regfree(), and regerror().
 *
 * Approach: Compile pattern to an internal node array, then match via
 * recursive backtracking NFA.  Supports BRE (default) and ERE
 * (REG_EXTENDED), REG_ICASE, REG_NEWLINE, REG_NOSUB, REG_NOTBOL,
 * REG_NOTEOL, and subgroup capture via regmatch_t.
 *
 * Test cases (verify with BusyBox grep on VeridianOS):
 *   echo "hello world" | grep "hello"     -> matches
 *   echo "hello world" | grep "^hello"    -> matches
 *   echo "hello world" | grep "world$"    -> matches
 *   echo "hello world" | grep "hell."     -> matches
 *   echo "hello world" | grep "h.*d"      -> matches
 *   echo "hello world" | grep "[hw]"      -> matches
 *   echo "hello world" | grep -E "he|wo"  -> matches (ERE)
 *   echo "hello world" | grep "xyz"       -> no match
 *   echo "HELLO" | grep -i "hello"        -> matches (REG_ICASE)
 *   echo "abc" | grep -E "a(b|c)c"        -> matches
 *   echo "aXc" | grep -E "a.+c"           -> matches
 *   echo "ac"  | grep -E "a.?c"           -> matches
 *   echo "aab" | grep "a*b"               -> matches (BRE star zero-or-more)
 *   echo "abc" | grep '\(ab\)c'           -> matches (BRE group)
 *   echo "abc" | grep -E '(ab)c'          -> matches (ERE group)
 */

#include <regex.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Internal types                                                            */
/* ========================================================================= */

/* Maximum number of compiled nodes */
#define MAX_NODES      1024
/* Maximum capturing groups (POSIX says at least 9, we support 32) */
#define MAX_GROUPS     32
/* Maximum recursion depth for match engine */
#define MAX_DEPTH      4096
/* Character class bitmap: 256 bits = 32 bytes */
#define CLASS_BYTES    32

enum node_type {
    NODE_LITERAL,       /* Match a specific character */
    NODE_DOT,           /* Match any character (except newline w/ REG_NEWLINE) */
    NODE_CLASS,         /* Character class [...]  */
    NODE_ANCHOR_START,  /* ^ */
    NODE_ANCHOR_END,    /* $ */
    NODE_GROUP_START,   /* ( or \( -- begins capture group */
    NODE_GROUP_END,     /* ) or \) -- ends capture group */
    NODE_BRANCH,        /* | alternation -- try left, if fail skip to .alt */
    NODE_JUMP,          /* Unconditional jump (end of branch) */
    NODE_MATCH,         /* Successful end of pattern */
};

struct regex_node {
    enum node_type  type;
    /* Literal character (for NODE_LITERAL) */
    unsigned char   ch;
    /* Character class bitmap (for NODE_CLASS) -- heap-allocated */
    unsigned char  *cclass;
    /* Negated class flag */
    int             negate;
    /* Group index (for NODE_GROUP_START / NODE_GROUP_END) */
    int             group;
    /* For NODE_BRANCH: index of the alternate path (past the branch) */
    int             alt;
    /* For NODE_JUMP: target index */
    int             target;
};

struct compiled_regex {
    struct regex_node nodes[MAX_NODES];
    int               num_nodes;
    int               num_groups;   /* Number of capturing groups */
    int               cflags;       /* Original compile flags */
};

/* ========================================================================= */
/* Character utilities                                                       */
/* ========================================================================= */

static int char_lower(int c)
{
    if (c >= 'A' && c <= 'Z')
        return c + ('a' - 'A');
    return c;
}

static int char_upper(int c)
{
    if (c >= 'a' && c <= 'z')
        return c - ('a' - 'A');
    return c;
}

/* ========================================================================= */
/* Character class bitmap helpers                                            */
/* ========================================================================= */

static void class_set(unsigned char *bm, unsigned char c)
{
    bm[c >> 3] |= (unsigned char)(1 << (c & 7));
}

static int class_test(const unsigned char *bm, unsigned char c)
{
    return (bm[c >> 3] >> (c & 7)) & 1;
}

/* ========================================================================= */
/* Compiler: parse regex pattern into node array                             */
/* ========================================================================= */

/* Forward declarations for recursive descent */
struct parse_state {
    const char             *pat;
    int                     pos;
    int                     len;
    int                     cflags;
    struct compiled_regex   *re;
    int                     group_count;
    int                     err;
};

static int emit(struct parse_state *ps, enum node_type type)
{
    if (ps->re->num_nodes >= MAX_NODES) {
        ps->err = REG_ESPACE;
        return -1;
    }
    int idx = ps->re->num_nodes++;
    memset(&ps->re->nodes[idx], 0, sizeof(struct regex_node));
    ps->re->nodes[idx].type = type;
    ps->re->nodes[idx].group = -1;
    ps->re->nodes[idx].alt = -1;
    ps->re->nodes[idx].target = -1;
    return idx;
}

static int peek(struct parse_state *ps)
{
    if (ps->pos >= ps->len)
        return -1;
    return (unsigned char)ps->pat[ps->pos];
}

static int next_ch(struct parse_state *ps)
{
    if (ps->pos >= ps->len)
        return -1;
    return (unsigned char)ps->pat[ps->pos++];
}

/* Parse a POSIX character class name like [:alpha:] */
static int parse_posix_class(struct parse_state *ps, unsigned char *bm)
{
    /* We're right after '[:', try to find ':]' */
    int start = ps->pos;
    while (ps->pos < ps->len && ps->pat[ps->pos] != ':')
        ps->pos++;
    if (ps->pos + 1 >= ps->len || ps->pat[ps->pos + 1] != ']') {
        /* Not a valid POSIX class, rewind */
        ps->pos = start;
        return 0;
    }

    int clen = ps->pos - start;
    ps->pos += 2;  /* skip ':' and ']' */

    int i;
    if (clen == 5 && memcmp(ps->pat + start, "alpha", 5) == 0) {
        for (i = 'A'; i <= 'Z'; i++) class_set(bm, (unsigned char)i);
        for (i = 'a'; i <= 'z'; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "digit", 5) == 0) {
        for (i = '0'; i <= '9'; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "alnum", 5) == 0) {
        for (i = 'A'; i <= 'Z'; i++) class_set(bm, (unsigned char)i);
        for (i = 'a'; i <= 'z'; i++) class_set(bm, (unsigned char)i);
        for (i = '0'; i <= '9'; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "space", 5) == 0) {
        class_set(bm, ' '); class_set(bm, '\t'); class_set(bm, '\n');
        class_set(bm, '\r'); class_set(bm, '\f'); class_set(bm, '\v');
    } else if (clen == 5 && memcmp(ps->pat + start, "upper", 5) == 0) {
        for (i = 'A'; i <= 'Z'; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "lower", 5) == 0) {
        for (i = 'a'; i <= 'z'; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "print", 5) == 0) {
        for (i = 0x20; i <= 0x7e; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "graph", 5) == 0) {
        for (i = 0x21; i <= 0x7e; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "cntrl", 5) == 0) {
        for (i = 0; i < 0x20; i++) class_set(bm, (unsigned char)i);
        class_set(bm, 0x7f);
    } else if (clen == 5 && memcmp(ps->pat + start, "punct", 5) == 0) {
        for (i = 0x21; i <= 0x2f; i++) class_set(bm, (unsigned char)i);
        for (i = 0x3a; i <= 0x40; i++) class_set(bm, (unsigned char)i);
        for (i = 0x5b; i <= 0x60; i++) class_set(bm, (unsigned char)i);
        for (i = 0x7b; i <= 0x7e; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 6 && memcmp(ps->pat + start, "xdigit", 6) == 0) {
        for (i = '0'; i <= '9'; i++) class_set(bm, (unsigned char)i);
        for (i = 'A'; i <= 'F'; i++) class_set(bm, (unsigned char)i);
        for (i = 'a'; i <= 'f'; i++) class_set(bm, (unsigned char)i);
    } else if (clen == 5 && memcmp(ps->pat + start, "blank", 5) == 0) {
        class_set(bm, ' '); class_set(bm, '\t');
    } else {
        return 0;  /* unknown class name */
    }
    return 1;
}

/* Parse a bracket expression [...]
 * Returns the node index, or -1 on error. */
static int parse_class(struct parse_state *ps)
{
    int idx = emit(ps, NODE_CLASS);
    if (idx < 0)
        return -1;

    unsigned char *bm = (unsigned char *)malloc(CLASS_BYTES);
    if (!bm) {
        ps->err = REG_ESPACE;
        return -1;
    }
    memset(bm, 0, CLASS_BYTES);
    ps->re->nodes[idx].cclass = bm;

    int negate = 0;
    int c = peek(ps);

    /* Check for negation */
    if (c == '^') {
        negate = 1;
        next_ch(ps);
    }
    ps->re->nodes[idx].negate = negate;

    /* A leading ']' or '-' is treated as literal */
    int first = 1;

    while (ps->pos < ps->len) {
        c = peek(ps);
        if (c == ']' && !first) {
            next_ch(ps);
            /* Apply REG_ICASE */
            if (ps->cflags & REG_ICASE) {
                int i;
                for (i = 0; i < 256; i++) {
                    if (class_test(bm, (unsigned char)i)) {
                        class_set(bm, (unsigned char)char_lower(i));
                        class_set(bm, (unsigned char)char_upper(i));
                    }
                }
            }
            return idx;
        }

        first = 0;

        /* POSIX character class [:name:] */
        if (c == '[' && ps->pos + 1 < ps->len && ps->pat[ps->pos + 1] == ':') {
            ps->pos += 2; /* skip '[' and ':' */
            if (parse_posix_class(ps, bm)) {
                continue;
            }
            /* Not valid posix class -- treat '[' as literal, rewind past ':' */
            ps->pos -= 1; /* put back ':', the '[' was already consumed */
            class_set(bm, '[');
            continue;
        }

        c = next_ch(ps);

        /* Backslash escape inside class */
        if (c == '\\' && ps->pos < ps->len) {
            c = next_ch(ps);
        }

        /* Check for range a-b */
        if (peek(ps) == '-' && ps->pos + 1 < ps->len &&
            ps->pat[ps->pos + 1] != ']') {
            next_ch(ps); /* consume '-' */
            int end = next_ch(ps);
            if (end == '\\' && ps->pos < ps->len)
                end = next_ch(ps);
            if (end < c) {
                ps->err = REG_ERANGE;
                return -1;
            }
            int i;
            for (i = c; i <= end; i++)
                class_set(bm, (unsigned char)i);
        } else {
            class_set(bm, (unsigned char)c);
        }
    }

    /* Unterminated bracket expression */
    ps->err = REG_EBRACK;
    return -1;
}

/* Parse a bounded repetition {m,n} or \{m,n\}
 * On entry, we've already consumed the opening { (or \{).
 * Returns 1 on success, 0 on failure.
 * Sets *min_out and *max_out. max_out = -1 means unbounded. */
static int parse_bound(struct parse_state *ps, int is_bre,
                       int *min_out, int *max_out)
{
    int m = 0, n = -1;

    /* Parse minimum */
    if (peek(ps) < '0' || peek(ps) > '9') {
        ps->err = REG_BADBR;
        return 0;
    }
    while (peek(ps) >= '0' && peek(ps) <= '9') {
        m = m * 10 + (next_ch(ps) - '0');
        if (m > 255) { ps->err = REG_BADBR; return 0; }
    }

    int c = peek(ps);
    if (c == ',') {
        next_ch(ps); /* consume ',' */
        if ((is_bre && peek(ps) == '\\') ||
            (!is_bre && peek(ps) == '}')) {
            /* {m,} -- unbounded */
            n = -1;
        } else if (peek(ps) >= '0' && peek(ps) <= '9') {
            n = 0;
            while (peek(ps) >= '0' && peek(ps) <= '9') {
                n = n * 10 + (next_ch(ps) - '0');
                if (n > 255) { ps->err = REG_BADBR; return 0; }
            }
            if (n < m) { ps->err = REG_BADBR; return 0; }
        } else {
            ps->err = REG_BADBR;
            return 0;
        }
    } else {
        /* {m} -- exact count */
        n = m;
    }

    /* Expect closing brace */
    if (is_bre) {
        if (peek(ps) != '\\' || (ps->pos + 1 < ps->len &&
            ps->pat[ps->pos + 1] != '}')) {
            ps->err = REG_EBRACE;
            return 0;
        }
        next_ch(ps); /* '\\' */
        next_ch(ps); /* '}' */
    } else {
        if (peek(ps) != '}') {
            ps->err = REG_EBRACE;
            return 0;
        }
        next_ch(ps);
    }

    *min_out = m;
    *max_out = n;
    return 1;
}

/* Duplicate a range of nodes [from, to) at the current end.
 * Returns the starting index of the copy, or -1 on error. */
static int dup_nodes(struct parse_state *ps, int from, int to)
{
    int count = to - from;
    if (ps->re->num_nodes + count > MAX_NODES) {
        ps->err = REG_ESPACE;
        return -1;
    }
    int base = ps->re->num_nodes;
    int i;
    for (i = 0; i < count; i++) {
        ps->re->nodes[base + i] = ps->re->nodes[from + i];
        /* Deep-copy character class bitmaps */
        if (ps->re->nodes[base + i].type == NODE_CLASS &&
            ps->re->nodes[base + i].cclass) {
            unsigned char *copy = (unsigned char *)malloc(CLASS_BYTES);
            if (!copy) { ps->err = REG_ESPACE; return -1; }
            memcpy(copy, ps->re->nodes[base + i].cclass, CLASS_BYTES);
            ps->re->nodes[base + i].cclass = copy;
        }
        /* Adjust relative references */
        if (ps->re->nodes[base + i].alt >= 0)
            ps->re->nodes[base + i].alt += (base - from);
        if (ps->re->nodes[base + i].target >= 0)
            ps->re->nodes[base + i].target += (base - from);
    }
    ps->re->num_nodes += count;
    return base;
}

/* Apply a quantifier to the atom at nodes [atom_start, current_end).
 * This generates the repeated pattern in-place. */
static int apply_quantifier(struct parse_state *ps, int atom_start,
                            int qt, int qmin, int qmax)
{
    int atom_end = ps->re->num_nodes;
    int atom_len = atom_end - atom_start;
    int i;

    if (atom_len == 0)
        return 0; /* nothing to quantify */

    /* qt: 0=QUEST, 1=STAR, 2=PLUS, 3=RANGE */
    #define QT_QUEST  0
    #define QT_STAR   1
    #define QT_PLUS   2
    #define QT_RANGE  3

    if (qt == QT_QUEST) {
        /* a? => branch(a, skip) */
        /* Move atom forward by 1 to insert branch at atom_start */
        if (ps->re->num_nodes + 1 > MAX_NODES) {
            ps->err = REG_ESPACE; return -1;
        }
        /* Shift nodes [atom_start..end) right by 1 */
        memmove(&ps->re->nodes[atom_start + 1],
                &ps->re->nodes[atom_start],
                (size_t)atom_len * sizeof(struct regex_node));
        ps->re->num_nodes++;
        /* Fix up references in shifted nodes */
        for (i = atom_start + 1; i < ps->re->num_nodes; i++) {
            if (ps->re->nodes[i].alt >= atom_start)
                ps->re->nodes[i].alt++;
            if (ps->re->nodes[i].target >= atom_start)
                ps->re->nodes[i].target++;
        }
        /* Insert branch */
        memset(&ps->re->nodes[atom_start], 0, sizeof(struct regex_node));
        ps->re->nodes[atom_start].type = NODE_BRANCH;
        ps->re->nodes[atom_start].group = -1;
        ps->re->nodes[atom_start].alt = ps->re->num_nodes;
        ps->re->nodes[atom_start].target = -1;
        return 0;
    }

    if (qt == QT_STAR) {
        /* a* => branch(a + jump back, skip) */
        if (ps->re->num_nodes + 2 > MAX_NODES) {
            ps->err = REG_ESPACE; return -1;
        }
        /* Shift atom right by 1 for branch */
        memmove(&ps->re->nodes[atom_start + 1],
                &ps->re->nodes[atom_start],
                (size_t)atom_len * sizeof(struct regex_node));
        ps->re->num_nodes++;
        for (i = atom_start + 1; i < ps->re->num_nodes; i++) {
            if (ps->re->nodes[i].alt >= atom_start)
                ps->re->nodes[i].alt++;
            if (ps->re->nodes[i].target >= atom_start)
                ps->re->nodes[i].target++;
        }
        /* branch at atom_start: try atom, alt = past jump */
        memset(&ps->re->nodes[atom_start], 0, sizeof(struct regex_node));
        ps->re->nodes[atom_start].type = NODE_BRANCH;
        ps->re->nodes[atom_start].group = -1;
        ps->re->nodes[atom_start].alt = ps->re->num_nodes + 1; /* past jump */
        ps->re->nodes[atom_start].target = -1;
        /* jump at end: back to branch */
        int jmp_idx = ps->re->num_nodes;
        if (jmp_idx >= MAX_NODES) { ps->err = REG_ESPACE; return -1; }
        memset(&ps->re->nodes[jmp_idx], 0, sizeof(struct regex_node));
        ps->re->nodes[jmp_idx].type = NODE_JUMP;
        ps->re->nodes[jmp_idx].group = -1;
        ps->re->nodes[jmp_idx].alt = -1;
        ps->re->nodes[jmp_idx].target = atom_start;
        ps->re->num_nodes++;
        return 0;
    }

    if (qt == QT_PLUS) {
        /* a+ => a, branch back to atom_start, alt = past */
        if (ps->re->num_nodes + 2 > MAX_NODES) {
            ps->err = REG_ESPACE; return -1;
        }
        int br_idx = ps->re->num_nodes;
        memset(&ps->re->nodes[br_idx], 0, sizeof(struct regex_node));
        ps->re->nodes[br_idx].type = NODE_BRANCH;
        ps->re->nodes[br_idx].group = -1;
        ps->re->nodes[br_idx].alt = ps->re->num_nodes + 2;
        ps->re->nodes[br_idx].target = -1;
        ps->re->num_nodes++;

        int jmp_idx = ps->re->num_nodes;
        memset(&ps->re->nodes[jmp_idx], 0, sizeof(struct regex_node));
        ps->re->nodes[jmp_idx].type = NODE_JUMP;
        ps->re->nodes[jmp_idx].group = -1;
        ps->re->nodes[jmp_idx].alt = -1;
        ps->re->nodes[jmp_idx].target = atom_start;
        ps->re->num_nodes++;
        return 0;
    }

    if (qt == QT_RANGE) {
        /* {m,n}: duplicate atom m times mandatory, then up to (n-m) optional */
        if (qmin == 0 && qmax == 0) {
            /* {0,0} or {0}: match empty, remove atom */
            ps->re->num_nodes = atom_start;
            return 0;
        }

        /* Already have one copy from the atom.  Need (qmin - 1) more mandatory. */
        for (i = 1; i < qmin; i++) {
            if (dup_nodes(ps, atom_start, atom_end) < 0)
                return -1;
        }

        if (qmax < 0) {
            /* {m,} -- mandatory copies done, now add * loop on one copy */
            int loop_start = ps->re->num_nodes;
            if (dup_nodes(ps, atom_start, atom_end) < 0)
                return -1;
            return apply_quantifier(ps, loop_start, QT_STAR, 0, 0);
        }

        /* Add (qmax - qmin) optional copies */
        for (i = qmin; i < qmax; i++) {
            int opt_start = ps->re->num_nodes;
            if (dup_nodes(ps, atom_start, atom_end) < 0)
                return -1;
            if (apply_quantifier(ps, opt_start, QT_QUEST, 0, 0) != 0)
                return -1;
        }

        /* If qmin == 0, the original atom should also be optional */
        if (qmin == 0) {
            int cur_len = ps->re->num_nodes - atom_start;
            if (ps->re->num_nodes + 1 > MAX_NODES) {
                ps->err = REG_ESPACE; return -1;
            }
            memmove(&ps->re->nodes[atom_start + 1],
                    &ps->re->nodes[atom_start],
                    (size_t)cur_len * sizeof(struct regex_node));
            ps->re->num_nodes++;
            for (i = atom_start + 1; i < ps->re->num_nodes; i++) {
                if (ps->re->nodes[i].alt >= atom_start)
                    ps->re->nodes[i].alt++;
                if (ps->re->nodes[i].target >= atom_start)
                    ps->re->nodes[i].target++;
            }
            memset(&ps->re->nodes[atom_start], 0, sizeof(struct regex_node));
            ps->re->nodes[atom_start].type = NODE_BRANCH;
            ps->re->nodes[atom_start].group = -1;
            ps->re->nodes[atom_start].alt = ps->re->num_nodes;
            ps->re->nodes[atom_start].target = -1;
        }

        return 0;
    }

    #undef QT_QUEST
    #undef QT_STAR
    #undef QT_PLUS
    #undef QT_RANGE

    return 0;
}

/* Parse and compile a regex branch (sequence of atoms) up to | or ) or end.
 * Returns 0 on success, error code on failure. */
static int compile_branch(struct parse_state *ps);
static int compile_regex(struct parse_state *ps);

static int compile_atom(struct parse_state *ps)
{
    int is_ere = (ps->cflags & REG_EXTENDED) != 0;
    int c = peek(ps);

    if (c < 0)
        return 0;

    int atom_start = ps->re->num_nodes;

    /* Anchor: ^ */
    if (c == '^') {
        next_ch(ps);
        emit(ps, NODE_ANCHOR_START);
        return 0;
    }

    /* Anchor: $ */
    if (c == '$') {
        next_ch(ps);
        emit(ps, NODE_ANCHOR_END);
        return 0;
    }

    /* Character class: [...] */
    if (c == '[') {
        next_ch(ps);
        if (parse_class(ps) < 0)
            return ps->err;
        goto check_quantifier;
    }

    /* Dot: . */
    if (c == '.') {
        next_ch(ps);
        emit(ps, NODE_DOT);
        goto check_quantifier;
    }

    /* ERE: ( opens group */
    if (is_ere && c == '(') {
        next_ch(ps);
        int grp = ++(ps->group_count);
        if (grp >= MAX_GROUPS) {
            ps->err = REG_EPAREN;
            return ps->err;
        }
        int gs = emit(ps, NODE_GROUP_START);
        if (gs < 0) return ps->err;
        ps->re->nodes[gs].group = grp;

        if (compile_regex(ps) != 0)
            return ps->err;

        if (peek(ps) != ')') {
            ps->err = REG_EPAREN;
            return ps->err;
        }
        next_ch(ps);
        int ge = emit(ps, NODE_GROUP_END);
        if (ge < 0) return ps->err;
        ps->re->nodes[ge].group = grp;
        goto check_quantifier;
    }

    /* ERE: ) or | are not atoms -- return */
    if (is_ere && (c == ')' || c == '|'))
        return 0;

    /* ERE: standalone * + ? { at start of pattern is an error for + and ? */
    if (is_ere && (c == '*' || c == '+' || c == '?'))
        return 0;

    /* Backslash */
    if (c == '\\') {
        next_ch(ps);
        c = peek(ps);
        if (c < 0) {
            ps->err = REG_EESCAPE;
            return ps->err;
        }

        /* BRE: \( opens group */
        if (!is_ere && c == '(') {
            next_ch(ps);
            int grp = ++(ps->group_count);
            if (grp >= MAX_GROUPS) {
                ps->err = REG_EPAREN;
                return ps->err;
            }
            int gs = emit(ps, NODE_GROUP_START);
            if (gs < 0) return ps->err;
            ps->re->nodes[gs].group = grp;

            if (compile_regex(ps) != 0)
                return ps->err;

            if (peek(ps) != '\\' || (ps->pos + 1 < ps->len &&
                ps->pat[ps->pos + 1] != ')')) {
                ps->err = REG_EPAREN;
                return ps->err;
            }
            next_ch(ps); /* \\ */
            next_ch(ps); /* ) */
            int ge = emit(ps, NODE_GROUP_END);
            if (ge < 0) return ps->err;
            ps->re->nodes[ge].group = grp;
            goto check_quantifier;
        }

        /* BRE: \) ends group -- not an atom, return; put back the \ */
        if (!is_ere && c == ')') {
            ps->pos--;
            return 0;
        }

        /* BRE: \| alternation -- not an atom, return; put back the \ */
        if (!is_ere && c == '|') {
            ps->pos--;
            return 0;
        }

        /* BRE: \{ opens bounded repetition -- handled below in quantifier.
         * Put back the backslash so we don't consume it here. */
        if (!is_ere && c == '{') {
            ps->pos--;
            /* No atom was emitted; there's nothing to quantify.
             * Treat as literal backslash. */
            int idx = emit(ps, NODE_LITERAL);
            if (idx < 0) return ps->err;
            ps->re->nodes[idx].ch = (unsigned char)'\\';
            return 0;
        }

        /* Escaped literal */
        next_ch(ps);
        {
            int idx = emit(ps, NODE_LITERAL);
            if (idx < 0) return ps->err;
            if (ps->cflags & REG_ICASE)
                ps->re->nodes[idx].ch = (unsigned char)char_lower(c);
            else
                ps->re->nodes[idx].ch = (unsigned char)c;
        }
        goto check_quantifier;
    }

    /* Literal character */
    next_ch(ps);
    {
        int idx = emit(ps, NODE_LITERAL);
        if (idx < 0) return ps->err;
        if (ps->cflags & REG_ICASE)
            ps->re->nodes[idx].ch = (unsigned char)char_lower(c);
        else
            ps->re->nodes[idx].ch = (unsigned char)c;
    }

check_quantifier:
    if (ps->err)
        return ps->err;

    /* Check for quantifier */
    c = peek(ps);

    if (c == '*') {
        next_ch(ps);
        return apply_quantifier(ps, atom_start, 1 /*STAR*/, 0, 0);
    }

    if (is_ere && c == '+') {
        next_ch(ps);
        return apply_quantifier(ps, atom_start, 2 /*PLUS*/, 0, 0);
    }

    if (is_ere && c == '?') {
        next_ch(ps);
        return apply_quantifier(ps, atom_start, 0 /*QUEST*/, 0, 0);
    }

    /* ERE: { bounded repetition */
    if (is_ere && c == '{') {
        int save_pos = ps->pos;
        next_ch(ps); /* consume '{' */
        int qmin, qmax;
        if (parse_bound(ps, 0, &qmin, &qmax)) {
            return apply_quantifier(ps, atom_start, 3 /*RANGE*/, qmin, qmax);
        }
        /* Not a valid bound -- treat '{' as literal */
        ps->pos = save_pos;
        ps->err = 0;
    }

    /* BRE: \{m,n\} bounded repetition */
    if (!is_ere && c == '\\' && ps->pos + 1 < ps->len &&
        ps->pat[ps->pos + 1] == '{') {
        int save_pos = ps->pos;
        next_ch(ps); /* consume '\\' */
        next_ch(ps); /* consume '{' */
        int qmin, qmax;
        if (parse_bound(ps, 1, &qmin, &qmax)) {
            return apply_quantifier(ps, atom_start, 3 /*RANGE*/, qmin, qmax);
        }
        /* Not a valid bound -- rewind, treat as literal */
        ps->pos = save_pos;
        ps->err = 0;
    }

    return 0;
}

/* Compile a branch: a sequence of atoms (concatenation) */
static int compile_branch(struct parse_state *ps)
{
    int is_ere = (ps->cflags & REG_EXTENDED) != 0;

    while (ps->pos < ps->len && ps->err == 0) {
        int c = peek(ps);
        if (c < 0) break;

        /* End of branch in ERE: | or ) */
        if (is_ere && (c == '|' || c == ')'))
            break;

        /* End of branch in BRE: \| or \) */
        if (!is_ere && c == '\\' && ps->pos + 1 < ps->len) {
            char nc = ps->pat[ps->pos + 1];
            if (nc == ')' || nc == '|')
                break;
        }

        int prev_pos = ps->pos;
        int prev_nodes = ps->re->num_nodes;
        if (compile_atom(ps) != 0)
            return ps->err;

        /* If no progress was made, break to avoid infinite loop */
        if (ps->re->num_nodes == prev_nodes && ps->pos == prev_pos)
            break;
    }

    return 0;
}

/* Compile a full regex: alternation of branches */
static int compile_regex(struct parse_state *ps)
{
    int is_ere = (ps->cflags & REG_EXTENDED) != 0;

    int start = ps->re->num_nodes;
    if (compile_branch(ps) != 0)
        return ps->err;

    /* Check for alternation */
    while (ps->err == 0) {
        int c = peek(ps);
        int is_alt = 0;

        if (is_ere && c == '|') {
            next_ch(ps);
            is_alt = 1;
        } else if (!is_ere && c == '\\' && ps->pos + 1 < ps->len &&
                   ps->pat[ps->pos + 1] == '|') {
            next_ch(ps); /* \\ */
            next_ch(ps); /* | */
            is_alt = 1;
        }

        if (!is_alt)
            break;

        /* We need to wrap the previous branch in BRANCH...JUMP */
        int branch_end = ps->re->num_nodes;
        int branch_len = branch_end - start;

        /* Make room for a BRANCH at 'start' and a JUMP at 'branch_end+1' */
        if (ps->re->num_nodes + 2 > MAX_NODES) {
            ps->err = REG_ESPACE;
            return ps->err;
        }

        /* Shift existing nodes right by 1 for the BRANCH */
        memmove(&ps->re->nodes[start + 1],
                &ps->re->nodes[start],
                (size_t)branch_len * sizeof(struct regex_node));
        ps->re->num_nodes++;

        /* Fix up all references that point at or past 'start' */
        int i;
        for (i = 0; i < ps->re->num_nodes; i++) {
            if (i == start) continue;
            if (ps->re->nodes[i].alt >= start)
                ps->re->nodes[i].alt++;
            if (ps->re->nodes[i].target >= start)
                ps->re->nodes[i].target++;
        }

        /* Add JUMP at end of first branch */
        int jmp_idx = ps->re->num_nodes;
        if (jmp_idx >= MAX_NODES) { ps->err = REG_ESPACE; return ps->err; }
        memset(&ps->re->nodes[jmp_idx], 0, sizeof(struct regex_node));
        ps->re->nodes[jmp_idx].type = NODE_JUMP;
        ps->re->nodes[jmp_idx].group = -1;
        ps->re->nodes[jmp_idx].alt = -1;
        ps->re->nodes[jmp_idx].target = -1; /* patched below */
        ps->re->num_nodes++;

        /* BRANCH at 'start': alt points to after the JUMP (the alternate) */
        memset(&ps->re->nodes[start], 0, sizeof(struct regex_node));
        ps->re->nodes[start].type = NODE_BRANCH;
        ps->re->nodes[start].group = -1;
        ps->re->nodes[start].alt = ps->re->num_nodes; /* next branch starts here */
        ps->re->nodes[start].target = -1;

        /* Compile the alternate branch */
        int alt_start = ps->re->num_nodes;
        if (compile_branch(ps) != 0)
            return ps->err;

        /* Patch JUMP to point past all alternates */
        ps->re->nodes[jmp_idx].target = ps->re->num_nodes;

        /* For subsequent alternations, 'start' moves to alt_start */
        start = alt_start;
    }

    return 0;
}

/* ========================================================================= */
/* regcomp: compile a regular expression                                     */
/* ========================================================================= */

int regcomp(regex_t *preg, const char *pattern, int cflags)
{
    if (!preg || !pattern)
        return REG_BADPAT;

    struct compiled_regex *re = (struct compiled_regex *)malloc(sizeof(*re));
    if (!re)
        return REG_ESPACE;

    memset(re, 0, sizeof(*re));
    re->cflags = cflags;

    struct parse_state ps;
    memset(&ps, 0, sizeof(ps));
    ps.pat = pattern;
    ps.pos = 0;
    ps.len = (int)strlen(pattern);
    ps.cflags = cflags;
    ps.re = re;
    ps.group_count = 0;
    ps.err = 0;

    compile_regex(&ps);

    if (ps.err) {
        /* Free any allocated class bitmaps */
        int i;
        for (i = 0; i < re->num_nodes; i++) {
            if (re->nodes[i].type == NODE_CLASS && re->nodes[i].cclass)
                free(re->nodes[i].cclass);
        }
        free(re);
        preg->re_nsub = 0;
        preg->__internal = NULL;
        return ps.err;
    }

    /* Emit final MATCH node */
    if (re->num_nodes < MAX_NODES) {
        memset(&re->nodes[re->num_nodes], 0, sizeof(struct regex_node));
        re->nodes[re->num_nodes].type = NODE_MATCH;
        re->nodes[re->num_nodes].group = -1;
        re->nodes[re->num_nodes].alt = -1;
        re->nodes[re->num_nodes].target = -1;
        re->num_nodes++;
    }

    re->num_groups = ps.group_count;
    preg->re_nsub = (size_t)ps.group_count;
    preg->__internal = re;

    return 0;
}

/* ========================================================================= */
/* Match engine: recursive backtracking NFA                                  */
/* ========================================================================= */

struct match_state {
    const char            *str;
    int                    str_len;
    int                    eflags;
    int                    cflags;
    struct compiled_regex  *re;
    /* Group capture positions */
    int                    group_start[MAX_GROUPS + 1];
    int                    group_end[MAX_GROUPS + 1];
    int                    num_groups;
    /* Recursion depth guard */
    int                    depth;
};

/* Try to match starting at node 'ni' against str at position 'si'.
 * Returns the end position on success, or -1 on failure. */
static int match_here(struct match_state *ms, int ni, int si)
{
    if (ms->depth++ > MAX_DEPTH) {
        ms->depth--;
        return -1;  /* Stack overflow protection */
    }

    while (ni < ms->re->num_nodes) {
        struct regex_node *nd = &ms->re->nodes[ni];

        switch (nd->type) {
        case NODE_MATCH:
            ms->depth--;
            return si;

        case NODE_LITERAL: {
            if (si >= ms->str_len) {
                ms->depth--;
                return -1;
            }
            unsigned char sc = (unsigned char)ms->str[si];
            if (ms->cflags & REG_ICASE)
                sc = (unsigned char)char_lower(sc);
            if (sc != nd->ch) {
                ms->depth--;
                return -1;
            }
            si++;
            ni++;
            break;
        }

        case NODE_DOT: {
            if (si >= ms->str_len) {
                ms->depth--;
                return -1;
            }
            char sc = ms->str[si];
            /* REG_NEWLINE: . doesn't match \n */
            if ((ms->cflags & REG_NEWLINE) && sc == '\n') {
                ms->depth--;
                return -1;
            }
            si++;
            ni++;
            break;
        }

        case NODE_CLASS: {
            if (si >= ms->str_len) {
                ms->depth--;
                return -1;
            }
            unsigned char sc = (unsigned char)ms->str[si];
            /* REG_NEWLINE: [^...] doesn't match \n */
            if ((ms->cflags & REG_NEWLINE) && nd->negate && sc == '\n') {
                ms->depth--;
                return -1;
            }
            int in_class = class_test(nd->cclass, sc);
            if (ms->cflags & REG_ICASE) {
                in_class = in_class || class_test(nd->cclass,
                    (unsigned char)char_lower(sc)) ||
                    class_test(nd->cclass, (unsigned char)char_upper(sc));
            }
            if (nd->negate)
                in_class = !in_class;
            if (!in_class) {
                ms->depth--;
                return -1;
            }
            si++;
            ni++;
            break;
        }

        case NODE_ANCHOR_START: {
            int at_start = 0;
            if (si == 0 && !(ms->eflags & REG_NOTBOL))
                at_start = 1;
            /* REG_NEWLINE: ^ matches after \n */
            if ((ms->cflags & REG_NEWLINE) && si > 0 &&
                ms->str[si - 1] == '\n')
                at_start = 1;
            if (!at_start) {
                ms->depth--;
                return -1;
            }
            ni++;
            break;
        }

        case NODE_ANCHOR_END: {
            int at_end = 0;
            if (si == ms->str_len && !(ms->eflags & REG_NOTEOL))
                at_end = 1;
            /* REG_NEWLINE: $ matches before \n */
            if ((ms->cflags & REG_NEWLINE) && si < ms->str_len &&
                ms->str[si] == '\n')
                at_end = 1;
            if (!at_end) {
                ms->depth--;
                return -1;
            }
            ni++;
            break;
        }

        case NODE_GROUP_START: {
            int g = nd->group;
            int old_start = -1;
            if (g >= 0 && g <= MAX_GROUPS)
                old_start = ms->group_start[g];
            if (g >= 0 && g <= MAX_GROUPS)
                ms->group_start[g] = si;

            int result = match_here(ms, ni + 1, si);
            if (result >= 0) {
                ms->depth--;
                return result;
            }

            /* Backtrack */
            if (g >= 0 && g <= MAX_GROUPS)
                ms->group_start[g] = old_start;
            ms->depth--;
            return -1;
        }

        case NODE_GROUP_END: {
            int g = nd->group;
            int old_end = -1;
            if (g >= 0 && g <= MAX_GROUPS)
                old_end = ms->group_end[g];
            if (g >= 0 && g <= MAX_GROUPS)
                ms->group_end[g] = si;

            int result = match_here(ms, ni + 1, si);
            if (result >= 0) {
                ms->depth--;
                return result;
            }

            /* Backtrack */
            if (g >= 0 && g <= MAX_GROUPS)
                ms->group_end[g] = old_end;
            ms->depth--;
            return -1;
        }

        case NODE_BRANCH: {
            /* Try the first alternative (ni+1) */
            int result = match_here(ms, ni + 1, si);
            if (result >= 0) {
                ms->depth--;
                return result;
            }
            /* Try the second alternative (nd->alt) */
            if (nd->alt >= 0)
                ni = nd->alt;
            else {
                ms->depth--;
                return -1;
            }
            break;
        }

        case NODE_JUMP: {
            if (nd->target >= 0)
                ni = nd->target;
            else
                ni++;
            break;
        }

        default:
            ms->depth--;
            return -1;
        }
    }

    ms->depth--;
    return -1;
}

/* ========================================================================= */
/* regexec: execute a compiled regular expression                            */
/* ========================================================================= */

int regexec(const regex_t *preg, const char *string,
            size_t nmatch, regmatch_t pmatch[], int eflags)
{
    if (!preg || !preg->__internal || !string)
        return REG_NOMATCH;

    struct compiled_regex *re = (struct compiled_regex *)preg->__internal;

    struct match_state ms;
    memset(&ms, 0, sizeof(ms));
    ms.str = string;
    ms.str_len = (int)strlen(string);
    ms.eflags = eflags;
    ms.cflags = re->cflags;
    ms.re = re;
    ms.num_groups = re->num_groups;
    ms.depth = 0;

    int i;

    /* Try matching at each position in the string */
    int si;
    for (si = 0; si <= ms.str_len; si++) {
        /* Reset group captures for this attempt */
        for (i = 0; i <= MAX_GROUPS; i++) {
            ms.group_start[i] = -1;
            ms.group_end[i] = -1;
        }
        ms.depth = 0;

        int end = match_here(&ms, 0, si);
        if (end >= 0) {
            /* Match found at [si, end) */
            if (pmatch && nmatch > 0 &&
                !(re->cflags & REG_NOSUB)) {
                pmatch[0].rm_so = si;
                pmatch[0].rm_eo = end;

                /* Fill in group matches */
                for (i = 1; i < (int)nmatch; i++) {
                    if (i <= re->num_groups) {
                        pmatch[i].rm_so = ms.group_start[i];
                        pmatch[i].rm_eo = ms.group_end[i];
                    } else {
                        pmatch[i].rm_so = -1;
                        pmatch[i].rm_eo = -1;
                    }
                }
            }
            return 0;
        }
    }

    return REG_NOMATCH;
}

/* ========================================================================= */
/* regfree: free a compiled regular expression                               */
/* ========================================================================= */

void regfree(regex_t *preg)
{
    if (!preg || !preg->__internal)
        return;

    struct compiled_regex *re = (struct compiled_regex *)preg->__internal;

    /* Free character class bitmaps */
    int i;
    for (i = 0; i < re->num_nodes; i++) {
        if (re->nodes[i].type == NODE_CLASS && re->nodes[i].cclass) {
            free(re->nodes[i].cclass);
            re->nodes[i].cclass = NULL;
        }
    }

    free(re);
    preg->__internal = NULL;
    preg->re_nsub = 0;
}

/* ========================================================================= */
/* regerror: format an error message                                         */
/* ========================================================================= */

size_t regerror(int errcode, const regex_t *preg,
                char *errbuf, size_t errbuf_size)
{
    (void)preg;

    const char *msg;

    switch (errcode) {
    case 0:            msg = "Success"; break;
    case REG_NOMATCH:  msg = "No match"; break;
    case REG_BADPAT:   msg = "Invalid regular expression"; break;
    case REG_ECOLLATE: msg = "Invalid collating element"; break;
    case REG_ECTYPE:   msg = "Invalid character class name"; break;
    case REG_EESCAPE:  msg = "Trailing backslash"; break;
    case REG_ESUBREG:  msg = "Invalid back reference"; break;
    case REG_EBRACK:   msg = "Unmatched [ or [^"; break;
    case REG_EPAREN:   msg = "Unmatched ( or \\("; break;
    case REG_EBRACE:   msg = "Unmatched \\{"; break;
    case REG_BADBR:    msg = "Invalid content of \\{\\}"; break;
    case REG_ERANGE:   msg = "Invalid range end"; break;
    case REG_ESPACE:   msg = "Out of memory"; break;
    case REG_BADRPT:   msg = "Invalid preceding regular expression"; break;
    case REG_NOSYS:    msg = "Function not implemented"; break;
    default:           msg = "Unknown error"; break;
    }

    size_t len = strlen(msg) + 1;
    if (errbuf && errbuf_size > 0) {
        size_t copy = (len < errbuf_size) ? len : errbuf_size;
        memcpy(errbuf, msg, copy - 1);
        errbuf[copy - 1] = '\0';
    }
    return len;
}
