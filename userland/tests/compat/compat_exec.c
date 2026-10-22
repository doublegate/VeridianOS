/*
 * Run one conformance test with a time limit, for the compat runner
 * (run.sh, scripts/run-compat-tests.sh).
 *
 *   compat_exec SECONDS OUTFILE N ID DIR PROGRAM [ARG...]
 *
 * The test runs in DIR (`-` for /tmp) in a process group of its own, with
 * stdin from /dev/null and stdout and stderr to OUTFILE. PROGRAM is looked
 * up in PATH unless it contains a slash. When the limit passes, the whole
 * group is killed (SIGKILL), so children a test leaves behind go with it.
 *
 * Prints, for test number N with id ID:
 *
 *   COMPAT-LOG N TEXT                       (only when it did not exit 0)
 *   COMPAT-END N ID rc=R sig=S timeout=T ms=M
 *
 * The log lines are up to six from OUTFILE that name a problem (LTP's
 * TFAIL/TBROK/TWARN/TCONF, FAIL, error, ...), or else its last three. R is
 * the exit status (or -1 if the test was killed by signal S), T is 1 when
 * the limit killed it, M is the run time in milliseconds. A program that
 * cannot be run gives rc=127 (exec failed in the child), as sh does.
 *
 * Exits 3 when the limit killed the test (the runner then removes what it
 * left in /tmp), 0 otherwise, 2 on bad usage.
 *
 * Doing all of this in one program keeps the runner to one process per
 * test: creating processes is slow enough on VeridianOS that a shell
 * pipeline per test cost more than the tests. The limit is kept by
 * polling waitpid, not by alarm or an interval timer, so the runner works
 * on kernels without those (where LTP's own timeout, which uses alarm,
 * does nothing).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define MAX_LOG 6
#define TAIL_LOG 3
#define LINE_MAX_OUT 200

static long long now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static void child(const char *out, const char *dir, char **argv)
{
    int fd;

    /* Its own group, so a timeout can kill everything it started. */
    setpgid(0, 0);
    /* OUTFILE first: a relative name is the caller's, not DIR's. */
    fd = open(out, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) {
        dup2(fd, 1);
        dup2(fd, 2);
        if (fd > 2)
            close(fd);
    }
    fd = open("/dev/null", O_RDONLY);
    if (fd >= 0) {
        dup2(fd, 0);
        if (fd > 0)
            close(fd);
    }
    if (strcmp(dir, "-") == 0)
        dir = "/tmp";
    if (chdir(dir) != 0) {
        perror(dir);
        _exit(127);
    }
    execvp(argv[0], argv);
    fprintf(stderr, "compat_exec: %s: %s\n", argv[0], strerror(errno));
    _exit(127);
}

/* A line that says what went wrong; LTP's closing counts do not. */
static int names_a_problem(const char *line)
{
    static const char *const words[] = {
        "TFAIL", "TBROK", "TWARN", "TCONF", "FAIL", "UNRESOLVED", "UNSUPPORTED",
        "UNTESTED", "error", "Error", "failed",
    };
    static const char *const counts[] = {"passed ", "failed ", "broken ", "skipped ", "warnings "};
    size_t i;

    for (i = 0; i < sizeof(counts) / sizeof(counts[0]); i++) {
        size_t n = strlen(counts[i]);
        if (strncmp(line, counts[i], n) == 0) {
            const char *p = line + n;
            while (*p == ' ')
                p++;
            while (*p >= '0' && *p <= '9')
                p++;
            if (*p == '\0')
                return 0;
        }
    }
    for (i = 0; i < sizeof(words) / sizeof(words[0]); i++)
        if (strstr(line, words[i]))
            return 1;
    return 0;
}

static void print_log(const char *out, const char *n)
{
    char line[1024], tail[TAIL_LOG][LINE_MAX_OUT + 1];
    int found = 0, seen = 0, i;
    FILE *f = fopen(out, "r");

    if (!f)
        return;
    while (fgets(line, sizeof(line), f)) {
        size_t len = strcspn(line, "\r\n");
        line[len] = '\0';
        if (len == 0)
            continue;
        if (len > LINE_MAX_OUT)
            line[LINE_MAX_OUT] = '\0';
        if (found < MAX_LOG && names_a_problem(line)) {
            printf("COMPAT-LOG %s %s\n", n, line);
            found++;
        }
        snprintf(tail[seen % TAIL_LOG], sizeof(tail[0]), "%.*s", LINE_MAX_OUT, line);
        seen++;
    }
    fclose(f);
    if (found == 0)
        for (i = seen > TAIL_LOG ? seen - TAIL_LOG : 0; i < seen; i++)
            printf("COMPAT-LOG %s %s\n", n, tail[i % TAIL_LOG]);
}

int main(int argc, char **argv)
{
    long limit;
    long long start, deadline;
    int status = 0, timed_out = 0, rc = -1, sig = 0;
    pid_t pid, w;
    const struct timespec tick = {0, 20 * 1000 * 1000};

    if (argc < 7) {
        fprintf(stderr, "usage: compat_exec SECONDS OUTFILE N ID DIR PROGRAM [ARG...]\n");
        return 2;
    }
    limit = strtol(argv[1], NULL, 10);
    if (limit <= 0)
        limit = 60;

    fflush(stdout);
    start = now_ms();
    deadline = start + limit * 1000;
    pid = fork();
    if (pid < 0) {
        printf("COMPAT-END %s %s rc=127 sig=0 timeout=0 ms=0\n", argv[3], argv[4]);
        return 0;
    }
    if (pid == 0)
        child(argv[2], argv[5], &argv[6]);
    /* Set it from this side too: the child may not have run yet. */
    setpgid(pid, pid);

    for (;;) {
        w = waitpid(pid, &status, WNOHANG);
        if (w == pid)
            break;
        if (w < 0 && errno != EINTR) {
            status = 127 << 8;
            break;
        }
        if (now_ms() >= deadline) {
            timed_out = 1;
            kill(-pid, SIGKILL);
            kill(pid, SIGKILL);
            while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
                ;
            break;
        }
        nanosleep(&tick, NULL);
    }
    /* Anything still in the group (a test's children) goes too. */
    kill(-pid, SIGKILL);

    if (WIFEXITED(status))
        rc = WEXITSTATUS(status);
    else if (WIFSIGNALED(status))
        sig = WTERMSIG(status);
    if (rc != 0)
        print_log(argv[2], argv[3]);
    printf("COMPAT-END %s %s rc=%d sig=%d timeout=%d ms=%lld\n", argv[3], argv[4], rc, sig,
           timed_out, now_ms() - start);
    return timed_out ? 3 : 0;
}
