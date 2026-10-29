/*
 * linux-uname: run a command in Linux mode, or in the native mode
 * (ADR 0021), as setarch(8) runs one with a personality.
 *
 *   linux-uname CMD [ARG...]      uname reports "Linux" to CMD and its children
 *   linux-uname -n CMD [ARG...]   uname reports "VeridianOS"
 *   linux-uname                   print the current mode ("linux" or "native")
 *
 * Exit status: CMD's (exec replaces this program); 125 for a bad option or
 * a failed prctl, 126 if CMD cannot be run, 127 if it is not found.
 */
#define VERIDIAN_LINUX_UNAME_NO_NOTE /* this tool itself stays native */
#include <veridian/linux-uname.h>

#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <unistd.h>

static int usage(void)
{
    fputs("usage: linux-uname [-n] CMD [ARG...]\n", stderr);
    return 125;
}

int main(int argc, char **argv)
{
    int mode = PR_VERIDIAN_UNAME_LINUX;
    int first = 1;

    if (argc == 1) {
        int m = prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_GET, 0, 0, 0);
        if (m < 0) {
            perror("linux-uname: prctl");
            return 125;
        }
        puts(m == PR_VERIDIAN_UNAME_LINUX ? "linux" : "native");
        return 0;
    }
    if (strcmp(argv[1], "-n") == 0) {
        mode = PR_VERIDIAN_UNAME_NATIVE;
        first = 2;
    } else if (strcmp(argv[1], "--") == 0) {
        first = 2;
    } else if (argv[1][0] == '-') {
        return usage();
    }
    if (first >= argc)
        return usage();
    if (prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_SET, mode, 0, 0) != 0) {
        perror("linux-uname: prctl");
        return 125;
    }
    execvp(argv[first], &argv[first]);
    fprintf(stderr, "linux-uname: %s: %s\n", argv[first], strerror(errno));
    return errno == ENOENT ? 127 : 126;
}
