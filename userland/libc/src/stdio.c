/*
 * VeridianOS libc -- stdio.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal buffered I/O implementation.  All formatted output routes
 * through vsnprintf() which does the actual formatting into a stack
 * buffer, then write() pushes it to the fd.
 */

#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <stdarg.h>
#include <ctype.h>

/* ========================================================================= */
/* Standard streams                                                          */
/* ========================================================================= */

static FILE __stdin_file  = { .fd = 0, .flags = __FILE_READ,  .buf_mode = _IOLBF };
static FILE __stdout_file = { .fd = 1, .flags = __FILE_WRITE, .buf_mode = _IOLBF };
static FILE __stderr_file = { .fd = 2, .flags = __FILE_WRITE, .buf_mode = _IONBF };

FILE *stdin  = &__stdin_file;
FILE *stdout = &__stdout_file;
FILE *stderr = &__stderr_file;

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

/*
 * Flush the write buffer to the underlying fd.
 * Returns 0 on success, EOF on error.
 */
static int __flush_write(FILE *stream)
{
    if (stream->buf_pos == 0)
        return 0;

    size_t written = 0;
    while (written < stream->buf_pos) {
        ssize_t r = write(stream->fd,
                          stream->buf + written,
                          stream->buf_pos - written);
        if (r < 0) {
            stream->flags |= __FILE_ERROR;
            return EOF;
        }
        written += (size_t)r;
    }
    stream->buf_pos = 0;
    return 0;
}

/*
 * Ensure the stream has an allocated buffer.
 */
static void __ensure_buf(FILE *stream)
{
    if (stream->buf == NULL && stream->buf_mode != _IONBF) {
        stream->buf = (unsigned char *)malloc(BUFSIZ);
        if (stream->buf) {
            stream->buf_size = BUFSIZ;
            stream->flags |= __FILE_MYBUF;
        } else {
            /* Fallback to unbuffered if malloc fails. */
            stream->buf_mode = _IONBF;
        }
    }
}

/* ========================================================================= */
/* File operations                                                           */
/* ========================================================================= */

FILE *fopen(const char *pathname, const char *mode)
{
    int flags = 0;
    int fflags = 0;

    /* Parse mode string. */
    switch (mode[0]) {
    case 'r':
        flags = O_RDONLY;
        fflags = __FILE_READ;
        break;
    case 'w':
        flags = O_WRONLY | O_CREAT | O_TRUNC;
        fflags = __FILE_WRITE;
        break;
    case 'a':
        flags = O_WRONLY | O_CREAT | O_APPEND;
        fflags = __FILE_WRITE | __FILE_APPEND;
        break;
    default:
        errno = EINVAL;
        return NULL;
    }

    /* Check for '+' (read+write). */
    if (mode[1] == '+' || (mode[1] && mode[2] == '+')) {
        flags = (flags & ~(O_RDONLY | O_WRONLY)) | O_RDWR;
        fflags |= __FILE_READ | __FILE_WRITE;
    }

    int fd = open(pathname, flags, 0666);
    if (fd < 0)
        return NULL;

    FILE *f = (FILE *)malloc(sizeof(FILE));
    if (!f) {
        close(fd);
        return NULL;
    }

    memset(f, 0, sizeof(FILE));
    f->fd = fd;
    f->flags = fflags;
    f->buf_mode = _IOFBF;  /* Fully buffered by default. */
    return f;
}

int fclose(FILE *stream)
{
    if (!stream)
        return EOF;

    /* Flush any buffered writes. */
    int ret = 0;
    if (stream->flags & __FILE_WRITE)
        ret = fflush(stream);

    if (close(stream->fd) < 0)
        ret = EOF;

    /* Free buffer if we allocated it. */
    if (stream->flags & __FILE_MYBUF)
        free(stream->buf);

    /* Don't free the static stdin/stdout/stderr. */
    if (stream != stdin && stream != stdout && stream != stderr)
        free(stream);

    return ret;
}

