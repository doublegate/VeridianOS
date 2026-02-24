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

/*
 * strftime -- format broken-down time into a string buffer.
 *
 * Supports the following POSIX format specifiers:
 *   %a %A %b %B %c %C %d %D %e %F %H %I %j %m %M %n %p %r
 *   %S %t %T %u %w %x %X %y %Y %Z %%
 *
 * All locale-dependent specifiers use the C/POSIX locale.
 */

/* Forward-declare snprintf to avoid pulling in <stdio.h> (which pulls
 * in our FILE type and may conflict with the local typedef above). */
extern int snprintf(char *buf, unsigned long size, const char *fmt, ...);

static const char *__wday_abbr[] = {
    "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"
};
static const char *__wday_full[] = {
    "Sunday", "Monday", "Tuesday", "Wednesday",
    "Thursday", "Friday", "Saturday"
};
static const char *__mon_abbr[] = {
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
};
static const char *__mon_full[] = {
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December"
};

/*
 * Append at most (max - *pos) characters from src to s at offset *pos.
 * Returns 0 on success, -1 if the output would overflow.
 */
static int __strftime_append(char *s, size_t max, size_t *pos, const char *src)
{
    while (*src) {
        if (*pos + 1 >= max)
            return -1;
        s[(*pos)++] = *src++;
    }
    return 0;
}

/*
 * Append a fixed-width zero-padded integer to the buffer.
 */
static int __strftime_int(char *s, size_t max, size_t *pos,
                          int val, int width)
{
    char tmp[16];
    int i = 0;
    int neg = 0;

    if (val < 0) {
        neg = 1;
        val = -val;
    }

    /* Generate digits in reverse. */
    do {
        tmp[i++] = '0' + (val % 10);
        val /= 10;
    } while (val > 0);

    /* Pad with zeros. */
    while (i < width)
        tmp[i++] = '0';

    if (neg) {
        if (*pos + 1 >= max)
            return -1;
        s[(*pos)++] = '-';
    }

    /* Write in correct order. */
    while (i > 0) {
        if (*pos + 1 >= max)
            return -1;
        s[(*pos)++] = tmp[--i];
    }
    return 0;
}

/*
 * Append a space-padded integer (used by %e).
 */
static int __strftime_int_space(char *s, size_t max, size_t *pos,
                                int val, int width)
{
    char tmp[16];
    int i = 0;

    if (val < 0) val = 0;

    do {
        tmp[i++] = '0' + (val % 10);
        val /= 10;
    } while (val > 0);

    while (i < width)
        tmp[i++] = ' ';

    while (i > 0) {
        if (*pos + 1 >= max)
            return -1;
        s[(*pos)++] = tmp[--i];
    }
    return 0;
}

