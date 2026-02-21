/*
 * VeridianOS libc -- Additional POSIX function stubs
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Stub implementations for POSIX functions required by Ninja build
 * system and other cross-compiled C++ programs.
 */

#include <stddef.h>
#include <errno.h>

/* Forward declarations to avoid pulling in full headers */
typedef unsigned long sigset_t;
typedef long pid_t;
typedef unsigned int mode_t;

/* ========================================================================= */
/* putchar                                                                   */
/* ========================================================================= */

extern long write(int fd, const void *buf, unsigned long count);

int putchar(int c)
{
    unsigned char ch = (unsigned char)c;
    if (write(1, &ch, 1) != 1)
        return -1;
    return c;
}

/* ========================================================================= */
/* sysconf                                                                   */
/* ========================================================================= */

#define _SC_NPROCESSORS_ONLN  84
#define _SC_PAGESIZE          30

long sysconf(int name)
{
    switch (name) {
    case _SC_NPROCESSORS_ONLN:
        return 1; /* Single CPU for now */
    case _SC_PAGESIZE:
        return 4096;
    default:
        return -1;
    }
}

/* ========================================================================= */
/* getloadavg                                                                */
/* ========================================================================= */

int getloadavg(double loadavg[], int nelem)
{
    int i;
    if (nelem <= 0) return 0;
    if (nelem > 3) nelem = 3;
    for (i = 0; i < nelem; i++)
        loadavg[i] = 0.0;
    return nelem;
}

/* ========================================================================= */
/* sigpending                                                                */
/* ========================================================================= */

int sigpending(sigset_t *set)
{
    if (set) *set = 0;
    return 0;
}

/* ========================================================================= */
/* posix_spawn and related functions                                         */
/* ========================================================================= */

typedef struct { int _flags; } posix_spawnattr_t;
typedef struct { int _a; int _u; void *_p; } posix_spawn_file_actions_t;

int posix_spawn(pid_t *pid, const char *path,
                const posix_spawn_file_actions_t *fa,
                const posix_spawnattr_t *attr,
                char *const argv[], char *const envp[])
{
    (void)pid; (void)path; (void)fa; (void)attr;
    (void)argv; (void)envp;
    /* Stub: spawning not supported yet */
    return 38; /* ENOSYS */
}

int posix_spawnp(pid_t *pid, const char *file,
                 const posix_spawn_file_actions_t *fa,
                 const posix_spawnattr_t *attr,
                 char *const argv[], char *const envp[])
{
    (void)pid; (void)file; (void)fa; (void)attr;
    (void)argv; (void)envp;
    return 38; /* ENOSYS */
}

int posix_spawnattr_init(posix_spawnattr_t *attr)
{
    if (attr) attr->_flags = 0;
    return 0;
}

int posix_spawnattr_destroy(posix_spawnattr_t *attr)
{
    (void)attr;
    return 0;
}

int posix_spawnattr_setflags(posix_spawnattr_t *attr, short flags)
{
    if (attr) attr->_flags = flags;
    return 0;
}

int posix_spawnattr_getflags(const posix_spawnattr_t *attr, short *flags)
{
    if (attr && flags) *flags = (short)attr->_flags;
    return 0;
}

int posix_spawnattr_setsigmask(posix_spawnattr_t *attr,
                                const sigset_t *sigmask)
{
    (void)attr; (void)sigmask;
    return 0;
}

int posix_spawnattr_getsigmask(const posix_spawnattr_t *attr,
                                sigset_t *sigmask)
{
    (void)attr;
    if (sigmask) *sigmask = 0;
    return 0;
}

int posix_spawn_file_actions_init(posix_spawn_file_actions_t *fa)
{
    if (fa) { fa->_a = 0; fa->_u = 0; fa->_p = (void *)0; }
    return 0;
}

int posix_spawn_file_actions_destroy(posix_spawn_file_actions_t *fa)
{
    (void)fa;
    return 0;
}

int posix_spawn_file_actions_addclose(posix_spawn_file_actions_t *fa, int fd)
{
    (void)fa; (void)fd;
    return 0;
}

int posix_spawn_file_actions_adddup2(posix_spawn_file_actions_t *fa,
                                     int fd, int newfd)
{
    (void)fa; (void)fd; (void)newfd;
    return 0;
}

