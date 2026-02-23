/* selfhost_test.c -- Self-hosting compilation test program
 *
 * This file is included in the rootfs as /usr/src/selfhost_test.c.
 * It is designed to be compiled ON VeridianOS using the native GCC toolchain,
 * verifying the full self-hosting compilation pipeline:
 *
 *   cc1 + as + ld (step-by-step):
 *     /usr/libexec/gcc/x86_64-veridian/14.2.0/cc1 -isystem /usr/include \
 *       -isystem /usr/lib/gcc/x86_64-veridian/14.2.0/include \
 *       /usr/src/selfhost_test.c -o /tmp/test.s -quiet
 *     /usr/bin/as -o /tmp/test.o /tmp/test.s
 *     /usr/bin/ld -static -o /tmp/selfhost_test \
 *       /usr/lib/crt0.o /usr/lib/crti.o \
 *       /usr/lib/gcc/x86_64-veridian/14.2.0/crtbegin.o \
 *       /tmp/test.o -L/usr/lib -L/usr/lib/gcc/x86_64-veridian/14.2.0 \
 *       -lc -lgcc \
 *       /usr/lib/gcc/x86_64-veridian/14.2.0/crtend.o /usr/lib/crtn.o
 *     /tmp/selfhost_test
 *
 * Expected output: "SELF_HOSTED_PASS"
 */

#include <veridian/syscall.h>

/* Raw syscall: write(fd, buf, len) via VeridianOS SYS_FILE_WRITE (53) */
static long do_write(int fd, const void *buf, unsigned long len) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a" (ret)
        : "0" ((long)SYS_FILE_WRITE),
          "D" ((long)fd),     /* rdi = fd */
          "S" ((long)buf),    /* rsi = buf */
          "d" ((long)len)     /* rdx = len */
        : "rcx", "r11", "memory"
    );
    return ret;
}

/* Raw syscall: exit(code) via VeridianOS SYS_PROCESS_EXIT (11) */
static void do_exit(int code) {
    __asm__ volatile (
        "syscall"
        :
        : "a" ((long)SYS_PROCESS_EXIT),
          "D" ((long)code)
        : "rcx", "r11", "memory"
    );
    __builtin_unreachable();
}

int main(void) {
    const char msg[] = "SELF_HOSTED_PASS\n";
    long ret = do_write(1, msg, sizeof(msg) - 1);
    if (ret < 0) {
        do_exit(1);
    }
    return 0;
}