int fflush(FILE *stream)
{
    if (!stream) {
        /* Flush all streams -- just flush stdout/stderr. */
        int r = 0;
        if (__flush_write(stdout) == EOF) r = EOF;
        if (__flush_write(stderr) == EOF) r = EOF;
        return r;
    }

    if (!(stream->flags & __FILE_WRITE))
        return 0;

    if (stream->buf == NULL)
        return 0;

    return __flush_write(stream);
}

/* ========================================================================= */
/* Character I/O                                                             */
/* ========================================================================= */

int fgetc(FILE *stream)
{
    if (!stream || !(stream->flags & __FILE_READ))
        return EOF;

    if (stream->flags & __FILE_EOF)
        return EOF;

    __ensure_buf(stream);

    /* Unbuffered: read one byte directly. */
    if (stream->buf_mode == _IONBF || stream->buf == NULL) {
        unsigned char c;
        ssize_t r = read(stream->fd, &c, 1);
        if (r <= 0) {
            stream->flags |= (r == 0) ? __FILE_EOF : __FILE_ERROR;
            return EOF;
        }
        return c;
    }

    /* Buffered: refill if needed. */
    if (stream->buf_pos >= stream->buf_len) {
        ssize_t r = read(stream->fd, stream->buf, stream->buf_size);
        if (r <= 0) {
            stream->flags |= (r == 0) ? __FILE_EOF : __FILE_ERROR;
            return EOF;
        }
        stream->buf_len = (size_t)r;
        stream->buf_pos = 0;
    }

    return stream->buf[stream->buf_pos++];
}

int fputc(int c, FILE *stream)
{
    if (!stream || !(stream->flags & __FILE_WRITE))
        return EOF;

    unsigned char ch = (unsigned char)c;

    /* Unbuffered: write directly. */
    if (stream->buf_mode == _IONBF) {
        ssize_t r = write(stream->fd, &ch, 1);
        if (r != 1) {
            stream->flags |= __FILE_ERROR;
            return EOF;
        }
        return ch;
    }

    __ensure_buf(stream);
    if (stream->buf == NULL) {
        /* Fallback if alloc failed. */
        ssize_t r = write(stream->fd, &ch, 1);
        return (r == 1) ? ch : EOF;
    }

    stream->buf[stream->buf_pos++] = ch;

    /* Flush on buffer full, or on newline for line-buffered. */
    if (stream->buf_pos >= stream->buf_size ||
        (stream->buf_mode == _IOLBF && ch == '\n')) {
        if (__flush_write(stream) == EOF)
            return EOF;
    }

    return ch;
}

/* ========================================================================= */
/* String I/O                                                                */
/* ========================================================================= */

char *fgets(char *s, int size, FILE *stream)
{
    if (size <= 0)
        return NULL;

    char *p = s;
    int n = size - 1;

    while (n > 0) {
        int c = fgetc(stream);
        if (c == EOF) {
            if (p == s)
                return NULL;
            break;
        }
        *p++ = (char)c;
        n--;
        if (c == '\n')
            break;
    }
    *p = '\0';
    return s;
}

int fputs(const char *s, FILE *stream)
{
    while (*s) {
        if (fputc(*s, stream) == EOF)
            return EOF;
        s++;
    }
    return 0;
}

int puts(const char *s)
{
    if (fputs(s, stdout) == EOF)
        return EOF;
    if (fputc('\n', stdout) == EOF)
        return EOF;
    return 0;
}

/* ========================================================================= */
/* Block I/O                                                                 */
/* ========================================================================= */

size_t fread(void *ptr, size_t size, size_t count, FILE *stream)
{
    size_t total = size * count;
    if (total == 0)
        return 0;

    unsigned char *p = (unsigned char *)ptr;
    size_t done = 0;

    while (done < total) {
        int c = fgetc(stream);
        if (c == EOF)
            break;
        *p++ = (unsigned char)c;
        done++;
    }

    return done / size;
}

