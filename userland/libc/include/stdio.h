/*
 * VeridianOS libc -- <stdio.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal buffered I/O: FILE, fopen/fclose, fread/fwrite, printf family,
 * puts, fgets, fputs, fputc, fgetc, fflush, and the standard streams.
 */

#ifndef _STDIO_H
#define _STDIO_H

#include <stddef.h>
#include <stdarg.h>

/* ssize_t for getline/getdelim -- avoid pulling in veridian/types.h */
#ifndef __ssize_t_defined
#define __ssize_t_defined
typedef long ssize_t;
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

/** Default I/O buffer size. */
#define BUFSIZ          1024

/** End-of-file indicator. */
#define EOF             (-1)

/** Maximum number of simultaneously open FILE streams. */
#define FOPEN_MAX       64

/** Maximum file name length. */
#define FILENAME_MAX    256

/** Seek whence values (also in <fcntl.h>). */
#ifndef SEEK_SET
#define SEEK_SET        0
#define SEEK_CUR        1
#define SEEK_END        2
#endif

/** File position type (for fgetpos/fsetpos). */
typedef long fpos_t;

/** Buffering modes for setvbuf(). */
#define _IOFBF          0   /* Fully buffered */
#define _IOLBF          1   /* Line buffered */
#define _IONBF          2   /* Unbuffered */

/* ========================================================================= */
/* FILE structure                                                            */
/* ========================================================================= */

/** Internal flags for FILE. */
#define __FILE_READ     0x01
#define __FILE_WRITE    0x02
#define __FILE_APPEND   0x04
#define __FILE_EOF      0x08
#define __FILE_ERROR    0x10
#define __FILE_MYBUF    0x20    /* Buffer was allocated by stdio */

typedef struct _FILE {
    int             fd;             /* Underlying file descriptor */
    int             flags;          /* Internal flags */
    int             buf_mode;       /* _IOFBF, _IOLBF, or _IONBF */
    unsigned char  *buf;            /* I/O buffer */
    size_t          buf_size;       /* Size of buffer */
    size_t          buf_pos;        /* Current position in buffer */
    size_t          buf_len;        /* Valid bytes in buffer (read mode) */
} FILE;

/* ========================================================================= */
/* Standard streams                                                          */
/* ========================================================================= */

extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

/* ========================================================================= */
/* File operations                                                           */
/* ========================================================================= */

/** Open a file. */
FILE *fopen(const char *pathname, const char *mode);

/** Close a file. */
int fclose(FILE *stream);

/** Flush buffered output to the underlying fd. */
int fflush(FILE *stream);

/** Reopen a stream with a different file or mode. */
FILE *freopen(const char *pathname, const char *mode, FILE *stream);

/* ========================================================================= */
/* Character I/O                                                             */
/* ========================================================================= */

/** Read one character (returns EOF on end-of-file or error). */
int fgetc(FILE *stream);

/** Write one character. */
int fputc(int c, FILE *stream);

/** Push a character back onto the input stream. */
int ungetc(int c, FILE *stream);

/** Function declarations (required by C++ <cstdio>). */
int getc(FILE *stream);
int putc(int c, FILE *stream);
int getchar(void);
int putchar(int c);

/** Macro overrides for C (not applied in C++ to avoid conflicts). */
#ifndef __cplusplus
#define getc(stream)        fgetc(stream)
#define putc(c, stream)     fputc((c), (stream))
#define getchar()           fgetc(stdin)
#define putchar(c)          fputc((c), stdout)
#endif

/* ========================================================================= */
/* String I/O                                                                */
/* ========================================================================= */

/** Read a line (up to size-1 chars, or newline). */
char *fgets(char *s, int size, FILE *stream);

/** Write a string. */
int fputs(const char *s, FILE *stream);

/** Write a string followed by a newline to stdout. */
int puts(const char *s);

/* ========================================================================= */
/* Block I/O                                                                 */
/* ========================================================================= */

/** Read count objects of size bytes each. */
size_t fread(void *ptr, size_t size, size_t count, FILE *stream);

/** Write count objects of size bytes each. */
size_t fwrite(const void *ptr, size_t size, size_t count, FILE *stream);

/* ========================================================================= */
/* Seeking                                                                   */
/* ========================================================================= */

/** Set stream position. */
int fseek(FILE *stream, long offset, int whence);

/** Get stream position. */
long ftell(FILE *stream);

/** Reset stream to beginning. */
void rewind(FILE *stream);