size_t strftime(char *s, size_t max, const char *format, const struct tm *tm)
{
    if (!s || max == 0 || !format || !tm)
        return 0;

    size_t pos = 0;

    while (*format) {
        if (*format != '%') {
            if (pos + 1 >= max)
                return 0;
            s[pos++] = *format++;
            continue;
        }
        format++; /* skip '%' */

        switch (*format) {
        case '\0':
            /* Trailing '%' with no specifier -- stop. */
            goto done;

        case '%':
            if (pos + 1 >= max) return 0;
            s[pos++] = '%';
            break;

        case 'a': /* Abbreviated weekday name */
            if (tm->tm_wday >= 0 && tm->tm_wday <= 6) {
                if (__strftime_append(s, max, &pos, __wday_abbr[tm->tm_wday]) < 0)
                    return 0;
            }
            break;

        case 'A': /* Full weekday name */
            if (tm->tm_wday >= 0 && tm->tm_wday <= 6) {
                if (__strftime_append(s, max, &pos, __wday_full[tm->tm_wday]) < 0)
                    return 0;
            }
            break;

        case 'b': /* Abbreviated month name */
        case 'h': /* Same as %b */
            if (tm->tm_mon >= 0 && tm->tm_mon <= 11) {
                if (__strftime_append(s, max, &pos, __mon_abbr[tm->tm_mon]) < 0)
                    return 0;
            }
            break;

        case 'B': /* Full month name */
            if (tm->tm_mon >= 0 && tm->tm_mon <= 11) {
                if (__strftime_append(s, max, &pos, __mon_full[tm->tm_mon]) < 0)
                    return 0;
            }
            break;

        case 'c': /* Locale date and time: "%a %b %e %H:%M:%S %Y" */
        {
            size_t r = strftime(s + pos, max - pos,
                                "%a %b %e %H:%M:%S %Y", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'C': /* Century (year / 100) */
            if (__strftime_int(s, max, &pos, (tm->tm_year + 1900) / 100, 2) < 0)
                return 0;
            break;

        case 'd': /* Day of month, zero-padded [01, 31] */
            if (__strftime_int(s, max, &pos, tm->tm_mday, 2) < 0)
                return 0;
            break;

        case 'D': /* Equivalent to "%m/%d/%y" */
        {
            size_t r = strftime(s + pos, max - pos, "%m/%d/%y", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'e': /* Day of month, space-padded [ 1, 31] */
            if (__strftime_int_space(s, max, &pos, tm->tm_mday, 2) < 0)
                return 0;
            break;

        case 'F': /* Equivalent to "%Y-%m-%d" */
        {
            size_t r = strftime(s + pos, max - pos, "%Y-%m-%d", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'H': /* Hour (24-hour clock), zero-padded [00, 23] */
            if (__strftime_int(s, max, &pos, tm->tm_hour, 2) < 0)
                return 0;
            break;

        case 'I': /* Hour (12-hour clock), zero-padded [01, 12] */
        {
            int h = tm->tm_hour % 12;
            if (h == 0) h = 12;
            if (__strftime_int(s, max, &pos, h, 2) < 0)
                return 0;
            break;
        }

        case 'j': /* Day of year, zero-padded [001, 366] */
            if (__strftime_int(s, max, &pos, tm->tm_yday + 1, 3) < 0)
                return 0;
            break;

        case 'm': /* Month [01, 12] */
            if (__strftime_int(s, max, &pos, tm->tm_mon + 1, 2) < 0)
                return 0;
            break;

        case 'M': /* Minute [00, 59] */
            if (__strftime_int(s, max, &pos, tm->tm_min, 2) < 0)
                return 0;
            break;

        case 'n': /* Newline */
            if (pos + 1 >= max) return 0;
            s[pos++] = '\n';
            break;

        case 'p': /* AM/PM */
            if (__strftime_append(s, max, &pos,
                                  tm->tm_hour < 12 ? "AM" : "PM") < 0)
                return 0;
            break;

        case 'r': /* 12-hour time: "%I:%M:%S %p" */
        {
            size_t r = strftime(s + pos, max - pos, "%I:%M:%S %p", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'R': /* Equivalent to "%H:%M" */
        {
            size_t r = strftime(s + pos, max - pos, "%H:%M", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'S': /* Second [00, 60] (60 for leap second) */
            if (__strftime_int(s, max, &pos, tm->tm_sec, 2) < 0)
                return 0;
            break;

        case 't': /* Tab */
            if (pos + 1 >= max) return 0;
            s[pos++] = '\t';
            break;

        case 'T': /* Equivalent to "%H:%M:%S" */
        {
            size_t r = strftime(s + pos, max - pos, "%H:%M:%S", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'u': /* Weekday [1, 7] (Monday = 1) */
        {
            int wd = tm->tm_wday == 0 ? 7 : tm->tm_wday;
            if (__strftime_int(s, max, &pos, wd, 1) < 0)
                return 0;
            break;
        }

        case 'w': /* Weekday [0, 6] (Sunday = 0) */
            if (__strftime_int(s, max, &pos, tm->tm_wday, 1) < 0)
                return 0;
            break;

        case 'x': /* Locale date: "%m/%d/%y" */
        {
            size_t r = strftime(s + pos, max - pos, "%m/%d/%y", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'X': /* Locale time: "%H:%M:%S" */
        {
            size_t r = strftime(s + pos, max - pos, "%H:%M:%S", tm);
            if (r == 0 && max - pos > 1)
                return 0;
            pos += r;
            break;
        }

        case 'y': /* Year within century [00, 99] */
            if (__strftime_int(s, max, &pos, (tm->tm_year + 1900) % 100, 2) < 0)
                return 0;
            break;

        case 'Y': /* Full year (e.g. 2026) */
            if (__strftime_int(s, max, &pos, tm->tm_year + 1900, 4) < 0)
                return 0;
            break;

        case 'Z': /* Timezone name */
            if (__strftime_append(s, max, &pos, "UTC") < 0)
                return 0;
            break;

        default:
            /* Unknown specifier -- write it literally. */
            if (pos + 2 >= max) return 0;
            s[pos++] = '%';
            s[pos++] = *format;
            break;
        }

        format++;
    }

done:
    s[pos] = '\0';
    return pos;
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
