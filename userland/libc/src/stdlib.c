/*
 * VeridianOS libc -- stdlib.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Memory allocation (sbrk-backed free-list allocator), process control,
 * environment variables, sorting, and number generation.
 */

#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <signal.h>
#include <ctype.h>
#include <sys/wait.h>
#include <veridian/syscall.h>

/* ========================================================================= */
/* Memory allocator                                                          */
/* ========================================================================= */

/*
 * Free-list allocator with coalescing.
 *
 * Each allocation is prefixed with a header that records the block size.
 * free() inserts blocks into an address-sorted singly-linked free list
 * and coalesces adjacent free blocks to reduce fragmentation.  malloc()
 * scans the free list (first-fit) before falling back to sbrk().
 *
 * This replaces the earlier bump allocator (where free was a no-op)
 * which caused ash shell and other BusyBox applets to OOM instantly.
 */

/* Alignment: all allocations are aligned to this boundary. */
#define ALLOC_ALIGN     16

/* Minimum sbrk increment (avoid many small brk calls). */
#define SBRK_MIN        4096

typedef struct block_header {
    size_t                  size;   /* Usable size (excluding header) */
    struct block_header    *next;   /* Next free block (only valid when free) */
} block_header_t;

#define HEADER_SIZE     ((sizeof(block_header_t) + ALLOC_ALIGN - 1) & ~(ALLOC_ALIGN - 1))

static block_header_t *free_list = NULL;

/*
 * Round size up to alignment boundary.
 */
static inline size_t align_up(size_t n)
{
    return (n + ALLOC_ALIGN - 1) & ~(ALLOC_ALIGN - 1);
}

/*
 * Insert a freed block into the free list, sorted by address.
 * Coalesces with adjacent free blocks when possible.
 */
static void free_insert(block_header_t *blk)
{
    block_header_t *prev_blk = NULL;
    block_header_t *cur = free_list;

    /* Walk to find insertion point (address order). */
    while (cur && cur < blk) {
        prev_blk = cur;
        cur = cur->next;
    }

    /* Try to coalesce blk with the next free block (cur). */
    if (cur && (char *)blk + HEADER_SIZE + blk->size == (char *)cur) {
        blk->size += HEADER_SIZE + cur->size;
        blk->next = cur->next;
    } else {
        blk->next = cur;
    }

    /* Try to coalesce the previous free block with blk. */
    if (prev_blk &&
        (char *)prev_blk + HEADER_SIZE + prev_blk->size == (char *)blk) {
        prev_blk->size += HEADER_SIZE + blk->size;
        prev_blk->next = blk->next;
    } else if (prev_blk) {
        prev_blk->next = blk;
    } else {
        free_list = blk;
    }
}

void *malloc(size_t size)
{
    if (size == 0)
        return NULL;

    size = align_up(size);

    /* First-fit search on the free list. */
    block_header_t *prev = NULL;
    block_header_t *cur = free_list;

    while (cur) {
        if (cur->size >= size) {
            /* Found a fit.  Split if remainder is large enough. */
            if (cur->size >= size + HEADER_SIZE + ALLOC_ALIGN) {
                block_header_t *rem =
                    (block_header_t *)((char *)cur + HEADER_SIZE + size);
                rem->size = cur->size - size - HEADER_SIZE;
                rem->next = cur->next;
                if (prev)
                    prev->next = rem;
                else
                    free_list = rem;
                cur->size = size;
            } else {
                /* Use the whole block (no split). */
                if (prev)
                    prev->next = cur->next;
                else
                    free_list = cur->next;
            }
            cur->next = NULL;
            return (char *)cur + HEADER_SIZE;
        }
        prev = cur;
        cur = cur->next;
    }

    /* No suitable free block.  Extend the heap via sbrk(). */
    size_t total = HEADER_SIZE + size;
    if (total < SBRK_MIN)
        total = SBRK_MIN;

    void *mem = sbrk((intptr_t)total);
    if (mem == (void *)-1) {
        errno = ENOMEM;
        return NULL;
    }

    block_header_t *blk = (block_header_t *)mem;
    blk->size = total - HEADER_SIZE;
    blk->next = NULL;

    /* If sbrk gave us more than needed, free the remainder. */
    if (blk->size > size + HEADER_SIZE + ALLOC_ALIGN) {
        block_header_t *rem =
            (block_header_t *)((char *)blk + HEADER_SIZE + size);
        rem->size = blk->size - size - HEADER_SIZE;
        rem->next = NULL;
        blk->size = size;
        free_insert(rem);
    }

    return (char *)blk + HEADER_SIZE;
}

void free(void *ptr)
{
    if (!ptr)
        return;

    block_header_t *blk = (block_header_t *)((char *)ptr - HEADER_SIZE);
    free_insert(blk);
}

