/* pipeline_test.c -- Capstone fork/exec/waitpid/pipe/dup2 test
 *
 * VeridianOS coreutil.  Validates the full process lifecycle and IPC pipeline.
 *
 * Three sub-tests:
 *   1. File I/O round-trip (create, write, read-back, verify, unlink)
 *   2. fork+exec+waitpid (run /bin/cat with stdout redirect)
 *   3. Two-stage pipe (cat | sort via fork+pipe+dup2+exec)
 *
 * Expected output: SUBTEST1_PASS, SUBTEST2_PASS, SUBTEST3_PASS, PIPELINE_PASS
 *
 * Syscalls exercised: fork, execve, waitpid, pipe, dup2, open, read, write,
 *                     close, unlink
 */

#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define BUF_SIZE 4096

/* ----------------------------------------------------------------------- */
/* Helpers                                                                  */
/* ----------------------------------------------------------------------- */

static int write_file(const char *path, const char *data, int len)
{
    int fd = open(path, O_CREAT | O_WRONLY | O_TRUNC);
    if (fd < 0) {
        perror(path);
        return -1;
    }
    int w = write(fd, data, len);
    close(fd);
    return (w == len) ? 0 : -1;
}

static int read_file(const char *path, char *buf, int bufsize)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        perror(path);
        return -1;
    }
    int total = 0;
    int n;
    while ((n = read(fd, buf + total, bufsize - total - 1)) > 0) {
        total += n;
    }
    close(fd);
    buf[total] = '\0';
    return total;
}

/* ----------------------------------------------------------------------- */
/* Sub-test 1: File I/O round-trip                                          */
/* ----------------------------------------------------------------------- */

static int subtest1(void)
{
    const char *path = "/tmp/pt_test.txt";
    const char *data = "Hello from VeridianOS pipeline test\n";
    int datalen = strlen(data);

    /* Write */
    if (write_file(path, data, datalen) < 0) {
        write(1, "SUBTEST1_FAIL: write\n", 21);
        return 0;
    }

    /* Read back */
    char buf[BUF_SIZE];
    int n = read_file(path, buf, BUF_SIZE);
    if (n != datalen) {
        write(1, "SUBTEST1_FAIL: length\n", 22);
        unlink(path);
        return 0;
    }

    /* Verify */
    if (memcmp(buf, data, datalen) != 0) {
        write(1, "SUBTEST1_FAIL: content\n", 23);
        unlink(path);
        return 0;
    }

    unlink(path);
    write(1, "SUBTEST1_PASS\n", 14);
    return 1;
}

/* ----------------------------------------------------------------------- */
/* Sub-test 2: fork + exec + waitpid (cat with stdout redirect)             */
/* ----------------------------------------------------------------------- */

static int subtest2(void)
{
    const char *input_path = "/tmp/pt_input.txt";
    const char *output_path = "/tmp/pt_output.txt";
    const char *data = "fork-exec test data line 1\nline 2\n";
    int datalen = strlen(data);

    /* Create input file */
    if (write_file(input_path, data, datalen) < 0) {
        write(1, "SUBTEST2_FAIL: create input\n", 28);
        return 0;
    }

    pid_t pid = fork();
    if (pid < 0) {
        write(1, "SUBTEST2_FAIL: fork\n", 20);
        unlink(input_path);
        return 0;
    }

    if (pid == 0) {
        /* Child: redirect stdout to output file, exec cat */
        int fd = open(output_path, O_CREAT | O_WRONLY | O_TRUNC);
        if (fd < 0)
            _exit(1);
        dup2(fd, 1);
        close(fd);

        char *cat_argv[] = { "cat", (char *)input_path, NULL };
        execve("/bin/cat", cat_argv, NULL);
        /* If execve returns, it failed */
        _exit(2);
    }

    /* Parent: wait for child */
    int status = 0;
    waitpid(pid, &status, 0);

    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        write(1, "SUBTEST2_FAIL: child exit\n", 26);
        unlink(input_path);
        unlink(output_path);
        return 0;
    }

    /* Verify output matches input */
    char buf[BUF_SIZE];
    int n = read_file(output_path, buf, BUF_SIZE);
    if (n != datalen || memcmp(buf, data, datalen) != 0) {
        write(1, "SUBTEST2_FAIL: content\n", 23);
        unlink(input_path);
        unlink(output_path);
        return 0;
    }

    unlink(input_path);
    unlink(output_path);
    write(1, "SUBTEST2_PASS\n", 14);
    return 1;
}

