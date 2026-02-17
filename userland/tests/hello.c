/*
 * VeridianOS End-to-End Test -- hello.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Comprehensive userland test that exercises libc functionality:
 *   - write() via raw syscall wrapper
 *   - printf() formatted output
 *   - argv/argc access
 *   - getpid() process identity
 *   - string operations (strcpy, strcat, strlen)
 *   - heap allocation (malloc/free)
 *
 * Requires: libc.a, crt0.o (full C runtime)
 *
 * Expected output ends with "E2E_TEST_PASS\n" for automated detection.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv, char **envp)
{
    (void)envp;

    /* Test 1: Basic stdout write via raw syscall wrapper */
    const char *msg = "Hello from VeridianOS userland!\n";
    write(STDOUT_FILENO, msg, strlen(msg));

    /* Test 2: printf formatted output */
    printf("argc = %d\n", argc);

    /* Test 3: argv access */
    for (int i = 0; i < argc; i++) {
        printf("argv[%d] = %s\n", i, argv[i]);
    }

    /* Test 4: getpid */
    printf("pid = %d\n", (int)getpid());

    /* Test 5: String operations */
    char buf[64];
    strcpy(buf, "VeridianOS");
    strcat(buf, " works!");
    printf("%s\n", buf);

    /* Test 6: malloc/free */
    char *heap = malloc(128);
    if (heap) {
        strcpy(heap, "heap allocation OK");
        printf("%s\n", heap);
        free(heap);
    } else {
        const char *fail = "heap allocation FAILED\n";
        write(STDERR_FILENO, fail, strlen(fail));
        return 1;
    }

    /* Success marker for automated detection */
    write(STDOUT_FILENO, "E2E_TEST_PASS\n", 14);

    return 0;
}
