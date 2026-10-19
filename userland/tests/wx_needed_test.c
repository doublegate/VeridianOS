/*
 * A program that opts in to writable-and-executable memory (ADR 0017),
 * run by musl_runtime_test.
 *
 *   wx_needed_test          expect the opt-in: an RWX mapping is allowed,
 *                           code written to it runs, mprotect may make
 *                           memory RWX; then PR_SET_MDWE takes it away.
 *   wx_needed_test refused  expect W^X anyway (run from a process with
 *                           PR_SET_MDWE set, which exec keeps).
 *
 * Prints one line ("WX ... OK" or what failed) and exits 0 on success.
 *
 * Built with -DWX_NO_NOTE it carries no note, as a binary built for Linux:
 * the rootfs scripts run tools/veridian-elfctl on such copies to test the
 * note it adds (and one it clears).
 */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/prctl.h>

#ifndef WX_NO_NOTE
#include <veridian/wxneeded.h>
#endif

#ifndef PR_SET_MDWE
#define PR_SET_MDWE 65
#define PR_MDWE_REFUSE_EXEC_GAIN 1
#endif

#define PG 4096

/* return 42 */
#if defined(__x86_64__)
static const unsigned char code[] = {0xb8, 0x2a, 0x00, 0x00, 0x00, 0xc3};
#elif defined(__aarch64__)
static const unsigned int code[] = {0x52800540, 0xd65f03c0};
#elif defined(__riscv)
static const unsigned int code[] = {0x02a00513, 0x00008067};
#else
#error "unsupported architecture"
#endif

static int fail(const char *what)
{
    printf("WX FAIL %s: errno %d\n", what, errno);
    return 1;
}

static void *rwx(void)
{
    return mmap(0, PG, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "refused") == 0) {
        if (rwx() != MAP_FAILED || errno != EACCES)
            return fail("RWX mmap not refused under MDWE");
        printf("WX REFUSED OK\n");
        return 0;
    }

    /* Code written into an RWX mapping runs. */
    char *p = rwx();
    if (p == MAP_FAILED)
        return fail("RWX mmap");
    memcpy(p, code, sizeof(code));
    __builtin___clear_cache(p, p + sizeof(code));
    int r = ((int (*)(void))(void *)p)();
    if (r != 42) {
        printf("WX FAIL code returned %d\n", r);
        return 1;
    }

    /* mprotect may make writable memory RWX. */
    char *q = mmap(0, PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (q == MAP_FAILED)
        return fail("RW mmap");
    if (mprotect(q, PG, PROT_READ | PROT_WRITE | PROT_EXEC) != 0)
        return fail("mprotect RWX");

    /* PR_SET_MDWE is stronger than the note. */
    if (prctl(PR_SET_MDWE, PR_MDWE_REFUSE_EXEC_GAIN, 0, 0, 0) != 0)
        return fail("PR_SET_MDWE");
    if (rwx() != MAP_FAILED || errno != EACCES)
        return fail("RWX mmap not refused after PR_SET_MDWE");
    printf("WX OK 42\n");
    return 0;
}