size_t fwrite(const void *ptr, size_t size, size_t count, FILE *stream)
{
    size_t total = size * count;
    if (total == 0)
        return 0;

    const unsigned char *p = (const unsigned char *)ptr;
    size_t done = 0;

    while (done < total) {
        if (fputc(*p, stream) == EOF)
            break;
        p++;
        done++;
    }

    return done / size;
}

/* ========================================================================= */
/* Seeking                                                                   */
/* ========================================================================= */

int fseek(FILE *stream, long offset, int whence)
{
    if (!stream)
        return -1;

    /* Flush writes before seeking. */
    if (stream->flags & __FILE_WRITE)
        fflush(stream);

    /* Discard read buffer. */
    stream->buf_pos = 0;
    stream->buf_len = 0;
    stream->flags &= ~(__FILE_EOF | __FILE_ERROR);

    if (lseek(stream->fd, offset, whence) < 0)
        return -1;

    return 0;
}

long ftell(FILE *stream)
{
    if (!stream)
        return -1;

    long pos = (long)lseek(stream->fd, 0, SEEK_CUR);
    if (pos < 0)
        return -1;

    /* Adjust for buffered data. */
    if (stream->flags & __FILE_READ)
        pos -= (long)(stream->buf_len - stream->buf_pos);
    else if (stream->flags & __FILE_WRITE)
        pos += (long)stream->buf_pos;

    return pos;
}

void rewind(FILE *stream)
{
    fseek(stream, 0, SEEK_SET);
    if (stream)
        stream->flags &= ~__FILE_ERROR;
}

/* ========================================================================= */
/* Error / EOF queries                                                       */
/* ========================================================================= */

int feof(FILE *stream)
{
    return stream ? (stream->flags & __FILE_EOF) != 0 : 0;
}

int ferror(FILE *stream)
{
    return stream ? (stream->flags & __FILE_ERROR) != 0 : 0;
}

void clearerr(FILE *stream)
{
    if (stream)
        stream->flags &= ~(__FILE_EOF | __FILE_ERROR);
}

void perror(const char *s)
{
    if (s && *s) {
        fputs(s, stderr);
        fputs(": ", stderr);
    }
    fputs(strerror(errno), stderr);
    fputc('\n', stderr);
}

/* ========================================================================= */
/* Formatted output: vsnprintf (the core engine)                             */
/* ========================================================================= */

/*
 * Minimal vsnprintf supporting: %d, %i, %u, %x, %X, %o, %c, %s, %p, %ld,
 * %lu, %lx, %lX, %lo, %li, %%, %02d-style width/zero-pad, and %-*s.
 *
 * This is not a full C99 printf -- no floating point, no %n, no positional
 * arguments.  Adequate for OS userland bootstrap.
 */

/* Write a single character to the output buffer if space remains. */
static inline void __put(char *buf, size_t size, size_t *pos, char c)
{
    if (*pos < size - 1)
        buf[*pos] = c;
    (*pos)++;
}

/* Write a string of known length. */
static void __puts(char *buf, size_t size, size_t *pos,
                   const char *s, size_t len)
{
    for (size_t i = 0; i < len; i++)
        __put(buf, size, pos, s[i]);
}

/* Format an unsigned long in the given base (2-36). */
static void __format_ulong(char *buf, size_t size, size_t *pos,
                            unsigned long val, int base, int upper,
                            int width, int zero_pad, int left_align)
{
    char tmp[22]; /* enough for 64-bit in base 2 */
    int len = 0;
    const char *digits = upper ? "0123456789ABCDEF" : "0123456789abcdef";

    if (val == 0) {
        tmp[len++] = '0';
    } else {
        while (val) {
            tmp[len++] = digits[val % (unsigned long)base];
            val /= (unsigned long)base;
        }
    }

    /* Padding. */
    int pad = width - len;
    if (!left_align && pad > 0) {
        char pc = zero_pad ? '0' : ' ';
        while (pad-- > 0)
            __put(buf, size, pos, pc);
    }

    /* Digits (reversed). */
    while (len > 0)
        __put(buf, size, pos, tmp[--len]);

    /* Right-pad if left-aligned. */
    if (left_align && pad > 0) {
        while (pad-- > 0)
            __put(buf, size, pos, ' ');
    }
}

