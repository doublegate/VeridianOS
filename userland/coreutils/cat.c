/* cat.c -- Concatenate files to stdout
 *
 * VeridianOS coreutil.  Validates file open/read/write/close loop.
 *
 * Usage: cat [FILE...]
 *   With no FILE, or when FILE is -, read standard input.
 *
 * Self-test: cat /usr/src/cat_test.txt -> output ends with "CAT_PASS"
 *
 * Syscalls exercised: open, read, write, close
 */

#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#define BUF_SIZE 4096

static int cat_fd(int fd)
{
    char buf[BUF_SIZE];
    int n;

    while ((n = read(fd, buf, BUF_SIZE)) > 0) {
        int written = 0;
        while (written < n) {
            int w = write(1, buf + written, n - written);
            if (w < 0) {
                perror("cat: write");
                return 1;
            }
            written += w;
        }
    }

    if (n < 0) {
        perror("cat: read");
        return 1;
    }

    return 0;
}

int main(int argc, char *argv[])
{
    int ret = 0;

    if (argc <= 1) {
        return cat_fd(0);
    }

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-") == 0) {
            if (cat_fd(0) != 0)
                ret = 1;
        } else {
            int fd = open(argv[i], O_RDONLY);
            if (fd < 0) {
                perror(argv[i]);
                ret = 1;
                continue;
            }
            if (cat_fd(fd) != 0)
                ret = 1;
            close(fd);
        }
    }

    return ret;
}
