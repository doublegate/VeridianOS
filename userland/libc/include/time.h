/*
 * VeridianOS libc -- <time.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Time types and functions.  The struct timespec, struct timeval, time_t
 * and clockid_t types are defined in <veridian/types.h>.
 */

#ifndef _TIME_H
#define _TIME_H

#include <veridian/types.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Clock functions                                                           */
/* ========================================================================= */

/**
 * Retrieve the time of the specified clock.
 *
 * @param clk_id   CLOCK_REALTIME, CLOCK_MONOTONIC, etc.
 * @param tp       Receives the current time.
 * @return 0 on success, -1 on error.
 */
int clock_gettime(clockid_t clk_id, struct timespec *tp);

/**
 * Retrieve the resolution of the specified clock.
 *
 * @param clk_id   CLOCK_REALTIME, CLOCK_MONOTONIC, etc.
 * @param res      Receives the clock resolution.
 * @return 0 on success, -1 on error.
 */
int clock_getres(clockid_t clk_id, struct timespec *res);

/* ========================================================================= */
/* Sleep                                                                     */
/* ========================================================================= */

/**
 * High-resolution sleep.
 *
 * @param req      Requested sleep duration.
 * @param rem      If non-NULL and interrupted, receives remaining time.
 * @return 0 on success, -1 if interrupted (errno == EINTR).
 */
int nanosleep(const struct timespec *req, struct timespec *rem);

/* ========================================================================= */
/* Legacy time-of-day                                                        */
/* ========================================================================= */

/** Timezone structure (obsolete, kept for compatibility). */
struct timezone {
    int tz_minuteswest;     /* Minutes west of Greenwich */
    int tz_dsttime;         /* Type of DST correction */
};

/**
 * Get the current time of day (legacy interface).
 *
 * @param tv       Receives current time (may be NULL).
 * @param tz       Timezone (ignored on VeridianOS, may be NULL).
 * @return 0 on success, -1 on error.
 */
int gettimeofday(struct timeval *tv, struct timezone *tz);

/**
 * Return the current calendar time.
 *
 * @param tloc     If non-NULL, also stores the time here.
 * @return Current time in seconds since epoch, or (time_t)-1 on error.
 */
time_t time(time_t *tloc);

/* ========================================================================= */
/* Broken-down time (minimal)                                                */
/* ========================================================================= */

/** Broken-down time representation. */
struct tm {
    int tm_sec;     /* Seconds [0, 60] (60 for leap second) */
    int tm_min;     /* Minutes [0, 59] */
    int tm_hour;    /* Hours [0, 23] */
    int tm_mday;    /* Day of month [1, 31] */
    int tm_mon;     /* Month [0, 11] */
    int tm_year;    /* Years since 1900 */
    int tm_wday;    /* Day of week [0, 6] (Sunday = 0) */
    int tm_yday;    /* Day of year [0, 365] */
    int tm_isdst;   /* Daylight saving time flag */
};

/** Convert time_t to broken-down UTC time. */
struct tm *gmtime(const time_t *timep);

/** Convert broken-down time back to time_t. */
time_t mktime(struct tm *tm);

#ifdef __cplusplus
}
#endif

#endif /* _TIME_H */
