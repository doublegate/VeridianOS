/*
 * VeridianOS libc -- POSIX function stubs
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal stub implementations for POSIX functions required by
 * cross-compiled user-space programs (GNU Make, etc.).
 * These return appropriate error values until full implementations exist.
 */

#include <stddef.h>
#include <errno.h>
#include <time.h>
#include <pwd.h>
#include <dlfcn.h>

/* ========================================================================= */
/* Time functions                                                            */
/* ========================================================================= */

static struct tm _stub_tm;
static char _stub_time_buf[26] = "Thu Jan  1 00:00:00 1970\n";

struct tm *localtime(const time_t *timep)
{
    /* Stub: return a zeroed struct tm (epoch) */
    (void)timep;
    _stub_tm.tm_year = 70;
    _stub_tm.tm_mday = 1;
    return &_stub_tm;
}

char *ctime(const time_t *timep)
{
    (void)timep;
    return _stub_time_buf;
}

char *asctime(const struct tm *tm)
{
    (void)tm;
    return _stub_time_buf;
}

double difftime(time_t time1, time_t time0)
{
    return (double)(time1 - time0);
}

size_t strftime(char *s, size_t max, const char *format, const struct tm *tm)
{
    (void)format;
    (void)tm;
    if (max > 0)
        s[0] = '\0';
    return 0;
}

/* ========================================================================= */
/* Environment                                                               */
/* ========================================================================= */

int putenv(char *string)
{
    (void)string;
    /* Stub: no environment support yet */
    return 0;
}

/* ========================================================================= */
/* User/Group database                                                       */
/* ========================================================================= */

static struct passwd _stub_pw = {
    .pw_name   = "root",
    .pw_passwd = "",
    .pw_uid    = 0,
    .pw_gid    = 0,
    .pw_gecos  = "root",
    .pw_dir    = "/",
    .pw_shell  = "/bin/sh"
};

struct passwd *getpwnam(const char *name)
{
    (void)name;
    return &_stub_pw;
}

struct passwd *getpwuid(uid_t uid)
{
    (void)uid;
    return &_stub_pw;
}

char *getlogin(void)
{
    return "root";
}

/* ========================================================================= */
/* Dynamic loading (stubs -- no dynamic linking on VeridianOS yet)            */
/* ========================================================================= */

static char _dl_error[] = "dynamic loading not supported";

void *dlopen(const char *filename, int flags)
{
    (void)filename;
    (void)flags;
    return NULL;
}

void *dlsym(void *handle, const char *symbol)
{
    (void)handle;
    (void)symbol;
    return NULL;
}

int dlclose(void *handle)
{
    (void)handle;
    return -1;
}

char *dlerror(void)
{
    return _dl_error;
}

/* ========================================================================= */
/* Assert                                                                    */
/* ========================================================================= */

/* Forward-declare write() to avoid pulling in unistd.h */
long write(int fd, const void *buf, unsigned long count);
void _Exit(int status) __attribute__((noreturn));

void __assert_fail(const char *expr, const char *file,
                   unsigned int line, const char *func)
{
    /* Minimal assertion failure: print message to stderr, then abort */
    const char *msg = "Assertion failed: ";
    write(2, msg, 18);
    if (expr)
        write(2, expr, __builtin_strlen(expr));
    write(2, "\n", 1);
    (void)file;
    (void)line;
    (void)func;
    _Exit(134); /* 128 + SIGABRT(6) */
}

/* ========================================================================= */
/* File locking                                                              */
/* ========================================================================= */

int flock(int fd, int operation)
{
    (void)fd;
    (void)operation;
    return 0;
}

/* gethostname() is already implemented in unistd.c */

/* ========================================================================= */
/* regex -- full implementation in regex.c (regcomp, regexec, regfree,       */
/*          regerror).  Stubs removed in Sprint B-17.                        */
/* ========================================================================= */