/* Format a signed long. */
static void __format_long(char *buf, size_t size, size_t *pos,
                           long val, int base, int width, int zero_pad,
                           int left_align)
{
    if (val < 0) {
        __put(buf, size, pos, '-');
        if (width > 0) width--;
        val = -val;
    }
    __format_ulong(buf, size, pos, (unsigned long)val, base, 0,
                   width, zero_pad, left_align);
}

int vsnprintf(char *buf, size_t size, const char *fmt, va_list ap)
{
    size_t pos = 0;

    if (size == 0) {
        /* Just count characters. */
        /* We still need a valid buffer to not crash, but can't write. */
    }

    while (*fmt) {
        if (*fmt != '%') {
            __put(buf, size, &pos, *fmt++);
            continue;
        }
        fmt++; /* skip '%' */

        /* Flags. */
        int left_align = 0;
        int zero_pad = 0;

        while (*fmt == '-' || *fmt == '0') {
            if (*fmt == '-') left_align = 1;
            if (*fmt == '0') zero_pad = 1;
            fmt++;
        }
        if (left_align) zero_pad = 0; /* '-' overrides '0' */

        /* Width. */
        int width = 0;
        if (*fmt == '*') {
            width = va_arg(ap, int);
            if (width < 0) {
                left_align = 1;
                width = -width;
            }
            fmt++;
        } else {
            while (*fmt >= '0' && *fmt <= '9') {
                width = width * 10 + (*fmt - '0');
                fmt++;
            }
        }

        /* Precision (parsed but only used for strings). */
        int precision = -1;
        if (*fmt == '.') {
            fmt++;
            precision = 0;
            if (*fmt == '*') {
                precision = va_arg(ap, int);
                fmt++;
            } else {
                while (*fmt >= '0' && *fmt <= '9') {
                    precision = precision * 10 + (*fmt - '0');
                    fmt++;
                }
            }
        }

        /* Length modifier. */
        int is_long = 0;
        if (*fmt == 'l') {
            is_long = 1;
            fmt++;
            if (*fmt == 'l') {
                /* 'll' -- treat same as 'l' on 64-bit. */
                fmt++;
            }
        } else if (*fmt == 'z') {
            is_long = 1;
            fmt++;
        }

        /* Conversion specifier. */
        switch (*fmt) {
        case 'd':
        case 'i': {
            long val = is_long ? va_arg(ap, long) : (long)va_arg(ap, int);
            __format_long(buf, size, &pos, val, 10, width, zero_pad,
                          left_align);
            break;
        }
        case 'u': {
            unsigned long val = is_long
                ? va_arg(ap, unsigned long)
                : (unsigned long)va_arg(ap, unsigned int);
            __format_ulong(buf, size, &pos, val, 10, 0, width, zero_pad,
                           left_align);
            break;
        }
        case 'x':
        case 'X': {
            unsigned long val = is_long
                ? va_arg(ap, unsigned long)
                : (unsigned long)va_arg(ap, unsigned int);
            __format_ulong(buf, size, &pos, val, 16, (*fmt == 'X'),
                           width, zero_pad, left_align);
            break;
        }
        case 'o': {
            unsigned long val = is_long
                ? va_arg(ap, unsigned long)
                : (unsigned long)va_arg(ap, unsigned int);
            __format_ulong(buf, size, &pos, val, 8, 0, width, zero_pad,
                           left_align);
            break;
        }
        case 'p': {
            void *ptr = va_arg(ap, void *);
            __put(buf, size, &pos, '0');
            __put(buf, size, &pos, 'x');
            __format_ulong(buf, size, &pos, (unsigned long)ptr, 16, 0,
                           0, 0, 0);
            break;
        }
        case 'c': {
            char c = (char)va_arg(ap, int);
            int pad = width - 1;
            if (!left_align && pad > 0)
                while (pad-- > 0)
                    __put(buf, size, &pos, ' ');
            __put(buf, size, &pos, c);
            if (left_align && pad > 0)
                while (pad-- > 0)
                    __put(buf, size, &pos, ' ');
            break;
        }
        case 's': {
            const char *s = va_arg(ap, const char *);
            if (!s) s = "(null)";
            size_t slen = strlen(s);
            if (precision >= 0 && (size_t)precision < slen)
                slen = (size_t)precision;
            int pad = width - (int)slen;
            if (!left_align && pad > 0)
                while (pad-- > 0)
                    __put(buf, size, &pos, ' ');
            __puts(buf, size, &pos, s, slen);
            if (left_align && pad > 0)
                while (pad-- > 0)
                    __put(buf, size, &pos, ' ');
            break;
        }
        case '%':
            __put(buf, size, &pos, '%');
            break;
        case '\0':
            /* Trailing '%' at end of format string. */
            goto done;
        default:
            /* Unknown specifier -- emit literally. */
            __put(buf, size, &pos, '%');
            __put(buf, size, &pos, *fmt);
            break;
        }
        fmt++;
    }

done:
    /* NUL-terminate. */
    if (size > 0) {
        if (pos < size)
            buf[pos] = '\0';
        else
            buf[size - 1] = '\0';
    }

    return (int)pos;
}