int posix_spawn_file_actions_addopen(posix_spawn_file_actions_t *fa,
                                     int fd, const char *path,
                                     int oflag, mode_t mode)
{
    (void)fa; (void)fd; (void)path; (void)oflag; (void)mode;
    return 0;
}

/* ========================================================================= */
/* pathconf / fpathconf                                                      */
/* ========================================================================= */

#define _PC_LINK_MAX            0
#define _PC_MAX_CANON           1
#define _PC_MAX_INPUT           2
#define _PC_NAME_MAX            3
#define _PC_PATH_MAX            4
#define _PC_PIPE_BUF            5
#define _PC_CHOWN_RESTRICTED    6
#define _PC_NO_TRUNC            7
#define _PC_VDISABLE            8

long pathconf(const char *path, int name)
{
    (void)path;
    switch (name) {
    case _PC_PATH_MAX:          return 4096;
    case _PC_NAME_MAX:          return 255;
    case _PC_LINK_MAX:          return 32767;
    case _PC_MAX_CANON:         return 255;
    case _PC_MAX_INPUT:         return 255;
    case _PC_PIPE_BUF:          return 4096;
    case _PC_CHOWN_RESTRICTED:  return 1;
    case _PC_NO_TRUNC:          return 1;
    case _PC_VDISABLE:          return 0;
    default:                    return -1;
    }
}

long fpathconf(int fd, int name)
{
    (void)fd;
    return pathconf("/", name);
}

/* ========================================================================= */
/* Multibyte/wide character stubs                                            */
/* ========================================================================= */

typedef int wchar_t;

int mblen(const char *s, unsigned long n)
{
    (void)n;
    if (!s || *s == '\0') return 0;
    return 1; /* Assume single-byte encoding */
}

int mbtowc(wchar_t *pwc, const char *s, unsigned long n)
{
    (void)n;
    if (!s) return 0;
    if (*s == '\0') {
        if (pwc) *pwc = 0;
        return 0;
    }
    if (pwc) *pwc = (wchar_t)(unsigned char)*s;
    return 1;
}

int wctomb(char *s, wchar_t wc)
{
    if (!s) return 0;
    if (wc < 0 || wc > 127) return -1;
    *s = (char)wc;
    return 1;
}

unsigned long mbstowcs(wchar_t *dest, const char *src, unsigned long n)
{
    unsigned long i;
    if (!src) return 0;
    for (i = 0; i < n && src[i]; i++) {
        if (dest) dest[i] = (wchar_t)(unsigned char)src[i];
    }
    return i;
}

unsigned long wcstombs(char *dest, const wchar_t *src, unsigned long n)
{
    unsigned long i;
    if (!src) return 0;
    for (i = 0; i < n && src[i]; i++) {
        if (dest) {
            if (src[i] < 0 || src[i] > 127) {
                dest[i] = '?';
            } else {
                dest[i] = (char)src[i];
            }
        }
    }
    return i;
}

/* mbrtowc: Convert multibyte to wide character (single-byte locale stub) */
typedef struct { int __fill[6]; } mbstate_t_stub;

unsigned long mbrtowc(wchar_t *pwc, const char *s, unsigned long n,
                      mbstate_t_stub *ps)
{
    (void)ps;
    if (!s) return 0;
    if (n == 0) return (unsigned long)-2;
    if (*s == '\0') {
        if (pwc) *pwc = 0;
        return 0;
    }
    if (pwc) *pwc = (wchar_t)(unsigned char)*s;
    return 1;
}

/* wcrtomb: Convert wide character to multibyte (single-byte locale stub) */
unsigned long wcrtomb(char *s, wchar_t wc, mbstate_t_stub *ps)
{
    (void)ps;
    if (!s) return 1;
    if (wc <= 0x7f) {
        *s = (char)wc;
        return 1;
    }
    return (unsigned long)-1;
}

/* mbrlen: Determine bytes in next multibyte character */
unsigned long mbrlen(const char *s, unsigned long n, mbstate_t_stub *ps)
{
    return mbrtowc((wchar_t *)0, s, n, ps);
}

