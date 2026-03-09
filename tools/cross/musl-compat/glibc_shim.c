/* glibc_shim.c - Provide glibc-specific symbols needed by system libstdc++.a
 *
 * When using a system GCC's libstdc++.a (compiled against glibc) with musl,
 * a handful of glibc-internal symbols are referenced but not present in musl.
 * This shim provides compatible implementations.
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>
#include <unistd.h>
#include <pthread.h>
#include <sys/random.h>

/* glibc fortified I/O - just forward to standard versions */
int __sprintf_chk(char *s, int flag, size_t slen, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vsprintf(s, fmt, ap);
    va_end(ap);
    return ret;
}

int __fprintf_chk(FILE *stream, int flag, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vfprintf(stream, fmt, ap);
    va_end(ap);
    return ret;
}

ssize_t __read_chk(int fd, void *buf, size_t nbytes, size_t buflen) {
    return read(fd, buf, nbytes);
}

/* C23 strtoul - forward to standard strtoul */
unsigned long __isoc23_strtoul(const char *nptr, char **endptr, int base) {
    return strtoul(nptr, endptr, base);
}

/* glibc single-threaded optimization flag - always say multi-threaded (safe) */
char __libc_single_threaded = 0;

/* arc4random - use getrandom() as backend */
unsigned int arc4random(void) {
    unsigned int val;
    if (getrandom(&val, sizeof(val), 0) != sizeof(val)) {
        /* fallback: read from /dev/urandom */
        FILE *f = fopen("/dev/urandom", "r");
        if (f) {
            fread(&val, sizeof(val), 1, f);
            fclose(f);
        }
    }
    return val;
}

/* _dl_find_object - glibc dynamic linker function used by libgcc_eh for
 * exception handling. In static musl builds, return failure (-1) to fall
 * back to dl_iterate_phdr-based unwinding. */
struct dl_find_object;
int _dl_find_object(void *address, struct dl_find_object *result) {
    return -1;  /* not found - triggers fallback path */
}

/* __dso_handle - required for C++ static destructors in shared libraries.
 * GCC's crtbeginS.o normally provides this, but our nostdlib linking skips it. */
void *__dso_handle __attribute__((visibility("hidden"))) = &__dso_handle;

/* glibc FORTIFY_SOURCE functions - forward to standard versions */
#include <string.h>
#include <wchar.h>

void *__memcpy_chk(void *dest, const void *src, size_t len, size_t destlen) {
    return memcpy(dest, src, len);
}

void *__memmove_chk(void *dest, const void *src, size_t len, size_t destlen) {
    return memmove(dest, src, len);
}

void *__memset_chk(void *s, int c, size_t n, size_t slen) {
    return memset(s, c, n);
}

char *__strcpy_chk(char *dest, const char *src, size_t destlen) {
    return strcpy(dest, src);
}

char *__strcat_chk(char *dest, const char *src, size_t destlen) {
    return strcat(dest, src);
}

char *__stpcpy_chk(char *dest, const char *src, size_t destlen) {
    return stpcpy(dest, src);
}

int __snprintf_chk(char *s, size_t maxlen, int flag, size_t slen,
                   const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vsnprintf(s, maxlen, fmt, ap);
    va_end(ap);
    return ret;
}

size_t __mbsrtowcs_chk(wchar_t *dest, const char **src, size_t len,
                        mbstate_t *ps, size_t destlen) {
    return mbsrtowcs(dest, src, len, ps);
}

wchar_t *__wmemcpy_chk(wchar_t *dest, const wchar_t *src, size_t n,
                        size_t destlen) {
    return wmemcpy(dest, src, n);
}

wchar_t *__wmemmove_chk(wchar_t *dest, const wchar_t *src, size_t n,
                         size_t destlen) {
    return wmemmove(dest, src, n);
}

wchar_t *__wmemset_chk(wchar_t *dest, wchar_t c, size_t n, size_t destlen) {
    return wmemset(dest, c, n);
}

wchar_t *__wcscpy_chk(wchar_t *dest, const wchar_t *src, size_t destlen) {
    return wcscpy(dest, src);
}

wchar_t *__wcscat_chk(wchar_t *dest, const wchar_t *src, size_t destlen) {
    return wcscat(dest, src);
}

size_t __wcrtomb_chk(char *s, wchar_t wc, mbstate_t *ps, size_t buflen) {
    return wcrtomb(s, wc, ps);
}

size_t __mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps) {
    return mbrtowc(pwc, s, n, ps);
}

int __swprintf_chk(wchar_t *s, size_t maxlen, int flag, size_t slen,
                   const wchar_t *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int ret = vswprintf(s, maxlen, fmt, ap);
    va_end(ap);
    return ret;
}

/* fseeko64/ftello64 - musl's fseeko/ftello are already 64-bit on 64-bit systems */
int fseeko64(FILE *stream, long long offset, int whence) {
    return fseeko(stream, (off_t)offset, whence);
}

long long ftello64(FILE *stream) {
    return (long long)ftello(stream);
}

/* __cxa_thread_atexit_impl - thread-local destructor registration.
 * musl provides __cxa_thread_atexit but GCC's libstdc++ references _impl. */
int __cxa_thread_atexit_impl(void (*func)(void *), void *obj, void *dso_handle) {
    /* Forward to musl's __cxa_thread_atexit */
    extern int __cxa_thread_atexit(void (*)(void *), void *, void *);
    return __cxa_thread_atexit(func, obj, dso_handle);
}

/* pthread_*_clock* - glibc extensions for clock-specific pthread operations.
 * GCC 15 libstdc++ references these when _GLIBCXX_USE_PTHREAD_COND_CLOCKWAIT
 * is set. Provide fallback implementations using standard POSIX equivalents. */
#include <pthread.h>
#include <time.h>

int pthread_cond_clockwait(pthread_cond_t *cond,
                           pthread_mutex_t *mutex,
                           clockid_t clock_id,
                           const struct timespec *abstime) {
    (void)clock_id;
    return pthread_cond_timedwait(cond, mutex, abstime);
}

int pthread_mutex_clocklock(pthread_mutex_t *mutex,
                            clockid_t clock_id,
                            const struct timespec *abstime) {
    (void)clock_id;
    return pthread_mutex_timedlock(mutex, abstime);
}

int pthread_rwlock_clockwrlock(pthread_rwlock_t *rwlock,
                               clockid_t clock_id,
                               const struct timespec *abstime) {
    (void)clock_id;
    return pthread_rwlock_timedwrlock(rwlock, abstime);
}

int pthread_rwlock_clockrdlock(pthread_rwlock_t *rwlock,
                               clockid_t clock_id,
                               const struct timespec *abstime) {
    (void)clock_id;
    return pthread_rwlock_timedrdlock(rwlock, abstime);
}