int vsprintf(char *buf, const char *fmt, va_list ap)
{
    /* No bounds -- pass a very large size. */
    return vsnprintf(buf, (size_t)-1, fmt, ap);
}

/* ========================================================================= */
/* Formatted output: convenience wrappers                                    */
/* ========================================================================= */

int snprintf(char *buf, size_t size, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vsnprintf(buf, size, fmt, ap);
    va_end(ap);
    return ret;
}

int sprintf(char *buf, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vsprintf(buf, fmt, ap);
    va_end(ap);
    return ret;
}

int vfprintf(FILE *stream, const char *fmt, va_list ap)
{
    char buf[1024];
    va_list ap2;
    va_copy(ap2, ap);
    int len = vsnprintf(buf, sizeof(buf), fmt, ap2);
    va_end(ap2);

    if (len <= 0)
        return len;

    if ((size_t)len < sizeof(buf)) {
        /* Fits in stack buffer. */
        size_t written = fwrite(buf, 1, (size_t)len, stream);
        return (int)written;
    }

    /* Larger than stack buffer: allocate. */
    char *big = (char *)malloc((size_t)len + 1);
    if (!big)
        return -1;
    va_copy(ap2, ap);
    vsnprintf(big, (size_t)len + 1, fmt, ap2);
    va_end(ap2);
    size_t written = fwrite(big, 1, (size_t)len, stream);
    free(big);
    return (int)written;
}

int fprintf(FILE *stream, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vfprintf(stream, fmt, ap);
    va_end(ap);
    return ret;
}

int vprintf(const char *fmt, va_list ap)
{
    return vfprintf(stdout, fmt, ap);
}

int printf(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vprintf(fmt, ap);
    va_end(ap);
    return ret;
}

/* ========================================================================= */
/* Formatted input: minimal sscanf                                           */
/* ========================================================================= */

int sscanf(const char *str, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);

    int matched = 0;
    const char *s = str;

    while (*fmt) {
        if (isspace((unsigned char)*fmt)) {
            /* Skip whitespace in both format and input. */
            while (isspace((unsigned char)*s))
                s++;
            fmt++;
            continue;
        }

        if (*fmt != '%') {
            /* Literal match. */
            if (*s != *fmt)
                break;
            s++;
            fmt++;
            continue;
        }
        fmt++; /* skip '%' */

        /* Skip assignment suppression (not implemented). */

        switch (*fmt) {
        case 'd': {
            int *p = va_arg(ap, int *);
            while (isspace((unsigned char)*s)) s++;
            long val = strtol(s, (char **)&s, 10);
            *p = (int)val;
            matched++;
            break;
        }
        case 's': {
            char *p = va_arg(ap, char *);
            while (isspace((unsigned char)*s)) s++;
            while (*s && !isspace((unsigned char)*s))
                *p++ = *s++;
            *p = '\0';
            matched++;
            break;
        }
        case 'c': {
            char *p = va_arg(ap, char *);
            if (*s) {
                *p = *s++;
                matched++;
            }
            break;
        }
        default:
            goto done_sscanf;
        }
        fmt++;
    }

