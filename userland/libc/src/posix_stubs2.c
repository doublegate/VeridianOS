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

#define _SC_CLK_TCK           2
#define _SC_OPEN_MAX          4
#define _SC_NPROCESSORS_ONLN  84
#define _SC_PAGESIZE          30
#define _SC_ARG_MAX           0

long sysconf(int name)
{
    switch (name) {
    case _SC_CLK_TCK:
        return 100; /* 100 Hz tick (standard for POSIX) */
    case _SC_NPROCESSORS_ONLN:
        return 1; /* Single CPU for now */
    case _SC_PAGESIZE:
        return 4096;
    case _SC_OPEN_MAX:
        return 256;
    case _SC_ARG_MAX:
        return 131072; /* 128 KiB */
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

/* Forward declarations */
extern pid_t fork(void);
extern int execve(const char *path, char *const argv[], char *const envp[]);
extern int execvp(const char *file, char *const argv[]);
extern void _exit(int status);

int posix_spawn(pid_t *pid, const char *path,
                const posix_spawn_file_actions_t *fa,
                const posix_spawnattr_t *attr,
                char *const argv[], char *const envp[])
{
    (void)fa; (void)attr; /* file_actions and attrs not yet supported */

    pid_t child = fork();
    if (child < 0)
        return errno;
    if (child == 0) {
        /* Child: exec the program */
        execve(path, argv, envp);
        _exit(127); /* exec failed */
    }
    /* Parent: return child PID */
    if (pid)
        *pid = child;
    return 0;
}

int posix_spawnp(pid_t *pid, const char *file,
                 const posix_spawn_file_actions_t *fa,
                 const posix_spawnattr_t *attr,
                 char *const argv[], char *const envp[])
{
    (void)fa; (void)attr;

    pid_t child = fork();
    if (child < 0)
        return errno;
    if (child == 0) {
        /* Child: search PATH for the executable */
        /* execvp ignores envp -- use execve with resolved path instead.
         * For now, if file contains '/', use execve directly; otherwise
         * use execvp which searches PATH. */
        if (file && file[0] == '/') {
            execve(file, argv, envp);
        } else {
            /* execvp searches PATH but doesn't pass envp; however for
             * GCC's use case the child inherits the parent's environment
             * which is sufficient. */
            (void)envp;
            execvp(file, argv);
        }
        _exit(127);
    }
    if (pid)
        *pid = child;
    return 0;
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
/* Socket syscall wrappers                                                   */
/* ========================================================================= */

/*
 * All socket functions delegate to the VeridianOS kernel via the syscall
 * interface defined in <veridian/syscall.h>.
 *
 * Syscall number layout:
 *   SYS_SOCKET_CREATE  (220) -- socket()
 *   SYS_SOCKET_BIND    (221) -- bind()
 *   SYS_SOCKET_LISTEN  (222) -- listen()
 *   SYS_SOCKET_CONNECT (223) -- connect()
 *   SYS_SOCKET_ACCEPT  (224) -- accept()
 *   SYS_SOCKET_SEND    (225) -- send()
 *   SYS_SOCKET_RECV    (226) -- recv()
 *   SYS_SOCKET_CLOSE   (227) -- shutdown() (best-effort)
 *   SYS_SOCKET_PAIR    (228) -- socketpair()
 *   SYS_NET_SENDTO     (250) -- sendto()
 *   SYS_NET_RECVFROM   (251) -- recvfrom()
 *   SYS_NET_GETSOCKNAME(252) -- getsockname()
 *   SYS_NET_GETPEERNAME(253) -- getpeername()
 *   SYS_NET_SETSOCKOPT (254) -- setsockopt()
 *   SYS_NET_GETSOCKOPT (255) -- getsockopt()
 */

#include <veridian/syscall.h>

/*
 * Helper shared with posix_stubs3.c: translate a raw syscall return value
 * to the POSIX convention (negative -> errno + return -1).
 * (Cannot use the static __syscall_ret from syscall.c here since that file
 * is compiled separately; we duplicate the one-liner inline.)
 */
static inline long __sock_ret(long r)
{
    if (r < 0) {
        errno = (int)(-r);
        return -1L;
    }
    return r;
}

/*
 * socket() -- create an endpoint for communication.
 *
 * Kernel args: (domain, sock_type)
 * The protocol argument is not forwarded (kernel derives it from domain +
 * sock_type); ignored here following the same approach as Linux glibc.
 */
int socket(int domain, int type, int protocol)
{
    (void)protocol;
    long ret = veridian_syscall2(SYS_SOCKET_CREATE, domain, type);
    return (int)__sock_ret(ret);
}

/*
 * connect() -- initiate a connection on a socket.
 *
 * Kernel args: (socket_id, addr_ptr, addr_len)
 */
int connect(int sockfd, const void *addr, unsigned int addrlen)
{
    long ret = veridian_syscall3(SYS_SOCKET_CONNECT, sockfd, addr, addrlen);
    return (int)__sock_ret(ret);
}

/*
 * bind() -- bind a name to a socket.
 *
 * Kernel args: (socket_id, addr_ptr, addr_len)
 */
int bind(int sockfd, const void *addr, unsigned int addrlen)
{
    long ret = veridian_syscall3(SYS_SOCKET_BIND, sockfd, addr, addrlen);
    return (int)__sock_ret(ret);
}

/*
 * listen() -- listen for connections on a socket.
 *
 * Kernel args: (socket_id, backlog)
 */
int listen(int sockfd, int backlog)
{
    long ret = veridian_syscall2(SYS_SOCKET_LISTEN, sockfd, backlog);
    return (int)__sock_ret(ret);
}

/*
 * accept() -- accept a connection on a socket.
 *
 * The kernel's SYS_SOCKET_ACCEPT takes only the listening socket_id and
 * returns the new connected socket_id.  addr/addrlen output is not filled
 * by the kernel at this time; we zero them out to avoid stale data.
 *
 * Kernel args: (socket_id)
 */
int accept(int sockfd, void *addr, unsigned int *addrlen)
{
    long ret = veridian_syscall1(SYS_SOCKET_ACCEPT, sockfd);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    /* Clear addr output so callers don't see stale data. */
    if (addr && addrlen && *addrlen > 0) {
        unsigned int i;
        for (i = 0; i < *addrlen; i++)
            ((unsigned char *)addr)[i] = 0;
        *addrlen = 0;
    }
    return (int)ret;
}

/*
 * send() -- send a message on a socket.
 *
 * Kernel args: (socket_id, buf_ptr, buf_len)
 * The flags argument is not forwarded (not yet supported by the kernel).
 */
long send(int sockfd, const void *buf, unsigned long len, int flags)
{
    (void)flags;
    long ret = veridian_syscall3(SYS_SOCKET_SEND, sockfd, buf, len);
    return __sock_ret(ret);
}

/*
 * recv() -- receive a message from a socket.
 *
 * Kernel args: (socket_id, buf_ptr, buf_len)
 * The flags argument is not forwarded (not yet supported by the kernel).
 */
long recv(int sockfd, void *buf, unsigned long len, int flags)
{
    (void)flags;
    long ret = veridian_syscall3(SYS_SOCKET_RECV, sockfd, buf, len);
    return __sock_ret(ret);
}

/*
 * setsockopt() -- set options on sockets.
 *
 * Kernel args: (fd, level, optname, optval_ptr, optlen)
 */
int setsockopt(int sockfd, int level, int optname,
               const void *optval, unsigned int optlen)
{
    long ret = veridian_syscall5(SYS_NET_SETSOCKOPT,
                                  sockfd, level, optname, optval, optlen);
    return (int)__sock_ret(ret);
}

/*
 * getsockopt() -- get options on sockets.
 *
 * Kernel args: (fd, level, optname, optval_ptr)
 * The optlen pointer is not forwarded; the kernel writes a fixed 4-byte value.
 */
int getsockopt(int sockfd, int level, int optname,
               void *optval, unsigned int *optlen)
{
    long ret = veridian_syscall4(SYS_NET_GETSOCKOPT,
                                  sockfd, level, optname, optval);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    /* Kernel writes 4 bytes; reflect that in *optlen if provided. */
    if (optlen)
        *optlen = 4;
    return 0;
}

/*
 * shutdown() -- shut down part of a full-duplex connection.
 *
 * The kernel does not yet have a dedicated shutdown syscall.  We use
 * SYS_SOCKET_CLOSE as a best-effort implementation when SHUT_RDWR is
 * requested.  For SHUT_RD / SHUT_WR we return 0 without action (the
 * kernel's socket layer does not support half-close at this time).
 */
int shutdown(int sockfd, int how)
{
#define SHUT_RD_LOCAL   0
#define SHUT_WR_LOCAL   1
#define SHUT_RDWR_LOCAL 2
    if (how == SHUT_RDWR_LOCAL) {
        long ret = veridian_syscall1(SYS_SOCKET_CLOSE, sockfd);
        return (int)__sock_ret(ret);
    }
    /* Half-shutdown not supported; succeed silently. */
    (void)sockfd;
    return 0;
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

/* ========================================================================= */
/* Locale-aware string functions (C/POSIX locale only)                       */
/* ========================================================================= */

extern int strcmp(const char *s1, const char *s2);
extern unsigned long strlen(const char *s);

/* strcoll: locale-aware string compare -- in C locale, same as strcmp */
int strcoll(const char *s1, const char *s2)
{
    return strcmp(s1, s2);
}

/* strxfrm: locale-aware string transform -- in C locale, just copy */
unsigned long strxfrm(char *dest, const char *src, unsigned long n)
{
    unsigned long len = strlen(src);
    if (dest && n > 0) {
        unsigned long copy = len < n ? len : n - 1;
        for (unsigned long i = 0; i < copy; i++)
            dest[i] = src[i];
        if (copy < n)
            dest[copy] = '\0';
    }
    return len;
}

/* ========================================================================= */
/* stdio functions needed by C++ <cstdio>                                    */
/* ========================================================================= */

/* Forward declare FILE and streams (match stdio.h) */
struct _FILE;
typedef struct _FILE FILE;
extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;
extern int fgetc(FILE *stream);
extern int fputc(int c, FILE *stream);
extern long ftell(FILE *stream);
extern int fseek(FILE *stream, long offset, int whence);

typedef long fpos_t;

int getc(FILE *stream)
{
    return fgetc(stream);
}

int putc(int c, FILE *stream)
{
    return fputc(c, stream);
}

int getchar(void)
{
    return fgetc(stdin);
}

/* Note: putchar with write() already exists above for non-FILE usage.
   This version uses FILE-based fputc for C++ <cstdio> compatibility.
   The linker will use whichever is pulled in first. */

int fgetpos(FILE *stream, fpos_t *pos)
{
    if (!stream || !pos)
        return -1;
    long p = ftell(stream);
    if (p < 0)
        return -1;
    *pos = (fpos_t)p;
    return 0;
}

int fsetpos(FILE *stream, const fpos_t *pos)
{
    if (!stream || !pos)
        return -1;
    return fseek(stream, (long)*pos, 0 /* SEEK_SET */);
}

/* vscanf/vsscanf: minimal stubs -- return 0 (no items matched) */
extern int vfscanf(FILE *stream, const char *fmt, __builtin_va_list ap);

int vscanf(const char *fmt, __builtin_va_list ap)
{
    return vfscanf(stdin, fmt, ap);
}

/* Minimal vsscanf stub -- GCC's configure tests need it to exist,
   but the actual compilation pipeline doesn't call it. */
int vsscanf(const char *str, const char *fmt, __builtin_va_list ap)
{
    (void)str; (void)fmt; (void)ap;
    return 0;
}
