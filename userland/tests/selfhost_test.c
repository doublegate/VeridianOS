/* selfhost_test.c -- Self-hosting compilation test program
 *
 * This file is included in the rootfs as /usr/src/selfhost_test.c.
 * It is designed to be compiled ON VeridianOS using the native GCC toolchain,
 * verifying the full self-hosting compilation pipeline:
 *
 *   gcc -static /usr/src/selfhost_test.c -o /tmp/selfhost_test
 *   /tmp/selfhost_test
 *
 * Expected output: "SELF_HOSTED_PASS"
 *
 * The program deliberately uses only write() to avoid stdio buffering
 * complexity -- a raw syscall wrapper is the simplest possible test.
 */

#include <unistd.h>

int main(void) {
    const char msg[] = "SELF_HOSTED_PASS\n";
    write(1, msg, sizeof(msg) - 1);
    return 0;
}
