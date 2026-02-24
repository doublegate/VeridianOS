/* echo.c -- Print arguments to stdout
 *
 * VeridianOS coreutil.  Validates basic compilation, argc/argv, stdout.
 *
 * Usage: echo [-n] [STRING...]
 *   -n  Do not output the trailing newline.
 *
 * Self-test: echo ECHO_PASS -> prints "ECHO_PASS"
 *
 * Syscalls exercised: write
 */

#include <string.h>
#include <unistd.h>

int main(int argc, char *argv[])
{
    int no_newline = 0;
    int start = 1;

    if (argc > 1 && strcmp(argv[1], "-n") == 0) {
        no_newline = 1;
        start = 2;
    }

    for (int i = start; i < argc; i++) {
        write(1, argv[i], strlen(argv[i]));
        if (i + 1 < argc)
            write(1, " ", 1);
    }

    if (!no_newline)
        write(1, "\n", 1);

    return 0;
}