void *calloc(size_t count, size_t size)
{
    size_t total = count * size;
    /* Overflow check. */
    if (count != 0 && total / count != size) {
        errno = ENOMEM;
        return NULL;
    }
    void *p = malloc(total);
    if (p)
        memset(p, 0, total);
    return p;
}

void *realloc(void *ptr, size_t size)
{
    if (!ptr)
        return malloc(size);
    if (size == 0) {
        free(ptr);
        return NULL;
    }

    size = align_up(size);
    block_header_t *blk = (block_header_t *)((char *)ptr - HEADER_SIZE);

    if (blk->size >= size)
        return ptr;     /* Current block is big enough. */

    /*
     * Try to grow in place by coalescing with an adjacent free block.
     * This avoids a copy when the next block in memory is free and
     * large enough to satisfy the request.
     */
    block_header_t *next_addr =
        (block_header_t *)((char *)blk + HEADER_SIZE + blk->size);
    block_header_t *fprev = NULL;
    block_header_t *fcur = free_list;

    while (fcur) {
        if (fcur == next_addr) {
            size_t combined = blk->size + HEADER_SIZE + fcur->size;
            if (combined >= size) {
                /* Remove fcur from free list. */
                if (fprev)
                    fprev->next = fcur->next;
                else
                    free_list = fcur->next;
                blk->size = combined;

                /* Split if significantly larger than needed. */
                if (blk->size > size + HEADER_SIZE + ALLOC_ALIGN) {
                    block_header_t *rem =
                        (block_header_t *)((char *)blk + HEADER_SIZE + size);
                    rem->size = blk->size - size - HEADER_SIZE;
                    rem->next = NULL;
                    blk->size = size;
                    free_insert(rem);
                }
                return ptr;     /* Grew in place -- no copy needed. */
            }
            break;
        }
        fprev = fcur;
        fcur = fcur->next;
    }

    /* Cannot grow in place.  Allocate, copy, free. */
    void *newp = malloc(size);
    if (!newp)
        return NULL;
    memcpy(newp, ptr, blk->size);
    free(ptr);
    return newp;
}

/* ========================================================================= */
/* Process control                                                           */
/* ========================================================================= */

/* atexit handlers. */
#define ATEXIT_MAX  32
static void (*__atexit_funcs[ATEXIT_MAX])(void);
static int __atexit_count = 0;

int atexit(void (*func)(void))
{
    if (__atexit_count >= ATEXIT_MAX) {
        errno = ENOMEM;
        return -1;
    }
    __atexit_funcs[__atexit_count++] = func;
    return 0;
}

void exit(int status)
{
    /* Run atexit handlers in reverse order. */
    while (__atexit_count > 0)
        __atexit_funcs[--__atexit_count]();

    /* Flush stdio. */
    extern int fflush(void *);  /* FILE* but we just pass NULL */
    fflush(NULL);

    _exit(status);
}

void _Exit(int status)
{
    _exit(status);
}

void abort(void)
{
    /* Send SIGABRT to self.  If it returns (blocked/caught), force exit. */
    raise(SIGABRT);
    _exit(128 + SIGABRT);
}

/* ========================================================================= */
/* Environment                                                               */
/* ========================================================================= */

char **environ = NULL;

/*
 * Thread-local buffer for getenv() kernel fallback.  Sized to hold
 * typical environment values (PATH, COMPILER_PATH, etc.).
 */
static char __getenv_buf[4096];

char *getenv(const char *name)
{
    if (!name)
        return NULL;

    /* Fast path: use libc environ if available. */
    if (environ) {
        size_t len = strlen(name);
        for (char **ep = environ; *ep; ep++) {
            if (strncmp(*ep, name, len) == 0 && (*ep)[len] == '=')
                return *ep + len + 1;
        }
        return NULL;
    }

    /*
     * Fallback: environ is NULL (CRT didn't call __libc_start_main).
     * Ask the kernel for the value via SYS_PROCESS_GETENV.
     */
    size_t nlen = strlen(name);
    long ret = veridian_syscall4(SYS_PROCESS_GETENV,
                                 name, (const void *)(unsigned long)nlen,
                                 __getenv_buf,
                                 (const void *)(unsigned long)sizeof(__getenv_buf));
    if (ret < 0)
        return NULL;
    return __getenv_buf;
}

/*
 * Count the current environ entries.
 */
static int __env_count(void)
{
    if (!environ) return 0;
    int n = 0;
    while (environ[n]) n++;
    return n;
}

