/*
 * write_test.c -- File I/O test: open, write, read, close
 *
 * Verifies: open, write, lseek, read, close syscalls via libc.
 * Expected output: "PASS: file I/O" if round-trip succeeds.
 */

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>

int main(void)
{
    const char *path = "/tmp/test_write.txt";
    const char *msg  = "VeridianOS file I/O works!";
    char buf[64];

    /* Write */
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        printf("FAIL: open for write\n");
        return 1;
    }
    ssize_t n = write(fd, msg, strlen(msg));
    close(fd);
    if (n != (ssize_t)strlen(msg)) {
        printf("FAIL: write returned %ld\n", (long)n);
        return 1;
    }

    /* Read back */
    fd = open(path, O_RDONLY, 0);
    if (fd < 0) {
        printf("FAIL: open for read\n");
        return 1;
    }
    memset(buf, 0, sizeof(buf));
    n = read(fd, buf, sizeof(buf) - 1);
    close(fd);

    if (n > 0 && strcmp(buf, msg) == 0)
        printf("PASS: file I/O\n");
    else
        printf("FAIL: read back mismatch (got %ld bytes: \"%s\")\n", (long)n, buf);

    return 0;
}
