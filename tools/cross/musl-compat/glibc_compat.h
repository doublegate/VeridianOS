/* Compatibility defines for using GCC's C++ headers with musl libc.
 * musl doesn't define glibc-specific macros that libstdc++ headers expect.
 */
#ifndef _GLIBC_COMPAT_H
#define _GLIBC_COMPAT_H

/* musl uses locale_t directly; glibc uses __locale_t as internal name */
#include <locale.h>
typedef locale_t __locale_t;

/* musl doesn't provide __GLIBC_PREREQ */
#ifndef __GLIBC_PREREQ
#define __GLIBC_PREREQ(maj, min) 0
#endif

/* musl doesn't define __THROW */
#ifndef __THROW
#define __THROW
#endif

#ifndef __NTH
#define __NTH(fct) fct
#endif

/* GCC 15 libstdc++ headers reference glibc-specific pthread extensions.
 * Provide inline stubs that fall back to standard POSIX equivalents. */
#ifdef __cplusplus
extern "C" {
#endif

#include <pthread.h>
#include <errno.h>
#include <time.h>

/* pthread_cond_clockwait - glibc extension for clock-specific condvar wait.
 * Fall back to pthread_cond_timedwait (always uses CLOCK_REALTIME). */
static inline int pthread_cond_clockwait(pthread_cond_t *cond,
                                          pthread_mutex_t *mutex,
                                          clockid_t clock_id,
                                          const struct timespec *abstime) {
    (void)clock_id;
    return pthread_cond_timedwait(cond, mutex, abstime);
}

/* pthread_mutex_clocklock - glibc extension for clock-specific mutex lock.
 * Fall back to pthread_mutex_timedlock. */
static inline int pthread_mutex_clocklock(pthread_mutex_t *mutex,
                                           clockid_t clock_id,
                                           const struct timespec *abstime) {
    (void)clock_id;
    return pthread_mutex_timedlock(mutex, abstime);
}

#ifdef __cplusplus
}
#endif

#endif /* _GLIBC_COMPAT_H */