/* ========================================================================= */
/* utime                                                                     */
/* ========================================================================= */

typedef long time_t;

struct utimbuf {
    time_t actime;
    time_t modtime;
};

int utime(const char *filename, const struct utimbuf *times)
{
    (void)filename; (void)times;
    /* Stub: filesystem timestamps not yet supported */
    return 0;
}

/* ========================================================================= */
/* utimes (sys/time.h)                                                       */
/* ========================================================================= */

int utimes(const char *filename, const void *times)
{
    (void)filename; (void)times;
    return 0;
}

/* ========================================================================= */
/* alarm                                                                     */
/* ========================================================================= */

unsigned int alarm(unsigned int seconds)
{
    (void)seconds;
    /* Stub: no alarm/signal delivery yet */
    return 0;
}

/* ========================================================================= */
/* freopen                                                                   */
/* ========================================================================= */

/* FILE type stub -- matches stdio.h definition */
typedef struct _FILE FILE;

FILE *freopen(const char *pathname, const char *mode, FILE *stream)
{
    (void)pathname; (void)mode; (void)stream;
    /* Stub: cannot reopen files yet */
    return (FILE *)0;
}

/* ========================================================================= */
/* getaddrinfo / freeaddrinfo stubs                                          */
/* ========================================================================= */

struct addrinfo_stub {
    int              ai_flags;
    int              ai_family;
    int              ai_socktype;
    int              ai_protocol;
    unsigned int     ai_addrlen;
    void            *ai_addr;
    char            *ai_canonname;
    struct addrinfo_stub *ai_next;
};

int getaddrinfo(const char *node, const char *service,
                const struct addrinfo_stub *hints,
                struct addrinfo_stub **res)
{
    (void)node; (void)service; (void)hints; (void)res;
    return -2; /* EAI_NONAME */
}

void freeaddrinfo(struct addrinfo_stub *res)
{
    (void)res;
}

const char *gai_strerror(int errcode)
{
    (void)errcode;
    return "Name resolution not supported";
}

int getnameinfo(const void *sa, unsigned int salen,
                char *host, unsigned int hostlen,
                char *serv, unsigned int servlen, int flags)
{
    (void)sa; (void)salen; (void)host; (void)hostlen;
    (void)serv; (void)servlen; (void)flags;
    return -2; /* EAI_NONAME */
}

/* ========================================================================= */
/* Socket stubs                                                              */
/* ========================================================================= */

int socket(int domain, int type, int protocol)
{
    (void)domain; (void)type; (void)protocol;
    return -1;
}

int connect(int sockfd, const void *addr, unsigned int addrlen)
{
    (void)sockfd; (void)addr; (void)addrlen;
    return -1;
}

int bind(int sockfd, const void *addr, unsigned int addrlen)
{
    (void)sockfd; (void)addr; (void)addrlen;
    return -1;
}

int listen(int sockfd, int backlog)
{
    (void)sockfd; (void)backlog;
    return -1;
}

int accept(int sockfd, void *addr, unsigned int *addrlen)
{
    (void)sockfd; (void)addr; (void)addrlen;
    return -1;
}

long send(int sockfd, const void *buf, unsigned long len, int flags)
{
    (void)sockfd; (void)buf; (void)len; (void)flags;
    return -1;
}

long recv(int sockfd, void *buf, unsigned long len, int flags)
{
    (void)sockfd; (void)buf; (void)len; (void)flags;
    return -1;
}

int setsockopt(int sockfd, int level, int optname,
               const void *optval, unsigned int optlen)
{
    (void)sockfd; (void)level; (void)optname;
    (void)optval; (void)optlen;
    return -1;
}

/* ========================================================================= */
/* Math functions (libm)                                                     */
/* ========================================================================= */

/* ldexp: multiply x by 2^exp.  Used by GCC's sreal.cc */
double ldexp(double x, int exp)
{
    /* Handle special cases */
    if (x == 0.0 || exp == 0) return x;

    /* Build 2^exp via IEEE 754 bit manipulation */
    if (exp > 0) {
        while (exp > 0) {
            x *= 2.0;
            exp--;
        }
    } else {
        while (exp < 0) {
            x *= 0.5;
            exp++;
        }
    }
    return x;
}

