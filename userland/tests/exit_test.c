/*
 * exit_test.c -- Exit code test
 *
 * Verifies: _exit syscall with specific exit codes.
 * Expected: process exits with code 42.
 */

#include <unistd.h>

int main(void)
{
    /* Return 42 -- the caller can check the exit status. */
    return 42;
}