int setenv(const char *name, const char *value, int overwrite)
{
    if (!name || !*name || strchr(name, '=')) {
        errno = EINVAL;
        return -1;
    }

    size_t nlen = strlen(name);

    /* Check if already exists. */
    if (environ) {
        for (char **ep = environ; *ep; ep++) {
            if (strncmp(*ep, name, nlen) == 0 && (*ep)[nlen] == '=') {
                if (!overwrite) return 0;
                /* Replace in-place. */
                size_t vlen = strlen(value);
                char *entry = (char *)malloc(nlen + 1 + vlen + 1);
                if (!entry) return -1;
                memcpy(entry, name, nlen);
                entry[nlen] = '=';
                memcpy(entry + nlen + 1, value, vlen + 1);
                *ep = entry;
                return 0;
            }
        }
    }

    /* Add new entry. */
    int count = __env_count();
    char **new_env = (char **)malloc(sizeof(char *) * (count + 2));
    if (!new_env) return -1;

    if (environ)
        memcpy(new_env, environ, sizeof(char *) * count);

    size_t vlen = strlen(value);
    char *entry = (char *)malloc(nlen + 1 + vlen + 1);
    if (!entry) { free(new_env); return -1; }
    memcpy(entry, name, nlen);
    entry[nlen] = '=';
    memcpy(entry + nlen + 1, value, vlen + 1);

    new_env[count] = entry;
    new_env[count + 1] = NULL;
    environ = new_env;
    return 0;
}

int unsetenv(const char *name)
{
    if (!name || !*name || strchr(name, '=')) {
        errno = EINVAL;
        return -1;
    }
    if (!environ) return 0;

    size_t nlen = strlen(name);
    char **ep = environ;
    while (*ep) {
        if (strncmp(*ep, name, nlen) == 0 && (*ep)[nlen] == '=') {
            /* Shift remaining entries down. */
            char **p = ep;
            while (*p) {
                *p = *(p + 1);
                p++;
            }
            /* Don't advance ep -- the next entry slid into this slot. */
        } else {
            ep++;
        }
    }
    return 0;
}

/* ========================================================================= */
/* Pseudo-random number generation (LCG)                                     */
/* ========================================================================= */

static unsigned int __rand_seed = 1;

void srand(unsigned int seed)
{
    __rand_seed = seed;
}

int rand(void)
{
    /* Numerical Recipes LCG. */
    __rand_seed = __rand_seed * 1103515245 + 12345;
    return (int)((__rand_seed >> 16) & RAND_MAX);
}

/* ========================================================================= */
/* Sorting: qsort (shell sort for simplicity)                                */
/* ========================================================================= */

void qsort(void *base, size_t nmemb, size_t size,
            int (*compar)(const void *, const void *))
{
    /*
     * Shell sort.  O(n^(3/2)) worst case, much simpler than quicksort,
     * adequate for small-n use cases in early userland.
     */
    char *arr = (char *)base;
    char tmp[256]; /* For swapping; elements > 256 bytes use byte-at-a-time. */

    for (size_t gap = nmemb / 2; gap > 0; gap /= 2) {
        for (size_t i = gap; i < nmemb; i++) {
            /* Save arr[i] into tmp. */
            if (size <= sizeof(tmp))
                memcpy(tmp, arr + i * size, size);

            size_t j = i;
            while (j >= gap &&
                   compar(arr + (j - gap) * size,
                          (size <= sizeof(tmp)) ? tmp : arr + i * size) > 0) {
                memcpy(arr + j * size, arr + (j - gap) * size, size);
                j -= gap;
            }

            if (size <= sizeof(tmp))
                memcpy(arr + j * size, tmp, size);
            else if (j != i) {
                /* Swap arr[j] and saved element (still at original i). */
                /* This is a simplified fallback; works for insertion. */
                /* For > 256-byte elements, use memmove. */
                memmove(arr + j * size, arr + i * size, size);
            }
        }
    }
}

void *bsearch(const void *key, const void *base, size_t nmemb,
              size_t size, int (*compar)(const void *, const void *))
{
    const char *arr = (const char *)base;
    size_t lo = 0, hi = nmemb;

    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        int cmp = compar(key, arr + mid * size);
        if (cmp < 0)
            hi = mid;
        else if (cmp > 0)
            lo = mid + 1;
        else
            return (void *)(arr + mid * size);
    }
    return NULL;
}

/* ========================================================================= */
/* Integer arithmetic                                                        */
/* ========================================================================= */

int abs(int j)
{
    return j < 0 ? -j : j;
}

long labs(long j)
{
    return j < 0 ? -j : j;
}

div_t div(int numer, int denom)
{
    div_t result;
    result.quot = numer / denom;
    result.rem  = numer % denom;
    return result;
}

ldiv_t ldiv(long numer, long denom)
{
    ldiv_t result;
    result.quot = numer / denom;
    result.rem  = numer % denom;
    return result;
}

long long llabs(long long j)
{
    return j < 0 ? -j : j;
}

lldiv_t lldiv(long long numer, long long denom)
{
    lldiv_t result;
    result.quot = numer / denom;
    result.rem  = numer % denom;
    return result;
}

long long atoll(const char *nptr)
{
    return (long long)strtol(nptr, NULL, 10);
}

