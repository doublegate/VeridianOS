/*
 * init.c -- PID 1 init process for VeridianOS
 *
 * Minimal init that spawns /bin/sh in a loop. When the shell exits
 * (either normally or due to signal), init respawns it. This ensures
 * the user always has a prompt.
 *
 * Cross-compiled by the rootfs build script and installed to /sbin/init.
 */

#include <unistd.h>
#include <sys/wait.h>
#include <string.h>

/* Default shell and environment */
static const char *shell_path = "/bin/sh";
static char *const shell_argv[] = { "sh", NULL };
static char *const shell_envp[] = {
    "PATH=/bin:/usr/bin:/sbin:/usr/sbin",
    "HOME=/",
    "TERM=veridian",
    "SHELL=/bin/sh",
    "USER=root",
    "PWD=/",
    "TMPDIR=/tmp",
    "COMPILER_PATH=/usr/libexec/gcc/x86_64-veridian/14.2.0",
    "LIBRARY_PATH=/usr/lib:/usr/lib/gcc/x86_64-veridian/14.2.0",
    NULL
};

/* Write a message to fd 1 (no printf dependency). */
static void msg(const char *s)
{
    write(1, s, strlen(s));
}

int main(void)
{
    pid_t sh;
    int status;

    msg("[init] VeridianOS init started (PID 1)\n");

    for (;;) {
        sh = fork();
        if (sh < 0) {
            msg("[init] fork() failed, retrying in 1s\n");
            /* Simple busy-wait since we don't have sleep() yet */
            volatile int i;
            for (i = 0; i < 10000000; i++)
                ;
            continue;
        }

        if (sh == 0) {
            /* Child: exec the shell */
            execve(shell_path, shell_argv, shell_envp);
            /* execve only returns on failure */
            msg("[init] execve(/bin/sh) failed\n");
            _exit(127);
        }

        /* Parent: wait for child to exit */
        waitpid(sh, &status, 0);
        msg("[init] shell exited, respawning\n");
    }

    /* unreachable */
    return 0;
}
