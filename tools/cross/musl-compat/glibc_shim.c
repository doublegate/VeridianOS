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
