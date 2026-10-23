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
 * A test that did not exit 0 may not have cleaned up: everything in /tmp
 * (but OUTFILE) and /dev/shm is removed then.
 *
 * When the test cannot be started (no OUTFILE, no fork: the guest is short
 * of something) it prints `COMPAT-RETRY N ID REASON` instead of a result
 * and exits 4; the runner stops, and the host runs the test again on a
 * fresh guest.
 *
 * Exits 3 when the limit killed the test, 4 as above, 0 otherwise, 2 on
 * bad usage.
 *
 * Doing all of this in one program keeps the runner to one process per
 * test: creating processes is slow enough on VeridianOS that a shell
 * pipeline per test cost more than the tests. The limit is kept by
 * polling waitpid, not by alarm or an interval timer, so the runner works
 * on kernels without those (where LTP's own timeout, which uses alarm,
 * does nothing).
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <ftw.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
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

static void child(int out, const char *dir, char **argv)
{
    int fd;

    /* Its own group, so a timeout can kill everything it started. */
    setpgid(0, 0);
    dup2(out, 1);
    dup2(out, 2);
    if (out > 2)
        close(out);
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

static int remove_one(const char *path, const struct stat *st, int type, struct FTW *ftw)
{
    (void)st;
    (void)type;
    (void)ftw;
    remove(path);
    return 0;
}

#define MAX_CLEAN 256

/* Remove everything in `dir` but `keep`, without starting a process. The
 * names are read first: removing while readdir runs can skip some. */
static void clean_dir(const char *dir, const char *keep)
{
    static char names[MAX_CLEAN][256];
    char path[512];
    struct dirent *e;
    struct stat st;
    int count, removed, i;

    /* MAX_CLEAN names a pass; passes repeat while one fills up and still
     * removes something, so a directory with more entries is emptied too
     * and one that cannot be is not retried forever. */
    do {
        DIR *d = opendir(dir);

        if (!d)
            return;
        count = 0;
        removed = 0;
        while ((e = readdir(d)) != NULL && count < MAX_CLEAN) {
            if (strcmp(e->d_name, ".") == 0 || strcmp(e->d_name, "..") == 0)
                continue;
            if (keep && strcmp(e->d_name, keep) == 0)
                continue;
            snprintf(names[count++], sizeof(names[0]), "%.255s", e->d_name);
        }
        closedir(d);
        for (i = 0; i < count; i++) {
            snprintf(path, sizeof(path), "%.200s/%.255s", dir, names[i]);
            nftw(path, remove_one, 16, FTW_DEPTH | FTW_PHYS);
            if (lstat(path, &st) != 0)
                removed++;
        }
    } while (count == MAX_CLEAN && removed > 0);
}

/* What a test that died before its cleanup leaves: LTP's LTP_* directories
 * and ltp_* results files, and the files, shared memory objects and named
 * semaphores of Open POSIX tests, under names of their own. Tests run one at
 * a time, so all of /tmp and /dev/shm is theirs, but this run's OUTFILE. */
static void clean_tmp(const char *out)
{
    const char *base = strrchr(out, '/');
    int in_tmp = base && (size_t)(base - out) == 4 && strncmp(out, "/tmp", 4) == 0;

    clean_dir("/tmp", in_tmp ? base + 1 : NULL);
    clean_dir("/dev/shm", NULL);
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
        size_t full = strlen(line), len = strcspn(line, "\r\n");
        int c;
        /* A line longer than the buffer (fgets filled it and stopped short
         * of the newline): drop the rest of it, so its tail is not read as a
         * line of its own. A '\r' earlier in the line does not change that. */
        if (full == sizeof(line) - 1 && line[full - 1] != '\n')
            while ((c = fgetc(f)) != EOF && c != '\n')
                ;
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
    int status = 0, timed_out = 0, rc = -1, sig = 0, out;
    pid_t pid, w;
    const struct timespec tick = {0, 20 * 1000 * 1000};

    if (argc < 7) {
        fprintf(stderr, "usage: compat_exec SECONDS OUTFILE N ID DIR PROGRAM [ARG...]\n");
        return 2;
    }
    limit = strtol(argv[1], NULL, 10);
    if (limit <= 0)
        limit = 60;

    /* OUTFILE is opened (and emptied) here: a test whose output cannot be
     * captured is not run, since its output would reach the console
     * unmarked, and a relative name is the caller's, not DIR's. */
    /* Not O_CLOEXEC: if it came back as fd 1 or 2, dup2 onto itself would
     * leave the flag set and exec would close the test's output. */
    out = open(argv[2], O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (out < 0) {
        printf("COMPAT-RETRY %s %s %s: %s\n", argv[3], argv[4], argv[2], strerror(errno));
        return 4;
    }
    fflush(stdout);
    start = now_ms();
    deadline = start + limit * 1000LL;
    pid = fork();
    if (pid < 0) {
        /* Say so: out of processes or memory is not a missing program. */
        printf("COMPAT-RETRY %s %s fork: %s\n", argv[3], argv[4], strerror(errno));
        return 4;
    }
    if (pid == 0)
        child(out, argv[5], &argv[6]);
    close(out);
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
            /* Killed, unless waitpid says otherwise: a failing wait must not
             * leave the last WNOHANG status (0, "exited 0") as the result. */
            status = SIGKILL;
            while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
                ;
            break;
        }
        nanosleep(&tick, NULL);
    }
    /* Anything still in the group (a test's children) goes too. */
    kill(-pid, SIGKILL);

    if (WIFEXITED(status)) {
        rc = WEXITSTATUS(status);
        /* It exited on its own just as the limit passed. */
        timed_out = 0;
    } else if (WIFSIGNALED(status)) {
        sig = WTERMSIG(status);
    }
    if (rc != 0) {
        print_log(argv[2], argv[3]);
        clean_tmp(argv[2]);
    }
    printf("COMPAT-END %s %s rc=%d sig=%d timeout=%d ms=%lld\n", argv[3], argv[4], rc, sig,
           timed_out, now_ms() - start);
    return timed_out ? 3 : 0;
}