/** Get stream position (for fsetpos). */
int fgetpos(FILE *stream, fpos_t *pos);

/** Set stream position (from fgetpos). */
int fsetpos(FILE *stream, const fpos_t *pos);

/* ========================================================================= */
/* Error / EOF queries                                                       */
/* ========================================================================= */

/** Test end-of-file indicator. */
int feof(FILE *stream);

/** Test error indicator. */
int ferror(FILE *stream);

/** Clear error and EOF indicators. */
void clearerr(FILE *stream);

/** Print a message describing the current errno. */
void perror(const char *s);

/* ========================================================================= */
/* Formatted output                                                          */
/* ========================================================================= */

/** Print to stdout. */
int printf(const char *fmt, ...) __attribute__((format(printf, 1, 2)));

/** Print to a FILE stream. */
int fprintf(FILE *stream, const char *fmt, ...)
    __attribute__((format(printf, 2, 3)));

/** Print into a buffer (at most size bytes including NUL). */
int snprintf(char *buf, size_t size, const char *fmt, ...)
    __attribute__((format(printf, 3, 4)));

/** Print into a buffer (no bounds check -- avoid). */
int sprintf(char *buf, const char *fmt, ...)
    __attribute__((format(printf, 2, 3)));

/** va_list variants. */
int vprintf(const char *fmt, va_list ap);
int vfprintf(FILE *stream, const char *fmt, va_list ap);
int vsnprintf(char *buf, size_t size, const char *fmt, va_list ap);
int vsprintf(char *buf, const char *fmt, va_list ap);

/* ========================================================================= */
/* Formatted input                                                           */
/* ========================================================================= */

/** Read formatted input from a string. */
int sscanf(const char *str, const char *fmt, ...)
    __attribute__((format(scanf, 2, 3)));

/** Read formatted input from a FILE stream. */
int fscanf(FILE *stream, const char *fmt, ...)
    __attribute__((format(scanf, 2, 3)));

/** Read formatted input from stdin. */
int scanf(const char *fmt, ...)
    __attribute__((format(scanf, 1, 2)));

/** va_list variant of fscanf. */
int vfscanf(FILE *stream, const char *fmt, va_list ap);

/** va_list variant of scanf. */
int vscanf(const char *fmt, va_list ap);

/** va_list variant of sscanf. */
int vsscanf(const char *str, const char *fmt, va_list ap);

/* ========================================================================= */
/* Buffer control                                                            */
/* ========================================================================= */

/** Set stream buffering mode. */
int setvbuf(FILE *stream, char *buf, int mode, size_t size);

/** Set stream buffer (convenience wrapper for setvbuf). */
void setbuf(FILE *stream, char *buf);

/** Set line buffering on a stream. */
void setlinebuf(FILE *stream);

/* ========================================================================= */
/* Temporary files                                                           */
/* ========================================================================= */

/** Maximum length of a tmpnam-generated path. */
#define L_tmpnam        20

/** Create a temporary file (opened with "w+b"). */
FILE *tmpfile(void);

/** Generate a unique temporary filename. */
char *tmpnam(char *s);

/** Get file descriptor number from FILE stream. */
int fileno(FILE *stream);

/** Open a stream from an existing file descriptor. */
FILE *fdopen(int fd, const char *mode);

/* ========================================================================= */
/* Misc                                                                      */
/* ========================================================================= */

/** Remove a file. */
int remove(const char *pathname);

/** Rename a file. */
int rename(const char *oldpath, const char *newpath);

/* ========================================================================= */
/* POSIX extensions                                                          */
/* ========================================================================= */

/**
 * Read an entire line from stream, allocating or resizing the buffer
 * as needed.  On success *lineptr contains the line (including newline)
 * and *n holds the buffer size.  Returns the number of characters read
 * (including newline, excluding NUL), or -1 on failure or EOF.
 */
ssize_t getline(char **lineptr, size_t *n, FILE *stream);

/**
 * Read until delimiter, allocating/resizing buffer as needed.
 */
ssize_t getdelim(char **lineptr, size_t *n, int delim, FILE *stream);

/* ========================================================================= */
/* Process I/O                                                               */
/* ========================================================================= */

/** Open a pipe to/from a process. */
FILE *popen(const char *command, const char *type);

/** Close a pipe opened by popen(). */
int pclose(FILE *stream);

#ifdef __cplusplus
}
#endif

#endif /* _STDIO_H */
