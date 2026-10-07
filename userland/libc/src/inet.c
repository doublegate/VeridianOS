/*
 * VeridianOS libc -- inet.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Internet address conversion (<arpa/inet.h>):
 *   inet_aton / inet_addr  -- the BSD numbers-and-dots forms (1 to 4 parts,
 *                             each decimal, octal with 0 or hex with 0x)
 *   inet_ntoa              -- dotted decimal into a static buffer
 *   inet_pton / inet_ntop  -- AF_INET (strict dotted decimal) and AF_INET6
 *                             (RFC 4291 text forms; output per RFC 5952,
 *                             with the embedded IPv4 forms glibc prints)
 *
 * Addresses are kept as byte arrays in network order, so nothing here
 * depends on the host byte order.
 */

#include <arpa/inet.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>

static int is_space(char c)
{
    return c == ' ' || (c >= '\t' && c <= '\r');
}

static int hex_value(char c)
{
    if (c >= '0' && c <= '9')
        return c - '0';
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    if (c >= 'A' && c <= 'F')
        return c - 'A' + 10;
    return -1;
}

int inet_aton(const char *cp, struct in_addr *inp)
{
    unsigned long parts[4];
    int n = 0;

    if (cp == NULL)
        return 0;
    for (;;) {
        unsigned long v = 0;
        int base = 10, digits = 0;

        if (*cp == '0') {
            base = 8;
            cp++;
            digits = 1;  /* "0" alone is a valid zero */
            if (*cp == 'x' || *cp == 'X') {
                base = 16;
                cp++;
                digits = 0;
            }
        }
        for (;; cp++) {
            int d = hex_value(*cp);
            if (d < 0 || d >= base)
                break;
            v = v * (unsigned long)base + (unsigned long)d;
            if (v > 0xffffffffUL)
                return 0;
            digits = 1;
        }
        if (!digits)
            return 0;
        parts[n++] = v;
        if (*cp != '.')
            break;
        if (n == 4)
            return 0;
        cp++;
    }
    /* Anything after the address must be whitespace (as in BSD/glibc) */
    if (*cp != '\0' && !is_space(*cp))
        return 0;

    unsigned long addr;
    switch (n) {
    case 1:  /* a: 32 bits */
        addr = parts[0];
        break;
    case 2:  /* a.b: 8.24 */
        if (parts[0] > 0xff || parts[1] > 0xffffff)
            return 0;
        addr = parts[0] << 24 | parts[1];
        break;
    case 3:  /* a.b.c: 8.8.16 */
        if (parts[0] > 0xff || parts[1] > 0xff || parts[2] > 0xffff)
            return 0;
        addr = parts[0] << 24 | parts[1] << 16 | parts[2];
        break;
    default: /* a.b.c.d */
        if (parts[0] > 0xff || parts[1] > 0xff || parts[2] > 0xff ||
            parts[3] > 0xff)
            return 0;
        addr = parts[0] << 24 | parts[1] << 16 | parts[2] << 8 | parts[3];
        break;
    }
    if (inp) {
        unsigned char *b = (unsigned char *)&inp->s_addr;
        b[0] = (unsigned char)(addr >> 24);
        b[1] = (unsigned char)(addr >> 16);
        b[2] = (unsigned char)(addr >> 8);
        b[3] = (unsigned char)addr;
    }
    return 1;
}

in_addr_t inet_addr(const char *cp)
{
    struct in_addr a;

    if (!inet_aton(cp, &a))
        return INADDR_NONE;
    return a.s_addr;
}

char *inet_ntoa(struct in_addr in)
{
    static char buf[16];
    const unsigned char *b = (const unsigned char *)&in.s_addr;

    snprintf(buf, sizeof(buf), "%u.%u.%u.%u", b[0], b[1], b[2], b[3]);
    return buf;
}

/* Strict dotted decimal: four parts 0..255, no leading zeros. */
static int pton4(const char *src, unsigned char *dst)
{
    unsigned char out[4];
    int parts = 0;

    while (parts < 4) {
        unsigned int v = 0;
        int digits = 0;

        while (*src >= '0' && *src <= '9') {
            if (digits > 0 && v == 0)
                return 0;  /* leading zero */
            v = v * 10 + (unsigned int)(*src++ - '0');
            if (v > 255)
                return 0;
            digits++;
        }
        if (digits == 0)
            return 0;
        out[parts++] = (unsigned char)v;
        if (parts < 4) {
            if (*src != '.')
                return 0;
            src++;
        }
    }
    if (*src != '\0')
        return 0;
    memcpy(dst, out, 4);
    return 1;
}

