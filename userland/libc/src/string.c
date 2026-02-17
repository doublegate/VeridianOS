/*
 * VeridianOS libc -- string.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * String and memory manipulation functions.
 * All implementations are freestanding -- no host libc dependency.
 */

#include <string.h>
#include <stdlib.h>
#include <errno.h>
#include <ctype.h>

/* ========================================================================= */
/* Memory operations                                                         */
/* ========================================================================= */

void *memcpy(void *dest, const void *src, size_t n)
{
    unsigned char *d = (unsigned char *)dest;
    const unsigned char *s = (const unsigned char *)src;
    while (n--)
        *d++ = *s++;
    return dest;
}

void *memmove(void *dest, const void *src, size_t n)
{
    unsigned char *d = (unsigned char *)dest;
    const unsigned char *s = (const unsigned char *)src;

    if (d < s) {
        /* Forward copy. */
        while (n--)
            *d++ = *s++;
    } else if (d > s) {
        /* Backward copy to handle overlap. */
        d += n;
        s += n;
        while (n--)
            *--d = *--s;
    }
    return dest;
}

void *memset(void *dest, int c, size_t n)
{
    unsigned char *d = (unsigned char *)dest;
    while (n--)
        *d++ = (unsigned char)c;
    return dest;
}

int memcmp(const void *s1, const void *s2, size_t n)
{
    const unsigned char *a = (const unsigned char *)s1;
    const unsigned char *b = (const unsigned char *)s2;
    while (n--) {
        if (*a != *b)
            return *a - *b;
        a++;
        b++;
    }
    return 0;
}

void *memchr(const void *s, int c, size_t n)
{
    const unsigned char *p = (const unsigned char *)s;
    unsigned char uc = (unsigned char)c;
    while (n--) {
        if (*p == uc)
            return (void *)p;
        p++;
    }
    return NULL;
}

/* ========================================================================= */
/* String length                                                             */
/* ========================================================================= */

size_t strlen(const char *s)
{
    const char *p = s;
    while (*p)
        p++;
    return (size_t)(p - s);
}

size_t strnlen(const char *s, size_t maxlen)
{
    const char *p = s;
    while (maxlen-- && *p)
        p++;
    return (size_t)(p - s);
}

/* ========================================================================= */
/* String comparison                                                         */
/* ========================================================================= */

int strcmp(const char *s1, const char *s2)
{
    while (*s1 && *s1 == *s2) {
        s1++;
        s2++;
    }
    return (unsigned char)*s1 - (unsigned char)*s2;
}

int strncmp(const char *s1, const char *s2, size_t n)
{
    while (n && *s1 && *s1 == *s2) {
        s1++;
        s2++;
        n--;
    }
    if (n == 0)
        return 0;
    return (unsigned char)*s1 - (unsigned char)*s2;
}

/* ========================================================================= */
/* String copy                                                               */
/* ========================================================================= */

char *strcpy(char *dest, const char *src)
{
    char *d = dest;
    while ((*d++ = *src++))
        ;
    return dest;
}

char *strncpy(char *dest, const char *src, size_t n)
{
    char *d = dest;
    while (n && (*d = *src)) {
        d++;
        src++;
        n--;
    }
    /* Pad remainder with NULs. */
    while (n--)
        *d++ = '\0';
    return dest;
}

/* ========================================================================= */
/* String concatenation                                                      */
/* ========================================================================= */

char *strcat(char *dest, const char *src)
{
    char *d = dest;
    while (*d)
        d++;
    while ((*d++ = *src++))
        ;
    return dest;
}

char *strncat(char *dest, const char *src, size_t n)
{
    char *d = dest;
    while (*d)
        d++;
    while (n-- && *src)
        *d++ = *src++;
    *d = '\0';
    return dest;
}

/* ========================================================================= */
/* String searching                                                          */
/* ========================================================================= */