void *aligned_alloc(size_t alignment, size_t size)
{
    /* Simple stub: malloc always aligns to 16 bytes. */
    if (alignment <= ALLOC_ALIGN)
        return malloc(size);
    /* For larger alignments, over-allocate and align. */
    void *p = malloc(size + alignment);
    if (!p) return NULL;
    unsigned long addr = (unsigned long)p;
    unsigned long aligned = (addr + alignment - 1) & ~(alignment - 1);
    return (void *)aligned;
}

/* Multibyte/wide character stubs are in posix_stubs2.c */

/* ========================================================================= */
/* String-to-floating-point conversions                                      */
/* ========================================================================= */

double strtod(const char *nptr, char **endptr)
{
    const char *s = nptr;
    double result = 0.0;
    int neg = 0;

    /* Skip whitespace. */
    while (isspace((unsigned char)*s))
        s++;

    /* Optional sign. */
    if (*s == '-') {
        neg = 1;
        s++;
    } else if (*s == '+') {
        s++;
    }

    /* Integer part. */
    while (*s >= '0' && *s <= '9') {
        result = result * 10.0 + (*s - '0');
        s++;
    }

    /* Fractional part. */
    if (*s == '.') {
        s++;
        double frac = 0.1;
        while (*s >= '0' && *s <= '9') {
            result += (*s - '0') * frac;
            frac *= 0.1;
            s++;
        }
    }

    /* Exponent. */
    if (*s == 'e' || *s == 'E') {
        s++;
        int exp_neg = 0;
        if (*s == '-') {
            exp_neg = 1;
            s++;
        } else if (*s == '+') {
            s++;
        }

        int exp = 0;
        while (*s >= '0' && *s <= '9') {
            exp = exp * 10 + (*s - '0');
            s++;
        }

        double factor = 1.0;
        for (int i = 0; i < exp; i++)
            factor *= 10.0;

        if (exp_neg)
            result /= factor;
        else
            result *= factor;
    }

    if (endptr)
        *endptr = (char *)s;

    return neg ? -result : result;
}

float strtof(const char *nptr, char **endptr)
{
    return (float)strtod(nptr, endptr);
}

long double strtold(const char *nptr, char **endptr)
{
    return (long double)strtod(nptr, endptr);
}

double atof(const char *nptr)
{
    return strtod(nptr, NULL);
}

/* ========================================================================= */
/* Temporary files                                                           */
/* ========================================================================= */

int mkstemp(char *template)
{
    static const char chars[] =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    size_t len = strlen(template);

    if (len < 6) {
        errno = EINVAL;
        return -1;
    }

    /* Verify trailing XXXXXX. */
    char *suffix = template + len - 6;
    for (int i = 0; i < 6; i++) {
        if (suffix[i] != 'X') {
            errno = EINVAL;
            return -1;
        }
    }

    /*
     * Try up to 100 random names before giving up.
     * Each attempt replaces the 6 suffix chars with random characters
     * from the alphanumeric set, then tries O_CREAT|O_EXCL to ensure
     * uniqueness.
     */
    for (int attempt = 0; attempt < 100; attempt++) {
        for (int i = 0; i < 6; i++)
            suffix[i] = chars[(unsigned int)rand() % (sizeof(chars) - 1)];

        int fd = open(template, O_CREAT | O_EXCL | O_RDWR, 0600);
        if (fd >= 0)
            return fd;

        /* EEXIST means name collision -- retry with new random suffix. */
        if (errno != EEXIST)
            return -1;
    }

    errno = EEXIST;
    return -1;
}

char *mktemp(char *template)
{
    static const char chars[] =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    size_t len = strlen(template);

    if (len < 6) {
        errno = EINVAL;
        template[0] = '\0';
        return template;
    }

    char *suffix = template + len - 6;
    for (int i = 0; i < 6; i++) {
        if (suffix[i] != 'X') {
            errno = EINVAL;
            template[0] = '\0';
            return template;
        }
    }

    for (int i = 0; i < 6; i++)
        suffix[i] = chars[(unsigned int)rand() % (sizeof(chars) - 1)];

    return template;
}

/* ========================================================================= */
/* Command execution                                                         */
/* ========================================================================= */

int system(const char *command)
{
    if (!command)
        return 1;  /* Shell is available. */

    pid_t pid = fork();
    if (pid < 0)
        return -1;

    if (pid == 0) {
        /* Child: exec the shell. */
        char *argv[4];
        argv[0] = "sh";
        argv[1] = "-c";
        argv[2] = (char *)command;
        argv[3] = NULL;
        execve("/bin/sh", argv, environ);
        _exit(127);  /* execve failed. */
    }

    /* Parent: wait for the child. */
    int status;
    if (waitpid(pid, &status, 0) < 0)
        return -1;

    return status;
}