/* ----------------------------------------------------------------------- */
/* Sub-test 3: Two-stage pipe (cat | sort)                                  */
/* ----------------------------------------------------------------------- */

static int subtest3(void)
{
    const char *unsorted_path = "/tmp/pt_unsorted.txt";
    const char *sorted_path = "/tmp/pt_sorted.txt";
    const char *unsorted_data = "cherry\napple\nbanana\n";
    const char *expected = "apple\nbanana\ncherry\n";
    int unsorted_len = strlen(unsorted_data);
    int expected_len = strlen(expected);

    /* Create unsorted input */
    if (write_file(unsorted_path, unsorted_data, unsorted_len) < 0) {
        write(1, "SUBTEST3_FAIL: create input\n", 28);
        return 0;
    }

    /* Create pipe */
    int pipefd[2];
    if (pipe(pipefd) < 0) {
        write(1, "SUBTEST3_FAIL: pipe\n", 20);
        unlink(unsorted_path);
        return 0;
    }

    /* Fork cat process */
    pid_t cat_pid = fork();
    if (cat_pid < 0) {
        write(1, "SUBTEST3_FAIL: fork cat\n", 24);
        close(pipefd[0]);
        close(pipefd[1]);
        unlink(unsorted_path);
        return 0;
    }

    if (cat_pid == 0) {
        /* Child (cat): stdout -> pipe write end */
        dup2(pipefd[1], 1);
        close(pipefd[0]);
        close(pipefd[1]);

        char *cat_argv[] = { "cat", (char *)unsorted_path, NULL };
        execve("/bin/cat", cat_argv, NULL);
        _exit(2);
    }

    /* Fork sort process */
    pid_t sort_pid = fork();
    if (sort_pid < 0) {
        write(1, "SUBTEST3_FAIL: fork sort\n", 25);
        close(pipefd[0]);
        close(pipefd[1]);
        unlink(unsorted_path);
        return 0;
    }

    if (sort_pid == 0) {
        /* Child (sort): stdin <- pipe read end, stdout -> file */
        dup2(pipefd[0], 0);
        close(pipefd[0]);
        close(pipefd[1]);

        int fd = open(sorted_path, O_CREAT | O_WRONLY | O_TRUNC);
        if (fd < 0)
            _exit(1);
        dup2(fd, 1);
        close(fd);

        char *sort_argv[] = { "sort", NULL };
        execve("/bin/sort", sort_argv, NULL);
        _exit(2);
    }

    /* Parent: close pipe, wait for both children */
    close(pipefd[0]);
    close(pipefd[1]);

    int status;
    waitpid(cat_pid, &status, 0);
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        write(1, "SUBTEST3_FAIL: cat exit\n", 24);
        unlink(unsorted_path);
        unlink(sorted_path);
        return 0;
    }

    waitpid(sort_pid, &status, 0);
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        write(1, "SUBTEST3_FAIL: sort exit\n", 25);
        unlink(unsorted_path);
        unlink(sorted_path);
        return 0;
    }

    /* Verify sorted output */
    char buf[BUF_SIZE];
    int n = read_file(sorted_path, buf, BUF_SIZE);
    if (n != expected_len || memcmp(buf, expected, expected_len) != 0) {
        /* Print what we got for diagnostics */
        write(1, "SUBTEST3_FAIL: content (got: ", 28);
        write(1, buf, n);
        write(1, ")\n", 2);
        unlink(unsorted_path);
        unlink(sorted_path);
        return 0;
    }

    unlink(unsorted_path);
    unlink(sorted_path);
    write(1, "SUBTEST3_PASS\n", 14);
    return 1;
}

/* ----------------------------------------------------------------------- */
/* Main                                                                     */
/* ----------------------------------------------------------------------- */

int main(void)
{
    int pass = 1;

    if (!subtest1()) pass = 0;
    if (!subtest2()) pass = 0;
    if (!subtest3()) pass = 0;

    if (pass) {
        write(1, "PIPELINE_PASS\n", 14);
    } else {
        write(1, "PIPELINE_FAIL\n", 14);
    }

    return pass ? 0 : 1;
}
