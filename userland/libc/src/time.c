/*
 * VeridianOS libc -- time.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Time-related functions backed by VeridianOS syscalls.
 */

#include <time.h>
#include <veridian/syscall.h>
#include <errno.h>
#include <stddef.h>

/* ========================================================================= */
/* clock_gettime                                                             */
/* ========================================================================= */

int clock_gettime(clockid_t clk_id, struct timespec *tp)
{
    long ret = veridian_syscall2(SYS_CLOCK_GETTIME, clk_id, tp);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

int clock_getres(clockid_t clk_id, struct timespec *res)
{
    long ret = veridian_syscall2(SYS_CLOCK_GETRES, clk_id, res);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* nanosleep                                                                 */
/* ========================================================================= */

int nanosleep(const struct timespec *req, struct timespec *rem)
{
    long ret = veridian_syscall2(SYS_NANOSLEEP, req, rem);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* gettimeofday                                                              */
/* ========================================================================= */

int gettimeofday(struct timeval *tv, struct timezone *tz)
{
    (void)tz;   /* Timezone is ignored on VeridianOS. */

    if (!tv)
        return 0;

    long ret = veridian_syscall2(SYS_GETTIMEOFDAY, tv, tz);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* time                                                                      */
/* ========================================================================= */

time_t time(time_t *tloc)
{
    struct timespec ts;
    if (clock_gettime(CLOCK_REALTIME, &ts) < 0)
        return (time_t)-1;

    if (tloc)
        *tloc = ts.tv_sec;
    return ts.tv_sec;
}

/* ========================================================================= */
/* gmtime (minimal, no leap second handling)                                 */
/* ========================================================================= */

/* Days per month (non-leap year). */
static const int days_per_month[] = {
    31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31
};

static int __is_leap(int year)
{
    return (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
}

static struct tm __gmtime_buf;

struct tm *gmtime(const time_t *timep)
{
    time_t t = *timep;
    struct tm *tm = &__gmtime_buf;

    /* Seconds within the day. */
    long dayclock = (long)(t % 86400);
    long dayno = (long)(t / 86400);

    if (dayclock < 0) {
        dayclock += 86400;
        dayno--;
    }

    tm->tm_sec  = (int)(dayclock % 60);
    tm->tm_min  = (int)((dayclock / 60) % 60);
    tm->tm_hour = (int)(dayclock / 3600);

    /* Day of week: Jan 1, 1970 was Thursday (4). */
    tm->tm_wday = (int)((dayno + 4) % 7);
    if (tm->tm_wday < 0)
        tm->tm_wday += 7;

    /* Year calculation. */
    int year = 1970;
    while (dayno >= (__is_leap(year) ? 366 : 365)) {
        dayno -= __is_leap(year) ? 366 : 365;
        year++;
    }
    while (dayno < 0) {
        year--;
        dayno += __is_leap(year) ? 366 : 365;
    }
    tm->tm_year = year - 1900;
    tm->tm_yday = (int)dayno;

    /* Month calculation. */
    int month;
    for (month = 0; month < 12; month++) {
        int dim = days_per_month[month];
        if (month == 1 && __is_leap(year))
            dim = 29;
        if (dayno < dim)
            break;
        dayno -= dim;
    }
    tm->tm_mon  = month;
    tm->tm_mday = (int)dayno + 1;
    tm->tm_isdst = 0;

    return tm;
}

/* ========================================================================= */
/* mktime (minimal)                                                          */
/* ========================================================================= */

time_t mktime(struct tm *tm)
{
    int year = tm->tm_year + 1900;
    int month = tm->tm_mon;
    int day = tm->tm_mday;

    /* Normalize month. */
    while (month < 0) {
        month += 12;
        year--;
    }
    while (month >= 12) {
        month -= 12;
        year++;
    }

    /* Days from epoch (Jan 1, 1970) to Jan 1 of `year`. */
    long days = 0;
    if (year >= 1970) {
        for (int y = 1970; y < year; y++)
            days += __is_leap(y) ? 366 : 365;
    } else {
        for (int y = year; y < 1970; y++)
            days -= __is_leap(y) ? 366 : 365;
    }

    /* Add days for complete months. */
    for (int m = 0; m < month; m++) {
        days += days_per_month[m];
        if (m == 1 && __is_leap(year))
            days++;
    }

    days += day - 1;

    time_t result = (time_t)days * 86400
                  + (time_t)tm->tm_hour * 3600
                  + (time_t)tm->tm_min * 60
                  + (time_t)tm->tm_sec;

    /* Fill in derived fields. */
    struct tm *back = gmtime(&result);
    if (back)
        *tm = *back;

    return result;
}