done_sscanf:
    va_end(ap);
    return matched;
}

/* ========================================================================= */
/* Formatted input: vfscanf (core engine for fscanf/scanf)                   */
/* ========================================================================= */

int vfscanf(FILE *stream, const char *fmt, va_list ap)
{
    int matched = 0;

    while (*fmt) {
        if (isspace((unsigned char)*fmt)) {
            /* Skip whitespace in format and input. */
            int c;
            while ((c = fgetc(stream)) != EOF && isspace(c))
                ;
            if (c != EOF)
                ungetc(c, stream);
            fmt++;
            continue;
        }

        if (*fmt != '%') {
            /* Literal match. */
            int c = fgetc(stream);
            if (c == EOF || c != *fmt)
                break;
            fmt++;
            continue;
        }
        fmt++; /* skip '%' */

        switch (*fmt) {
        case 'd': {
            int *p = va_arg(ap, int *);
            /* Skip whitespace. */
            int c;
            while ((c = fgetc(stream)) != EOF && isspace(c))
                ;
            if (c == EOF)
                goto done_vfscanf;

            /* Read digits into a small buffer. */
            char numbuf[24];
            int ni = 0;
            if (c == '-' || c == '+') {
                numbuf[ni++] = (char)c;
                c = fgetc(stream);
            }
            while (c != EOF && c >= '0' && c <= '9' && ni < 23) {
                numbuf[ni++] = (char)c;
                c = fgetc(stream);
            }
            if (c != EOF)
                ungetc(c, stream);
            if (ni == 0 || (ni == 1 && (numbuf[0] == '-' || numbuf[0] == '+')))
                goto done_vfscanf;
            numbuf[ni] = '\0';
            *p = (int)strtol(numbuf, NULL, 10);
            matched++;
            break;
        }
        case 's': {
            char *p = va_arg(ap, char *);
            int c;
            while ((c = fgetc(stream)) != EOF && isspace(c))
                ;
            if (c == EOF)
                goto done_vfscanf;
            while (c != EOF && !isspace(c)) {
                *p++ = (char)c;
                c = fgetc(stream);
            }
            if (c != EOF)
                ungetc(c, stream);
            *p = '\0';
            matched++;
            break;
        }
        case 'c': {
            char *p = va_arg(ap, char *);
            int c = fgetc(stream);
            if (c == EOF)
                goto done_vfscanf;
            *p = (char)c;
            matched++;
            break;
        }
        default:
            goto done_vfscanf;
        }
        fmt++;
    }

done_vfscanf:
    return matched;
}

int fscanf(FILE *stream, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vfscanf(stream, fmt, ap);
    va_end(ap);
    return ret;
}

int scanf(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vfscanf(stdin, fmt, ap);
    va_end(ap);
    return ret;
}

/* ========================================================================= */
/* Ungetc                                                                    */
/* ========================================================================= */

int ungetc(int c, FILE *stream)
{
    if (c == EOF || !stream)
        return EOF;

    __ensure_buf(stream);
    if (stream->buf == NULL)
        return EOF;

    /* If the buffer has space at the front, push back. */
    if (stream->buf_pos > 0) {
        stream->buf[--stream->buf_pos] = (unsigned char)c;
    } else {
        /* Shift buffer contents right to make room. */
        if (stream->buf_len >= stream->buf_size)
            return EOF;
        memmove(stream->buf + 1, stream->buf, stream->buf_len);
        stream->buf[0] = (unsigned char)c;
        stream->buf_len++;
    }

    stream->flags &= ~__FILE_EOF;
    return (unsigned char)c;
}

