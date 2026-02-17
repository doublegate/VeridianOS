/*
 * getpid_test.c -- Process identity test
 *
 * Verifies: getpid, getppid syscalls.
 * Expected output: PID and PPID values (both > 0).
 */

#include <stdio.h>
#include <unistd.h>

int main(void)
{
    pid_t pid  = getpid();
    pid_t ppid = getppid();

    printf("PID:  %ld\n", (long)pid);
    printf("PPID: %ld\n", (long)ppid);

    if (pid > 0)
        printf("PASS: getpid\n");
    else
        printf("FAIL: getpid returned %ld\n", (long)pid);

    return 0;
}