char *strchr(const char *s, int c)
{
    char ch = (char)c;
    while (*s) {
        if (*s == ch)
            return (char *)s;
        s++;
    }
    /* strchr must also find the terminating NUL if c == '\0'. */
    return ch == '\0' ? (char *)s : NULL;
}

char *strrchr(const char *s, int c)
{
    const char *last = NULL;
    char ch = (char)c;
    while (*s) {
        if (*s == ch)
            last = s;
        s++;
    }
    if (ch == '\0')
        return (char *)s;
    return (char *)last;
}

char *strstr(const char *haystack, const char *needle)
{
    if (*needle == '\0')
        return (char *)haystack;

    size_t nlen = strlen(needle);
    while (*haystack) {
        if (*haystack == *needle && strncmp(haystack, needle, nlen) == 0)
            return (char *)haystack;
        haystack++;
    }
    return NULL;
}

size_t strspn(const char *s, const char *accept)
{
    const char *p = s;
    while (*p) {
        const char *a = accept;
        int found = 0;
        while (*a) {
            if (*p == *a) {
                found = 1;
                break;
            }
            a++;
        }
        if (!found)
            break;
        p++;
    }
    return (size_t)(p - s);
}

size_t strcspn(const char *s, const char *reject)
{
    const char *p = s;
    while (*p) {
        const char *r = reject;
        while (*r) {
            if (*p == *r)
                return (size_t)(p - s);
            r++;
        }
        p++;
    }
    return (size_t)(p - s);
}

char *strdup(const char *s)
{
    size_t len = strlen(s) + 1;
    char *d = (char *)malloc(len);
    if (d)
        memcpy(d, s, len);
    return d;
}

char *strndup(const char *s, size_t n)
{
    size_t len = strnlen(s, n);
    char *d = (char *)malloc(len + 1);
    if (d) {
        memcpy(d, s, len);
        d[len] = '\0';
    }
    return d;
}

char *strpbrk(const char *s, const char *accept)
{
    while (*s) {
        const char *a = accept;
        while (*a) {
            if (*s == *a)
                return (char *)s;
            a++;
        }
        s++;
    }
    return NULL;
}

/* ========================================================================= */
/* Case-insensitive comparison                                               */
/* ========================================================================= */

int strcasecmp(const char *s1, const char *s2)
{
    while (*s1 && *s2) {
        int c1 = tolower((unsigned char)*s1);
        int c2 = tolower((unsigned char)*s2);
        if (c1 != c2)
            return c1 - c2;
        s1++;
        s2++;
    }
    return tolower((unsigned char)*s1) - tolower((unsigned char)*s2);
}

int strncasecmp(const char *s1, const char *s2, size_t n)
{
    while (n && *s1 && *s2) {
        int c1 = tolower((unsigned char)*s1);
        int c2 = tolower((unsigned char)*s2);
        if (c1 != c2)
            return c1 - c2;
        s1++;
        s2++;
        n--;
    }
    if (n == 0)
        return 0;
    return tolower((unsigned char)*s1) - tolower((unsigned char)*s2);
}

/* ========================================================================= */
/* Extended memory operations                                                */
/* ========================================================================= */

void *mempcpy(void *dest, const void *src, size_t n)
{
    memcpy(dest, src, n);
    return (char *)dest + n;
}

void *memmem(const void *haystack, size_t haystacklen,
             const void *needle, size_t needlelen)
{
    if (needlelen == 0)
        return (void *)haystack;
    if (needlelen > haystacklen)
        return NULL;

    const unsigned char *h = (const unsigned char *)haystack;
    const unsigned char *n = (const unsigned char *)needle;

    for (size_t i = 0; i <= haystacklen - needlelen; i++) {
        if (h[i] == n[0] && memcmp(h + i, n, needlelen) == 0)
            return (void *)(h + i);
    }
    return NULL;
}

