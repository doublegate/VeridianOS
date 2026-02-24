/*
 * VeridianOS libc -- <stdlib.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * General utilities: memory allocation, process control, conversions.
 */

#ifndef _STDLIB_H
#define _STDLIB_H

#include <stddef.h>
#include <alloca.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

#define EXIT_SUCCESS    0
#define EXIT_FAILURE    1

/** Maximum value returned by rand(). */
#define RAND_MAX        2147483647

/** Maximum bytes in a multibyte character for the current locale. */
#ifndef MB_CUR_MAX
#define MB_CUR_MAX      1   /* Single-byte locale only */
#endif

/* ========================================================================= */
/* Memory allocation                                                         */
/* ========================================================================= */

/** Allocate size bytes of uninitialized memory. */
void *malloc(size_t size);

/** Allocate zero-initialized memory for count objects of size bytes each. */
void *calloc(size_t count, size_t size);

/** Resize a previously allocated block. */
void *realloc(void *ptr, size_t size);

/** Free a previously allocated block. */
void free(void *ptr);

/* ========================================================================= */
/* Process control                                                           */
/* ========================================================================= */

/** Terminate the process normally (runs atexit handlers, flushes stdio). */
void exit(int status) __attribute__((noreturn));

/** Terminate immediately without cleanup. */
void _Exit(int status) __attribute__((noreturn));

/** Register a function to be called at normal process exit. */
int atexit(void (*func)(void));

/** Abnormal termination (sends SIGABRT). */
void abort(void) __attribute__((noreturn));

/* ========================================================================= */
/* Environment                                                               */
/* ========================================================================= */

/** Pointer to the environment variable array. */
extern char **environ;

/** Look up an environment variable by name. */
char *getenv(const char *name);

/** Set or overwrite an environment variable. */
int setenv(const char *name, const char *value, int overwrite);

/** Remove an environment variable. */
int unsetenv(const char *name);

/** Insert or reset a NAME=VALUE environment string. */
int putenv(char *string);

/** Clear the entire environment. */
int clearenv(void);

/* ========================================================================= */
/* String-to-number conversions                                              */
/* ========================================================================= */

/** Convert string to int. */
int atoi(const char *nptr);

/** Convert string to long. */
long atol(const char *nptr);

/** Convert string to long with base and error detection. */
long strtol(const char *nptr, char **endptr, int base);

/** Convert string to unsigned long. */
unsigned long strtoul(const char *nptr, char **endptr, int base);

/** Convert string to long long. */
long long strtoll(const char *nptr, char **endptr, int base);

/** Convert string to unsigned long long. */
unsigned long long strtoull(const char *nptr, char **endptr, int base);

/** Convert string to double. */
double strtod(const char *nptr, char **endptr);

/** Convert string to float. */
float strtof(const char *nptr, char **endptr);

/** Convert string to double (convenience). */
double atof(const char *nptr);

/* ========================================================================= */
/* Pseudo-random number generation                                           */
/* ========================================================================= */

/** Return a pseudo-random integer in [0, RAND_MAX]. */
int rand(void);

/** Seed the random number generator. */
void srand(unsigned int seed);

/* ========================================================================= */
/* Sorting / searching                                                       */
/* ========================================================================= */

/** Sort an array. */
void qsort(void *base, size_t nmemb, size_t size,
            int (*compar)(const void *, const void *));

/** Binary search a sorted array. */
void *bsearch(const void *key, const void *base, size_t nmemb,
              size_t size, int (*compar)(const void *, const void *));

/* ========================================================================= */
/* Integer arithmetic                                                        */
/* ========================================================================= */

/** Absolute value. */
int abs(int j);
long labs(long j);
long long llabs(long long j);

/** Division result with quotient and remainder. */
typedef struct { int quot; int rem; } div_t;
typedef struct { long quot; long rem; } ldiv_t;
typedef struct { long long quot; long long rem; } lldiv_t;

div_t div(int numer, int denom);
ldiv_t ldiv(long numer, long denom);
lldiv_t lldiv(long long numer, long long denom);

/** Convert string to long long. */
long long atoll(const char *nptr);

/** Convert string to long double (stub -- returns double precision). */
long double strtold(const char *nptr, char **endptr);

/** Allocate aligned memory (C11). */
void *aligned_alloc(size_t alignment, size_t size);

/* ========================================================================= */
/* Multibyte/wide character stubs                                            */
/* ========================================================================= */

#ifndef __wchar_t_defined
#define __wchar_t_defined
#ifndef __cplusplus
typedef int wchar_t;
#endif
#endif

/** Determine length of a multibyte character. */
int mblen(const char *s, size_t n);

/** Convert multibyte string to wide-char string. */
size_t mbstowcs(wchar_t *dest, const char *src, size_t n);

/** Convert a single multibyte character to wide character. */
int mbtowc(wchar_t *pwc, const char *s, size_t n);

/** Convert wide-char string to multibyte string. */
size_t wcstombs(char *dest, const wchar_t *src, size_t n);

/** Convert a wide character to multibyte character. */
int wctomb(char *s, wchar_t wc);

/* ========================================================================= */
/* Temporary files                                                           */
/* ========================================================================= */

/**
 * Create a unique temporary file from a template.
 *
 * The last six characters of template must be "XXXXXX" and will be
 * replaced to produce a unique filename.  The file is opened with
 * O_CREAT | O_EXCL | O_RDWR, mode 0600.
 *
 * @param tmpl  Mutable path ending in "XXXXXX".
 * @return Open file descriptor on success, -1 on error.
 */
int mkstemp(char *tmpl);

/**
 * Generate a unique temporary filename (deprecated -- prefer mkstemp).
 *
 * Replaces the trailing "XXXXXX" in template with random characters.
 * Does NOT create the file.  Subject to race conditions.
 *
 * @param tmpl  Mutable path ending in "XXXXXX".
 * @return template on success (modified in place), or template with
 *         first byte set to '\0' on error.
 */
char *mktemp(char *tmpl);

/* ========================================================================= */
/* Command execution                                                         */
/* ========================================================================= */

/**
 * Execute a command via the shell.
 *
 * If command is NULL, returns non-zero (shell is available).
 * Otherwise fork/exec "/bin/sh -c command" and return the exit status.
 *
 * @param command  Shell command string, or NULL to test shell availability.
 * @return Exit status of the command, or -1 on error.
 */
int system(const char *command);

/* ========================================================================= */
/* System information                                                        */
/* ========================================================================= */

/**
 * Get system load averages.
 *
 * @param loadavg  Array to fill with 1, 5, 15-minute load averages.
 * @param nelem    Number of elements to fill (up to 3).
 * @return Number of averages returned, or -1 on error.
 */
int getloadavg(double loadavg[], int nelem);

#ifdef __cplusplus
}
#endif

#endif /* _STDLIB_H */
