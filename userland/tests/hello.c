/*
 * hello.c -- Minimal test: printf and exit
 *
 * Verifies: CRT startup, printf (write syscall), process exit.
 * Expected output: "Hello from VeridianOS!" followed by exit code 0.
 */

#include <stdio.h>

int main(int argc, char *argv[])
{
    printf("Hello from VeridianOS!\n");
    return 0;
}