int strerror_r(int errnum, char *buf, size_t buflen)
{
    const char *msg = strerror(errnum);
    size_t len = strlen(msg);
    if (len >= buflen) {
        if (buflen > 0) {
            memcpy(buf, msg, buflen - 1);
            buf[buflen - 1] = '\0';
        }
        return ERANGE;
    }
    memcpy(buf, msg, len + 1);
    return 0;
}

/* ========================================================================= */
/* Tokenization                                                              */
/* ========================================================================= */

char *strtok(char *str, const char *delim)
{
    static char *saved = NULL;

    if (str)
        saved = str;

    if (!saved)
        return NULL;

    /* Skip leading delimiters. */
    saved += strspn(saved, delim);
    if (*saved == '\0') {
        saved = NULL;
        return NULL;
    }

    /* Find end of token. */
    char *token = saved;
    saved += strcspn(saved, delim);
    if (*saved) {
        *saved = '\0';
        saved++;
    } else {
        saved = NULL;
    }
    return token;
}

/* ========================================================================= */
/* Error string                                                              */
/* ========================================================================= */

char *strerror(int errnum)
{
    switch (errnum) {
    case 0:         return (char *)"Success";
    case ENOSYS:    return (char *)"Function not implemented";
    case EINVAL:    return (char *)"Invalid argument";
    case EPERM:     return (char *)"Operation not permitted";
    case ENOENT:    return (char *)"No such file or directory";
    case ENOMEM:    return (char *)"Cannot allocate memory";
    case EAGAIN:    return (char *)"Resource temporarily unavailable";
    case EINTR:     return (char *)"Interrupted system call";
    case EFAULT:    return (char *)"Bad address";
    case EACCES:    return (char *)"Permission denied";
    case ESRCH:     return (char *)"No such process";
    default:        return (char *)"Unknown error";
    }
}

/* ========================================================================= */
/* Number parsing (strtol, atoi)                                             */
/* ========================================================================= */

long strtol(const char *nptr, char **endptr, int base)
{
    const char *s = nptr;
    long result = 0;
    int neg = 0;

    /* Skip whitespace. */
    while (isspace((unsigned char)*s))
        s++;

    /* Optional sign. */
    if (*s == '-') {
        neg = 1;
        s++;
    } else if (*s == '+') {
        s++;
    }

    /* Auto-detect base. */
    if (base == 0) {
        if (*s == '0') {
            s++;
            if (*s == 'x' || *s == 'X') {
                base = 16;
                s++;
            } else {
                base = 8;
            }
        } else {
            base = 10;
        }
    } else if (base == 16) {
        /* Skip optional 0x/0X prefix. */
        if (*s == '0' && (s[1] == 'x' || s[1] == 'X'))
            s += 2;
    }

    while (*s) {
        int digit;
        if (*s >= '0' && *s <= '9')
            digit = *s - '0';
        else if (*s >= 'A' && *s <= 'Z')
            digit = *s - 'A' + 10;
        else if (*s >= 'a' && *s <= 'z')
            digit = *s - 'a' + 10;
        else
            break;

        if (digit >= base)
            break;

        result = result * base + digit;
        s++;
    }

    if (endptr)
        *endptr = (char *)s;

    return neg ? -result : result;
}

unsigned long strtoul(const char *nptr, char **endptr, int base)
{
    /* Reuse strtol logic; cast handles the unsigned case for simple use. */
    return (unsigned long)strtol(nptr, endptr, base);
}

long long strtoll(const char *nptr, char **endptr, int base)
{
    /* On 64-bit platforms, long == long long.  Delegate to strtol. */
    return (long long)strtol(nptr, endptr, base);
}

unsigned long long strtoull(const char *nptr, char **endptr, int base)
{
    return (unsigned long long)strtoul(nptr, endptr, base);
}

int atoi(const char *nptr)
{
    return (int)strtol(nptr, NULL, 10);
}

long atol(const char *nptr)
{
    return strtol(nptr, NULL, 10);
}