/* ========================================================================= */
/* Buffer control                                                            */
/* ========================================================================= */

int setvbuf(FILE *stream, char *buf, int mode, size_t size)
{
    if (!stream)
        return -1;

    /* Validate mode. */
    if (mode != _IOFBF && mode != _IOLBF && mode != _IONBF)
        return -1;

    /* Flush any existing buffered data. */
    if (stream->flags & __FILE_WRITE)
        fflush(stream);

    /* Free old buffer if we own it. */
    if (stream->flags & __FILE_MYBUF) {
        free(stream->buf);
        stream->flags &= ~__FILE_MYBUF;
    }

    stream->buf_mode = mode;
    stream->buf_pos = 0;
    stream->buf_len = 0;

    if (mode == _IONBF) {
        stream->buf = NULL;
        stream->buf_size = 0;
    } else if (buf) {
        stream->buf = (unsigned char *)buf;
        stream->buf_size = size;
    } else {
        stream->buf = (unsigned char *)malloc(size ? size : BUFSIZ);
        if (stream->buf) {
            stream->buf_size = size ? size : BUFSIZ;
            stream->flags |= __FILE_MYBUF;
        } else {
            stream->buf_mode = _IONBF;
            stream->buf_size = 0;
        }
    }
    return 0;
}

void setbuf(FILE *stream, char *buf)
{
    setvbuf(stream, buf, buf ? _IOFBF : _IONBF, BUFSIZ);
}

void setlinebuf(FILE *stream)
{
    setvbuf(stream, NULL, _IOLBF, BUFSIZ);
}

/* ========================================================================= */
/* Temporary files                                                           */
/* ========================================================================= */

static int __tmpnam_counter = 0;

FILE *tmpfile(void)
{
    char tmpl[] = "/tmp/tmpXXXXXX";
    int fd = mkstemp(tmpl);
    if (fd < 0)
        return NULL;

    /* Unlink immediately so the file disappears on close. */
    unlink(tmpl);

    FILE *f = fdopen(fd, "w+b");
    if (!f) {
        close(fd);
        return NULL;
    }
    return f;
}

char *tmpnam(char *s)
{
    static char __tmpnam_buf[L_tmpnam];
    char *buf = s ? s : __tmpnam_buf;

    snprintf(buf, L_tmpnam, "/tmp/tmp%06d", __tmpnam_counter++);
    return buf;
}

/* ========================================================================= */
/* File descriptor / FILE stream bridging                                    */
/* ========================================================================= */

int fileno(FILE *stream)
{
    if (!stream) {
        errno = EBADF;
        return -1;
    }
    return stream->fd;
}

FILE *fdopen(int fd, const char *mode)
{
    if (fd < 0) {
        errno = EBADF;
        return NULL;
    }

    int fflags = 0;
    switch (mode[0]) {
    case 'r':
        fflags = __FILE_READ;
        break;
    case 'w':
        fflags = __FILE_WRITE;
        break;
    case 'a':
        fflags = __FILE_WRITE | __FILE_APPEND;
        break;
    default:
        errno = EINVAL;
        return NULL;
    }

    if (mode[1] == '+' || (mode[1] && mode[2] == '+'))
        fflags |= __FILE_READ | __FILE_WRITE;

    FILE *f = (FILE *)malloc(sizeof(FILE));
    if (!f)
        return NULL;

    memset(f, 0, sizeof(FILE));
    f->fd = fd;
    f->flags = fflags;
    f->buf_mode = _IOFBF;
    return f;
}

/* ========================================================================= */
/* Misc file operations                                                      */
/* ========================================================================= */

int remove(const char *pathname)
{
    /* Try unlink first; if it fails with EISDIR, try rmdir. */
    extern int unlink(const char *);
    extern int rmdir(const char *);

    if (unlink(pathname) == 0)
        return 0;
    return rmdir(pathname);
}