float ldexpf(float x, int exp)
{
    return (float)ldexp((double)x, exp);
}

/* frexp: extract mantissa and exponent */
double frexp(double x, int *exp)
{
    /* Minimal implementation via successive halving/doubling */
    int e = 0;
    if (x == 0.0) { *exp = 0; return 0.0; }
    if (x < 0.0) {
        double r = frexp(-x, exp);
        return -r;
    }
    while (x >= 1.0) { x *= 0.5; e++; }
    while (x < 0.5)  { x *= 2.0; e--; }
    *exp = e;
    return x;
}

float frexpf(float x, int *exp)
{
    return (float)frexp((double)x, exp);
}

/* fabs: absolute value */
double fabs(double x)
{
    return x < 0.0 ? -x : x;
}

float fabsf(float x)
{
    return x < 0.0f ? -x : x;
}

/* floor: round down to integer */
double floor(double x)
{
    long i = (long)x;
    if (x < 0.0 && x != (double)i) return (double)(i - 1);
    return (double)i;
}

float floorf(float x)
{
    return (float)floor((double)x);
}

/* ceil: round up to integer */
double ceil(double x)
{
    long i = (long)x;
    if (x > 0.0 && x != (double)i) return (double)(i + 1);
    return (double)i;
}

float ceilf(float x)
{
    return (float)ceil((double)x);
}

/* log: natural logarithm (used by GCC's realmpfr.o) */
double log(double x)
{
    /* Simple series expansion: ln(x) = 2 * sum((t^(2n+1))/(2n+1)) where t=(x-1)/(x+1) */
    if (x <= 0.0) return -1.0 / 0.0; /* -inf */
    if (x == 1.0) return 0.0;

    /* Range reduction: x = m * 2^e, ln(x) = ln(m) + e*ln(2) */
    int e;
    double m = frexp(x, &e);
    /* m is in [0.5, 1.0) */
    /* ln(2) ~= 0.693147180559945 */
    double ln2 = 0.6931471805599453;

    double t = (m - 1.0) / (m + 1.0);
    double t2 = t * t;
    double sum = t;
    double term = t;
    int i;
    for (i = 1; i < 30; i++) {
        term *= t2;
        sum += term / (double)(2 * i + 1);
    }
    return 2.0 * sum + (double)e * ln2;
}

/* exp: e^x (used by GCC's realmpfr.o) */
double exp(double x)
{
    /* Handle extremes */
    if (x == 0.0) return 1.0;
    if (x > 709.0) return 1.0 / 0.0;  /* +inf */
    if (x < -709.0) return 0.0;

    /* Range reduction: e^x = 2^k * e^r where r = x - k*ln2 */
    double ln2 = 0.6931471805599453;
    int k = (int)(x / ln2 + (x > 0.0 ? 0.5 : -0.5));
    double r = x - (double)k * ln2;

    /* Taylor series for e^r, r is small */
    double sum = 1.0;
    double term = 1.0;
    int i;
    for (i = 1; i < 25; i++) {
        term *= r / (double)i;
        sum += term;
    }
    return ldexp(sum, k);
}

/* sqrt: square root via Newton's method */
double sqrt(double x)
{
    if (x < 0.0) return 0.0 / 0.0;  /* NaN */
    if (x == 0.0) return 0.0;
    double guess = x * 0.5;
    int i;
    for (i = 0; i < 60; i++) {
        guess = 0.5 * (guess + x / guess);
    }
    return guess;
}

float sqrtf(float x)
{
    return (float)sqrt((double)x);
}

/* pow: x^y */
double pow(double x, double y)
{
    if (y == 0.0) return 1.0;
    if (x == 0.0) return 0.0;
    if (x == 1.0) return 1.0;

    /* Check if y is an integer for negative bases */
    long iy = (long)y;
    if (y == (double)iy && x < 0.0) {
        double r = exp((double)iy * log(-x));
        return (iy & 1) ? -r : r;
    }
    return exp(y * log(x));
}

/* modf: split into integer and fractional parts */
double modf(double x, double *iptr)
{
    long i = (long)x;
    *iptr = (double)i;
    return x - (double)i;
}

/* strtod/strtof/strtold already defined in stdlib.c -- not duplicated here */