/* RFC 4291 section 2.2: eight groups, one "::", optional trailing IPv4. */
static int pton6(const char *src, unsigned char *dst)
{
    unsigned char out[16];
    int len = 0, gap = -1;
    const char *group;

    memset(out, 0, sizeof(out));
    if (*src == ':') {
        if (*++src != ':')
            return 0;
    }
    group = src;
    for (;;) {
        unsigned int v = 0;
        int digits = 0;

        if (*src == ':') {
            /* "::" -- at most one */
            if (gap >= 0)
                return 0;
            gap = len;
            src++;
            group = src;
            if (*src == '\0')
                break;
            continue;
        }
        while (hex_value(*src) >= 0) {
            v = v << 4 | (unsigned int)hex_value(*src++);
            if (++digits > 4)
                return 0;
        }
        if (*src == '.' && len <= 12) {
            /* Trailing dotted IPv4 in the last 32 bits */
            if (!pton4(group, out + len))
                return 0;
            len += 4;
            break;
        }
        if (digits == 0 || len > 14)
            return 0;
        out[len++] = (unsigned char)(v >> 8);
        out[len++] = (unsigned char)v;
        if (*src == '\0')
            break;
        if (*src != ':')
            return 0;
        src++;
        group = src;
        if (*src == '\0')
            return 0;  /* trailing single ':' */
    }
    if (gap >= 0) {
        if (len == 16)
            return 0;  /* "::" must stand for at least one group */
        memmove(out + 16 - (len - gap), out + gap, (size_t)(len - gap));
        memset(out + gap, 0, (size_t)(16 - len));
    } else if (len != 16) {
        return 0;
    }
    memcpy(dst, out, 16);
    return 1;
}

int inet_pton(int af, const char *src, void *dst)
{
    if (src == NULL || dst == NULL) {
        errno = EINVAL;
        return -1;
    }
    switch (af) {
    case AF_INET:
        return pton4(src, dst);
    case AF_INET6:
        return pton6(src, dst);
    default:
        errno = EAFNOSUPPORT;
        return -1;
    }
}

static const char *ntop4(const unsigned char *a, char *dst, socklen_t size)
{
    char tmp[16];
    int n = snprintf(tmp, sizeof(tmp), "%u.%u.%u.%u", a[0], a[1], a[2], a[3]);

    if (n < 0 || (socklen_t)n >= size) {
        errno = ENOSPC;
        return NULL;
    }
    memcpy(dst, tmp, (size_t)n + 1);
    return dst;
}

static const char *ntop6(const unsigned char *a, char *dst, socklen_t size)
{
    char tmp[46], *p = tmp;
    unsigned int w[8];
    int best = -1, best_len = 0, cur = -1, cur_len = 0;

    for (int i = 0; i < 8; i++)
        w[i] = (unsigned int)a[2 * i] << 8 | a[2 * i + 1];
    /* Longest run of two or more zero groups, leftmost on a tie */
    for (int i = 0; i < 8; i++) {
        if (w[i] == 0) {
            if (cur < 0) {
                cur = i;
                cur_len = 0;
            }
            if (++cur_len > best_len) {
                best = cur;
                best_len = cur_len;
            }
        } else {
            cur = -1;
        }
    }
    if (best_len < 2)
        best = -1;

    for (int i = 0; i < 8; i++) {
        if (best >= 0 && i >= best && i < best + best_len) {
            if (i == best)
                *p++ = ':';
            continue;
        }
        if (i > 0)
            *p++ = ':';
        /* IPv4-compatible (::a.b.c.d) and IPv4-mapped (::ffff:a.b.c.d) */
        if (i == 6 && best == 0 &&
            (best_len == 6 || (best_len == 5 && w[5] == 0xffff))) {
            p += snprintf(p, sizeof(tmp) - (size_t)(p - tmp), "%u.%u.%u.%u",
                          a[12], a[13], a[14], a[15]);
            break;
        }
        p += snprintf(p, sizeof(tmp) - (size_t)(p - tmp), "%x", w[i]);
    }
    if (best >= 0 && best + best_len == 8)
        *p++ = ':';
    *p = '\0';

    if ((socklen_t)(p - tmp) >= size) {
        errno = ENOSPC;
        return NULL;
    }
    memcpy(dst, tmp, (size_t)(p - tmp) + 1);
    return dst;
}

const char *inet_ntop(int af, const void *src, char *dst, socklen_t size)
{
    if (src == NULL || dst == NULL) {
        errno = EINVAL;
        return NULL;
    }
    switch (af) {
    case AF_INET:
        return ntop4(src, dst, size);
    case AF_INET6:
        return ntop6(src, dst, size);
    default:
        errno = EAFNOSUPPORT;
        return NULL;
    }
}
