/*
 * musl_runtime_test -- in-guest checks for programs built against musl
 * (tools/cross/build-musl.sh), as the KDE binaries are.
 *
 * audit_runtime_test is built with VeridianOS's own C library. This program
 * exercises what musl does its own way: fsync and flock, prctl, raise/abort
 * (tkill), waitid, the memory protections, threads (tasks of their own
 * since stage D2), and process start-up. It is built as a static PIE, so
 * the kernel loads it at a base of its choosing and musl relocates it from
 * the auxiliary vector before main runs (ADR 0010).
 *
 * Run from a BusyBox shell: /bin/musl_runtime_test
 * Prints "PASS <name>" or "FAIL <name>: <why>" per check and a final
 * "MUSL-RUNTIME: <passed>/<total>" summary.
 */

#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <grp.h>
#include <sys/stat.h>
#include <limits.h>
#include <poll.h>
#include <sys/epoll.h>
#include <sys/eventfd.h>
#include <stdint.h>
#include <pthread.h>
#include <sched.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <termios.h>
#include <sys/inotify.h>
#include <sys/ioctl.h>
#include <sys/fsuid.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/prctl.h>
#include <sys/personality.h>
#include <sys/resource.h>
#include <sys/sysinfo.h>
#include <sys/statfs.h>
#include <sys/sysmacros.h>
#include <sys/time.h>
#include <sys/times.h>
#include <sys/select.h>
#include <sys/sendfile.h>
#include <spawn.h>
#include <sys/wait.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <sys/un.h>
#include <sys/auxv.h>
#include <sys/signalfd.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <sys/utsname.h>
#include <sys/xattr.h>
#include <shadow.h>
#include <crypt.h>
#include <pwd.h>
#include <setjmp.h>
#include <sys/random.h>
#include <linux/futex.h>
#include <sys/timerfd.h>
#include <sys/timex.h>
#include <time.h>
#include <unistd.h>

static int passed, total;

static void report(const char *name, int ok, const char *why)
{
    total++;
    if (ok) {
        passed++;
        printf("PASS %s\n", name);
    } else {
        printf("FAIL %s: %s\n", name, why);
    }
    fflush(stdout);
}

/* fsync must reach fsync, not flock (both are native 73; N-120). */
static void test_fsync(void)
{
    int fd = open("/tmp/musl_fsync", O_CREAT | O_RDWR | O_TRUNC, 0644);
    int ok = fd >= 0 && write(fd, "data", 4) == 4 && fsync(fd) == 0;
    static char why[64];
    snprintf(why, sizeof(why), "fd=%d errno=%d", fd, errno);
    report("musl_fsync", ok, why);
    if (fd >= 0)
        close(fd);
}

/* flock: an exclusive lock held by the parent blocks a child's
 * non-blocking attempt, and is released on unlock (N-120). */
static void test_flock(void)
{
    int fd = open("/tmp/musl_flock", O_CREAT | O_RDWR, 0644);
    if (fd < 0 || flock(fd, LOCK_EX) != 0) {
        report("musl_flock_conflict", 0, "parent could not lock");
        return;
    }
    pid_t pid = fork();
    if (pid == 0) {
        int cfd = open("/tmp/musl_flock", O_RDWR);
        if (cfd < 0)
            _exit(2);
        int r = flock(cfd, LOCK_EX | LOCK_NB);
        _exit(r == -1 && errno == EWOULDBLOCK ? 0 : 1);
    }
    int st = 0;
    int blocked = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;
    flock(fd, LOCK_UN);
    pid = fork();
    if (pid == 0) {
        int cfd = open("/tmp/musl_flock", O_RDWR);
        _exit(cfd >= 0 && flock(cfd, LOCK_EX | LOCK_NB) == 0 ? 0 : 1);
    }
    int st2 = 0;
    int freed = pid > 0 && waitpid(pid, &st2, 0) == pid && WIFEXITED(st2) && WEXITSTATUS(st2) == 0;
    static char why[64];
    snprintf(why, sizeof(why), "blocked=%d freed=%d", blocked, freed);
    report("musl_flock_conflict", blocked && freed, why);
    close(fd);
}

/* prctl reaches prctl (it used to land on native unlink; N-103); the
 * command name is stored (cut at 15 bytes) and read back (N-227);
 * no_new_privs is set one way, with Linux's argument checks, and inherited
 * by a fork child; unknown options fail closed (N-151). */
static void test_prctl(void)
{
    char name[16] = {0}, cut[16] = {0};
    int set = prctl(PR_SET_NAME, "musltest", 0, 0, 0);
    int get = prctl(PR_GET_NAME, name, 0, 0, 0);
    prctl(PR_SET_NAME, "a-name-longer-than-fifteen", 0, 0, 0);
    prctl(PR_GET_NAME, cut, 0, 0, 0);
    prctl(PR_SET_NAME, "musltest", 0, 0, 0);
    pid_t pid = fork();
    if (pid == 0) {
        int f = 0;
        if (prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 0) f |= 1;
        if (prctl(PR_SET_NO_NEW_PRIVS, 2, 0, 0, 0) != -1 || errno != EINVAL) f |= 2;
        if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0) f |= 4;
        if (prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 1) f |= 8;
        if (prctl(PR_GET_NO_NEW_PRIVS, 1, 0, 0, 0) != -1 || errno != EINVAL) f |= 16;
        pid_t child = fork();
        if (child == 0)
            _exit(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) == 1 ? 0 : 1);
        int st = 0;
        if (waitpid(child, &st, 0) != child || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 32;
        errno = 0;
        if (prctl(0x7fff, 0, 0, 0, 0) != -1 || errno != EINVAL) f |= 64;
        /* PR_SET_DUMPABLE: 0 and 1 only; the flag reads back. */
        if (prctl(PR_GET_DUMPABLE, 0, 0, 0, 0) != 1) f |= 128;
        if (prctl(PR_SET_DUMPABLE, 0, 0, 0, 0) != 0 || prctl(PR_GET_DUMPABLE, 0, 0, 0, 0) != 0)
            f |= 128;
        if (prctl(PR_SET_DUMPABLE, 2, 0, 0, 0) != -1 || errno != EINVAL) f |= 128;
        _exit(f);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    static char why[96];
    snprintf(why, sizeof(why), "set=%d get=%d name=%s cut=%s nnp-child=%d", set, get, name, cut,
             WIFEXITED(st) ? WEXITSTATUS(st) : 255);
    report("musl_prctl",
           set == 0 && get == 0 && strcmp(name, "musltest") == 0
               && strcmp(cut, "a-name-longer-t") == 0 && WIFEXITED(st) && WEXITSTATUS(st) == 0,
           why);
}

/* Spin for at least `ms` of CPU time on this thread. */
/* User plus system time of `ru` in milliseconds. */
static long rusage_ms(const struct rusage *ru)
{
    long us = (ru->ru_utime.tv_sec + ru->ru_stime.tv_sec) * 1000000L + ru->ru_utime.tv_usec
              + ru->ru_stime.tv_usec;
    return us / 1000;
}

static void burn_cpu(long ms)
{
    struct timespec start, now;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &start);
    /* In nanoseconds: across a second boundary the nanosecond difference
     * is negative, and dividing it before adding stopped ~1 ms early. */
    long long until = (long long)start.tv_sec * 1000000000LL + start.tv_nsec + ms * 1000000LL;
    do {
        for (volatile int i = 0; i < 10000; i++)
            ;
        clock_gettime(CLOCK_THREAD_CPUTIME_ID, &now);
    } while ((long long)now.tv_sec * 1000000000LL + now.tv_nsec < until);
}

/* Spin for at least `ms` of CPU time, nearly all of it in user mode: a
 * long loop between clock reads (each read is a system call, and under
 * emulation one costs about as much as burn_cpu's whole inner loop). */
static void burn_user_cpu(long ms)
{
    struct timespec start, now;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &start);
    long long until = (long long)start.tv_sec * 1000000000LL + start.tv_nsec + ms * 1000000LL;
    do {
        for (volatile int i = 0; i < 1000000; i++)
            ;
        clock_gettime(CLOCK_THREAD_CPUTIME_ID, &now);
    } while ((long long)now.tv_sec * 1000000000LL + now.tv_nsec < until);
}

static long long ms_of(struct timespec t)
{
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static int cond_waited;
static pthread_mutex_t cond_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cond_never = PTHREAD_COND_INITIALIZER;

/* Clocks (N-218 to N-220) and resource usage (N-223, N-213): the wall
 * clock counts from 1970; CPU clocks advance with work, also through
 * clock_getcpuclockid and pthread_getcpuclockid; resolution is reported;
 * gettimeofday takes NULL; only root sets the clock; absolute
 * CLOCK_REALTIME deadlines (clock_nanosleep, a condition wait, which
 * musl times with FUTEX_CLOCK_REALTIME) end when the wall clock reaches
 * them; a TFD_TIMER_CANCEL_ON_SET timerfd sees the clock set; getrusage,
 * times, wait4's rusage and sysinfo report real numbers. */
static void test_clocks_and_usage(void)
{
    int f = 0;
    struct timespec rt, mono, res, a, b;
    if (clock_gettime(CLOCK_REALTIME, &rt) != 0 || rt.tv_sec < 1700000000) f |= 1;
    if (clock_gettime(CLOCK_MONOTONIC, &mono) != 0 || clock_gettime(CLOCK_BOOTTIME, &a) != 0) f |= 1;
    if (clock_getres(CLOCK_MONOTONIC, &res) != 0 || res.tv_sec != 0 || res.tv_nsec != 1) f |= 2;
    errno = 0;
    if (clock_gettime(12, &a) != -1 || errno != EINVAL) f |= 2;

    struct timespec ta, tb;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &a);
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ta);
    burn_cpu(30);
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &tb);
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &b);
    long proc_us = (b.tv_sec - a.tv_sec) * 1000000L + (b.tv_nsec - a.tv_nsec) / 1000;
    long thr_us = (tb.tv_sec - ta.tv_sec) * 1000000L + (tb.tv_nsec - ta.tv_nsec) / 1000;
    if (ms_of(b) - ms_of(a) < 30) f |= 4;
    clockid_t pclk, tclk;
    struct timespec pc, tc;
    if (clock_getcpuclockid(getpid(), &pclk) != 0 || clock_gettime(pclk, &pc) != 0
        || ms_of(pc) < ms_of(b))
        f |= 8;
    if (pthread_getcpuclockid(pthread_self(), &tclk) != 0 || clock_gettime(tclk, &tc) != 0
        || ms_of(tc) < 30)
        f |= 8;
    if (clock() <= 0) f |= 8;

    struct timeval tv;
    struct timezone tz;
    if (gettimeofday(&tv, NULL) != 0 || tv.tv_sec < 1700000000 || gettimeofday(NULL, &tz) != 0)
        f |= 16;

    /* Root sets the clock (and puts it back); a user may not. */
    clock_gettime(CLOCK_REALTIME, &rt);
    struct timespec moved = {rt.tv_sec + 3600, rt.tv_nsec};
    int set_ok = clock_settime(CLOCK_REALTIME, &moved) == 0;
    clock_gettime(CLOCK_REALTIME, &a);
    moved.tv_sec -= 3600;
    clock_settime(CLOCK_REALTIME, &moved);
    if (!set_ok || a.tv_sec < rt.tv_sec + 3599) f |= 32;
    pid_t pid = fork();
    if (pid == 0) {
        setresuid(1000, 1000, 1000);
        _exit(clock_settime(CLOCK_REALTIME, &moved) == -1 && errno == EPERM ? 0 : 1);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    if (!WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 32;

    /* Absolute wall-clock deadlines end on time. */
    clock_gettime(CLOCK_MONOTONIC, &a);
    clock_gettime(CLOCK_REALTIME, &rt);
    rt.tv_nsec += 50000000;
    if (rt.tv_nsec >= 1000000000) {
        rt.tv_sec++;
        rt.tv_nsec -= 1000000000;
    }
    int slept = clock_nanosleep(CLOCK_REALTIME, TIMER_ABSTIME, &rt, NULL);
    clock_gettime(CLOCK_MONOTONIC, &b);
    if (slept != 0 || ms_of(b) - ms_of(a) < 40 || ms_of(b) - ms_of(a) > 2000) f |= 64;
    /* The stopwatch starts before the deadline is taken. Started after it,
     * any delay in between (this thread preempted; the lock itself is
     * uncontended) fell outside the measurement, and a wait that ended on
     * time measured short (seen once on RISC-V with four CPUs). A delay
     * before the deadline can now only lengthen the measurement. */
    clock_gettime(CLOCK_MONOTONIC, &a);
    clock_gettime(CLOCK_REALTIME, &rt);
    rt.tv_nsec += 50000000;
    if (rt.tv_nsec >= 1000000000) {
        rt.tv_sec++;
        rt.tv_nsec -= 1000000000;
    }
    pthread_mutex_lock(&cond_lock);
    while (!cond_waited) {
        if (pthread_cond_timedwait(&cond_never, &cond_lock, &rt) == ETIMEDOUT)
            break;
    }
    clock_gettime(CLOCK_MONOTONIC, &b);
    pthread_mutex_unlock(&cond_lock);
    if (ms_of(b) - ms_of(a) < 40 || ms_of(b) - ms_of(a) > 2000) f |= 128;

    /* A wall-clock timerfd armed to cancel on a clock change sees one. */
    int tfd = timerfd_create(CLOCK_REALTIME, TFD_NONBLOCK);
    clock_gettime(CLOCK_REALTIME, &rt);
    struct itimerspec its = {{0, 0}, {rt.tv_sec + 100, 0}};
    uint64_t ticks;
    if (tfd < 0 || timerfd_settime(tfd, TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET, &its, NULL) != 0)
        f |= 256;
    clock_gettime(CLOCK_REALTIME, &rt);
    clock_settime(CLOCK_REALTIME, &rt);
    errno = 0;
    if (read(tfd, &ticks, sizeof(ticks)) != -1 || errno != ECANCELED) f |= 256;
    if (read(tfd, &ticks, sizeof(ticks)) != -1 || errno != EAGAIN) f |= 256;
    close(tfd);

    /* Usage. */
    struct rusage ru;
    if (getrusage(RUSAGE_SELF, &ru) != 0 || rusage_ms(&ru) < 30 || ru.ru_maxrss <= 0) f |= 512;
    errno = 0;
    if (getrusage(7, &ru) != -1 || errno != EINVAL) f |= 512;
    pid = fork();
    if (pid == 0) {
        burn_user_cpu(40);
        _exit(0);
    }
    struct rusage child_ru;
    memset(&child_ru, 0, sizeof(child_ru));
    /* A signal from an earlier test's timer may interrupt the wait. */
    pid_t got;
    int wait_errno = 0;
    while ((got = wait4(pid, &st, 0, &child_ru)) == -1 && errno == EINTR)
        wait_errno = EINTR;
    if (got == -1)
        wait_errno = errno;
    /* Microseconds summed before converting: truncating utime and stime to
     * milliseconds separately lost up to 2 ms (40 ms read as 39). */
    if (got != pid || rusage_ms(&child_ru) < 40) f |= 1024;
    /* The child computed 40 ms in user mode (burn_user_cpu): at least half
     * of that shows as user time. Its fork and exit add system time of
     * their own, so the total is no yardstick. AArch64 counted every timer
     * tick as system time, which made this 0 (PR #19 review). */
    long child_user_us = child_ru.ru_utime.tv_sec * 1000000L + child_ru.ru_utime.tv_usec;
    if (child_user_us < 20000) f |= 8192;
    if (getrusage(RUSAGE_CHILDREN, &ru) != 0 || rusage_ms(&ru) < 40) f |= 1024;
    struct tms tm;
    clock_t since_boot = times(&tm);
    /* User and system time are each truncated to clock ticks (10 ms), so
     * their sum can be one tick short: 30+ ms of our own time is at least 2
     * ticks, the child's 40+ ms at least 3. */
    if (since_boot <= 0 || tm.tms_utime + tm.tms_stime < 2 || tm.tms_cutime + tm.tms_cstime < 3)
        f |= 2048;
    struct sysinfo si;
    if (sysinfo(&si) != 0 || si.totalram == 0 || si.freeram == 0 || si.freeram > si.totalram
        || si.procs == 0 || si.mem_unit != 1 || si.uptime <= 0)
        f |= 4096;

    static char why[160];
    snprintf(why, sizeof(why),
             "fail=%#x proc_us=%ld thread_us=%ld wait_errno=%d child_ms=%ld child_user_us=%ld "
             "children_ms=%ld",
             f, proc_us, thr_us, wait_errno, rusage_ms(&child_ru), child_user_us, rusage_ms(&ru));
    report("musl_clocks_and_usage", f == 0, why);
}

/* Resource limits (N-224): each limit is set in a child of its own, which
 * reports through its exit status (0: as Linux) or how it died. */

static volatile sig_atomic_t rt_caught;
static int xcpu_pipe = -1;

static void count_rt(int sig)
{
    (void)sig;
    rt_caught++;
}

static void note_xcpu(int sig)
{
    (void)sig;
    char c = 'x';
    (void)!write(xcpu_pipe, &c, 1);
}

static int recurse_deep(int depth)
{
    volatile char frame[4096];
    frame[0] = (char)depth;
    return depth == 0 ? frame[0] : recurse_deep(depth - 1) + frame[0];
}

/* Run `fn` in a child; its wait status. */
static int in_child(int (*fn)(void))
{
    pid_t pid = fork();
    if (pid == 0)
        _exit(fn());
    int st = -1;
    if (pid < 0 || waitpid(pid, &st, 0) != pid)
        return -1;
    return st;
}

static int exited_zero(int st)
{
    return st != -1 && WIFEXITED(st) && WEXITSTATUS(st) == 0;
}

static int rl_query(void)
{
    int f = 0;
    struct rlimit r, old;
    if (getrlimit(RLIMIT_NOFILE, &r) != 0 || r.rlim_cur != 1024 || r.rlim_max != 1024) f |= 1;
    if (getrlimit(RLIMIT_STACK, &r) != 0 || r.rlim_cur != 8u << 20) f |= 1;
    if (getrlimit(RLIMIT_CORE, &r) != 0 || r.rlim_cur != 0 || r.rlim_max != RLIM_INFINITY) f |= 1;
    errno = 0;
    if (getrlimit(RLIM_NLIMITS, &r) != -1 || errno != EINVAL) f |= 2;
    /* Soft above hard. */
    r.rlim_cur = 10;
    r.rlim_max = 5;
    errno = 0;
    if (setrlimit(RLIMIT_CORE, &r) != -1 || errno != EINVAL) f |= 2;
    /* prlimit reads back and swaps in one call. */
    r.rlim_cur = 100;
    r.rlim_max = 200;
    if (prlimit(0, RLIMIT_CORE, &r, &old) != 0 || old.rlim_cur != 0
        || prlimit(getpid(), RLIMIT_CORE, NULL, &old) != 0 || old.rlim_cur != 100
        || old.rlim_max != 200)
        f |= 4;
    errno = 0;
    if (prlimit(999999, RLIMIT_CORE, NULL, &old) != -1 || errno != ESRCH) f |= 8;
    /* Without privilege: no hard limit raised, no other user's process
     * (a root child; there is no pid 1 to ask about). */
    int hold[2] = {-1, -1};
    if (pipe(hold) != 0) f |= 32;
    pid_t root_child = fork();
    if (root_child == 0) {
        /* Lives until the parent closes its end (it cannot kill a root
         * process once it has dropped privilege). */
        char c;
        close(hold[1]);
        (void)!read(hold[0], &c, 1);
        _exit(0);
    }
    close(hold[0]);
    if (setresuid(4320, 4320, 4320) != 0) f |= 16;
    r.rlim_cur = 100;
    r.rlim_max = 300;
    errno = 0;
    if (setrlimit(RLIMIT_CORE, &r) != -1 || errno != EPERM) f |= 16;
    errno = 0;
    if (root_child < 0 || prlimit(root_child, RLIMIT_CORE, NULL, &old) != -1 || errno != EPERM)
        f |= 32;
    close(hold[1]);
    if (root_child > 0)
        waitpid(root_child, NULL, 0);
    return f;
}

static int rl_nofile(void)
{
    int f = 0;
    struct rlimit r = {8, 8};
    if (setrlimit(RLIMIT_NOFILE, &r) != 0) return 1;
    int fd, last = -1;
    errno = 0;
    while ((fd = open("/dev/null", O_RDONLY)) >= 0)
        last = fd;
    if (errno != EMFILE || last != 7) f |= 2;
    errno = 0;
    if (dup2(0, 8) != -1 || errno != EBADF) f |= 4;
    errno = 0;
    if (fcntl(0, F_DUPFD, 8) != -1 || errno != EINVAL) f |= 4;
    close(last);
    if (dup2(0, 7) != 7) f |= 8;
    r.rlim_cur = r.rlim_max = 1 << 20;
    errno = 0;
    if (setrlimit(RLIMIT_NOFILE, &r) != -1 || errno != EPERM) f |= 16;
    return f;
}

static int rl_fsize_ignored(void)
{
    int f = 0;
    signal(SIGXFSZ, SIG_IGN);
    struct rlimit r = {100, 100};
    if (setrlimit(RLIMIT_FSIZE, &r) != 0) return 1;
    int fd = open("/tmp/musl_fsize", O_CREAT | O_RDWR | O_TRUNC, 0644);
    char buf[150];
    memset(buf, 'f', sizeof(buf));
    /* A write crossing the limit is shortened; one at it fails. */
    if (write(fd, buf, sizeof(buf)) != 100) f |= 2;
    errno = 0;
    if (write(fd, buf, 1) != -1 || errno != EFBIG) f |= 4;
    errno = 0;
    if (pwrite(fd, buf, 10, 200) != -1 || errno != EFBIG) f |= 8;
    if (pwrite(fd, buf, 10, 95) != 5) f |= 8;
    errno = 0;
    if (ftruncate(fd, 200) != -1 || errno != EFBIG) f |= 16;
    if (ftruncate(fd, 50) != 0) f |= 16;
    /* writev counts the vector as one write. */
    lseek(fd, 90, SEEK_SET);
    struct iovec iov[2] = {{buf, 5}, {buf, 20}};
    if (writev(fd, iov, 2) != 10) f |= 32;
    close(fd);
    unlink("/tmp/musl_fsize");
    return f;
}

static int rl_fsize_signal(void)
{
    struct rlimit r = {10, 10};
    setrlimit(RLIMIT_FSIZE, &r);
    int fd = open("/tmp/musl_fsize2", O_CREAT | O_RDWR | O_TRUNC, 0644);
    unlink("/tmp/musl_fsize2");
    char buf[20] = {0};
    (void)!write(fd, buf, 10);
    (void)!write(fd, buf, 1); /* SIGXFSZ, default action: core */
    return 1;
}

static int rl_memory(void)
{
    int f = 0;
    struct rlimit r = {1u << 30, 1u << 30};
    if (setrlimit(RLIMIT_AS, &r) != 0) return 1;
    errno = 0;
    if (mmap(NULL, 2u << 30, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) != MAP_FAILED
        || errno != ENOMEM)
        f |= 2;
    void *p = mmap(NULL, 1 << 20, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) f |= 4;
    else munmap(p, 1 << 20);
    /* RLIMIT_DATA 0: no private writable memory, brk included. */
    r.rlim_cur = r.rlim_max = 0;
    if (setrlimit(RLIMIT_DATA, &r) != 0) f |= 8;
    errno = 0;
    if (mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) != MAP_FAILED
        || errno != ENOMEM)
        f |= 16;
    p = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) f |= 32;
    /* ... nor private memory made writable later. */
    p = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    errno = 0;
    if (p == MAP_FAILED || mprotect(p, 4096, PROT_READ | PROT_WRITE) != -1 || errno != ENOMEM)
        f |= 128;
    /* The raw call: musl's sbrk refuses any increment by itself. A
     * refused brk returns the unchanged break. */
    long brk0 = syscall(SYS_brk, 0);
    if (syscall(SYS_brk, brk0 + (1 << 20)) != brk0) f |= 64;
    return f;
}

static int rl_stack(void)
{
    struct rlimit r = {512u << 10, 512u << 10};
    setrlimit(RLIMIT_STACK, &r);
    return recurse_deep(256) == 12345; /* 1 MiB of frames: SIGSEGV */
}

static void *rl_thread(void *arg)
{
    return arg;
}

static int rl_nproc(void)
{
    int f = 0;
    if (setresuid(4321, 4321, 4321) != 0) return 1;
    struct rlimit r = {1, 1};
    if (setrlimit(RLIMIT_NPROC, &r) != 0) return 2;
    errno = 0;
    pid_t pid = fork();
    if (pid == 0) _exit(0);
    if (pid != -1 || errno != EAGAIN) f |= 4;
    pthread_t t;
    if (pthread_create(&t, NULL, rl_thread, NULL) != EAGAIN) f |= 8;
    return f;
}

static int rl_priorities(void)
{
    int f = 0;
    struct rlimit r = {25, 25};
    if (setrlimit(RLIMIT_NICE, &r) != 0) return 1;
    r.rlim_cur = r.rlim_max = 10;
    if (setrlimit(RLIMIT_RTPRIO, &r) != 0) return 1;
    if (setresuid(4322, 4322, 4322) != 0) return 1;
    /* RLIMIT_NICE 25: down to nice -5, not past it. */
    if (setpriority(PRIO_PROCESS, 0, 10) != 0) f |= 2;
    if (setpriority(PRIO_PROCESS, 0, -5) != 0) f |= 4;
    errno = 0;
    if (getpriority(PRIO_PROCESS, 0) != -5 || errno != 0) f |= 4;
    errno = 0;
    if (setpriority(PRIO_PROCESS, 0, -6) != -1 || errno != EACCES) f |= 8;
    /* RLIMIT_RTPRIO 10: SCHED_FIFO up to priority 10. */
    struct sched_param sp = {.sched_priority = 11};
    errno = 0;
    if (syscall(SYS_sched_setscheduler, 0, SCHED_FIFO, &sp) != -1 || errno != EPERM) f |= 16;
    sp.sched_priority = 10;
    if (syscall(SYS_sched_setscheduler, 0, SCHED_FIFO, &sp) != 0) f |= 32;
    sp.sched_priority = 0;
    syscall(SYS_sched_setscheduler, 0, SCHED_OTHER, &sp);
    return f;
}

static int rl_sigpending(void)
{
    int f = 0;
    int sig = SIGRTMIN + 1;
    struct rlimit r = {2, 2};
    if (setrlimit(RLIMIT_SIGPENDING, &r) != 0) return 1;
    if (setresuid(4323, 4323, 4323) != 0) return 1;
    signal(sig, count_rt);
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, sig);
    sigprocmask(SIG_BLOCK, &set, NULL);
    pid_t me = getpid(), tid = gettid();
    if (syscall(SYS_tgkill, me, tid, sig) != 0 || syscall(SYS_tgkill, me, tid, sig) != 0) f |= 2;
    errno = 0;
    if (syscall(SYS_tgkill, me, tid, sig) != -1 || errno != EAGAIN) f |= 4;
    /* kill() past the limit still succeeds: the signal pends on the
     * process without a queued instance. The two queued for the thread and
     * the process's one are delivered: three, as on Linux. */
    if (kill(me, sig) != 0) f |= 8;
    sigprocmask(SIG_UNBLOCK, &set, NULL);
    if (rt_caught != 3) f |= 16;
    return f;
}

static int rl_cpu(void)
{
    /* SIGXCPU at the soft limit (1 s), caught; SIGKILL at the hard (2 s). */
    signal(SIGXCPU, note_xcpu);
    struct rlimit r = {1, 2};
    setrlimit(RLIMIT_CPU, &r);
    burn_cpu(10000);
    return 1;
}

static void test_rlimits(void)
{
    int f = 0, st, codes[9] = {0};
    if (!exited_zero(codes[0] = in_child(rl_query))) f |= 1;
    if (!exited_zero(codes[1] = in_child(rl_nofile))) f |= 2;
    if (!exited_zero(codes[2] = in_child(rl_fsize_ignored))) f |= 4;
    st = in_child(rl_fsize_signal);
    if (!WIFSIGNALED(st) || WTERMSIG(st) != SIGXFSZ) f |= 8;
    if (!exited_zero(codes[4] = in_child(rl_memory))) f |= 16;
    st = in_child(rl_stack);
    if (!WIFSIGNALED(st) || WTERMSIG(st) != SIGSEGV) f |= 32;
    if (!exited_zero(codes[6] = in_child(rl_nproc))) f |= 64;
    if (!exited_zero(codes[7] = in_child(rl_priorities))) f |= 128;
    if (!exited_zero(codes[8] = in_child(rl_sigpending))) f |= 256;
    int fds[2];
    if (pipe(fds) == 0) {
        xcpu_pipe = fds[1];
        st = in_child(rl_cpu);
        close(fds[1]);
        char c = 0;
        if (!WIFSIGNALED(st) || WTERMSIG(st) != SIGKILL || read(fds[0], &c, 1) != 1 || c != 'x')
            f |= 512;
        close(fds[0]);
    } else {
        f |= 512;
    }
    /* Each child's wait status (exit code << 8): which checks failed. */
    static char why[160];
    snprintf(why, sizeof(why), "fail=%#x st=%#x,%#x,%#x,%#x,%#x,%#x,%#x", f, codes[0], codes[1],
             codes[2], codes[4], codes[6], codes[7], codes[8]);
    report("musl_rlimits", f == 0, why);
}

/* SIGPIPE (Linux): a write or send that fails with EPIPE raises it, unless
 * a send passes MSG_NOSIGNAL; a pipe raises it even after a partial write. */
static int sp_pipe_default(void)
{
    int p[2];
    if (pipe(p) != 0) return 1;
    close(p[0]);
    (void)!write(p[1], "x", 1); /* SIGPIPE: terminates */
    return 2;
}

static int sp_ignored(void)
{
    int f = 0, p[2], s[2];
    signal(SIGPIPE, SIG_IGN);
    if (pipe(p) != 0) return 1;
    close(p[0]);
    errno = 0;
    if (write(p[1], "x", 1) != -1 || errno != EPIPE) f |= 2;
    struct iovec iov = {(void *)"xy", 2};
    errno = 0;
    if (writev(p[1], &iov, 1) != -1 || errno != EPIPE) f |= 2;
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, s) != 0) return f | 4;
    close(s[1]);
    errno = 0;
    if (send(s[0], "x", 1, 0) != -1 || errno != EPIPE) f |= 8;
    errno = 0;
    if (write(s[0], "x", 1) != -1 || errno != EPIPE) f |= 8;
    return f;
}

static volatile sig_atomic_t sigpipes;

static void count_sigpipe(int sig)
{
    (void)sig;
    sigpipes++;
}

static int sp_nosignal(void)
{
    int f = 0, s[2];
    signal(SIGPIPE, count_sigpipe);
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, s) != 0) return 1;
    close(s[1]);
    errno = 0;
    if (send(s[0], "x", 1, MSG_NOSIGNAL) != -1 || errno != EPIPE || sigpipes != 0) f |= 2;
    struct msghdr mh = {0};
    struct iovec iov = {(void *)"x", 1};
    mh.msg_iov = &iov;
    mh.msg_iovlen = 1;
    errno = 0;
    if (sendmsg(s[0], &mh, MSG_NOSIGNAL) != -1 || errno != EPIPE || sigpipes != 0) f |= 4;
    errno = 0;
    if (send(s[0], "x", 1, 0) != -1 || errno != EPIPE || sigpipes != 1) f |= 8;
    return f;
}

static void test_sigpipe(void)
{
    int f = 0;
    int st = in_child(sp_pipe_default);
    if (!WIFSIGNALED(st) || WTERMSIG(st) != SIGPIPE) f |= 1;
    int a = in_child(sp_ignored), b = in_child(sp_nosignal);
    if (!exited_zero(a)) f |= 2;
    if (!exited_zero(b)) f |= 4;
    static char why[96];
    snprintf(why, sizeof(why), "fail=%#x st=%#x,%#x,%#x", f, st, a, b);
    report("musl_sigpipe", f == 0, why);
}

/* Every signal carries its siginfo (D3): sigqueue values arrive in send
 * order with SI_QUEUE and the sender; kill is SI_USER and raise SI_TKILL;
 * sigtimedwait takes a pending or arriving signal or times out (EAGAIN);
 * SIGCHLD says how the child ended; a fault gives its code and address;
 * an SA_RESTART handler does not restart sigsuspend; rt_sigqueueinfo
 * refuses to pose as kill for another process; queued sends past
 * RLIMIT_SIGPENDING fail with EAGAIN while kill still pends. */
static volatile int si_values[4], si_n, si_code_seen, si_pid_seen, si_status_seen;
static volatile int si_usr2_seen;

static void on_info(int sig, siginfo_t *si, void *uc)
{
    (void)sig;
    (void)uc;
    if (si_n < 4)
        si_values[si_n] = si->si_value.sival_int;
    si_n++;
    si_code_seen = si->si_code;
    si_pid_seen = si->si_pid;
    si_status_seen = si->si_status;
}

static void on_usr2_count(int sig)
{
    (void)sig;
    si_usr2_seen++;
}

static sigjmp_buf si_jump;
static volatile int si_fault_code;
static volatile uintptr_t si_fault_addr;

static void on_segv_info(int sig, siginfo_t *si, void *uc)
{
    (void)sig;
    (void)uc;
    si_fault_code = si->si_code;
    si_fault_addr = (uintptr_t)si->si_addr;
    siglongjmp(si_jump, 1);
}

static int si_child_exit7(void)
{
    return 7;
}

static int si_restart_child_body(pid_t parent)
{
    struct timespec t = {0, 200 * 1000 * 1000};
    nanosleep(&t, 0);
    kill(parent, SIGUSR1);
    nanosleep(&t, 0);
    kill(parent, SIGUSR2);
    return 0;
}

static int si_queue_child_body(pid_t parent)
{
    struct timespec t = {0, 100 * 1000 * 1000};
    nanosleep(&t, 0);
    union sigval v = {.sival_int = 4242};
    return sigqueue(parent, SIGRTMIN + 5, v) == 0 ? 0 : 1;
}

static void test_siginfo(void)
{
    /* volatile: kept across the siglongjmp out of the SIGSEGV handler. */
    volatile int f = 0;
    struct sigaction sa;
    sigset_t set, empty;
    sigemptyset(&empty);
    memset(&sa, 0, sizeof(sa));
    sa.sa_sigaction = on_info;
    sa.sa_flags = SA_SIGINFO;
    int rt = SIGRTMIN + 3;
    sigaction(rt, &sa, 0);
    sigaction(SIGUSR1, &sa, 0);

    /* 1: queued values in order, SI_QUEUE, the sender's pid. */
    sigemptyset(&set);
    sigaddset(&set, rt);
    sigprocmask(SIG_BLOCK, &set, 0);
    for (int v = 11; v <= 33; v += 11) {
        union sigval sv = {.sival_int = v};
        if (sigqueue(getpid(), rt, sv) != 0) f |= 1;
    }
    si_n = 0;
    sigprocmask(SIG_UNBLOCK, &set, 0);
    if (si_n != 3 || si_values[0] != 11 || si_values[1] != 22 || si_values[2] != 33) f |= 2;
    if (si_code_seen != SI_QUEUE || si_pid_seen != getpid()) f |= 4;

    /* 2: kill is SI_USER, raise (tkill) SI_TKILL. */
    si_code_seen = 99;
    kill(getpid(), SIGUSR1);
    if (si_code_seen != SI_USER || si_pid_seen != getpid()) f |= 8;
    si_code_seen = 99;
    raise(SIGUSR1);
    if (si_code_seen != SI_TKILL) f |= 0x10;

    /* 3: sigtimedwait takes a pending one with its value; nothing pending
     * and a zero or short timeout is EAGAIN. */
    sigemptyset(&set);
    sigaddset(&set, SIGUSR2);
    sigprocmask(SIG_BLOCK, &set, 0);
    union sigval sv77 = {.sival_int = 77};
    sigqueue(getpid(), SIGUSR2, sv77);
    siginfo_t si;
    struct timespec zero = {0, 0}, short_wait = {0, 50 * 1000 * 1000};
    memset(&si, 0, sizeof(si));
    if (sigtimedwait(&set, &si, &zero) != SIGUSR2 || si.si_value.sival_int != 77 ||
        si.si_code != SI_QUEUE)
        f |= 0x20;
    errno = 0;
    if (sigtimedwait(&set, &si, &zero) != -1 || errno != EAGAIN) f |= 0x40;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    errno = 0;
    int r = sigtimedwait(&set, &si, &short_wait);
    clock_gettime(CLOCK_MONOTONIC, &t1);
    long ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
    if (r != -1 || errno != EAGAIN || ms < 40) f |= 0x80;
    sigprocmask(SIG_UNBLOCK, &set, 0);

    /* 4: sigwaitinfo wakes for a signal another process queues. */
    int rq = SIGRTMIN + 5;
    sigemptyset(&set);
    sigaddset(&set, rq);
    sigprocmask(SIG_BLOCK, &set, 0);
    pid_t parent = getpid(), q = fork();
    if (q == 0)
        _exit(si_queue_child_body(parent));
    memset(&si, 0, sizeof(si));
    int got_rq;
    do {
        got_rq = sigwaitinfo(&set, &si);
    } while (got_rq == -1 && errno == EINTR);
    if (got_rq != rq || si.si_value.sival_int != 4242 || si.si_pid != q) f |= 0x100;
    int qst = 0;
    waitpid(q, &qst, 0);
    sigprocmask(SIG_UNBLOCK, &set, 0);

    /* 5: SIGCHLD says CLD_EXITED, the status and the child. */
    sigaction(SIGCHLD, &sa, 0);
    si_code_seen = 0;
    pid_t c = fork();
    if (c == 0)
        _exit(si_child_exit7());
    int cst = 0;
    waitpid(c, &cst, 0);
    for (int i = 0; i < 100 && si_code_seen == 0; i++) {
        struct timespec t = {0, 10 * 1000 * 1000};
        nanosleep(&t, 0);
    }
    if (si_code_seen != CLD_EXITED || si_status_seen != 7 || si_pid_seen != c) f |= 0x200;
    signal(SIGCHLD, SIG_DFL);

    /* 6: a SIGSEGV handler sees SEGV_ACCERR and the address. */
    struct sigaction sv;
    memset(&sv, 0, sizeof(sv));
    sv.sa_sigaction = on_segv_info;
    sv.sa_flags = SA_SIGINFO | SA_NODEFER;
    sigaction(SIGSEGV, &sv, 0);
    char *ro = mmap(0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    si_fault_code = 0;
    if (ro != MAP_FAILED && sigsetjmp(si_jump, 1) == 0)
        *(volatile char *)(ro + 8) = 1;
    if (si_fault_code != SEGV_ACCERR || si_fault_addr != (uintptr_t)(ro + 8)) f |= 0x400;
    munmap(ro, 4096);
    signal(SIGSEGV, SIG_DFL);

    /* 7: an SA_RESTART handler does not restart sigsuspend: it returns
     * EINTR after the first signal, before the child's second. */
    struct sigaction rs;
    memset(&rs, 0, sizeof(rs));
    rs.sa_sigaction = on_info;
    rs.sa_flags = SA_SIGINFO | SA_RESTART;
    sigaction(SIGUSR1, &rs, 0);
    signal(SIGUSR2, on_usr2_count);
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    sigaddset(&set, SIGUSR2);
    sigprocmask(SIG_BLOCK, &set, 0);
    si_usr2_seen = 0;
    parent = getpid();
    pid_t rc = fork();
    if (rc == 0)
        _exit(si_restart_child_body(parent));
    errno = 0;
    int sr = sigsuspend(&empty);
    int restart_ok = sr == -1 && errno == EINTR && si_usr2_seen == 0;
    sigprocmask(SIG_UNBLOCK, &set, 0);
    int rcs = 0;
    while (waitpid(rc, &rcs, 0) == -1 && errno == EINTR) {
    }
    if (!restart_ok) f |= 0x800;
    signal(SIGUSR1, SIG_DFL);
    signal(SIGUSR2, SIG_DFL);

    /* 8: rt_sigqueueinfo may not pose as kill (si_code >= 0) for another
     * process, but may for itself. */
    pid_t holder = fork();
    if (holder == 0) {
        pause();
        _exit(0);
    }
    siginfo_t forged;
    memset(&forged, 0, sizeof(forged));
    forged.si_code = SI_USER;
    errno = 0;
    if (syscall(SYS_rt_sigqueueinfo, holder, SIGUSR1, &forged) != -1 || errno != EPERM) f |= 0x1000;
    signal(SIGUSR1, SIG_IGN);
    if (syscall(SYS_rt_sigqueueinfo, getpid(), SIGUSR1, &forged) != 0) f |= 0x1000;
    signal(SIGUSR1, SIG_DFL);
    kill(holder, SIGKILL);
    waitpid(holder, 0, 0);

    /* 9: past RLIMIT_SIGPENDING sigqueue is EAGAIN; kill still pends. */
    int rl = SIGRTMIN + 7;
    struct rlimit old_rl, two = {2, 2};
    getrlimit(RLIMIT_SIGPENDING, &old_rl);
    pid_t lim = fork();
    if (lim == 0) {
        int g = 0;
        sigset_t b;
        sigemptyset(&b);
        sigaddset(&b, rl);
        sigprocmask(SIG_BLOCK, &b, 0);
        if (setrlimit(RLIMIT_SIGPENDING, &two) != 0) g |= 1;
        union sigval z = {0};
        if (sigqueue(getpid(), rl, z) != 0 || sigqueue(getpid(), rl, z) != 0) g |= 2;
        errno = 0;
        if (sigqueue(getpid(), rl, z) != -1 || errno != EAGAIN) g |= 4;
        if (kill(getpid(), rl) != 0) g |= 8;
        _exit(g);
    }
    int lst = 0;
    waitpid(lim, &lst, 0);
    if (!exited_zero(lst)) f |= 0x2000;

    static char why[160];
    snprintf(why, sizeof(why), "fail=%#x n=%d code=%d q=%#x l=%#x segv=%d@%#lx ro=%#lx", f, si_n,
             si_code_seen, qst, lst, si_fault_code, (unsigned long)si_fault_addr,
             (unsigned long)(uintptr_t)ro);
    report("musl_siginfo", f == 0, why);
    signal(rt, SIG_DFL);
}

/* Ptys (the private OpenPty call, 1304: musl's openpty needs /dev/ptmx,
 * which is v0.29). An empty side reads as EAGAIN when non-blocking, not as
 * end of file (it used to return 0); poll reports what the buffers hold;
 * a blocking reader sleeps until the other side writes and gets the data. */
#define SYS_VERIDIAN_OPENPTY 1304

static void test_pty(void)
{
    int m = -1, sl = -1, f = 0;
    if (syscall(SYS_VERIDIAN_OPENPTY, &m, &sl) != 0 || m < 0 || sl < 0) {
        report("musl_pty_readiness_and_blocking", 0, "openpty failed");
        return;
    }
    struct pollfd p = {.fd = m, .events = POLLIN | POLLOUT};
    if (poll(&p, 1, 0) != 1 || (p.revents & POLLIN) || !(p.revents & POLLOUT)) f |= 1;
    fcntl(m, F_SETFL, fcntl(m, F_GETFL) | O_NONBLOCK);
    char buf[16];
    errno = 0;
    if (read(m, buf, sizeof(buf)) != -1 || errno != EAGAIN) f |= 2;
    if (write(sl, "hi", 2) != 2) f |= 4;
    p.revents = 0;
    if (poll(&p, 1, 0) != 1 || !(p.revents & POLLIN)) f |= 8;
    if (read(m, buf, sizeof(buf)) != 2 || memcmp(buf, "hi", 2) != 0) f |= 16;

    /* A child blocks reading the slave; the parent types after 100 ms. */
    pid_t c = fork();
    if (c == 0) {
        char in[8];
        ssize_t n = read(sl, in, sizeof(in));
        _exit(n == 3 && memcmp(in, "ok\n", 3) == 0 ? 0 : 1);
    }
    struct timespec t = {0, 100 * 1000 * 1000};
    nanosleep(&t, 0);
    if (write(m, "ok\n", 3) != 3) f |= 32;
    int st = 0;
    if (waitpid(c, &st, 0) != c || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 64;
    close(m);
    close(sl);
    static char why[48];
    snprintf(why, sizeof(why), "fail=%#x st=%#x", f, st);
    report("musl_pty_readiness_and_blocking", f == 0, why);
}

/* The console (N-118): with no input waiting it is not readable, a
 * non-blocking read is EAGAIN, and a blocking reader sleeps -- the parent
 * keeps running -- until a signal ends the read with EINTR. It used to spin
 * on the serial port with the CPU held, and poll saw it as always ready. */
static void console_sig(int s) { (void)s; }

static void test_console(void)
{
    int f = 0, st = 0;
    int fd = open("/dev/console", O_RDONLY | O_NONBLOCK);
    if (fd < 0) {
        report("musl_console_blocks_and_polls", 0, "open /dev/console failed");
        return;
    }
    struct pollfd p = {.fd = fd, .events = POLLIN};
    if (poll(&p, 1, 0) != 0) f |= 1;
    char buf[8];
    errno = 0;
    if (read(fd, buf, sizeof(buf)) != -1 || errno != EAGAIN) f |= 2;
    close(fd);

    pid_t c = fork();
    if (c == 0) {
        struct sigaction sa;
        memset(&sa, 0, sizeof(sa));
        sa.sa_handler = console_sig; /* no SA_RESTART */
        sigaction(SIGUSR1, &sa, 0);
        int in = open("/dev/console", O_RDONLY);
        errno = 0;
        ssize_t n = read(in, buf, sizeof(buf));
        _exit(n == -1 && errno == EINTR ? 0 : 1);
    }
    struct timespec t = {0, 100 * 1000 * 1000};
    nanosleep(&t, 0);
    if (kill(c, SIGUSR1) != 0) f |= 4;
    if (waitpid(c, &st, 0) != c || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 8;
    static char why[48];
    snprintf(why, sizeof(why), "fail=%#x st=%#x", f, st);
    report("musl_console_blocks_and_polls", f == 0, why);
}

/* The native thread join (1066, N-269) sleeps until the thread has exited
 * and is interruptible: joining a thread blocked on a pipe ends with EINTR
 * when a signal arrives (it spun on yield and never returned); once the
 * thread exits, the join succeeds and the thread is gone. */
#define SYS_VERIDIAN_THREAD_JOIN 1066

static int join_pipe[2];
static volatile pid_t join_tid;

static void *join_worker(void *arg)
{
    (void)arg;
    sigset_t s;
    sigemptyset(&s);
    sigaddset(&s, SIGUSR2);
    pthread_sigmask(SIG_BLOCK, &s, 0); /* the signal goes to the joiner */
    join_tid = (pid_t)syscall(SYS_gettid);
    char c;
    while (read(join_pipe[0], &c, 1) < 0 && errno == EINTR) {
    }
    return 0;
}

static void join_sig(int s) { (void)s; }

static void test_thread_join(void)
{
    int f = 0;
    long r1 = 0, r2 = 0, r3 = 0;
    int e1 = 0, e3 = 0;
    if (pipe(join_pipe) != 0) {
        report("native_thread_join_sleeps", 0, "pipe failed");
        return;
    }
    pthread_t t;
    if (pthread_create(&t, NULL, join_worker, NULL) != 0) {
        report("native_thread_join_sleeps", 0, "pthread_create failed");
        return;
    }
    while (join_tid == 0) sched_yield();
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = join_sig; /* no SA_RESTART */
    sigaction(SIGUSR2, &sa, 0);
    /* No interval timers yet (X2): a child sends the signal in 100 ms. */
    pid_t parent = getpid();
    pid_t c = fork();
    if (c == 0) {
        struct timespec d = {0, 100 * 1000 * 1000};
        nanosleep(&d, 0);
        kill(parent, SIGUSR2);
        _exit(0);
    }
    unsigned long ret = 1;
    errno = 0;
    r1 = syscall(SYS_VERIDIAN_THREAD_JOIN, (long)join_tid, &ret);
    e1 = errno;
    if (r1 != -1 || e1 != EINTR) f |= 1;
    if (write(join_pipe[1], "x", 1) != 1) f |= 2;
    r2 = syscall(SYS_VERIDIAN_THREAD_JOIN, (long)join_tid, &ret);
    if (r2 != 0 || ret != 0) f |= 4;
    /* Gone (joined, or reaped on its own as on Linux): a further join finds
     * nothing to wait for and reports it finished. */
    ret = 1;
    errno = 0;
    r3 = syscall(SYS_VERIDIAN_THREAD_JOIN, (long)join_tid, &ret);
    e3 = errno;
    if (r3 != 0 || ret != 0) f |= 8;
    pthread_join(t, 0);   /* musl's side: the tid word is already clear */
    waitpid(c, 0, 0);
    signal(SIGUSR2, SIG_DFL);
    close(join_pipe[0]);
    close(join_pipe[1]);
    static char why[80];
    snprintf(why, sizeof(why), "fail=%#x r=%ld/%d %ld %ld/%d", f, r1, e1, r2, r3, e3);
    report("native_thread_join_sleeps", f == 0, why);
}

/* Native IPC (1024-1028): an endpoint created through its capability is
 * the one send and receive reach (N-47, N-81: they used to reach none), and
 * a receive sleeps until a message arrives (N-84: it returned an empty
 * message at once); a signal ends the wait with EINTR. */
#define SYS_VERIDIAN_IPC_SEND 1024
#define SYS_VERIDIAN_IPC_RECEIVE 1025
#define SYS_VERIDIAN_IPC_CREATE_ENDPOINT 1028

struct veridian_small_msg {
    uint64_t capability;
    uint32_t opcode;
    uint32_t flags;
    uint64_t data[4];
};

static void ipc_sig(int s) { (void)s; }

static long ipc_send_op(long cap, uint32_t opcode)
{
    struct veridian_small_msg m = {0};
    m.opcode = opcode;
    m.data[0] = 0x1234;
    return syscall(SYS_VERIDIAN_IPC_SEND, cap, &m, sizeof(m), 0);
}

static void test_ipc(void)
{
    int f = 0, st = 0;
    /* The token is returned whole; its flag bits (60-63) make it negative
     * as a long, so only -1 with errno is a failure. */
    errno = 0;
    long cap = syscall(SYS_VERIDIAN_IPC_CREATE_ENDPOINT, 0);
    if (cap == -1) {
        static char cwhy[48];
        snprintf(cwhy, sizeof(cwhy), "create endpoint failed: errno %d", errno);
        report("native_ipc_send_receive_blocks", 0, cwhy);
        return;
    }
    struct veridian_small_msg in;
    /* Round trip within one process. */
    if (ipc_send_op(cap, 7) != 0) f |= 1;
    memset(&in, 0, sizeof(in));
    if (syscall(SYS_VERIDIAN_IPC_RECEIVE, cap, &in, sizeof(in)) < 0 || in.opcode != 7 ||
        in.data[0] != 0x1234)
        f |= 2;

    /* A receive sleeps until the forked child (same capabilities) sends. */
    pid_t c = fork();
    if (c == 0) {
        struct timespec d = {0, 100 * 1000 * 1000};
        nanosleep(&d, 0);
        _exit(ipc_send_op(cap, 9) == 0 ? 0 : 1);
    }
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    memset(&in, 0, sizeof(in));
    long r = syscall(SYS_VERIDIAN_IPC_RECEIVE, cap, &in, sizeof(in));
    clock_gettime(CLOCK_MONOTONIC, &t1);
    long waited_ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
    if (r < 0 || in.opcode != 9) f |= 4;
    if (waited_ms < 50) f |= 8; /* it did wait for the child */
    if (waitpid(c, &st, 0) != c || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 16;

    /* With nothing to receive, a signal ends the wait. */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = ipc_sig; /* no SA_RESTART */
    sigaction(SIGUSR2, &sa, 0);
    pid_t parent = getpid();
    c = fork();
    if (c == 0) {
        struct timespec d = {0, 100 * 1000 * 1000};
        nanosleep(&d, 0);
        kill(parent, SIGUSR2);
        _exit(0);
    }
    errno = 0;
    long r2 = syscall(SYS_VERIDIAN_IPC_RECEIVE, cap, &in, sizeof(in));
    int e2 = errno;
    if (r2 != -1 || e2 != EINTR) f |= 32;
    waitpid(c, 0, 0);
    signal(SIGUSR2, SIG_DFL);
    static char why[80];
    snprintf(why, sizeof(why), "fail=%#x r=%ld waited=%ldms r2=%ld/%d", f, r, waited_ms, r2, e2);
    report("native_ipc_send_receive_blocks", f == 0, why);
}

/* Priority inheritance (SCHED-INC-01, FUTEX_LOCK_PI/UNLOCK_PI): the
 * textbook inversion, everything on CPU 0. A fair thread L holds a PI mutex
 * and needs 50 ms of CPU to release it; a FIFO-20 thread M hogs the CPU for
 * 600 ms; a FIFO-40 thread H waits for the mutex for at most 400 ms. With
 * inheritance L runs at 40, above M, and H gets the mutex; without it M
 * starves L and H times out. (musl used to report PI mutexes unsupported.) */
static pthread_mutex_t pi_m;
static volatile int pi_locked, pi_go_m, pi_stop_m;
static volatile int pi_h_rc = -1;
static volatile long pi_h_ms = -1;

static long pi_now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static void pi_spin_ms(long ms)
{
    long end = pi_now_ms() + ms;
    while (pi_now_ms() < end) {
    }
}

static void *pi_low(void *a)
{
    (void)a;
    pthread_mutex_lock(&pi_m);
    pi_locked = 1;
    while (!pi_go_m) sched_yield(); /* hold it until M is hogging */
    pi_spin_ms(50);
    pthread_mutex_unlock(&pi_m);
    return 0;
}

static void *pi_medium(void *a)
{
    (void)a;
    long end = pi_now_ms() + 600;
    while (pi_now_ms() < end && !pi_stop_m) {
    }
    return 0;
}

static void *pi_high(void *a)
{
    (void)a;
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    t.tv_nsec += 400 * 1000000L;
    if (t.tv_nsec >= 1000000000L) {
        t.tv_sec++;
        t.tv_nsec -= 1000000000L;
    }
    long start = pi_now_ms();
    pi_h_rc = pthread_mutex_timedlock(&pi_m, &t);
    pi_h_ms = pi_now_ms() - start;
    if (pi_h_rc == 0) pthread_mutex_unlock(&pi_m);
    pi_stop_m = 1;
    return 0;
}

static int pi_spawn(pthread_t *t, void *(*fn)(void *), int policy, int prio)
{
    pthread_attr_t a;
    pthread_attr_init(&a);
    if (policy != SCHED_OTHER) {
        struct sched_param sp = {.sched_priority = prio};
        pthread_attr_setinheritsched(&a, PTHREAD_EXPLICIT_SCHED);
        pthread_attr_setschedpolicy(&a, policy);
        pthread_attr_setschedparam(&a, &sp);
    }
    int r = pthread_create(t, &a, fn, 0);
    pthread_attr_destroy(&a);
    return r;
}

static void test_priority_inheritance(void)
{
    int f = 0, proto_rc, sp_rc;
    cpu_set_t one;
    CPU_ZERO(&one);
    CPU_SET(0, &one);
    sched_setaffinity(0, sizeof(one), &one); /* inherited by the threads */
    pthread_mutexattr_t ma;
    pthread_mutexattr_init(&ma);
    proto_rc = pthread_mutexattr_setprotocol(&ma, PTHREAD_PRIO_INHERIT);
    if (proto_rc != 0) f |= 1;
    pthread_mutex_init(&pi_m, &ma);
    pthread_mutexattr_destroy(&ma);
    /* The orchestrating thread outranks all three. */
    struct sched_param mp = {.sched_priority = 50};
    sp_rc = pthread_setschedparam(pthread_self(), SCHED_FIFO, &mp);
    if (sp_rc != 0) f |= 2;
    pthread_t l, m, h;
    if (pi_spawn(&l, pi_low, SCHED_OTHER, 0) != 0) f |= 4;
    while (!pi_locked) {
        struct timespec d = {0, 1000000};
        nanosleep(&d, 0);
    }
    if (pi_spawn(&m, pi_medium, SCHED_FIFO, 20) != 0) f |= 4;
    pi_go_m = 1;
    struct timespec d = {0, 20 * 1000000};
    nanosleep(&d, 0); /* M now hogs the CPU; L, at fair priority, waits */
    if (pi_spawn(&h, pi_high, SCHED_FIFO, 40) != 0) f |= 4;
    pthread_join(h, 0);
    pthread_join(m, 0);
    pthread_join(l, 0);
    struct sched_param np = {.sched_priority = 0};
    pthread_setschedparam(pthread_self(), SCHED_OTHER, &np);
    if (pi_h_rc != 0) f |= 8;
    pthread_mutex_destroy(&pi_m);
    CPU_ZERO(&one);
    for (int i = 0; i < 64; i++) CPU_SET(i, &one);
    sched_setaffinity(0, sizeof(one), &one);
    static char why[96];
    snprintf(why, sizeof(why), "fail=%#x proto=%d setsched=%d h_rc=%d h_waited=%ldms", f,
             proto_rc, sp_rc, pi_h_rc, pi_h_ms);
    report("musl_priority_inheritance", f == 0, why);
}

/* munmap, mprotect, brk, mremap, madvise, getrandom and futex bitsets
 * (N-240, N-242, N-243, N-244, N-246, N-226), as Linux. */

#define PG 4096

static int touch_faults(volatile char *p, int write)
{
    pid_t pid = fork();
    if (pid == 0) {
        if (write)
            *p = 1;
        else
            (void)*p;
        _exit(0);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    return WIFSIGNALED(st) && WTERMSIG(st) == SIGSEGV;
}

#include "user_layout.h"

/* User addresses no mapping uses: 80 and 96 TiB, or under Sv39 192 and
 * 208 GiB, between the PIE and the thread stacks (ADR 0018). */
static unsigned long far_unmapped(int which)
{
    if (user_space_end() > (1UL << 40))
        return which ? 0x600000000000UL : 0x500000000000UL;
    return which ? 0x3400000000UL : 0x3000000000UL;
}

static int mm_args(void)
{
    int f = 0;
    char *p = mmap(NULL, 3 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return 1;
    /* The far addresses below come from the limit, which must be the
     * paging mode's. */
    if (!user_end_matches_mode()) f |= 256;
    /* munmap: address 0 and an empty range are fine, length 0 is not. */
    if (munmap(NULL, PG) != 0) f |= 2;
    errno = 0;
    if (munmap(p, 0) != -1 || errno != EINVAL) f |= 2;
    errno = 0;
    if (munmap(p + 1, PG) != -1 || errno != EINVAL) f |= 2;
    errno = 0;
    if (munmap((void *)0x7ffffffff000UL, 2 * PG) != -1 || errno != EINVAL) f |= 2;
    /* Lengths past 256 MiB are fine. */
    if (munmap((void *)far_unmapped(1), 512UL << 20) != 0) f |= 4;
    /* MAP_SHARED_VALIDATE checks its flags; MAP_FIXED_NOREPLACE does not
     * replace (N-241). */
    char *v = mmap(NULL, PG, PROT_READ, MAP_SHARED_VALIDATE | MAP_ANONYMOUS, -1, 0);
    if (v == MAP_FAILED) f |= 8;
    errno = 0;
    if (mmap(NULL, PG, PROT_READ, MAP_SHARED_VALIDATE | MAP_ANONYMOUS | 0x200, -1, 0) != MAP_FAILED
        || errno != EOPNOTSUPP)
        f |= 8;
    errno = 0;
    if (mmap(p, PG, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0) != MAP_FAILED
        || errno != EEXIST)
        f |= 16;
    munmap(p + 2 * PG, PG);
    if (mmap(p + 2 * PG, PG, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0)
        != p + 2 * PG)
        f |= 16;
    return f;
}

static int mm_protect(void)
{
    int f = 0;
    char *p = mmap(NULL, 3 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return 1;
    /* mprotect: zero length succeeds; unmapped or outside is ENOMEM. */
    if (mprotect(p, 0, PROT_READ) != 0) f |= 8;
    errno = 0;
    if (mprotect(NULL, PG, PROT_READ) != -1 || errno != ENOMEM) f |= 8;
    errno = 0;
    if (mprotect((void *)0xffff800000000000UL, PG, PROT_READ) != -1 || errno != ENOMEM) f |= 8;
    /* A partial mprotect protects exactly its pages. */
    if (mprotect(p + PG, PG, PROT_READ) != 0) f |= 16;
    p[0] = 1;
    p[2 * PG] = 1;
    if (!touch_faults(p + PG, 1) || touch_faults(p + PG, 0)) f |= 16;
    return f;
}

static int mm_brk(void)
{
    int f = 0;
    /* brk goes down as well as up; the pages above it go. (The raw call:
     * musl's sbrk refuses any increment by itself.) */
    char *b0 = (char *)syscall(SYS_brk, 0);
    char *top = b0 + 16 * PG;
    if ((char *)syscall(SYS_brk, top) != top) return f | 32;
    b0[15 * PG] = 1;
    if ((char *)syscall(SYS_brk, b0) != b0) f |= 64;
    if (!touch_faults(b0 + 15 * PG, 0)) {
        f |= 128;
        /* Diagnose: the mappings around the break. */
        printf("mm_brk: break %p, page %p still mapped; maps:\n", (void *)b0, (void *)(b0 + 15 * PG));
        FILE *maps = fopen("/proc/self/maps", "r");
        char line[160];
        while (maps && fgets(line, sizeof(line), maps))
            if (strncmp(line, "2000", 4) == 0 || strstr(line, "heap"))
                printf("  %s", line);
        if (maps)
            fclose(maps);
        fflush(stdout);
    }
    return f;
}

static int mm_mremap(void)
{
    int f = 0;
    char *p = mmap(NULL, 4 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return 1;
    memset(p, 'a', 4 * PG);
    /* Shrink in place; grow in place into the freed pages, zeroed. */
    if (mremap(p, 4 * PG, 2 * PG, 0) != p) f |= 2;
    if (!touch_faults(p + 3 * PG, 0)) f |= 2;
    if (mremap(p, 2 * PG, 4 * PG, 0) != p || p[PG] != 'a' || p[3 * PG] != 0) f |= 4;
    /* No room after, no MAYMOVE: ENOMEM. With it: moved, contents kept. */
    char *q = mmap(NULL, 3 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    memset(q, 'q', 3 * PG);
    errno = 0;
    if (mremap(q, PG, 2 * PG, 0) != MAP_FAILED || errno != ENOMEM) f |= 8;
    char *m = mremap(q, PG, 8 * PG, MREMAP_MAYMOVE);
    if (m == MAP_FAILED || m == q || m[0] != 'q' || m[7 * PG] != 0 || q[PG] != 'q') f |= 16;
    if (m != MAP_FAILED && !touch_faults(q, 0)) f |= 16;
    /* FIXED to a chosen place, replacing what is there. */
    char *t = mmap(NULL, 2 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (m != MAP_FAILED && mremap(m, PG, PG, MREMAP_MAYMOVE | MREMAP_FIXED, t) != t) f |= 32;
    if (t[0] != 'q') f |= 32;
    /* DONTUNMAP: moved, and the old place reads as zero. */
    char *d = mremap(p, PG, PG, MREMAP_MAYMOVE | MREMAP_DONTUNMAP);
    if (d == MAP_FAILED || d[0] != 'a' || p[0] != 0) f |= 64;
    /* Invalid requests. */
    errno = 0;
    if (mremap(p, PG, PG, MREMAP_FIXED, t) != MAP_FAILED || errno != EINVAL) f |= 128;
    errno = 0;
    if (mremap(p, 0, PG, MREMAP_MAYMOVE) != MAP_FAILED || errno != EINVAL) f |= 128;
    errno = 0;
    if (mremap((void *)far_unmapped(0), PG, 2 * PG, MREMAP_MAYMOVE) != MAP_FAILED || errno != EFAULT)
        f |= 128;
    /* Old length 0 on a shared mapping: the same pages, twice. */
    char *s = mmap(NULL, PG, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    char *s2 = mremap(s, 0, PG, MREMAP_MAYMOVE);
    if (s2 == MAP_FAILED || s2 == s) f |= 256;
    else {
        s[10] = 'S';
        if (s2[10] != 'S') f |= 256;
    }
    /* A file mapping grows with the file's next pages. */
    int fd = open("/tmp/musl_mremap", O_CREAT | O_RDWR | O_TRUNC, 0644);
    char page[PG];
    for (int i = 0; i < 3; i++) {
        memset(page, '0' + i, PG);
        (void)!write(fd, page, PG);
    }
    char *fm = mmap(NULL, PG, PROT_READ, MAP_PRIVATE, fd, 0);
    char *fg = fm == MAP_FAILED ? MAP_FAILED : mremap(fm, PG, 3 * PG, MREMAP_MAYMOVE);
    if (fg == MAP_FAILED || fg[0] != '0' || fg[PG] != '1' || fg[2 * PG] != '2') f |= 512;
    close(fd);
    unlink("/tmp/musl_mremap");
    /* realloc of a large block keeps its contents (musl uses mremap). */
    char *big = malloc(1 << 20);
    memset(big, 'r', 1 << 20);
    big = realloc(big, 4 << 20);
    if (!big || big[(1 << 20) - 1] != 'r') f |= 1024;
    free(big);
    return f;
}

static int mm_madvise(void)
{
    int f = 0;
    char *p = mmap(NULL, 2 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return 1;
    memset(p, 'x', 2 * PG);
    /* DONTNEED: private anonymous memory reads as zero again. */
    if (madvise(p, PG, MADV_DONTNEED) != 0 || p[0] != 0 || p[PG] != 'x') f |= 2;
    /* ... a private file mapping as the file. */
    int fd = open("/tmp/musl_madvise", O_CREAT | O_RDWR | O_TRUNC, 0644);
    memset(p, 'F', PG);
    (void)!write(fd, p, PG);
    char *fm = mmap(NULL, PG, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    if (fm == MAP_FAILED) f |= 4;
    else {
        fm[0] = 'w';
        if (madvise(fm, PG, MADV_DONTNEED) != 0 || fm[0] != 'F') f |= 4;
        errno = 0;
        if (madvise(fm, PG, MADV_FREE) != -1 || errno != EINVAL) f |= 8;
    }
    close(fd);
    unlink("/tmp/musl_madvise");
    /* REMOVE: shared only; zeroes it. */
    errno = 0;
    if (madvise(p, PG, MADV_REMOVE) != -1 || errno != EINVAL) f |= 16;
    char *s = mmap(NULL, PG, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    s[0] = 's';
    if (madvise(s, PG, MADV_REMOVE) != 0 || s[0] != 0) f |= 16;
    /* ... and not on what may not be written: a read-only file. */
    int ro = open("/bin/musl_runtime_test", O_RDONLY);
    char *rom = ro < 0 ? MAP_FAILED : mmap(NULL, PG, PROT_READ, MAP_SHARED, ro, 0);
    errno = 0;
    if (rom == MAP_FAILED || madvise(rom, PG, MADV_REMOVE) != -1 || errno != EACCES) f |= 16;
    if (ro >= 0) close(ro);
    /* Shared memory keeps its contents through DONTNEED. */
    s[0] = 's';
    if (madvise(s, PG, MADV_DONTNEED) != 0 || s[0] != 's') f |= 16;
    /* WIPEONFORK: the child sees zero, the parent keeps its data. */
    p[PG] = 'w';
    if (madvise(p + PG, PG, MADV_WIPEONFORK) != 0) f |= 32;
    pid_t pid = fork();
    if (pid == 0)
        _exit(p[PG] == 0 ? 0 : 1);
    int st = 0;
    waitpid(pid, &st, 0);
    if (!WIFEXITED(st) || WEXITSTATUS(st) != 0 || p[PG] != 'w') f |= 32;
    errno = 0;
    if (madvise(s, PG, MADV_WIPEONFORK) != -1 || errno != EINVAL) f |= 32;
    /* DONTFORK: the child does not have it. */
    if (madvise(s, PG, MADV_DONTFORK) != 0 || !touch_faults(s, 0)) f |= 64;
    /* Errors: unknown advice, an unaligned start, an unmapped range;
     * a zero length and hints succeed. */
    errno = 0;
    if (madvise(p, PG, 999) != -1 || errno != EINVAL) f |= 128;
    errno = 0;
    if (madvise(p + 1, PG, MADV_NORMAL) != -1 || errno != EINVAL) f |= 128;
    errno = 0;
    if (madvise((void *)far_unmapped(0), PG, MADV_WILLNEED) != -1 || errno != ENOMEM) f |= 128;
    if (madvise(p, 0, MADV_DONTNEED) != 0 || madvise(p, 2 * PG, MADV_HUGEPAGE) != 0) f |= 128;
    return f;
}

static volatile int bitset_waiting;
static int bitset_word;
static volatile long bitset_result = 99, bitset_errno;

static void *bitset_waiter(void *arg)
{
    (void)arg;
    /* An absolute deadline, so a missed wake fails the test, not hangs it. */
    struct timespec until;
    clock_gettime(CLOCK_MONOTONIC, &until);
    until.tv_sec += 5;
    bitset_waiting = 1;
    bitset_result = syscall(SYS_futex, &bitset_word, FUTEX_WAIT_BITSET | FUTEX_PRIVATE_FLAG, 0,
                            &until, NULL, 2);
    bitset_errno = errno;
    return NULL;
}

static int mm_random_futex(void)
{
    int f = 0;
    /* getrandom: whole requests, flags checked. */
    static unsigned char buf[1 << 20];
    if (getrandom(buf, sizeof(buf), 0) != (ssize_t)sizeof(buf)) f |= 2;
    if (getrandom(buf, 16, GRND_NONBLOCK) != 16) f |= 2;
    errno = 0;
    if (getrandom(buf, 16, 0x8) != -1 || errno != EINVAL) f |= 4;
    errno = 0;
    if (getrandom(buf, 16, GRND_RANDOM | 4) != -1 || errno != EINVAL) f |= 4;
    /* futex: WAKE_BITSET wakes only waiters sharing a bit; the requeue-PI
     * commands (glibc PI condition variables) and unknown commands are
     * ENOSYS. (LOCK_PI itself works since SCHED-INC-01.) */
    pthread_t t;
    pthread_create(&t, NULL, bitset_waiter, NULL);
    while (!bitset_waiting)
        sched_yield();
    struct timespec ms = {0, 20 * 1000 * 1000};
    nanosleep(&ms, NULL);
    if (syscall(SYS_futex, &bitset_word, FUTEX_WAKE_BITSET, 1, NULL, NULL, 1) != 0) f |= 8;
    long woken = 0;
    for (int i = 0; i < 100 && woken == 0; i++) {
        woken = syscall(SYS_futex, &bitset_word, FUTEX_WAKE_BITSET, 1, NULL, NULL, 2);
        if (woken == 0)
            nanosleep(&ms, NULL);
    }
    pthread_join(t, NULL);
    if (woken != 1 || bitset_result != 0) {
        f |= 8;
        printf("mm_random_futex: woken %ld, wait returned %ld errno %ld\n", woken, bitset_result,
               bitset_errno);
        fflush(stdout);
    }
    errno = 0;
    if (syscall(SYS_futex, &bitset_word, FUTEX_WAKE_BITSET, 1, NULL, NULL, 0) != -1 || errno != EINVAL)
        f |= 16;
    errno = 0;
    if (syscall(SYS_futex, &bitset_word, FUTEX_WAIT_REQUEUE_PI, 0, NULL, &bitset_word, 0) != -1 ||
        errno != ENOSYS)
        f |= 32;
    errno = 0;
    if (syscall(SYS_futex, &bitset_word, 0x200 | FUTEX_WAKE, 1, NULL, NULL, 0) != -1 || errno != ENOSYS)
        f |= 32;
    return f;
}

static void test_memory_calls(void)
{
    int (*const parts[])(void) = {mm_args, mm_protect, mm_brk, mm_mremap, mm_madvise, mm_random_futex};
    int st[6], f = 0;
    for (int i = 0; i < 6; i++)
        if (!exited_zero(st[i] = in_child(parts[i]))) f |= 1 << i;
    static char why[128];
    snprintf(why, sizeof(why), "fail=%#x st=%#x,%#x,%#x,%#x,%#x,%#x", f, st[0], st[1], st[2], st[3],
             st[4], st[5]);
    report("musl_memory_calls", f == 0, why);
}

/* unlink, rmdir, unlinkat, fstatat and path arguments (N-195, N-196,
 * N-203), as Linux. */
static int fs_paths(void)
{
    int f = 0;
    struct stat st;
    mkdir("/tmp/fsx", 0755);
    if (chdir("/tmp/fsx") != 0) return 1;
    /* A relative path; a directory is EISDIR to unlink, a file ENOTDIR to
     * rmdir; AT_REMOVEDIR removes a directory. */
    close(open("rel", O_CREAT | O_WRONLY, 0644));
    if (unlink("rel") != 0 || access("rel", F_OK) == 0) f |= 2;
    mkdir("d", 0755);
    close(open("file", O_CREAT | O_WRONLY, 0644));
    errno = 0;
    if (unlink("d") != -1 || errno != EISDIR) f |= 4;
    errno = 0;
    if (rmdir("file") != -1 || errno != ENOTDIR) f |= 4;
    errno = 0;
    if (unlink("file/") != -1 || errno != ENOTDIR) f |= 4;
    errno = 0;
    if (unlinkat(AT_FDCWD, "d", 0x1) != -1 || errno != EINVAL) f |= 8;
    errno = 0;
    if (rmdir(".") != -1 || errno != EINVAL) f |= 8;
    errno = 0;
    if (rmdir("/") != -1 || errno != EBUSY) f |= 8;
    if (unlinkat(AT_FDCWD, "d/", AT_REMOVEDIR) != 0 || access("d", F_OK) == 0) f |= 16;
    /* A symlink is removed, not its target; stat follows it, lstat and
     * AT_SYMLINK_NOFOLLOW do not. */
    if (symlink("file", "link") != 0) f |= 32;
    if (stat("link", &st) != 0 || !S_ISREG(st.st_mode)) f |= 32;
    if (lstat("link", &st) != 0 || !S_ISLNK(st.st_mode)) f |= 32;
    if (fstatat(AT_FDCWD, "link", &st, AT_SYMLINK_NOFOLLOW) != 0 || !S_ISLNK(st.st_mode)) f |= 32;
    if (unlink("link") != 0 || access("file", F_OK) != 0) f |= 32;
    /* AT_EMPTY_PATH stats the fd itself; an empty path without it is
     * ENOENT; unknown flags EINVAL; a trailing slash needs a directory. */
    int fd = open("file", O_RDONLY);
    if (fstatat(fd, "", &st, AT_EMPTY_PATH) != 0 || !S_ISREG(st.st_mode)) f |= 64;
    errno = 0;
    if (fstatat(fd, "", &st, 0) != -1 || errno != ENOENT) f |= 64;
    errno = 0;
    if (fstatat(AT_FDCWD, "file", &st, 0x4) != -1 || errno != EINVAL) f |= 64;
    errno = 0;
    if (stat("file/", &st) != -1 || errno != ENOTDIR) f |= 64;
    close(fd);
    /* A path with no terminator within PATH_MAX is ENAMETOOLONG. */
    static char longpath[5000];
    memset(longpath, 'a', sizeof(longpath) - 1);
    errno = 0;
    if (stat(longpath, &st) != -1 || errno != ENAMETOOLONG) f |= 128;
    unlink("file");
    chdir("/");
    rmdir("/tmp/fsx");
    return f;
}

static void test_file_calls(void)
{
    int st = in_child(fs_paths);
    static char why[64];
    snprintf(why, sizeof(why), "st=%#x", st);
    report("musl_file_calls", exited_zero(st), why);
}

/* Accounts (N-131): veridian_auth sets and checks a password, the kernel
 * saves it to /etc/shadow (0600, root's), and musl's getspnam + crypt -- what
 * BusyBox's login and su use -- verify the kernel's hash. Each run sets a new
 * password: the image keeps the last run's. */
#define SYS_VERIDIAN_AUTH_NR 1352

static int acct_other_user(void)
{
    if (setresuid(4400, 4400, 4400) != 0) return 1;
    int f = 0;
    errno = 0;
    if (syscall(SYS_VERIDIAN_AUTH_NR, 0, "root", "x", NULL) != -1 || errno != EPERM) f |= 2;
    errno = 0;
    if (open("/etc/shadow", O_RDONLY) != -1 || errno != EACCES) f |= 4;
    return f;
}

static void test_accounts(void)
{
    int f = 0;
    char pw[64];
    snprintf(pw, sizeof(pw), "Run-%ld-%d", (long)time(NULL), (int)getpid());
    if (syscall(SYS_VERIDIAN_AUTH_NR, 2, "root", NULL, pw) != 0) f |= 1;
    if (syscall(SYS_VERIDIAN_AUTH_NR, 0, "root", pw, NULL) != 0) f |= 2;
    if (syscall(SYS_VERIDIAN_AUTH_NR, 0, "root", "not-it", NULL) != 1) f |= 2;
    struct stat st;
    if (stat("/etc/shadow", &st) != 0 || (st.st_mode & 07777) != 0600 || st.st_uid != 0) f |= 4;
    struct spwd *sp = getspnam("root");
    if (!sp || strncmp(sp->sp_pwdp, "$6$", 3) != 0) f |= 8;
    else {
        const char *c = crypt(pw, sp->sp_pwdp);
        if (!c || strcmp(c, sp->sp_pwdp) != 0) f |= 16;
        c = crypt("not-it", sp->sp_pwdp);
        if (c && strcmp(c, sp->sp_pwdp) == 0) f |= 16;
    }
    struct passwd *pwent = getpwnam("root");
    if (!pwent || pwent->pw_uid != 0) f |= 32;
    int st2 = in_child(acct_other_user);
    if (!exited_zero(st2)) f |= 64;
    static char why[64];
    snprintf(why, sizeof(why), "fail=%#x st=%#x", f, st2);
    report("musl_accounts", f == 0, why);
}

/* setfsuid/setfsgid: the filesystem IDs are what file access is checked
 * as and new files belong to, apart from the effective IDs; each call
 * returns the old value, -1 only reads it, and an unprivileged process can
 * only pick one of its own IDs. /proc/self/status shows them fourth. */
static int fs_ids_root(void)
{
    int f = 0;
    const char *dir = "/tmp/musl_fsuid", *mine = "/tmp/musl_fsuid/mine",
               *secret = "/tmp/musl_fsuid/secret";
    unlink(mine);
    unlink(secret);
    rmdir(dir);
    if (mkdir(dir, 0777) != 0 || chmod(dir, 0777) != 0) return 1;
    int s = open(secret, O_CREAT | O_WRONLY, 0600);
    if (s < 0) return 2;
    close(s);
    if (setfsuid(1000) != 0 || setfsuid(-1) != 1000 || geteuid() != 0) f |= 4;
    if (setfsgid(1001) != 0 || setfsgid(-1) != 1001 || getegid() != 0) f |= 4;
    int fd = open(mine, O_CREAT | O_WRONLY, 0600);
    struct stat st;
    if (fd < 0 || fstat(fd, &st) != 0 || st.st_uid != 1000 || st.st_gid != 1001) f |= 8;
    if (fd >= 0) close(fd);
    errno = 0;
    if (open(secret, O_RDONLY) != -1 || errno != EACCES) f |= 16;
    char buf[1024] = {0};
    int p = open("/proc/self/status", O_RDONLY);
    if (p < 0 || read(p, buf, sizeof(buf) - 1) <= 0 || !strstr(buf, "Uid:\t0\t0\t0\t1000\n")
        || !strstr(buf, "Gid:\t0\t0\t0\t1001\n"))
        f |= 32;
    if (p >= 0) close(p);
    if (setfsuid(0) != 1000 || setfsgid(0) != 1001 || open(secret, O_RDONLY) < 0) f |= 64;
    unlink(mine);
    unlink(secret);
    rmdir(dir);
    return f;
}

static int fs_ids_user(void)
{
    if (setresgid(1000, 1000, 1000) != 0 || setresuid(1000, 1000, 1000) != 0) return 1;
    if (setfsuid(0) != 1000 || setfsuid(-1) != 1000) return 2;
    if (setfsgid(0) != 1000 || setfsgid(-1) != 1000) return 4;
    return 0;
}

static void test_fs_ids(void)
{
    int a = in_child(fs_ids_root), b = in_child(fs_ids_user);
    static char why[64];
    snprintf(why, sizeof(why), "root st=%#x user st=%#x", a, b);
    report("musl_fs_ids", exited_zero(a) && exited_zero(b), why);
}

/* SO_PEERCRED (N-236): a socketpair's ends see their creator; across
 * connect, each side sees the other's process and effective IDs. */
static int pc_client(void)
{
    if (setresgid(4501, 4501, 4501) != 0 || setresuid(4500, 4500, 4500) != 0) return 1;
    int s = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    strcpy(a.sun_path, "/tmp/musl_peercred.sock");
    if (s < 0 || connect(s, (struct sockaddr *)&a, sizeof(a)) != 0) return 2;
    struct ucred c;
    socklen_t len = sizeof(c);
    if (getsockopt(s, SOL_SOCKET, SO_PEERCRED, &c, &len) != 0 || len != sizeof(c)) return 3;
    if (c.pid != getppid() || c.uid != 0 || c.gid != 0) return 4;
    char x;
    (void)!read(s, &x, 1); /* until the server is done */
    return 0;
}

static void test_peercred(void)
{
    int f = 0, sv[2];
    struct ucred c;
    socklen_t len = sizeof(c);
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) != 0) f |= 1;
    else {
        if (getsockopt(sv[0], SOL_SOCKET, SO_PEERCRED, &c, &len) != 0 || len != sizeof(c)
            || c.pid != getpid() || c.uid != geteuid() || c.gid != getegid())
            f |= 2;
        /* A short buffer gets the start of it. */
        len = 4;
        if (getsockopt(sv[1], SOL_SOCKET, SO_PEERCRED, &c, &len) != 0 || len != 4) f |= 2;
        close(sv[0]);
        close(sv[1]);
    }
    int l = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    strcpy(a.sun_path, "/tmp/musl_peercred.sock");
    unlink(a.sun_path);
    len = sizeof(c);
    if (getsockopt(l, SOL_SOCKET, SO_PEERCRED, &c, &len) != 0 || c.pid != 0 || c.uid != 65534) f |= 4;
    /* connect needs write permission on the socket file, which root's
     * umask leaves to root alone: open it to the uid-4500 client. */
    if (bind(l, (struct sockaddr *)&a, sizeof(a)) != 0 || chmod(a.sun_path, 0777) != 0
        || listen(l, 1) != 0)
        f |= 8;
    pid_t pid = fork();
    if (pid == 0)
        _exit(pc_client());
    int s = accept(l, NULL, NULL);
    len = sizeof(c);
    if (s < 0 || getsockopt(s, SOL_SOCKET, SO_PEERCRED, &c, &len) != 0 || c.pid != pid
        || c.uid != 4500 || c.gid != 4501)
        f |= 16;
    if (s >= 0) close(s);
    int st = 0;
    waitpid(pid, &st, 0);
    if (!exited_zero(st)) f |= 32;
    close(l);
    unlink(a.sun_path);
    static char why[64];
    snprintf(why, sizeof(why), "fail=%#x st=%#x", f, st);
    report("musl_peercred", f == 0, why);
}

/* A blocking accept on a Unix socket waits for a connection; a
 * non-blocking one is EAGAIN. (It returned EAGAIN at once, so a server
 * whose client had not connected yet failed.) The client connects 100 ms
 * late on purpose. */
static void test_unix_accept_blocks(void)
{
    int f = 0, st = 0;
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    strcpy(a.sun_path, "/tmp/musl_accept_wait.sock");
    unlink(a.sun_path);
    int l = socket(AF_UNIX, SOCK_STREAM, 0);
    if (l < 0 || bind(l, (struct sockaddr *)&a, sizeof(a)) != 0 || listen(l, 1) != 0) f |= 1;
    int nb = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0);
    struct sockaddr_un b = {.sun_family = AF_UNIX};
    strcpy(b.sun_path, "/tmp/musl_accept_nb.sock");
    unlink(b.sun_path);
    if (nb < 0 || bind(nb, (struct sockaddr *)&b, sizeof(b)) != 0 || listen(nb, 1) != 0) f |= 1;
    errno = 0;
    if (accept(nb, NULL, NULL) != -1 || errno != EAGAIN) f |= 2;
    pid_t c = fork();
    if (c == 0) {
        struct timespec d = {0, 100 * 1000 * 1000};
        nanosleep(&d, 0);
        int s = socket(AF_UNIX, SOCK_STREAM, 0);
        _exit(s >= 0 && connect(s, (struct sockaddr *)&a, sizeof(a)) == 0 ? 0 : 1);
    }
    int s = accept(l, NULL, NULL);
    if (s < 0) f |= 4;
    else close(s);
    waitpid(c, &st, 0);
    if (!WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 8;
    close(l);
    close(nb);
    unlink(a.sun_path);
    unlink(b.sun_path);
    static char why[48];
    snprintf(why, sizeof(why), "fail=%#x st=%#x", f, st);
    report("musl_unix_accept_blocks", f == 0, why);
}

/* The first line of a small file, without its newline, in `out`. */
static int read_line(const char *path, char *out, size_t len)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0)
        return -1;
    ssize_t n = read(fd, out, len - 1);
    close(fd);
    if (n < 0)
        return -1;
    out[n] = 0;
    char *nl = strchr(out, '\n');
    if (nl)
        *nl = 0;
    return 0;
}

/* What the system reports about itself (ADR 0021): uname gives the system
 * name, a Linux release (so programs comparing kernel versions recognise
 * it) and the VeridianOS version; /proc/sys/kernel and /proc/version say
 * the same. */
static void test_uname_identity(void)
{
    struct utsname u;
    char ostype[80] = "", osrelease[80] = "", version[80] = "", procver[200] = "";
    int ok = uname(&u) == 0;
    ok = ok && read_line("/proc/sys/kernel/ostype", ostype, sizeof(ostype)) == 0;
    ok = ok && read_line("/proc/sys/kernel/osrelease", osrelease, sizeof(osrelease)) == 0;
    ok = ok && read_line("/proc/sys/kernel/version", version, sizeof(version)) == 0;
    ok = ok && read_line("/proc/version", procver, sizeof(procver)) == 0;
    char want_ver[200];
    snprintf(want_ver, sizeof(want_ver), "%s version %s (", u.sysname, u.release);
    static char why[600];
    snprintf(why, sizeof(why),
             "sys=%s node=%s rel=%s ver=%s ostype=%s osrelease=%s version=%s proc=%s", u.sysname,
             u.nodename, u.release, u.version, ostype, osrelease, version, procver);
    report("musl_uname_identity",
           ok && strcmp(u.sysname, "VeridianOS") == 0 && strcmp(u.nodename, "veridian") == 0
               && strcmp(u.release, "7.3.0-veridian") == 0
               && strncmp(u.version, "#1 SMP VeridianOS ", 18) == 0
               && strcmp(ostype, u.sysname) == 0 && strcmp(osrelease, u.release) == 0
               && strcmp(version, u.version) == 0
               && strncmp(procver, want_ver, strlen(want_ver)) == 0,
           why);
}

/* The --ids mode: this program's IDs, AT_SECURE, dumpable flag and
 * command name, for test_setuid_exec. */
static int print_ids(void)
{
    uid_t r, e, s;
    char comm[16] = {0};
    getresuid(&r, &e, &s);
    prctl(PR_GET_NAME, comm, 0, 0, 0);
    printf("%u %u %u %lu %d %s\n", r, e, s, getauxval(AT_SECURE),
           prctl(PR_GET_DUMPABLE, 0, 0, 0, 0), comm);
    return 0;
}

/* How run_ids_as_user starts the program: plainly, under no_new_privs, or
 * in a child that shares this process's directories and umask (CLONE_FS). */
enum ids_run { IDS_PLAIN, IDS_NO_NEW_PRIVS, IDS_SHARED_FS };

/* Run `path --ids` as user 1000; its output line in `out`. */
static int run_ids_as_user(const char *path, enum ids_run how, char *out, size_t len)
{
    int fds[2];
    if (pipe(fds) != 0)
        return -1;
    pid_t pid = how == IDS_SHARED_FS ? (pid_t)syscall(SYS_clone, CLONE_FS | SIGCHLD, 0, 0, 0, 0)
                                     : fork();
    if (pid == 0) {
        dup2(fds[1], 1);
        close(fds[0]);
        if (setresuid(1000, 1000, 1000) != 0)
            _exit(2);
        if (how == IDS_NO_NEW_PRIVS && prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0)
            _exit(3);
        execl(path, "suid_ids", "--ids", (char *)0);
        _exit(4);
    }
    close(fds[1]);
    size_t n = 0;
    ssize_t r;
    while (n < len - 1 && (r = read(fds[0], out + n, len - 1 - n)) > 0)
        n += (size_t)r;
    out[n] = 0;
    close(fds[0]);
    int st = 0;
    waitpid(pid, &st, 0);
    char *nl = strchr(out, '\n');
    if (nl)
        *nl = 0;
    return WIFEXITED(st) ? WEXITSTATUS(st) : 255;
}

/* Set-user-ID programs (Linux's bprm_fill_uid): a root-owned 04755 copy of
 * this program, run by user 1000, runs with effective and saved user 0 and
 * AT_SECURE (so the loader ignores LD_PRELOAD); under no_new_privs the bit
 * is ignored (bprm_fill_uid); when another process shares its directories
 * and umask (LSM_UNSAFE_SHARE) it gains nothing but still runs with AT_SECURE,
 * since the exec asked for another ID (cap_bprm_creds_from_file's
 * id_changed), and stays dumpable; the command name is the file's. A write
 * by a non-root owner and any chown clear the bit (file_remove_privs,
 * chown). */
static void test_setuid_exec(const char *self)
{
    const char *copy = "/tmp/suid_ids";
    int ok_copy = 0;
    int in = open(self, O_RDONLY), outfd = open(copy, O_WRONLY | O_CREAT | O_TRUNC, 0755);
    if (in >= 0 && outfd >= 0) {
        char buf[4096];
        ssize_t n;
        ok_copy = 1;
        while ((n = read(in, buf, sizeof(buf))) > 0)
            if (write(outfd, buf, (size_t)n) != n)
                ok_copy = 0;
    }
    if (in >= 0)
        close(in);
    if (outfd >= 0)
        close(outfd);
    struct stat sb = {0};
    int chmodded = ok_copy && chmod(copy, 04755) == 0 && stat(copy, &sb) == 0
                   && (sb.st_mode & 07777) == 04755;

    static char gained[64], kept[64], shared[64], why[320];
    int st_gained = run_ids_as_user(copy, IDS_PLAIN, gained, sizeof(gained));
    int st_kept = run_ids_as_user(copy, IDS_NO_NEW_PRIVS, kept, sizeof(kept));
    int st_shared = run_ids_as_user(copy, IDS_SHARED_FS, shared, sizeof(shared));
    snprintf(why, sizeof(why), "copy=%d chmod=%d mode=%o gained='%s'/%d nnp='%s'/%d fs='%s'/%d",
             ok_copy, chmodded, (unsigned)(sb.st_mode & 07777), gained, st_gained, kept, st_kept,
             shared, st_shared);
    report("musl_setuid_exec",
           chmodded && st_gained == 0 && strcmp(gained, "1000 0 0 1 0 suid_ids") == 0
               && st_kept == 0 && strcmp(kept, "1000 1000 1000 0 1 suid_ids") == 0
               && st_shared == 0 && strcmp(shared, "1000 1000 1000 1 1 suid_ids") == 0,
           why);

    /* The owner (1000) writing its own set-user-ID file clears the bit;
     * so does root changing the owner. */
    int f = 0;
    if (chown(copy, 1000, (gid_t)-1) != 0 || stat(copy, &sb) != 0 || (sb.st_mode & 07777) != 0755)
        f |= 1;
    if (chmod(copy, 04755) != 0)
        f |= 2;
    pid_t pid = fork();
    if (pid == 0) {
        if (setresuid(1000, 1000, 1000) != 0)
            _exit(1);
        int fd = open(copy, O_WRONLY | O_APPEND);
        _exit(fd >= 0 && write(fd, "", 1) == 1 ? 0 : 2);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    if (!WIFEXITED(st) || WEXITSTATUS(st) != 0)
        f |= 4;
    if (stat(copy, &sb) != 0 || (sb.st_mode & 07777) != 0755)
        f |= 8;
    snprintf(why, sizeof(why), "fail=%d mode=%o", f, (unsigned)(sb.st_mode & 07777));
    report("musl_setuid_cleared", f == 0, why);

    /* So does punching a hole through a shared mapping (MADV_REMOVE ends in
     * file_modified, as fallocate does); a store through the mapping does
     * not (Linux's page_mkwrite only updates the times). The file also
     * carries a capability attribute, which the hole must remove too. It
     * is a tmpfs file, whose pages the mapping shares, so the hole reaches
     * the file itself (a /tmp mapping is a private copy until the page
     * cache, docs/KNOWN-LIMITATIONS.md). */
    unlink(copy);
    const char *hole = "/dev/shm/suid_hole";
    f = 0;
    char page[4096];
    memset(page, 0x5a, sizeof(page));
    /* /dev/shm is world-writable: create the file afresh, never through
     * a name someone else left there (PR #33 review). */
    unlink(hole);
    int hfd = open(hole, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW, 0755);
    uint32_t fcap[5] = {0x02000001, 1u << 10, 0, 0, 0};
    if (hfd < 0 || write(hfd, page, sizeof(page)) != (ssize_t)sizeof(page) ||
        fchown(hfd, 1000, (gid_t)-1) != 0 || fchmod(hfd, 04755) != 0)
        f |= 1;
    else if (fsetxattr(hfd, "security.capability", fcap, sizeof(fcap), 0) != 0)
        f |= 128;
    if (hfd >= 0)
        close(hfd);
    if (f != 0) {
        /* No path-based call on a file this test did not create. */
        snprintf(why, sizeof(why), "setup fail=%d", f);
        report("musl_setuid_cleared_by_hole", 0, why);
        return;
    }
    pid = fork();
    if (pid == 0) {
        if (setresuid(1000, 1000, 1000) != 0)
            _exit(1);
        int fd = open(hole, O_RDWR);
        char *m = fd >= 0 ? mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0)
                          : MAP_FAILED;
        if (m == MAP_FAILED)
            _exit(2);
        volatile char *vm = m;
        vm[0] = vm[0];
        struct stat ms;
        if (fstat(fd, &ms) != 0 || (ms.st_mode & 07777) != 04755)
            _exit(3);
        _exit(madvise(m, 4096, MADV_REMOVE) == 0 ? 0 : 4);
    }
    st = 0;
    waitpid(pid, &st, 0);
    if (!WIFEXITED(st) || WEXITSTATUS(st) != 0)
        f |= 2 << (WIFEXITED(st) ? WEXITSTATUS(st) : 0);
    if (stat(hole, &sb) != 0 || (sb.st_mode & 07777) != 0755)
        f |= 64;
    errno = 0;
    if (getxattr(hole, "security.capability", NULL, 0) != -1 || errno != ENODATA)
        f |= 256;
    char first = 1;
    hfd = open(hole, O_RDONLY);
    if (hfd < 0 || pread(hfd, &first, 1, 0) != 1 || first != 0)
        f |= 512;
    if (hfd >= 0)
        close(hfd);
    snprintf(why, sizeof(why), "fail=%d mode=%o", f, (unsigned)(sb.st_mode & 07777));
    report("musl_setuid_cleared_by_hole", f == 0, why);
    unlink(hole);
}

/* raise() and abort() go through tkill (N-103): the child must die by the
 * signal, and wait must say so (N-99). */
static void test_raise_abort(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        raise(SIGTERM);
        _exit(0);
    }
    int st = 0;
    int termed = pid > 0 && waitpid(pid, &st, 0) == pid && WIFSIGNALED(st) &&
                 WTERMSIG(st) == SIGTERM;
    pid = fork();
    if (pid == 0) {
        abort();
    }
    int st2 = 0;
    int aborted = pid > 0 && waitpid(pid, &st2, 0) == pid && WIFSIGNALED(st2) &&
                  WTERMSIG(st2) == SIGABRT;
    static char why[64];
    snprintf(why, sizeof(why), "raise st=0x%x abort st=0x%x", st, st2);
    report("musl_raise_abort", termed && aborted, why);
}

/* waitid (N-103) reports an exited child in siginfo_t; ECHILD when there
 * are no children left (N-99). */
static void test_waitid(void)
{
    pid_t pid = fork();
    if (pid == 0)
        _exit(7);
    siginfo_t info;
    memset(&info, 0, sizeof(info));
    int r = waitid(P_PID, pid, &info, WEXITED);
    int ok = r == 0 && info.si_pid == pid && info.si_code == CLD_EXITED && info.si_status == 7;
    errno = 0;
    int none = waitpid(-1, NULL, 0);
    static char why[96];
    snprintf(why, sizeof(why), "r=%d pid=%d code=%d status=%d; none=%d errno=%d", r,
             info.si_pid, info.si_code, info.si_status, none, errno);
    report("musl_waitid_echild", ok && none == -1 && errno == ECHILD, why);
}

/* Memory protections as musl requests them (N-132). */
static void test_mprotect(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        volatile char *p = mmap(0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (p == MAP_FAILED)
            _exit(2);
        p[0] = 1;
        _exit(0);
    }
    int st = 0;
    int ok = pid > 0 && waitpid(pid, &st, 0) == pid && WIFSIGNALED(st) && WTERMSIG(st) == SIGSEGV;
    static char why[48];
    snprintf(why, sizeof(why), "status=0x%x", st);
    report("musl_prot_read_blocks_write", ok, why);
}

/* W^X (ADR 0017): memory writable and executable at once is EACCES by
 * default (Linux allows it); exec-gain alone (RW -> RX) is allowed. */
#ifndef PR_SET_MDWE
#define PR_SET_MDWE 65
#define PR_GET_MDWE 66
#define PR_MDWE_REFUSE_EXEC_GAIN 1
#define PR_MDWE_NO_INHERIT 2
#endif

static void test_wx_default(void)
{
    int f = 0;
    void *p = mmap(0, 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p != MAP_FAILED || errno != EACCES)
        f |= 1;
    char *q = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (q == MAP_FAILED)
        f |= 2;
    else {
        if (mprotect(q, 4096, PROT_READ | PROT_WRITE | PROT_EXEC) != -1 || errno != EACCES)
            f |= 4;
        if (mprotect(q, 4096, PROT_READ | PROT_EXEC) != 0)
            f |= 8;
        munmap(q, 4096);
    }
    if (prctl(PR_GET_MDWE, 0, 0, 0, 0) != 0)
        f |= 16;
    static char why[32];
    snprintf(why, sizeof(why), "fail=%#x", f);
    report("musl_wx_refused_by_default", f == 0, why);
}

/* Run `path` with `arg` (or none), its standard output into `out`; the
 * wait status, or -1. */
static int run_capture(const char *path, const char *arg, char *out, size_t len)
{
    extern char **environ;
    int fds[2];
    if (pipe(fds) != 0)
        return -1;
    posix_spawn_file_actions_t actions;
    posix_spawn_file_actions_init(&actions);
    posix_spawn_file_actions_adddup2(&actions, fds[1], 1);
    posix_spawn_file_actions_addclose(&actions, fds[0]);
    char *args[] = {(char *)path, (char *)arg, NULL};
    pid_t pid = -1;
    int spawned = posix_spawn(&pid, path, &actions, NULL, args, environ);
    posix_spawn_file_actions_destroy(&actions);
    close(fds[1]);
    size_t n = 0;
    ssize_t r;
    while (n < len - 1 && (r = read(fds[0], out + n, len - 1 - n)) > 0)
        n += (size_t)r;
    out[n] = 0;
    close(fds[0]);
    int status = -1;
    if (spawned != 0 || waitpid(pid, &status, 0) != pid)
        return -1;
    char *nl = strchr(out, '\n');
    if (nl)
        *nl = 0;
    return status;
}

/* A program carrying the VeridianOS W^X note (<veridian/wxneeded.h>) gets
 * Linux's behaviour: RWX memory, code written to it runs
 * (userland/tests/wx_needed_test.c). */
static void test_wx_opt_in(void)
{
    static char out[128], why[200];
    int st = run_capture("/bin/wx_needed_test", NULL, out, sizeof(out));
    snprintf(why, sizeof(why), "status=%#x output: %s", st, out);
    report("musl_wx_opt_in", st == 0 && strcmp(out, "WX OK 42") == 0, why);
}

/* tools/veridian-elfctl (scripts/build-wx-note-tests.sh): a copy built
 * without the note is refused W+X like any Linux binary; the tool's added
 * note -- in a new program header table, static PIE and dynamic -- opts it
 * in; and clearing the bit of a program that has the note opts it out. */
static void test_wx_note_tool(void)
{
    static const struct {
        const char *path, *arg, *want;
    } cases[] = {
        {"/bin/wx_plain_test", "refused", "WX REFUSED OK"},
        {"/bin/wx_patched_test", NULL, "WX OK 42"},
        {"/bin/wx_patched_dyn_test", NULL, "WX OK 42"},
        {"/bin/wx_cleared_test", "refused", "WX REFUSED OK"},
    };
    static char out[128], why[200];
    int f = 0;
    why[0] = 0;
    for (unsigned i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        int st = run_capture(cases[i].path, cases[i].arg, out, sizeof(out));
        if (st != 0 || strcmp(out, cases[i].want) != 0) {
            f |= 1 << i;
            snprintf(why, sizeof(why), "%s: status=%#x output: %s", cases[i].path, st, out);
        }
    }
    report("musl_wx_note_tool", f == 0, why);
}

/* Linux uname mode (ADR 0021; <veridian/linux-uname.h>). */
#define PR_VERIDIAN_UNAME 0x56455249
#define PR_VERIDIAN_UNAME_GET 0
#define PR_VERIDIAN_UNAME_SET 1

/* uname reports "Linux" in Linux mode and "VeridianOS" otherwise; the mode
 * is entered at exec by the note (from the header or veridian-elfctl) or a
 * GNU ABI tag naming Linux, which hold for that image only, and with the
 * prctl, which is inherited across fork and exec; it is shown by /proc/sys/kernel/ostype and /proc/version too. The
 * launcher runs a command in it, and configure finds the site file. */
static void test_uname_linux_mode(void)
{
    static const struct {
        const char *path, *arg, *want;
    } cases[] = {
        {"/bin/uname_plain_test", NULL, "VeridianOS"},
        {"/bin/uname_linux_test", NULL, "Linux"},
        {"/bin/uname_gnu_tag_test", NULL, "Linux"},
        {"/bin/uname_patched_test", NULL, "Linux"},
        {"/bin/linux-uname", "/bin/uname_plain_test", "Linux"},
        {"/bin/linux-uname", NULL, "native"},
        /* A mode the image asked for ends at the next exec. */
        {"/bin/uname_gnu_tag_test", "/bin/uname_plain_test", "VeridianOS"},
        {"/bin/uname_linux_test", "/bin/uname_plain_test", "VeridianOS"},
    };
    static char out[128], why[300];
    int f = 0;
    why[0] = 0;
    if (prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_GET, 0, 0, 0) != 0)
        f |= 1;
    for (unsigned i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        int st = run_capture(cases[i].path, cases[i].arg, out, sizeof(out));
        if (st != 0 || strcmp(out, cases[i].want) != 0) {
            f |= 2;
            snprintf(why, sizeof(why), "%s %s: status=%#x output: %s", cases[i].path,
                     cases[i].arg ? cases[i].arg : "", st, out);
        }
    }
    /* The prctl in a child, so this process keeps its mode. */
    pid_t pid = fork();
    if (pid == 0) {
        int c = 0;
        struct utsname u;
        char line[200] = "";
        if (prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_SET, 1, 0, 0) != 0
            || prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_GET, 0, 0, 0) != 1)
            c |= 1;
        if (uname(&u) != 0 || strcmp(u.sysname, "Linux") != 0
            || strcmp(u.release, "7.3.0-veridian") != 0)
            c |= 2;
        if (read_line("/proc/sys/kernel/ostype", line, sizeof(line)) != 0
            || strcmp(line, "Linux") != 0)
            c |= 4;
        if (read_line("/proc/version", line, sizeof(line)) != 0
            || strncmp(line, "Linux version 7.3.0-veridian (", 30) != 0)
            c |= 4;
        /* Inherited by what this child runs, and by forks of it, through
         * any number of execs. */
        if (run_capture("/bin/uname_plain_test", NULL, out, sizeof(out)) != 0
            || strcmp(out, "Linux") != 0)
            c |= 8;
        if (run_capture("/bin/uname_gnu_tag_test", "/bin/uname_plain_test", out, sizeof(out)) != 0
            || strcmp(out, "Linux") != 0)
            c |= 8;
        if (run_capture("/bin/linux-uname", NULL, out, sizeof(out)) != 0
            || strcmp(out, "linux") != 0)
            c |= 8;
        /* Bad arguments are EINVAL; back to native. */
        if (prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_SET, 2, 0, 0) != -1 || errno != EINVAL)
            c |= 16;
        if (prctl(PR_VERIDIAN_UNAME, 2, 0, 0, 0) != -1 || errno != EINVAL)
            c |= 16;
        if (prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_SET, 0, 0, 0) != 0 || uname(&u) != 0
            || strcmp(u.sysname, "VeridianOS") != 0)
            c |= 32;
        /* A chosen native mode holds too, even for an image that asks for
         * Linux (what linux-uname -n does; PR #34 review). */
        if (run_capture("/bin/uname_gnu_tag_test", NULL, out, sizeof(out)) != 0
            || strcmp(out, "VeridianOS") != 0)
            c |= 64;
        _exit(c);
    }
    int st = -1;
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) {
        f |= 4;
        snprintf(why + strlen(why), sizeof(why) - strlen(why), " child=%#x", st);
    }
    /* init and /etc/profile point configure at the site file. */
    const char *site = getenv("CONFIG_SITE");
    if (!site || strcmp(site, "/usr/share/config.site") != 0 || access(site, R_OK) != 0
        || access("/usr/local/share/config.site", R_OK) != 0 || access("/etc/profile", R_OK) != 0)
        f |= 8;
    char tail[40];
    snprintf(tail, sizeof(tail), " fail=%#x", f);
    strncat(why, tail, sizeof(why) - strlen(why) - 1);
    report("musl_uname_linux_mode", f == 0, why);
}

/* PR_SET_MDWE as Linux: argument checks, one way (EPERM), no RWX and no
 * exec gain, kept across fork and exec -- where it overrides the W^X
 * note -- unless set with PR_MDWE_NO_INHERIT. In children: it is
 * permanent. */
static void test_mdwe(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        int f = 0;
        if (prctl(PR_SET_MDWE, PR_MDWE_NO_INHERIT, 0, 0, 0) != -1 || errno != EINVAL)
            f |= 1;
        if (prctl(PR_SET_MDWE, 4, 0, 0, 0) != -1 || errno != EINVAL)
            f |= 1;
        if (prctl(PR_SET_MDWE, PR_MDWE_REFUSE_EXEC_GAIN, 0, 0, 0) != 0)
            f |= 2;
        if (prctl(PR_GET_MDWE, 0, 0, 0, 0) != PR_MDWE_REFUSE_EXEC_GAIN)
            f |= 2;
        if (prctl(PR_SET_MDWE, PR_MDWE_REFUSE_EXEC_GAIN, 0, 0, 0) != 0)
            f |= 4;
        if (prctl(PR_SET_MDWE, 0, 0, 0, 0) != -1 || errno != EPERM)
            f |= 4;
        if (prctl(PR_SET_MDWE, 3, 0, 0, 0) != -1 || errno != EPERM)
            f |= 4;
        /* RX is fine; RW cannot become RX; RX stays RX. */
        char *x = mmap(0, 4096, PROT_READ | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        char *w = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (x == MAP_FAILED || w == MAP_FAILED)
            f |= 8;
        else {
            if (mprotect(w, 4096, PROT_READ | PROT_EXEC) != -1 || errno != EACCES)
                f |= 16;
            if (mprotect(x, 4096, PROT_READ | PROT_EXEC) != 0)
                f |= 16;
        }
        /* Kept across fork and exec; the note does not undo it. */
        pid_t g = fork();
        if (g == 0)
            _exit(prctl(PR_GET_MDWE, 0, 0, 0, 0) == PR_MDWE_REFUSE_EXEC_GAIN ? 0 : 1);
        int gst = -1;
        if (g < 0 || waitpid(g, &gst, 0) != g || !WIFEXITED(gst) || WEXITSTATUS(gst) != 0)
            f |= 32;
        char out[64];
        if (run_capture("/bin/wx_needed_test", "refused", out, sizeof(out)) != 0
            || strcmp(out, "WX REFUSED OK") != 0)
            f |= 64;
        _exit(f);
    }
    int st = -1, f2 = -1;
    int ok = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;
    /* NO_INHERIT: a fork child starts without it. */
    pid = fork();
    if (pid == 0) {
        if (prctl(PR_SET_MDWE, PR_MDWE_REFUSE_EXEC_GAIN | PR_MDWE_NO_INHERIT, 0, 0, 0) != 0
            || prctl(PR_GET_MDWE, 0, 0, 0, 0) != 3)
            _exit(1);
        pid_t g = fork();
        if (g == 0)
            _exit(prctl(PR_GET_MDWE, 0, 0, 0, 0) == 0 ? 0 : 1);
        int gst = -1;
        _exit(g > 0 && waitpid(g, &gst, 0) == g && WIFEXITED(gst) && WEXITSTATUS(gst) == 0 ? 0 : 2);
    }
    int st2 = -1;
    if (pid > 0 && waitpid(pid, &st2, 0) == pid && WIFEXITED(st2))
        f2 = WEXITSTATUS(st2);
    static char why[64];
    snprintf(why, sizeof(why), "status=%#x no_inherit=%d", st, f2);
    report("musl_mdwe", ok && f2 == 0, why);
}

/* Milliseconds of CLOCK_MONOTONIC. */
static long now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000L + t.tv_nsec / 1000000L;
}

/* FIFOs as Linux: mkfifo makes one (EEXIST again); a non-blocking
 * write-only open with no reader is ENXIO, a non-blocking read-only open
 * succeeds and reads end of file; a blocking open waits for the other end,
 * and data flows from the writer's open to the reader's. */
static void test_mkfifo(void)
{
    const char *path = "/tmp/musl_fifo";
    static char why[96];
    int f = 0;
    unlink(path);
    struct stat st;
    if (mkfifo(path, 0640) != 0 || stat(path, &st) != 0 || !S_ISFIFO(st.st_mode)
        || (st.st_mode & 0777) != 0640)
        f |= 1;
    if (mkfifo(path, 0640) != -1 || errno != EEXIST)
        f |= 2;
    if (open(path, O_WRONLY | O_NONBLOCK) != -1 || errno != ENXIO)
        f |= 4;
    int r = open(path, O_RDONLY | O_NONBLOCK);
    char b[32];
    struct pollfd pfd = {r, POLLIN, 0};
    if (r < 0 || read(r, b, sizeof(b)) != 0 || poll(&pfd, 1, 0) != 0)
        f |= 8;
    if (r >= 0)
        close(r);
    /* The child starts its clock, then says so; the parent's wait starts
     * after that, so a child scheduled late does not measure too short a
     * wait (PR #20 review: a slow start made it fail). */
    int started[2];
    if (pipe(started) != 0)
        f |= 128;
    pid_t pid = fork();
    if (pid == 0) {
        long t0 = now_ms();
        if (write(started[1], "s", 1) != 1)
            _exit(3);
        int w = open(path, O_WRONLY);
        long waited = now_ms() - t0;
        int ok = w >= 0 && waited >= 100 && write(w, "fifo data", 9) == 9;
        _exit(ok ? 0 : (w < 0 ? 1 : 2));
    }
    char go;
    if (pid > 0 && read(started[0], &go, 1) != 1)
        f |= 128;
    close(started[0]);
    close(started[1]);
    usleep(300000); /* the writer's open waits for this reader */
    r = open(path, O_RDONLY);
    int n = 0, k;
    while (r >= 0 && (k = read(r, b + n, sizeof(b) - 1 - n)) > 0)
        n += k;
    b[n] = 0;
    if (r < 0 || strcmp(b, "fifo data") != 0)
        f |= 16;
    if (r >= 0 && (fstat(r, &st) != 0 || !S_ISFIFO(st.st_mode)))
        f |= 32;
    if (r >= 0)
        close(r);
    int cst = -1;
    if (pid < 0 || waitpid(pid, &cst, 0) != pid || !WIFEXITED(cst) || WEXITSTATUS(cst) != 0)
        f |= 64;
    unlink(path);
    snprintf(why, sizeof(why), "f=%#x read=\"%s\" child=%#x errno=%d", f, b, cst, errno);
    report("musl_mkfifo", f == 0, why);
}

/* mknod and mknodat as Linux: a device node opens the device its number
 * names and stat reports the number; a number with no device and a socket
 * file are ENXIO to open; S_IFREG (or type 0) makes a regular file, a
 * directory type is EPERM and an unknown one EINVAL; "name/" is ENOENT;
 * and only root makes devices (EPERM), while anyone may make a FIFO. */
static void test_mknod(void)
{
    const char *dir = "/tmp/musl_mknod";
    static char why[96];
    char p[64];
    int f = 0;
    mkdir(dir, 0777);
    chmod(dir, 0777);
    struct stat st;
    snprintf(p, sizeof(p), "%s/null", dir);
    unlink(p);
    if (mknod(p, S_IFCHR | 0666, makedev(1, 3)) != 0 || stat(p, &st) != 0
        || !S_ISCHR(st.st_mode) || major(st.st_rdev) != 1 || minor(st.st_rdev) != 3)
        f |= 1;
    int fd = open(p, O_RDWR);
    char b[4] = {1, 1, 1, 1};
    if (fd < 0 || write(fd, "gone", 4) != 4 || read(fd, b, 4) != 0)
        f |= 2;
    if (fd >= 0 && (fstat(fd, &st) != 0 || st.st_rdev != makedev(1, 3)))
        f |= 2;
    if (fd >= 0)
        close(fd);
    unlink(p);
    if (stat("/dev/null", &st) != 0 || st.st_rdev != makedev(1, 3))
        f |= 4;
    snprintf(p, sizeof(p), "%s/none", dir);
    unlink(p);
    if (mknod(p, S_IFCHR | 0600, makedev(99, 1)) != 0 || open(p, O_RDONLY) != -1
        || errno != ENXIO)
        f |= 8;
    unlink(p);
    snprintf(p, sizeof(p), "%s/sock", dir);
    unlink(p);
    if (mknod(p, S_IFSOCK | 0600, 0) != 0 || stat(p, &st) != 0 || !S_ISSOCK(st.st_mode)
        || open(p, O_RDONLY) != -1 || errno != ENXIO)
        f |= 16;
    unlink(p);
    snprintf(p, sizeof(p), "%s/reg", dir);
    unlink(p);
    if (mknod(p, 0600, 0) != 0 || stat(p, &st) != 0 || !S_ISREG(st.st_mode))
        f |= 32;
    unlink(p);
    if (mknod(p, S_IFDIR | 0700, 0) != -1 || errno != EPERM)
        f |= 64;
    if (mknod(p, 0030000 | 0600, 0) != -1 || errno != EINVAL)
        f |= 64;
    snprintf(p, sizeof(p), "%s/slash/", dir);
    if (mknod(p, S_IFIFO | 0600, 0) != -1 || errno != ENOENT)
        f |= 128;
    int dfd = open(dir, O_RDONLY | O_DIRECTORY);
    if (dfd < 0 || mknodat(dfd, "at", S_IFIFO | 0600, 0) != 0
        || fstatat(dfd, "at", &st, 0) != 0 || !S_ISFIFO(st.st_mode))
        f |= 256;
    if (dfd >= 0) {
        unlinkat(dfd, "at", 0);
        close(dfd);
    }
    pid_t pid = fork();
    if (pid == 0) {
        char q[64];
        int g = 0;
        if (setuid(65534) != 0)
            _exit(1);
        snprintf(q, sizeof(q), "%s/userdev", dir);
        if (mknod(q, S_IFCHR | 0600, makedev(1, 3)) != -1 || errno != EPERM)
            g |= 2;
        snprintf(q, sizeof(q), "%s/userfifo", dir);
        if (mknod(q, S_IFIFO | 0600, 0) != 0 || stat(q, &st) != 0 || st.st_uid != 65534)
            g |= 4;
        unlink(q);
        _exit(g);
    }
    int cst = -1;
    if (pid < 0 || waitpid(pid, &cst, 0) != pid || !WIFEXITED(cst) || WEXITSTATUS(cst) != 0)
        f |= 512;
    rmdir(dir);
    snprintf(why, sizeof(why), "f=%#x child=%#x errno=%d", f, cst, errno);
    report("musl_mknod", f == 0, why);
}

static int un_socket(const char *path, int do_bind)
{
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    snprintf(a.sun_path, sizeof(a.sun_path), "%s", path);
    int s = socket(AF_UNIX, SOCK_STREAM, 0);
    if (s < 0)
        return -1;
    int r = do_bind ? bind(s, (struct sockaddr *)&a, sizeof(a))
                    : connect(s, (struct sockaddr *)&a, sizeof(a));
    if (r != 0) {
        int e = errno;
        close(s);
        errno = e;
        return -1;
    }
    return s;
}

/* bind() makes a socket file, as on Linux: stat shows S_IFSOCK with the
 * umask applied, a second bind to the name is EADDRINUSE, connect finds the
 * socket through the file (relative names against the cwd; ENOENT once it
 * is unlinked, while the socket itself goes on working; ECONNREFUSED for a
 * file that no listening socket holds), and the file outlives the socket. */
static void test_unix_socket_file(void)
{
    const char *dir = "/tmp/musl_sockfile";
    static char why[80];
    char p[64];
    int f = 0;
    struct stat st;
    mkdir(dir, 0777);
    snprintf(p, sizeof(p), "%s/s", dir);
    unlink(p);
    mode_t old = umask(022);
    int l = un_socket(p, 1);
    umask(old);
    if (l < 0 || listen(l, 4) != 0)
        f |= 1;
    if (stat(p, &st) != 0 || !S_ISSOCK(st.st_mode) || (st.st_mode & 0777) != 0755)
        f |= 2;
    int s2 = un_socket(p, 1);
    if (s2 != -1 || errno != EADDRINUSE)
        f |= 4;
    if (s2 >= 0)
        close(s2);
    char cwd[128];
    int c = -1;
    if (getcwd(cwd, sizeof(cwd)) && chdir(dir) == 0) {
        c = un_socket("s", 0);
        if (chdir(cwd) != 0)
            f |= 8;
    }
    if (c < 0)
        f |= 8;
    /* Only with a connection queued: accept would wait for ever. */
    int a = l >= 0 && c >= 0 ? accept(l, NULL, NULL) : -1;
    char b = 0;
    if (a < 0 || write(c, "x", 1) != 1 || read(a, &b, 1) != 1 || b != 'x')
        f |= 16;
    unlink(p);
    int c2 = un_socket(p, 0);
    if (c2 != -1 || errno != ENOENT)
        f |= 32;
    if (c2 >= 0)
        close(c2);
    if (a >= 0 && (write(a, "y", 1) != 1 || read(c, &b, 1) != 1 || b != 'y'))
        f |= 64;
    snprintf(p, sizeof(p), "%s/reg", dir);
    close(open(p, O_CREAT | O_WRONLY, 0666));
    int c3 = un_socket(p, 0);
    if (c3 != -1 || errno != ECONNREFUSED)
        f |= 128;
    unlink(p);
    snprintf(p, sizeof(p), "%s/gone", dir);
    unlink(p);
    int g = un_socket(p, 1);
    if (g >= 0)
        close(g);
    int c4 = un_socket(p, 0);
    if (g < 0 || stat(p, &st) != 0 || !S_ISSOCK(st.st_mode) || c4 != -1 || errno != ECONNREFUSED)
        f |= 256;
    unlink(p);
    int fds[] = {l, a, c, c3, c4};
    for (unsigned i = 0; i < sizeof(fds) / sizeof(fds[0]); i++)
        if (fds[i] >= 0)
            close(fds[i]);
    rmdir(dir);
    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("musl_unix_socket_file", f == 0, why);
}

/* open with O_CREAT names a file, so Linux refuses it with EISDIR for a
 * name ending in '/' (whether or not it exists, creating nothing) and for an
 * existing directory, even opened read-only. */
static void test_open_creat_dir(void)
{
    const char *dir = "/tmp/musl_creat_dir";
    static char why[64];
    int f = 0;
    struct stat st;
    mkdir(dir, 0777);
    int fd = open("/tmp/musl_creat_dir/new/", O_CREAT | O_WRONLY, 0600);
    if (fd != -1 || errno != EISDIR)
        f |= 1;
    if (stat("/tmp/musl_creat_dir/new", &st) != -1 || errno != ENOENT)
        f |= 2;
    if (fd >= 0)
        close(fd);
    unlink("/tmp/musl_creat_dir/new");
    fd = open(dir, O_CREAT | O_RDONLY, 0600);
    if (fd != -1 || errno != EISDIR)
        f |= 4;
    if (fd >= 0)
        close(fd);
    fd = open("/tmp/musl_creat_dir/.", O_CREAT | O_RDONLY, 0600);
    if (fd != -1 || errno != EISDIR)
        f |= 8;
    if (fd >= 0)
        close(fd);
    fd = open("/tmp/musl_creat_dir/file", O_CREAT | O_EXCL | O_WRONLY, 0600);
    if (fd < 0 || stat("/tmp/musl_creat_dir/file", &st) != 0 || !S_ISREG(st.st_mode))
        f |= 16;
    if (fd >= 0)
        close(fd);
    unlink("/tmp/musl_creat_dir/file");
    rmdir(dir);
    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("musl_open_creat_dir", f == 0, why);
}

/* Threads run concurrently as their own tasks (ADR 0006 stage D2):
 * pthread_create/join with a mutex-protected counter, and pthread_exit
 * ends only the calling thread (musl's exit was a process exit, N-106). */
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static long counter;

static void *adder(void *arg)
{
    for (int i = 0; i < 1000; i++) {
        pthread_mutex_lock(&lock);
        counter++;
        pthread_mutex_unlock(&lock);
        if (i % 100 == 0)
            sched_yield();
    }
    if (arg)
        pthread_exit((void *)42);
    return (void *)7;
}

static void test_threads(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        /* In a child, so a hang or crash cannot take the suite down. */
        pthread_t a, b;
        void *ra = 0, *rb = 0;
        if (pthread_create(&a, 0, adder, 0) != 0 || pthread_create(&b, 0, adder, (void *)1) != 0)
            _exit(2);
        if (pthread_join(a, &ra) != 0 || pthread_join(b, &rb) != 0)
            _exit(3);
        _exit(counter == 2000 && ra == (void *)7 && rb == (void *)42 ? 0 : 4);
    }
    int st = 0;
    int ok = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;
    static char why[48];
    snprintf(why, sizeof(why), "status=0x%x", st);
    report("musl_pthreads_join_mutex_exit", ok, why);
}

/* A clone with a non-canonical TLS base is refused (security review of
 * stage D2: it used to reach IA32_FS_BASE and #GP the kernel at every
 * switch). exec from a multithreaded process ends the other threads first,
 * as on Linux (N-101; it used to fail with EAGAIN, and before that freed
 * page tables they still used): one thread sleeping in the kernel, one
 * spinning in user mode, and /bin/true runs. */
static void *sleeper(void *arg)
{
    (void)arg;
    struct timespec ts = {5, 0};
    nanosleep(&ts, 0);
    return 0;
}

static void *spinner(void *arg)
{
    volatile unsigned long *n = arg;
    for (;;)
        ++*n;
    return 0;
}

static void test_thread_guards(void)
{
    pid_t pid = fork();
    if (pid == 0) {
#if defined(__x86_64__)
        /* x86_64 only: the TLS base goes into IA32_FS_BASE. arm64 and
         * RISC-V accept any value for TPIDR_EL0 / tp, as Linux does. */
        static char stack[16384] __attribute__((aligned(16)));
        long flags = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD |
                     CLONE_SYSVSEM | CLONE_SETTLS;
        errno = 0;
        long r = syscall(SYS_clone, flags, stack + sizeof(stack), 0, 0, 0x8000000000000000UL);
        if (!(r == -1 && errno == EINVAL))
            _exit(10);
#endif

        static volatile unsigned long spins;
        pthread_t a, b;
        if (pthread_create(&a, 0, sleeper, 0) != 0 ||
            pthread_create(&b, 0, spinner, (void *)&spins) != 0)
            _exit(11);
        while (spins == 0)
            sched_yield();
        char *argv[] = {"true", 0};
        execve("/bin/true", argv, 0);
        _exit(12); /* exec failed */
    }
    int st = 0;
    int ok = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;
    static char why[48];
    snprintf(why, sizeof(why), "status=0x%x", st);
    report("musl_clone_tls_and_multithreaded_exec", ok, why);
}

/* A write to a page a fork shares copies it, and the copy replaces the
 * shared page: unmapped, then mapped again. A thread on another CPU that
 * touches the page meanwhile faults, and the fault must be retried, not
 * turned into SIGSEGV (it was: musl_clone_tls_and_multithreaded_exec
 * crashed now and then on AArch64 with -smp 4). Readers keep reading
 * while the main thread writes every page; only with more than one CPU
 * can they meet the gap. */
#define COW_RACE_PAGES 256
static char cow_race_buf[COW_RACE_PAGES * 4096] __attribute__((aligned(4096)));
static volatile int cow_race_done;

static void *cow_race_reader(void *arg)
{
    volatile unsigned long *sum = arg;
    while (!cow_race_done)
        for (int i = 0; i < COW_RACE_PAGES; i++)
            *sum += ((volatile char *)cow_race_buf)[i * 4096];
    return 0;
}

static void test_cow_race(void)
{
    int ok = 1;
    int st = 0;
    for (int round = 0; round < 4 && ok; round++) {
        /* Backed in the parent, so the fork shares every page. */
        for (int i = 0; i < COW_RACE_PAGES; i++)
            cow_race_buf[i * 4096] = (char)round;
        pid_t pid = fork();
        if (pid == 0) {
            static volatile unsigned long sums[3];
            pthread_t t[3];
            for (int i = 0; i < 3; i++)
                if (pthread_create(&t[i], 0, cow_race_reader, (void *)&sums[i]) != 0)
                    _exit(2);
            sched_yield();
            for (int i = 0; i < COW_RACE_PAGES; i++)
                cow_race_buf[i * 4096 + 1] = 1;
            cow_race_done = 1;
            for (int i = 0; i < 3; i++)
                pthread_join(t[i], 0);
            _exit(0);
        }
        ok = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;
    }
    static char why[48];
    snprintf(why, sizeof(why), "status=0x%x", st);
    report("musl_cow_concurrent_readers", ok, why);
}

/* Signal handlers as musl installs them (SA_RESTORER, __restore_rt), and
 * musl's set layout (bit sig - 1): a mask naming SIGUSR1 must block
 * SIGUSR1, not the signal next to it (N-96). */
static volatile int got;
static void on_sig(int sig) { got = sig; }

static void test_signals(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_sig;
    sigaction(SIGUSR1, &sa, 0);
    sigaction(SIGUSR2, &sa, 0);

    got = 0;
    raise(SIGUSR1);
    int ran = got == SIGUSR1;

    sigset_t set, pend;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    sigprocmask(SIG_BLOCK, &set, 0);
    got = 0;
    raise(SIGUSR1);
    int blocked = got == 0;
    raise(SIGUSR2); /* the neighbour must still get through */
    int neighbour = got == SIGUSR2;
    sigemptyset(&pend);
    int pending = sigpending(&pend) == 0 && sigismember(&pend, SIGUSR1) == 1;
    got = 0;
    sigprocmask(SIG_UNBLOCK, &set, 0);
    int delivered = got == SIGUSR1;

    static char why[80];
    snprintf(why, sizeof(why), "ran=%d blocked=%d neighbour=%d pending=%d delivered=%d", ran, blocked,
             neighbour, pending, delivered);
    report("musl_signal_handler_and_mask", ran && blocked && neighbour && delivered, why);
    signal(SIGUSR1, SIG_DFL);
    signal(SIGUSR2, SIG_DFL);
}

/* N-114: a thread runs on its own stack (no kernel-chosen one is mapped
 * for it), and a child it forks can still exec. */
/* sigaltstack with musl's wrapper (N-222): an SA_ONSTACK handler runs on
 * the alternate stack and sees SS_ONSTACK; SS_AUTODISARM disarms the stack
 * during the handler and rt_sigreturn re-arms it. */
static char musl_alt[4 * SIGSTKSZ];
static volatile uintptr_t musl_alt_sp;
static volatile int musl_alt_flags;
static void on_alt(int sig)
{
    int local;
    stack_t now;
    (void)sig;
    musl_alt_sp = (uintptr_t)&local;
    musl_alt_flags = sigaltstack(0, &now) == 0 ? now.ss_flags : -1;
}

static void test_sigaltstack(void)
{
    stack_t st = {.ss_sp = musl_alt, .ss_size = sizeof(musl_alt), .ss_flags = 0}, q;
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_alt;
    sa.sa_flags = SA_ONSTACK;
    sigaction(SIGUSR1, &sa, 0);

    int set = sigaltstack(&st, 0) == 0;
    raise(SIGUSR1);
    int on = musl_alt_sp > (uintptr_t)musl_alt &&
             musl_alt_sp - (uintptr_t)musl_alt <= sizeof(musl_alt) &&
             (musl_alt_flags & SS_ONSTACK);
    st.ss_flags = SS_AUTODISARM;
    sigaltstack(&st, 0);
    raise(SIGUSR1);
    int disarmed = (musl_alt_flags & SS_DISABLE) != 0;
    int rearmed = sigaltstack(0, &q) == 0 && q.ss_sp == musl_alt && (q.ss_flags & SS_AUTODISARM);
    st.ss_flags = SS_DISABLE;
    sigaltstack(&st, 0);
    signal(SIGUSR1, SIG_DFL);

    static char why[80];
    snprintf(why, sizeof(why), "set=%d on=%d disarmed=%d rearmed=%d", set, on, disarmed, rearmed);
    report("musl_sigaltstack", set && on && disarmed && rearmed, why);
}

static void *fork_exec_from_thread(void *arg)
{
    (void)arg;
    pid_t pid = fork();
    if (pid == 0) {
        char *args[] = {"false", 0};
        char *env[] = {0};
        execve("/bin/false", args, env);
        _exit(42);
    }
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid)
        return (void *)-1L;
    return (void *)(long)(WIFEXITED(st) ? WEXITSTATUS(st) : 100 + WTERMSIG(st));
}

static void test_thread_fork_exec(void)
{
    pthread_t t;
    void *res = (void *)-2L;
    int created = pthread_create(&t, 0, fork_exec_from_thread, 0) == 0;
    if (created)
        pthread_join(t, &res);
    static char why[64];
    snprintf(why, sizeof(why), "created=%d result=%ld", created, (long)res);
    report("musl_thread_fork_exec", created && (long)res == 1, why);
}

/* Blocking sprint: sleeps leave the run queue until the timer or a
 * signal wakes them. A signal ends nanosleep with EINTR and the time
 * left; TIMER_ABSTIME sleeps until the absolute time. */
static volatile sig_atomic_t sleep_usr1;
static void on_sleep_usr1(int s)
{
    (void)s;
    sleep_usr1 = 1;
}

static long long mono_ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void test_sleeps(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_sleep_usr1; /* no SA_RESTART */
    sigaction(SIGUSR1, &sa, 0);
    sleep_usr1 = 0;

    pid_t parent = getpid();
    pid_t pid = fork();
    if (pid == 0) {
        struct timespec d = {0, 100 * 1000 * 1000};
        nanosleep(&d, 0);
        kill(parent, SIGUSR1);
        _exit(0);
    }
    struct timespec req = {3, 0}, rem = {0, 0};
    long long t0 = mono_ns();
    int r = nanosleep(&req, &rem);
    long long slept = mono_ns() - t0;
    int eintr = r == -1 && errno == EINTR && sleep_usr1;
    int rem_ok = rem.tv_sec >= 1 && rem.tv_sec <= 2;
    waitpid(pid, 0, 0);

    /* Absolute: wake no earlier than the target. */
    struct timespec now, abs;
    clock_gettime(CLOCK_MONOTONIC, &now);
    long long target = (long long)now.tv_sec * 1000000000LL + now.tv_nsec + 150000000LL;
    abs.tv_sec = target / 1000000000LL;
    abs.tv_nsec = target % 1000000000LL;
    int ar = clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &abs, 0);
    long long late = mono_ns() - target;
    int abs_ok = ar == 0 && late >= 0 && late < 1000000000LL;

    /* Relative: at least the time asked for. */
    struct timespec d = {0, 50 * 1000 * 1000};
    t0 = mono_ns();
    int rr = nanosleep(&d, 0);
    long long took = mono_ns() - t0;
    int rel_ok = rr == 0 && took >= 50000000LL;

    signal(SIGUSR1, SIG_DFL);
    static char why[112];
    snprintf(why, sizeof(why), "eintr=%d rem=%ld slept_ms=%lld abs=%d late_ms=%lld rel=%d", eintr,
             (long)rem.tv_sec, slept / 1000000, abs_ok, late / 1000000, rel_ok);
    report("musl_sleeps_block_and_wake", eintr && rem_ok && abs_ok && rel_ok, why);
}

/* Blocking sprint: futex waits sleep on keyed queues. Condition variable
 * broadcast (musl requeues), a timed wait ends with ETIMEDOUT (N-104),
 * and a futex in MAP_SHARED memory is shared across fork (N-114). */
static pthread_mutex_t cv_m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cv = PTHREAD_COND_INITIALIZER;
static int cv_go, cv_woke;

static void *cv_waiter(void *arg)
{
    (void)arg;
    pthread_mutex_lock(&cv_m);
    while (!cv_go)
        pthread_cond_wait(&cv, &cv_m);
    cv_woke++;
    pthread_mutex_unlock(&cv_m);
    return 0;
}

static void test_futex_queues(void)
{
    /* Broadcast to four waiters. */
    pthread_t t[4];
    int created = 0;
    for (int i = 0; i < 4; i++)
        created += pthread_create(&t[i], 0, cv_waiter, 0) == 0;
    struct timespec ms50 = {0, 50 * 1000 * 1000};
    nanosleep(&ms50, 0);
    pthread_mutex_lock(&cv_m);
    cv_go = 1;
    pthread_cond_broadcast(&cv);
    pthread_mutex_unlock(&cv_m);
    for (int i = 0; i < created; i++)
        pthread_join(t[i], 0);
    int broadcast = created == 4 && cv_woke == 4;

    /* Timed wait with nobody signalling. */
    struct timespec abs;
    clock_gettime(CLOCK_REALTIME, &abs);
    long long t0 = mono_ns();
    abs.tv_nsec += 100 * 1000 * 1000;
    if (abs.tv_nsec >= 1000000000) {
        abs.tv_sec++;
        abs.tv_nsec -= 1000000000;
    }
    pthread_mutex_lock(&cv_m);
    int tr = pthread_cond_timedwait(&cv, &cv_m, &abs);
    pthread_mutex_unlock(&cv_m);
    long long waited = mono_ns() - t0;
    int timed = tr == ETIMEDOUT && waited >= 90000000LL;

    /* Shared futex across fork: the child waits, the parent wakes it. */
    volatile int *w = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    int shared = 0;
    long woke = -1;
    if (w != MAP_FAILED) {
        *w = 0;
        pid_t pid = fork();
        if (pid == 0) {
            long r = syscall(SYS_futex, w, 0 /* FUTEX_WAIT */, 0, 0, 0, 0);
            _exit(r == 0 ? 0 : (errno == EAGAIN ? 2 : 1));
        }
        struct timespec ms100 = {0, 100 * 1000 * 1000};
        nanosleep(&ms100, 0);
        *w = 1;
        woke = syscall(SYS_futex, w, 1 /* FUTEX_WAKE */, 1, 0, 0, 0);
        int st = 0;
        waitpid(pid, &st, 0);
        shared = woke == 1 && WIFEXITED(st) && WEXITSTATUS(st) == 0;
        munmap((void *)w, 4096);
    }

    static char why[112];
    snprintf(why, sizeof(why), "broadcast=%d(%d) timed=%d(r=%d ms=%lld) shared=%d(woke=%ld)",
             broadcast, cv_woke, timed, tr, waited / 1000000, shared, woke);
    report("musl_futex_queues", broadcast && timed && shared, why);
}

/* Blocking sprint: a blocked pipe read, an infinite poll and a blocked
 * eventfd read each sleep until another thread or process acts. */
static int io_wfd = -1, io_efd = -1;
static void *io_writer(void *arg)
{
    (void)arg;
    struct timespec ms80 = {0, 80 * 1000 * 1000};
    nanosleep(&ms80, 0);
    if (write(io_wfd, "x", 1) != 1)
        return (void *)1L;
    nanosleep(&ms80, 0);
    uint64_t one = 1;
    if (write(io_efd, &one, sizeof(one)) != sizeof(one))
        return (void *)2L;
    return 0;
}

static void test_blocking_io(void)
{
    int fds[2];
    if (pipe(fds) != 0) {
        report("musl_blocking_io_wakes", 0, "pipe failed");
        return;
    }
    io_wfd = fds[1];
    io_efd = eventfd(0, 0);
    pthread_t t;
    int created = io_efd >= 0 && pthread_create(&t, 0, io_writer, 0) == 0;

    char c = 0;
    long long t0 = mono_ns();
    int got_pipe = created && read(fds[0], &c, 1) == 1 && c == 'x';
    long long pipe_ms = (mono_ns() - t0) / 1000000;

    uint64_t v = 0;
    int got_efd = created && read(io_efd, &v, sizeof(v)) == sizeof(v) && v == 1;
    if (created)
        pthread_join(t, 0);

    /* Infinite poll on a pipe a child writes to later. */
    pid_t pid = fork();
    if (pid == 0) {
        struct timespec ms80 = {0, 80 * 1000 * 1000};
        nanosleep(&ms80, 0);
        _exit(write(fds[1], "y", 1) == 1 ? 0 : 1);
    }
    struct pollfd pfd = {fds[0], POLLIN, 0};
    int pr = poll(&pfd, 1, -1);
    int polled = pr == 1 && (pfd.revents & POLLIN) && read(fds[0], &c, 1) == 1 && c == 'y';
    waitpid(pid, 0, 0);
    close(fds[0]);
    close(fds[1]);
    if (io_efd >= 0)
        close(io_efd);

    static char why[96];
    snprintf(why, sizeof(why), "pipe=%d(%lldms) eventfd=%d poll=%d", got_pipe, pipe_ms, got_efd,
             polled);
    report("musl_blocking_io_wakes", got_pipe && pipe_ms >= 50 && got_efd && polled, why);
}

/* Blocking sprint: a blocking pipe write returns only when all of it is
 * written, and writes of up to PIPE_BUF bytes from different writers are
 * never interleaved. */
static void test_pipe_write_semantics(void)
{
    int fds[2];
    if (pipe(fds) != 0) {
        report("musl_pipe_write_whole_and_atomic", 0, "pipe failed");
        return;
    }
    /* One 256 KiB write (four times the capacity) against a slow reader. */
    enum { BIG = 256 * 1024 };
    pid_t pid = fork();
    if (pid == 0) {
        close(fds[0]);
        static char big[BIG];
        memset(big, 'z', sizeof(big));
        ssize_t w = write(fds[1], big, sizeof(big));
        _exit(w == BIG ? 0 : 1);
    }
    static char buf[8192];
    long got = 0;
    struct timespec ms5 = {0, 5 * 1000 * 1000};
    while (got < BIG) {
        nanosleep(&ms5, 0);
        ssize_t n = read(fds[0], buf, sizeof(buf));
        if (n <= 0)
            break;
        got += n;
    }
    int st = 0;
    waitpid(pid, &st, 0);
    int whole = got == BIG && WIFEXITED(st) && WEXITSTATUS(st) == 0;

    /* Two writers, 64 blocks of PIPE_BUF bytes each. */
    pid_t w[2];
    for (int k = 0; k < 2; k++) {
        w[k] = fork();
        if (w[k] == 0) {
            close(fds[0]);
            char blk[4096];
            memset(blk, 'a' + k, sizeof(blk));
            for (int i = 0; i < 64; i++)
                if (write(fds[1], blk, sizeof(blk)) != (ssize_t)sizeof(blk))
                    _exit(1);
            _exit(0);
        }
    }
    close(fds[1]);
    int atomic = 1;
    long total = 0;
    static char blk[4096];
    for (;;) {
        /* Read exactly one block at a time. */
        long have = 0;
        while (have < (long)sizeof(blk)) {
            ssize_t n = read(fds[0], blk + have, sizeof(blk) - have);
            if (n <= 0)
                break;
            have += n;
        }
        if (have == 0)
            break;
        total += have;
        for (long i = 1; i < have; i++)
            if (blk[i] != blk[0])
                atomic = 0;
    }
    waitpid(w[0], 0, 0);
    waitpid(w[1], 0, 0);
    close(fds[0]);

    static char why[96];
    snprintf(why, sizeof(why), "whole=%d(got=%ld) atomic=%d total=%ld", whole, got, atomic, total);
    report("musl_pipe_write_whole_and_atomic", whole && atomic && total == 2 * 64 * 4096, why);
}

/* Blocking sprint: Unix sockets report readiness changes, so a blocked
 * read and an infinite poll wake as soon as the peer writes. */
static void test_unix_socket_wakeups(void)
{
    int sv[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) != 0) {
        report("musl_unix_socket_wakeups", 0, "socketpair failed");
        return;
    }
    pid_t pid = fork();
    if (pid == 0) {
        close(sv[0]);
        struct timespec ms60 = {0, 60 * 1000 * 1000};
        nanosleep(&ms60, 0);
        if (write(sv[1], "a", 1) != 1)
            _exit(1);
        nanosleep(&ms60, 0);
        _exit(write(sv[1], "b", 1) == 1 ? 0 : 2);
    }
    close(sv[1]);
    char c = 0;
    long long t0 = mono_ns();
    int got_a = read(sv[0], &c, 1) == 1 && c == 'a';
    long long waited = mono_ns() - t0;
    struct pollfd pfd = {sv[0], POLLIN, 0};
    int pr = poll(&pfd, 1, -1);
    int got_b = pr == 1 && read(sv[0], &c, 1) == 1 && c == 'b';
    int st = 0;
    waitpid(pid, &st, 0);
    close(sv[0]);
    static char why[112];
    snprintf(why, sizeof(why), "read=%d(%lldms) poll=%d(pr=%d rev=%#x c=%c) child=%d", got_a,
             waited / 1000000, got_b, pr, pfd.revents, c ? c : '0',
             WIFEXITED(st) ? WEXITSTATUS(st) : -1);
    report("musl_unix_socket_wakeups", got_a && got_b && WIFEXITED(st) && WEXITSTATUS(st) == 0, why);
}

/* musl's own paths to the credential, chroot and timestamp calls
 * (setresuid behind seteuid, utimensat behind futimens, getgrouplist):
 * N-248 to N-250. Each runs in a child so the changes do not leak. */
static void test_creds_and_paths(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        int f = 0;
        gid_t g[2] = {20, 30}, got[4];
        uid_t r, e, s;
        if (setgroups(2, g) != 0 || getgroups(4, got) != 2 || got[1] != 30) f |= 1;
        if (setresuid(1000, 0, 0) != 0 || seteuid(1000) != 0 || geteuid() != 1000) f |= 2;
        if (seteuid(0) != 0 || getresuid(&r, &e, &s) != 0 || r != 1000 || e != 0) f |= 4;
        if (setuid(1000) != 0 || seteuid(0) == 0 || errno != EPERM) f |= 8;
        _exit(f);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    static char why[48];
    snprintf(why, sizeof(why), "exit %d", WIFEXITED(st) ? WEXITSTATUS(st) : 255);
    report("musl_setresuid_seteuid_groups", WIFEXITED(st) && WEXITSTATUS(st) == 0, why);

    mkdir("/tmp/musl_jail", 0755);
    FILE *fp = fopen("/tmp/musl_jail/inside", "w");
    if (fp) fclose(fp);
    pid = fork();
    if (pid == 0) {
        int f = 0;
        char buf[32];
        if (chroot("/tmp/musl_jail") != 0 || chdir("/") != 0) f |= 1;
        if (access("/inside", F_OK) != 0) f |= 2;
        if (access("/../tmp", F_OK) == 0) f |= 4;
        if (!getcwd(buf, sizeof(buf)) || strcmp(buf, "/") != 0) f |= 8;
        _exit(f);
    }
    st = 0;
    waitpid(pid, &st, 0);
    snprintf(why, sizeof(why), "exit %d", WIFEXITED(st) ? WEXITSTATUS(st) : 255);
    report("musl_chroot", WIFEXITED(st) && WEXITSTATUS(st) == 0, why);

    int fd = open("/tmp/musl_jail/inside", O_RDWR);
    struct timespec ts[2] = {{1111, 0}, {2222, 0}};
    struct stat sb = {0};
    int ok = fd >= 0 && futimens(fd, ts) == 0 && fstat(fd, &sb) == 0 &&
             sb.st_atime == 1111 && sb.st_mtime == 2222;
    if (fd >= 0) close(fd);
    snprintf(why, sizeof(why), "atime %ld mtime %ld", (long)sb.st_atime, (long)sb.st_mtime);
    report("musl_futimens", ok, why);
}

/* posix_spawn and system: musl runs the child with
 * clone(CLONE_VM | CLONE_VFORK | SIGCHLD) on a stack of its own, which
 * was EINVAL (N-210). An exec failure comes back through a pipe. */
static void test_spawn(void)
{
    extern char **environ;
    pid_t pid = -1;
    char *argv[] = {"sh", "-c", "exit 4", NULL};
    int rc = posix_spawn(&pid, "/bin/sh", NULL, NULL, argv, environ);
    int st = 0;
    int ok = rc == 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 4;
    static char why[64];
    snprintf(why, sizeof(why), "rc %d status %d", rc, st);
    report("musl_posix_spawn", ok, why);

    char *bad[] = {"nope", NULL};
    rc = posix_spawn(&pid, "/no/such/program", NULL, NULL, bad, environ);
    snprintf(why, sizeof(why), "rc %d", rc);
    report("musl_posix_spawn_reports_enoent", rc == ENOENT, why);

    st = system("exit 6");
    snprintf(why, sizeof(why), "status %d", st);
    report("musl_system", WIFEXITED(st) && WEXITSTATUS(st) == 6, why);
}

/* The auxiliary vector (ADR 0010), as getauxval reads it. AT_PLATFORM is
 * Linux's string for the architecture. */
#if defined(__x86_64__)
#define AUXV_PLATFORM "x86_64"
#elif defined(__aarch64__)
#define AUXV_PLATFORM "aarch64"
#else
#define AUXV_PLATFORM "riscv64"
#endif
static void test_auxv(const char *argv0)
{
    const unsigned char *random = (const unsigned char *)getauxval(AT_RANDOM);
    const char *execfn = (const char *)getauxval(AT_EXECFN);
    const char *platform = (const char *)getauxval(AT_PLATFORM);
    int random_set = 0;
    for (int i = 0; random && i < 16; i++)
        random_set |= random[i];
    int ok = getauxval(AT_PAGESZ) == 4096 && random_set && execfn && argv0
             && strcmp(execfn, argv0) == 0 && platform && strcmp(platform, AUXV_PLATFORM) == 0
             && getauxval(AT_PHDR) != 0 && getauxval(AT_PHNUM) > 0
             && getauxval(AT_PHENT) == 56 && getauxval(AT_ENTRY) != 0
             && getauxval(AT_UID) == getuid() && getauxval(AT_EUID) == geteuid()
             && getauxval(AT_GID) == getgid() && getauxval(AT_EGID) == getegid()
             && getauxval(AT_SECURE) == 0 && getauxval(AT_HWCAP) != 0
             && getauxval(AT_MINSIGSTKSZ) >= 1024 && getauxval(AT_CLKTCK) == 100;
    static char why[160];
    snprintf(why, sizeof(why),
             "pagesz=%lu random_set=%d execfn=%s platform=%s phdr=%#lx phnum=%lu entry=%#lx "
             "hwcap=%#lx minsig=%lu",
             getauxval(AT_PAGESZ), random_set, execfn ? execfn : "(null)",
             platform ? platform : "(null)", getauxval(AT_PHDR), getauxval(AT_PHNUM),
             getauxval(AT_ENTRY), getauxval(AT_HWCAP), getauxval(AT_MINSIGSTKSZ));
    report("musl_auxv", ok, why);
}

/* This program is a static PIE: the kernel put it at its PIE base
 * (Linux's ELF_ET_DYN_BASE: 0x5555_5555_4000, or on RISC-V Sv39's
 * 0x2A_AAAA_A000, mm::user_layout), and musl relocated it before main, so
 * code, data and the pointers in data all refer to addresses there. */
#if defined(__riscv)
#define PIE_BASE 0x2aaaaaa000UL
#else
#define PIE_BASE 0x555555554000UL
#endif
static const char *const pie_string = "relocated";
static void test_pie(void)
{
    uintptr_t code = (uintptr_t)&test_pie;
    uintptr_t data = (uintptr_t)&pie_string;
    uintptr_t entry = getauxval(AT_ENTRY);
    int ok = code >= PIE_BASE && code < PIE_BASE + 0x10000000UL && data > code
             && entry >= PIE_BASE && entry < code + 0x10000000UL
             && strcmp(pie_string, "relocated") == 0;
    static char why[96];
    snprintf(why, sizeof(why), "code=%#lx data=%#lx entry=%#lx", (unsigned long)code,
             (unsigned long)data, (unsigned long)entry);
    report("musl_pie_loaded", ok, why);
}

/* A /proc/meminfo field, in kB (-1 if missing). */
static long meminfo_kb(const char *key)
{
    char info[2048];
    int fd = open("/proc/meminfo", O_RDONLY);
    ssize_t n = fd >= 0 ? read(fd, info, sizeof(info) - 1) : -1;
    if (fd >= 0)
        close(fd);
    if (n <= 0)
        return -1;
    info[n] = 0;
    const char *at = strstr(info, key);
    return at ? atol(at + strlen(key)) : -1;
}

/* Private file mappings share the file's cached pages (ADR 0010): the
 * first mapping fills the cache, a second process mapping the file adds
 * nothing and sees the same bytes, a write to a private mapping stays in
 * it (copy-on-write) -- also after mprotect makes a read-only mapping of
 * a cached page writable -- and writing the file drops its cache so a new
 * mapping sees the new contents. */
static void test_page_cache(void)
{
    enum { PAGES = 16, SIZE = PAGES * 4096 };
    const char *path = "/tmp/musl_page_cache";
    static unsigned char data[SIZE];
    for (int i = 0; i < SIZE; i++)
        data[i] = (unsigned char)(i / 4096 + 'a');
    int fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0644);
    if (fd < 0 || write(fd, data, SIZE) != SIZE) {
        report("musl_page_cache", 0, "file setup failed");
        return;
    }
    long c0 = meminfo_kb("Cached:");
    unsigned char *a = mmap(0, SIZE, PROT_READ, MAP_PRIVATE, fd, 0);
    long c1 = meminfo_kb("Cached:");
    int fds[2];
    if (a == MAP_FAILED || pipe(fds) != 0) {
        report("musl_page_cache", 0, "mmap or pipe failed");
        return;
    }
    pid_t pid = fork();
    if (pid == 0) {
        unsigned char *b = mmap(0, SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
        long c2 = meminfo_kb("Cached:");
        int same = b != MAP_FAILED && memcmp(b, data, SIZE) == 0;
        if (b != MAP_FAILED)
            b[100] = 'Z';
        int isolated = b != MAP_FAILED && b[100] == 'Z' && a[100] == 'a';
        /* A read-only mapping of a cached page made writable copies it on
         * the first write: the cache, and so a new mapping, keep the file. */
        int upgraded = mprotect(a, 4096, PROT_READ | PROT_WRITE) == 0;
        if (upgraded)
            a[200] = 'M';
        unsigned char *d = mmap(0, 4096, PROT_READ, MAP_PRIVATE, fd, 0);
        int cache_kept = upgraded && a[200] == 'M' && d != MAP_FAILED && d[200] == 'a';
        (void)!write(fds[1], &c2, sizeof(c2));
        _exit(same && isolated && cache_kept ? 0 : 1);
    }
    int status = -1;
    long c2 = -1;
    int child_ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status)
                   && WEXITSTATUS(status) == 0 && read(fds[0], &c2, sizeof(c2)) == sizeof(c2);
    unsigned char first = 0;
    int file_kept = pread(fd, &first, 1, 100) == 1 && first == 'a' && a[100] == 'a' && a[200] == 'a';
    /* Writing the file drops its cache; a new mapping sees the write. */
    int written = pwrite(fd, "Q", 1, 100) == 1;
    long c3 = meminfo_kb("Cached:");
    unsigned char *c = mmap(0, SIZE, PROT_READ, MAP_PRIVATE, fd, 0);
    int fresh = c != MAP_FAILED && c[100] == 'Q';
    int ok = c1 - c0 == SIZE / 1024 && c2 == c1 && child_ok && file_kept && written
             && c3 == c0 && fresh;
    static char why[160];
    snprintf(why, sizeof(why),
             "Cached kB: before %ld, mapped %ld, child %ld, after write %ld; child_ok=%d "
             "file_kept=%d fresh=%d",
             c0, c1, c2, c3, child_ok, file_kept, fresh);
    report("musl_page_cache", ok, why);
    munmap(a, SIZE);
    if (c != MAP_FAILED)
        munmap(c, SIZE);
    close(fds[0]);
    close(fds[1]);
    close(fd);
    unlink(path);
}

/* Programs share their pages through the cache too (the loader maps a
 * program's whole-file pages from it): the first run of a program nothing
 * has run yet fills the cache, a second run adds nothing. The program is
 * a fresh copy of BusyBox, so none of it is cached before. */
static int run_true(const char *path)
{
    extern char **environ;
    char *args[] = {"busybox", "true", NULL};
    pid_t pid;
    int status;
    return posix_spawn(&pid, path, NULL, NULL, args, environ) == 0
           && waitpid(pid, &status, 0) == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

static void test_program_pages_shared(void)
{
    const char *copy = "/tmp/musl_busybox_copy";
    int in = open("/bin/busybox", O_RDONLY);
    int out = open(copy, O_CREAT | O_WRONLY | O_TRUNC, 0755);
    static char buf[65536];
    ssize_t n = 0;
    int copied = in >= 0 && out >= 0;
    while (copied && (n = read(in, buf, sizeof(buf))) > 0)
        copied = write(out, buf, n) == n;
    copied = copied && n == 0;
    if (in >= 0)
        close(in);
    if (out >= 0)
        close(out);
    long c0 = meminfo_kb("Cached:");
    int first = copied && run_true(copy);
    long c1 = meminfo_kb("Cached:");
    int second = first && run_true(copy);
    long c2 = meminfo_kb("Cached:");
    static char why[112];
    snprintf(why, sizeof(why), "copied %d runs %d/%d, Cached kB %ld, %ld, %ld", copied, first,
             second, c0, c1, c2);
    report("musl_program_pages_shared", first && second && c1 > c0 && c2 == c1, why);
    unlink(copy);
}

/* A dynamically linked C++ program (ADR 0010): the kernel starts it in
 * musl's loader, which maps libstdc++ and libgcc_s; it throws and catches,
 * runs a thread with a thread_local destructor and dlopens a C++ library
 * (userland/tests/musl_dynamic_test.cpp). Its one line of output says how
 * each went. */
static void test_dynamic_program(void)
{
    extern char **environ;
    int fds[2];
    if (pipe(fds) != 0) {
        report("musl_dynamic_program", 0, "pipe failed");
        return;
    }
    posix_spawn_file_actions_t actions;
    posix_spawn_file_actions_init(&actions);
    posix_spawn_file_actions_adddup2(&actions, fds[1], 1);
    posix_spawn_file_actions_addclose(&actions, fds[0]);
    char *args[] = {"musl_dynamic_test", NULL};
    pid_t pid = -1;
    int spawned = posix_spawn(&pid, "/bin/musl_dynamic_test", &actions, NULL, args, environ);
    posix_spawn_file_actions_destroy(&actions);
    close(fds[1]);
    static char out[256];
    size_t len = 0;
    ssize_t n;
    while (len < sizeof(out) - 1 && (n = read(fds[0], out + len, sizeof(out) - 1 - len)) > 0)
        len += (size_t)n;
    out[len] = 0;
    close(fds[0]);
    int status = -1;
    int exited = spawned == 0 && waitpid(pid, &status, 0) == pid;
    char *nl = strchr(out, '\n');
    if (nl)
        *nl = 0;
    static char why[320];
    snprintf(why, sizeof(why), "spawn=%d status=%#x output: %s", spawned, status, out);
    report("musl_dynamic_program",
           exited && WIFEXITED(status) && WEXITSTATUS(status) == 0 && strstr(out, " OK"), why);
}

/* N-232: an event object created non-blocking is non-blocking as a file
 * (its reads used to block in the generic path), O_NONBLOCK set later by
 * fcntl works, and blocking reads wake: an eventfd write from another
 * process, a timerfd expiry (poll and read sleep until it). */
static void test_event_fds(void)
{
    static char why[192];
    int efd = eventfd(0, EFD_NONBLOCK);
    uint64_t v = 0;
    errno = 0;
    int nb_read = efd >= 0 && read(efd, &v, 8) == -1 && errno == EAGAIN;
    int nb_flag = efd >= 0 && (fcntl(efd, F_GETFL) & O_NONBLOCK) != 0;

    int bfd = eventfd(0, 0);
    int set_nb = bfd >= 0 && fcntl(bfd, F_SETFL, O_NONBLOCK) == 0;
    errno = 0;
    int set_nb_read = set_nb && read(bfd, &v, 8) == -1 && errno == EAGAIN;
    fcntl(bfd, F_SETFL, 0);
    pid_t pid = fork();
    if (pid == 0) {
        struct timespec d = {0, 30000000};
        nanosleep(&d, NULL);
        uint64_t one = 7;
        _exit(write(bfd, &one, 8) == 8 ? 0 : 1);
    }
    v = 0;
    int blocking_read = pid > 0 && read(bfd, &v, 8) == 8 && v == 7;
    int status = -1;
    waitpid(pid, &status, 0);
    /* The original eventfd(initval) call: eventfd2 without flags (x86_64
     * only: the generic system call table has no eventfd). */
#ifdef SYS_eventfd
    int lfd = syscall(SYS_eventfd, 3);
    v = 0;
    int legacy = lfd >= 0 && (fcntl(lfd, F_GETFL) & O_NONBLOCK) == 0 && read(lfd, &v, 8) == 8
                 && v == 3;
#else
    int lfd = -1;
    int legacy = 1;
#endif

    int tfd = timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK | TFD_CLOEXEC);
    errno = 0;
    int t_nb = tfd >= 0 && read(tfd, &v, 8) == -1 && errno == EAGAIN;
    struct itimerspec bad = {{0, 0}, {0, 1000000000}};
    errno = 0;
    int t_einval = timerfd_settime(tfd, 0, &bad, NULL) == -1 && errno == EINVAL;
    struct itimerspec soon = {{0, 0}, {0, 40000000}};
    long long t0 = mono_ns();
    timerfd_settime(tfd, 0, &soon, NULL);
    struct pollfd pfd = {tfd, POLLIN, 0};
    int polled = poll(&pfd, 1, 2000) == 1 && (pfd.revents & POLLIN);
    long long waited_ms = (mono_ns() - t0) / 1000000;
    int t_poll = polled && waited_ms >= 35 && waited_ms < 1000 && read(tfd, &v, 8) == 8 && v == 1;
    /* A blocking read sleeps until the expiry. */
    int bt = timerfd_create(CLOCK_MONOTONIC, 0);
    t0 = mono_ns();
    timerfd_settime(bt, 0, &soon, NULL);
    int t_block = read(bt, &v, 8) == 8 && v == 1 && (mono_ns() - t0) / 1000000 >= 35;

    snprintf(why, sizeof(why),
             "efd nonblock read=%d flag=%d; F_SETFL=%d read=%d; blocking=%d; tfd nonblock=%d "
             "einval=%d poll=%d (%lld ms) blocking=%d; eventfd=%d",
             nb_read, nb_flag, set_nb, set_nb_read, blocking_read, t_nb, t_einval, t_poll,
             waited_ms, t_block, legacy);
    report("musl_event_fds",
           nb_read && nb_flag && set_nb && set_nb_read && blocking_read && t_nb && t_einval
               && t_poll && t_block && legacy,
           why);
    close(lfd);
    close(efd);
    close(bfd);
    close(tfd);
    close(bt);
}

/* N-233: a signalfd reads the signals pending for the reader in its mask
 * (blocked, so not delivered), poll reports them, the mask can be changed
 * through the fd, sizemask must be the kernel's sigset size, and a blocking
 * read wakes when a signal arrives. */
static void test_signalfd(void)
{
    static char why[192];
    sigset_t set, old;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    sigaddset(&set, SIGUSR2);
    sigprocmask(SIG_BLOCK, &set, &old);

    sigset_t usr1;
    sigemptyset(&usr1);
    sigaddset(&usr1, SIGUSR1);
    int sfd = signalfd(-1, &usr1, SFD_NONBLOCK | SFD_CLOEXEC);
    struct signalfd_siginfo info;
    errno = 0;
    int empty = sfd >= 0 && read(sfd, &info, sizeof(info)) == -1 && errno == EAGAIN;
    kill(getpid(), SIGUSR1);
    struct pollfd pfd = {sfd, POLLIN, 0};
    int polled = poll(&pfd, 1, 1000) == 1 && (pfd.revents & POLLIN);
    int got = read(sfd, &info, sizeof(info)) == sizeof(info) && info.ssi_signo == SIGUSR1;
    errno = 0;
    int consumed = read(sfd, &info, sizeof(info)) == -1 && errno == EAGAIN;

    /* The mask, changed through the fd: now SIGUSR2. */
    sigset_t usr2;
    sigemptyset(&usr2);
    sigaddset(&usr2, SIGUSR2);
    int same_fd = signalfd(sfd, &usr2, 0) == sfd;
    kill(getpid(), SIGUSR2);
    int got2 = read(sfd, &info, sizeof(info)) == sizeof(info) && info.ssi_signo == SIGUSR2;
    /* The original signalfd(fd, mask, sizemask) call: signalfd4 without
     * flags, so a blocking descriptor (x86_64 only, as eventfd above). */
#ifdef SYS_signalfd
    int lsfd = syscall(SYS_signalfd, -1, &usr2, 8);
    kill(getpid(), SIGUSR2);
    int legacy = lsfd >= 0 && (fcntl(lsfd, F_GETFL) & O_NONBLOCK) == 0
                 && read(lsfd, &info, sizeof(info)) == sizeof(info) && info.ssi_signo == SIGUSR2;
#else
    int lsfd = -1;
    int legacy = 1;
#endif
    /* sizemask must be 8 (the kernel's sigset_t). */
    errno = 0;
    int bad_size = syscall(SYS_signalfd4, -1, &usr1, 4, 0) == -1 && errno == EINVAL;

    /* A blocking read wakes when the signal arrives from another process. */
    int bfd = signalfd(-1, &usr1, 0);
    pid_t pid = fork();
    if (pid == 0) {
        struct timespec d = {0, 30000000};
        nanosleep(&d, NULL);
        _exit(kill(getppid(), SIGUSR1) == 0 ? 0 : 1);
    }
    int blocking = pid > 0 && read(bfd, &info, sizeof(info)) == sizeof(info)
                   && info.ssi_signo == SIGUSR1;
    int status = -1;
    waitpid(pid, &status, 0);

    sigprocmask(SIG_SETMASK, &old, NULL);
    snprintf(why, sizeof(why),
             "empty=%d poll=%d got=%d consumed=%d same_fd=%d got2=%d signalfd=%d bad_size=%d "
             "blocking=%d",
             empty, polled, got, consumed, same_fd, got2, legacy, bad_size, blocking);
    report("musl_signalfd",
           empty && polled && got && consumed && same_fd && got2 && legacy && bad_size
               && blocking,
           why);
    close(lsfd);
    close(sfd);
    close(bfd);
}

/* N-234, N-245: epoll follows Linux -- its errors, registrations that
 * live as long as the open file (not the descriptor number), edge-triggered
 * and one-shot reports, nested epoll files, and epoll_pwait/epoll_pwait2/
 * ppoll with a signal mask for the wait. */
static volatile sig_atomic_t epoll_sig_seen;
static void epoll_on_usr1(int sig)
{
    (void)sig;
    epoll_sig_seen = 1;
}

static void test_epoll(void)
{
    static char why[256];
    struct epoll_event e = {EPOLLIN, {.u64 = 0}}, out[4];

    /* Errors. */
    errno = 0;
    int e_flags = epoll_create1(1) == -1 && errno == EINVAL;
    errno = 0;
#ifdef SYS_epoll_create
    int e_size = syscall(SYS_epoll_create, 0) == -1 && errno == EINVAL;
#else
    int e_size = 1; /* no epoll_create in the generic table */
#endif
    int ep = epoll_create1(EPOLL_CLOEXEC);
    int regular = open("/tmp/musl_epoll_file", O_CREAT | O_RDWR | O_TRUNC, 0644);
    int p[2];
    pipe(p);
    errno = 0;
    int e_perm = epoll_ctl(ep, EPOLL_CTL_ADD, regular, &e) == -1 && errno == EPERM;
    errno = 0;
    int e_self = epoll_ctl(ep, EPOLL_CTL_ADD, ep, &e) == -1 && errno == EINVAL;
    errno = 0;
    int e_notep = epoll_ctl(p[0], EPOLL_CTL_ADD, p[1], &e) == -1 && errno == EINVAL;
    errno = 0;
    int e_noent = epoll_ctl(ep, EPOLL_CTL_MOD, p[0], &e) == -1 && errno == ENOENT;
    int added = epoll_ctl(ep, EPOLL_CTL_ADD, p[0], &e) == 0;
    errno = 0;
    int e_exist = epoll_ctl(ep, EPOLL_CTL_ADD, p[0], &e) == -1 && errno == EEXIST;
    errno = 0;
    int e_badf = epoll_ctl(ep, EPOLL_CTL_DEL, 999, &e) == -1 && errno == EBADF;
    errno = 0;
    int e_wait = epoll_wait(p[0], out, 4, 0) == -1 && errno == EINVAL;
    int errors = e_flags && e_size && e_perm && e_self && e_notep && e_noent && added && e_exist
                 && e_badf && e_wait;
    epoll_ctl(ep, EPOLL_CTL_DEL, p[0], NULL);

    /* A registration lives as long as the open file: a duplicate keeps it
     * reporting after the registered number is closed. */
    int d = dup(p[0]);
    e.events = EPOLLIN;
    e.data.u64 = 77;
    epoll_ctl(ep, EPOLL_CTL_ADD, p[0], &e);
    close(p[0]);
    (void)!write(p[1], "x", 1);
    int n = epoll_wait(ep, out, 4, 0);
    int survives = n == 1 && out[0].data.u64 == 77;
    /* Still readable when its last descriptor closes: nothing reports. */
    close(d);
    int gone = epoll_wait(ep, out, 4, 0) == 0;
    char c;
    close(p[1]);

    /* Edge-triggered: an always-writable pipe end reports once. */
    pipe(p);
    e.events = EPOLLOUT | EPOLLET;
    epoll_ctl(ep, EPOLL_CTL_ADD, p[1], &e);
    int et = epoll_wait(ep, out, 4, 0) == 1 && epoll_wait(ep, out, 4, 0) == 0;
    epoll_ctl(ep, EPOLL_CTL_DEL, p[1], NULL);
    /* One-shot: once, until re-armed. */
    e.events = EPOLLOUT | EPOLLONESHOT;
    epoll_ctl(ep, EPOLL_CTL_ADD, p[1], &e);
    int os = epoll_wait(ep, out, 4, 0) == 1 && epoll_wait(ep, out, 4, 0) == 0
             && epoll_ctl(ep, EPOLL_CTL_MOD, p[1], &e) == 0 && epoll_wait(ep, out, 4, 0) == 1;
    epoll_ctl(ep, EPOLL_CTL_DEL, p[1], NULL);

    /* Nested: an epoll file watching another is readable when it is. */
    int inner = epoll_create1(0);
    e.events = EPOLLIN;
    epoll_ctl(inner, EPOLL_CTL_ADD, p[0], &e);
    e.data.u64 = 5;
    epoll_ctl(ep, EPOLL_CTL_ADD, inner, &e);
    int nested_idle = epoll_wait(ep, out, 4, 0) == 0;
    (void)!write(p[1], "z", 1);
    int nested = nested_idle && epoll_wait(ep, out, 4, 0) == 1 && out[0].data.u64 == 5;
    errno = 0;
    int loop = epoll_ctl(inner, EPOLL_CTL_ADD, ep, &e) == -1 && errno == ELOOP;
    epoll_ctl(ep, EPOLL_CTL_DEL, inner, NULL);
    close(inner);
    (void)!read(p[0], &c, 1);

    /* epoll_pwait2: a timespec timeout. */
    struct timespec ts = {0, 30000000};
    long long t0 = mono_ns();
    int pw2 = syscall(SYS_epoll_pwait2, ep, out, 4, &ts, NULL, 8) == 0
              && (mono_ns() - t0) / 1000000 >= 25;

    /* The wait's signal mask: SIGUSR1 is blocked, pending, and let through
     * only for the wait; the handler runs, the call is EINTR, and SIGUSR1
     * is blocked again afterwards. */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = epoll_on_usr1;
    sigaction(SIGUSR1, &sa, NULL);
    sigset_t usr1, none, now;
    sigemptyset(&usr1);
    sigaddset(&usr1, SIGUSR1);
    sigemptyset(&none);
    sigprocmask(SIG_BLOCK, &usr1, NULL);
    kill(getpid(), SIGUSR1);
    int still_pending = !epoll_sig_seen;
    errno = 0;
    int pw = epoll_pwait(ep, out, 4, 1000, &none) == -1 && errno == EINTR && epoll_sig_seen;
    sigprocmask(SIG_BLOCK, NULL, &now);
    int restored = sigismember(&now, SIGUSR1) == 1;
    /* The same through ppoll. */
    epoll_sig_seen = 0;
    kill(getpid(), SIGUSR1);
    struct pollfd pfd = {p[0], POLLIN, 0};
    struct timespec second = {1, 0};
    errno = 0;
    int pp = ppoll(&pfd, 1, &second, &none) == -1 && errno == EINTR && epoll_sig_seen;
    sigprocmask(SIG_BLOCK, NULL, &now);
    pp = pp && sigismember(&now, SIGUSR1) == 1;
    /* sigsetsize must be 8. */
    errno = 0;
    int bad_size = syscall(SYS_epoll_pwait, ep, out, 4, 0, &none, 4) == -1 && errno == EINVAL;
    sigprocmask(SIG_UNBLOCK, &usr1, NULL);
    sa.sa_handler = SIG_DFL;
    sigaction(SIGUSR1, &sa, NULL);

    snprintf(why, sizeof(why),
             "errors=%d (%d%d%d%d%d%d%d%d%d%d) survives=%d gone=%d et=%d oneshot=%d nested=%d "
             "eloop=%d pwait2=%d pwait=%d/%d/%d ppoll=%d sigsetsize=%d",
             errors, e_flags, e_size, e_perm, e_self, e_notep, e_noent, added, e_exist, e_badf,
             e_wait, survives, gone, et, os, nested, loop, pw2, still_pending, pw, restored, pp,
             bad_size);
    report("musl_epoll",
           errors && survives && gone && et && os && nested && loop && pw2 && still_pending && pw
               && restored && pp && bad_size,
           why);
    close(p[0]);
    close(p[1]);
    close(ep);
    close(regular);
    unlink("/tmp/musl_epoll_file");
}

/* N-194, N-206: select waits and reports readiness as Linux does (it
 * reported every open descriptor ready at once); pselect6 applies its
 * signal mask for the wait; poll reports POLLRDNORM. */
/* The raw select call with a timeval {sec, usec}, written back as select
 * does. The generic system call table (arm64, RISC-V) has no select, so
 * there it is pselect6 with the same time as a timespec, which Linux also
 * writes back. */
static long raw_select(int n, fd_set *r, long tv[2])
{
#ifdef SYS_select
    return syscall(SYS_select, n, r, NULL, NULL, tv);
#else
    long ts[2] = {tv[0], tv[1] * 1000};
    long ret = syscall(SYS_pselect6, n, r, NULL, NULL, ts, NULL);
    tv[0] = ts[0];
    tv[1] = ts[1] / 1000;
    return ret;
#endif
}

static void test_select(void)
{
    static char why[256];
    int p[2];
    pipe(p);
    fd_set r, w;
    struct timeval tv;

    /* Nothing to read: a timed select waits and returns 0, and the raw
     * call writes the time left back (zero here). */
    FD_ZERO(&r);
    FD_SET(p[0], &r);
    long tv_raw[2] = {0, 40000};
    long long t0 = mono_ns();
    int idle = raw_select(p[0] + 1, &r, tv_raw) == 0 && !FD_ISSET(p[0], &r);
    long waited_ms = (long)((mono_ns() - t0) / 1000000);
    int timed = idle && waited_ms >= 35 && tv_raw[0] == 0 && tv_raw[1] == 0;

    /* Readable and writable. */
    (void)!write(p[1], "x", 1);
    FD_ZERO(&r);
    FD_ZERO(&w);
    FD_SET(p[0], &r);
    FD_SET(p[1], &w);
    FD_SET(p[0], &w);
    tv.tv_sec = 0;
    tv.tv_usec = 0;
    int n = select(p[1] + 1, &r, &w, NULL, &tv);
    int ready = n == 2 && FD_ISSET(p[0], &r) && FD_ISSET(p[1], &w) && !FD_ISSET(p[0], &w);
    char c;
    (void)!read(p[0], &c, 1);

    /* A blocking select wakes when another process writes. */
    pid_t pid = fork();
    if (pid == 0) {
        struct timespec d = {0, 30000000};
        nanosleep(&d, NULL);
        _exit(write(p[1], "y", 1) == 1 ? 0 : 1);
    }
    FD_ZERO(&r);
    FD_SET(p[0], &r);
    int woke = select(p[0] + 1, &r, NULL, NULL, NULL) == 1 && FD_ISSET(p[0], &r);
    int status;
    waitpid(pid, &status, 0);
    (void)!read(p[0], &c, 1);

    /* Errors: a closed descriptor is EBADF, a bad timeval EINVAL. */
    FD_ZERO(&r);
    FD_SET(p[1] + 5, &r);
    tv.tv_sec = 0;
    tv.tv_usec = 0;
    errno = 0;
    int ebadf = select(p[1] + 6, &r, NULL, NULL, &tv) == -1 && errno == EBADF;
    long bad_tv[2] = {0, 1000000};
    FD_ZERO(&r);
    errno = 0;
    int einval = raw_select(1, &r, bad_tv) == -1 && errno == EINVAL;

    /* pselect: SIGUSR1 blocked and pending, let through for the wait. */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = epoll_on_usr1;
    sigaction(SIGUSR1, &sa, NULL);
    sigset_t usr1, none, now;
    sigemptyset(&usr1);
    sigaddset(&usr1, SIGUSR1);
    sigemptyset(&none);
    sigprocmask(SIG_BLOCK, &usr1, NULL);
    epoll_sig_seen = 0;
    kill(getpid(), SIGUSR1);
    FD_ZERO(&r);
    FD_SET(p[0], &r);
    struct timespec second = {1, 0};
    errno = 0;
    int ps = pselect(p[0] + 1, &r, NULL, NULL, &second, &none) == -1 && errno == EINTR
             && epoll_sig_seen;
    sigprocmask(SIG_BLOCK, NULL, &now);
    ps = ps && sigismember(&now, SIGUSR1) == 1;
    sigprocmask(SIG_UNBLOCK, &usr1, NULL);
    sa.sa_handler = SIG_DFL;
    sigaction(SIGUSR1, &sa, NULL);

    /* poll: POLLRDNORM alone is reported. */
    (void)!write(p[1], "z", 1);
    struct pollfd pfd = {p[0], POLLRDNORM, 0};
    int rdnorm = poll(&pfd, 1, 0) == 1 && pfd.revents == POLLRDNORM;

    snprintf(why, sizeof(why),
             "timed=%d (%ld ms, left %ld.%06ld) ready=%d (n=%d) woke=%d ebadf=%d einval=%d "
             "pselect=%d rdnorm=%d",
             timed, waited_ms, tv_raw[0], tv_raw[1], ready, n, woke, ebadf, einval, ps, rdnorm);
    report("musl_select", timed && ready && woke && ebadf && einval && ps && rdnorm, why);
    close(p[0]);
    close(p[1]);
}

#ifdef SYS_arch_prctl
/* N-215: arch_prctl refuses an FS base outside user space with EPERM, as
 * Linux (it was EACCES), and leaves the thread's TLS alone; GET_FS reports
 * the base the thread runs with; an unknown code is EINVAL. */
static void test_arch_prctl(void)
{
    static char why[128];
    static __thread int tls_probe = 7;
    unsigned long base = 0;
    int got = syscall(SYS_arch_prctl, 0x1003 /* ARCH_GET_FS */, &base) == 0 && base != 0;
    errno = 0;
    int eperm = syscall(SYS_arch_prctl, 0x1002 /* ARCH_SET_FS */, 0x800000000000UL) == -1
                && errno == EPERM;
    unsigned long after = 0;
    int kept = syscall(SYS_arch_prctl, 0x1003, &after) == 0 && after == base && tls_probe == 7;
    errno = 0;
    int einval = syscall(SYS_arch_prctl, 0x9999, 0) == -1 && errno == EINVAL;
    snprintf(why, sizeof(why), "get=%d eperm=%d kept=%d einval=%d", got, eperm, kept, einval);
    report("musl_arch_prctl", got && eperm && kept && einval, why);
}
#else
/* arch_prctl is x86_64's. */
static void test_arch_prctl(void) {}
#endif

/* N-217: thread IDs and process IDs share one space, as on Linux: the
 * first thread's ID is the process's, other threads get IDs no process has,
 * and a fork child's thread ID is its own PID. */
static pid_t tid_of_thread;
static void *record_tid(void *arg)
{
    (void)arg;
    tid_of_thread = (pid_t)syscall(SYS_gettid);
    return NULL;
}

static void test_tids(void)
{
    static char why[160];
    int leader = syscall(SYS_gettid) == getpid();
    pthread_t t;
    int made = pthread_create(&t, NULL, record_tid, NULL) == 0 && pthread_join(t, NULL) == 0;
    int distinct = made && tid_of_thread > 0 && tid_of_thread != getpid();
    int fds[2];
    pipe(fds);
    pid_t pid = fork();
    if (pid == 0) {
        pid_t pair[2] = {getpid(), (pid_t)syscall(SYS_gettid)};
        (void)!write(fds[1], pair, sizeof(pair));
        _exit(0);
    }
    pid_t pair[2] = {0, 0};
    int status;
    int got = read(fds[0], pair, sizeof(pair)) == sizeof(pair);
    waitpid(pid, &status, 0);
    int child = got && pair[0] == pid && pair[1] == pid && pid != tid_of_thread;
    close(fds[0]);
    close(fds[1]);
    snprintf(why, sizeof(why), "leader=%d thread tid %d (pid %d) child pid %d tid %d", leader,
             (int)tid_of_thread, (int)getpid(), (int)pair[0], (int)pair[1]);
    report("musl_tids", leader && distinct && child, why);
}

/* N-207: flock locks belong to open files (two opens in one process
 * conflict, a dup shares the lock), a blocking request sleeps until the
 * holder lets go, and a signal interrupts it with EINTR. */
static void test_flock_blocking(void)
{
    static char why[160];
    const char *path = "/tmp/musl_flock_blocking";
    int a = open(path, O_CREAT | O_RDWR, 0644);
    int b = open(path, O_RDWR);
    int held = a >= 0 && b >= 0 && flock(a, LOCK_EX) == 0;
    errno = 0;
    int per_file = held && flock(b, LOCK_EX | LOCK_NB) == -1 && errno == EWOULDBLOCK;
    int d = dup(a);
    int shared = flock(d, LOCK_EX | LOCK_NB) == 0;
    close(d);

    /* The child waits for the lock the parent holds; the parent lets go
     * after 40 ms. */
    int fds[2];
    pipe(fds);
    pid_t pid = fork();
    if (pid == 0) {
        int c = open(path, O_RDWR);
        /* Tell the parent the request is about to start, so its 40 ms are
         * measured from here (a slow start, as under emulation, used to eat
         * into them). */
        char ready = 'r';
        (void)!write(fds[1], &ready, 1);
        long long t0 = mono_ns();
        int ok = c >= 0 && flock(c, LOCK_EX) == 0;
        long waited = (long)((mono_ns() - t0) / 1000000);
        (void)!write(fds[1], &waited, sizeof(waited));
        _exit(ok ? 0 : 1);
    }
    char ready = 0;
    if (pid > 0)
        (void)!read(fds[0], &ready, 1);
    struct timespec d40 = {0, 40000000};
    nanosleep(&d40, NULL);
    flock(a, LOCK_UN);
    int status = -1;
    long waited = -1;
    int child_ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status)
                   && WEXITSTATUS(status) == 0 && read(fds[0], &waited, sizeof(waited)) == sizeof(waited);
    int slept = child_ok && waited >= 30;

    /* A signal interrupts the wait: EINTR (no SA_RESTART). */
    flock(a, LOCK_EX);
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = epoll_on_usr1;
    sigaction(SIGUSR1, &sa, NULL);
    epoll_sig_seen = 0;
    pid_t parent = getpid();
    pid = fork();
    if (pid == 0) {
        struct timespec d30 = {0, 30000000};
        nanosleep(&d30, NULL);
        _exit(kill(parent, SIGUSR1) == 0 ? 0 : 1);
    }
    errno = 0;
    int eintr = flock(b, LOCK_EX) == -1 && errno == EINTR && epoll_sig_seen;
    waitpid(pid, &status, 0);
    sa.sa_handler = SIG_DFL;
    sigaction(SIGUSR1, &sa, NULL);

    snprintf(why, sizeof(why), "per_file=%d shared=%d child=%d (waited %ld ms) eintr=%d",
             per_file, shared, child_ok, waited, eintr);
    report("musl_flock_blocking", per_file && shared && slept && eintr, why);
    close(fds[0]);
    close(fds[1]);
    close(a);
    close(b);
    unlink(path);
}

/* N-225: a robust, process-shared mutex whose owner dies holding it is
 * handed to the next locker with EOWNERDEAD (the kernel walks the dead
 * thread's robust list), including one already waiting for it; the list
 * is per thread and get_robust_list reports it. */
static void test_robust_futex(void)
{
    static char why[192];
    int mfd = memfd_create("robust", 0);
    pthread_mutex_t *m = MAP_FAILED;
    if (mfd >= 0 && ftruncate(mfd, 4096) == 0)
        m = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, mfd, 0);
    if (m == MAP_FAILED) {
        report("musl_robust_futex", 0, "shared memory failed");
        return;
    }
    pthread_mutexattr_t a;
    pthread_mutexattr_init(&a);
    pthread_mutexattr_setpshared(&a, PTHREAD_PROCESS_SHARED);
    pthread_mutexattr_setrobust(&a, PTHREAD_MUTEX_ROBUST);
    pthread_mutex_init(m, &a);

    /* The child takes the lock and dies holding it; the parent is already
     * waiting. */
    int fds[2];
    pipe(fds);
    pid_t pid = fork();
    if (pid == 0) {
        pthread_mutex_lock(m);
        (void)!write(fds[1], "L", 1);
        struct timespec d = {0, 50000000};
        nanosleep(&d, NULL);
        _exit(0);
    }
    char c;
    (void)!read(fds[0], &c, 1);
    int r = pthread_mutex_lock(m);
    int owner_dead = r == EOWNERDEAD;
    int recovered = owner_dead && pthread_mutex_consistent(m) == 0 && pthread_mutex_unlock(m) == 0
                    && pthread_mutex_lock(m) == 0 && pthread_mutex_unlock(m) == 0;
    int status;
    waitpid(pid, &status, 0);
    /* musl registered the parent's list at its first robust lock. */
    long head = 0, len = 0;
    int listed = syscall(SYS_get_robust_list, 0, &head, &len) == 0 && head != 0 && len == 24;

    /* Killed holding it (no waiter yet): the next lock is EOWNERDEAD. */
    pid = fork();
    if (pid == 0) {
        pthread_mutex_lock(m);
        (void)!write(fds[1], "L", 1);
        for (;;)
            pause();
    }
    (void)!read(fds[0], &c, 1);
    kill(pid, SIGKILL);
    waitpid(pid, &status, 0);
    int r2 = pthread_mutex_lock(m);
    int killed = r2 == EOWNERDEAD && pthread_mutex_consistent(m) == 0 && pthread_mutex_unlock(m) == 0;

    errno = 0;
    int bad_len = syscall(SYS_set_robust_list, head, 16) == -1 && errno == EINVAL;
    snprintf(why, sizeof(why), "listed=%d (len %ld) lock=%d recovered=%d killed=%d (%d) bad_len=%d",
             listed, len, r, recovered, killed, r2, bad_len);
    report("musl_robust_futex", listed && owner_dead && recovered && killed && bad_len, why);
    munmap(m, 4096);
    close(mfd);
    close(fds[0]);
    close(fds[1]);
}

/* N-221: scheduling policies, priorities and affinity follow Linux (musl's
 * sched_setscheduler/getscheduler/setparam/getparam return ENOSYS by design,
 * since Linux's are per thread; the system calls are used directly): the
 * calls report what was set (they were stubs returning 0), validate their
 * arguments, admit SCHED_DEADLINE reservations up to 95% of the CPU, reset
 * real-time policies in children with SCHED_RESET_ON_FORK, report the CPUs
 * that run tasks, and refuse unprivileged changes with EPERM/EACCES. */
/* N-262: a page replaced under a thread running on another CPU is flushed
 * there. The reader spins on the page from CPU 1 while the main thread
 * unmaps it and maps a new page at the same address with another value;
 * a CPU left with the old translation would keep reading the old frame.
 * (On one CPU it checks the same thing without the remote flush.) Under
 * KVM this cannot catch a missed flush reliably: the hypervisor refreshes
 * a vCPU's TLB when its thread moves between host cores, and with remote
 * flushes disabled the test still passed. mm::tlb's host tests cover the
 * targeting itself. */
static volatile int *tlb_page;
static volatile int tlb_seen_new, tlb_reading, tlb_reader_cpu = -1, tlb_first;
static cpu_set_t tlb_cpu;

/* Between the munmap and the MAP_FIXED mmap nothing is mapped at the
 * page, and a reader preempted into that gap faults (as on Linux): such a
 * fault is retried, a bounded number of times, once the main thread has
 * mapped the page again. Any other fault is fatal as usual. */
static volatile int tlb_gap_faults;

static void tlb_on_segv(int sig, siginfo_t *info, void *ctx)
{
    (void)ctx;
    if (info->si_addr == (void *)tlb_page && ++tlb_gap_faults < 1000) {
        sched_yield();
        return;
    }
    signal(sig, SIG_DFL);
}

static void *tlb_reader(void *arg)
{
    (void)arg;
    struct timespec t0, now;
    sched_setaffinity(0, sizeof(tlb_cpu), &tlb_cpu);
    tlb_reader_cpu = sched_getcpu();
    tlb_first = *tlb_page;
    tlb_reading = 1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    do {
        /* No system calls in the inner loop: nothing on this CPU may
         * reload its translations except the flush under test. */
        for (long i = 0; i < 1000000; i++) {
            if (*tlb_page == 2) {
                tlb_seen_new = 1;
                return 0;
            }
        }
        clock_gettime(CLOCK_MONOTONIC, &now);
    } while ((now.tv_sec - t0.tv_sec) * 1000000000L + (now.tv_nsec - t0.tv_nsec) < 2000000000L);
    return 0;
}

static void test_tlb_remote_flush(void)
{
    long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
    cpu_set_t all, mine;
    sched_getaffinity(0, sizeof(all), &all);
    CPU_ZERO(&mine);
    CPU_SET(0, &mine);
    CPU_ZERO(&tlb_cpu);
    CPU_SET(ncpu > 1 ? 1 : 0, &tlb_cpu);
    sched_setaffinity(0, sizeof(mine), &mine);
    int *p = mmap(0, PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    int ok = p != MAP_FAILED;
    pthread_t t;
    struct sigaction segv, old_segv;
    memset(&segv, 0, sizeof(segv));
    segv.sa_sigaction = tlb_on_segv;
    segv.sa_flags = SA_SIGINFO;
    tlb_gap_faults = 0;
    sigaction(SIGSEGV, &segv, &old_segv);
    if (ok) {
        *p = 1;
        tlb_page = p;
        tlb_seen_new = 0;
        tlb_reading = 0;
        ok = pthread_create(&t, 0, tlb_reader, 0) == 0;
    }
    if (ok) {
        while (!tlb_reading) sched_yield();
        struct timespec d = {0, 20 * 1000000};
        nanosleep(&d, 0); /* the reader has the old translation cached */
        int *q = MAP_FAILED, *blocker = MAP_FAILED;
        if (munmap(p, PG) == 0) {
            /* The freed frame would come straight back for the new page
             * (the per-CPU frame cache), hiding a stale translation: a
             * blocker page takes it first and fills it with 3. */
            blocker = mmap(0, PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (blocker != MAP_FAILED)
                *blocker = 3;
            q = mmap(p, PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
        }
        if (q == p && blocker != MAP_FAILED)
            *q = 2;
        else
            ok = 0;
        pthread_join(t, 0);
        munmap(p, PG);
        if (blocker != MAP_FAILED)
            munmap(blocker, PG);
    }
    sigaction(SIGSEGV, &old_segv, 0);
    sched_setaffinity(0, sizeof(all), &all);
    static char why[128];
    snprintf(why, sizeof(why), "cpus=%ld ok=%d seen_new=%d reader_cpu=%d main_cpu=%d first=%d", ncpu,
             ok, tlb_seen_new, tlb_reader_cpu, sched_getcpu(), tlb_first);
    report("musl_tlb_remote_flush", ok && tlb_seen_new, why);
}

/* N-51: creating an entry checks and creates in the same directory. An
 * unprivileged process swaps the symlink /tmp/n51/link between a directory
 * it may write (ok) and one it may not (prot, root's, 0755) on one thread
 * while creating link/fN on another, each on its own CPU. With the check
 * and the create resolving the path separately, a create could pass the
 * check through ok and land in prot. prot must stay empty. */
static volatile int n51_stop;

static void *n51_swapper(void *arg)
{
    (void)arg;
    while (!n51_stop) {
        symlink("/tmp/n51/prot", "/tmp/n51/to_prot");
        rename("/tmp/n51/to_prot", "/tmp/n51/link");
        symlink("/tmp/n51/ok", "/tmp/n51/to_ok");
        rename("/tmp/n51/to_ok", "/tmp/n51/link");
    }
    return 0;
}

static int n51_count(const char *dir)
{
    DIR *d = opendir(dir);
    int n = 0;
    if (!d) return -1;
    struct dirent *e;
    while ((e = readdir(d)))
        if (strcmp(e->d_name, ".") != 0 && strcmp(e->d_name, "..") != 0) n++;
    closedir(d);
    return n;
}

static void test_dir_check_race(void)
{
    int f = 0;
    mkdir("/tmp", 0777);
    mkdir("/tmp/n51", 0777);
    chmod("/tmp/n51", 0777);
    mkdir("/tmp/n51/prot", 0755);
    chmod("/tmp/n51/prot", 0755);
    mkdir("/tmp/n51/ok", 0777);
    chmod("/tmp/n51/ok", 0777);
    /* What an earlier run left (the test can be repeated with --only). */
    unlink("/tmp/n51/link");
    unlink("/tmp/n51/to_ok");
    unlink("/tmp/n51/to_prot");
    if (symlink("/tmp/n51/ok", "/tmp/n51/link") != 0) f |= 1;
    pid_t pid = fork();
    if (pid == 0) {
        if (setgid(1000) != 0 || setuid(1000) != 0) _exit(2);
        pthread_t t;
        n51_stop = 0;
        if (pthread_create(&t, 0, n51_swapper, 0) != 0) _exit(3);
        char path[64];
        /* 20000 creates or 3 s, whichever comes first: the race shows within
         * a second here, and a slow virtual machine (CI's, where spinning
         * vCPUs wait for a descheduled lock holder) must not run past the
         * suite's time limit. */
        long long stop_at = mono_ns() + 3000000000LL;
        for (int i = 0; i < 20000 && mono_ns() < stop_at; i++) {
            snprintf(path, sizeof(path), "/tmp/n51/link/f%d", i % 64);
            int fd = open(path, O_CREAT | O_WRONLY, 0644);
            if (fd >= 0) close(fd);
        }
        n51_stop = 1;
        pthread_join(t, 0);
        _exit(0);
    }
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0)
        f |= 2;
    int in_prot = n51_count("/tmp/n51/prot");
    int in_ok = n51_count("/tmp/n51/ok");
    if (in_prot != 0) f |= 4;
    if (in_ok <= 0) f |= 8; /* the creates through ok did happen */
    static char why[80];
    snprintf(why, sizeof(why), "fail=%#x in_prot=%d in_ok=%d status=%#x", f, in_prot, in_ok, st);
    report("musl_dir_check_race", f == 0, why);
}

/* Entry creation and removal with relative paths act in the working
 * directory: "sub/x" is under the cwd, not under "/" (a regression in the
 * N-51 change resolved relative parents against "/"). */
static void test_relative_entry_paths(void)
{
    int f = 0;
    char old_cwd[256];
    if (!getcwd(old_cwd, sizeof(old_cwd))) strcpy(old_cwd, "/");
    mkdir("/tmp", 0777);
    mkdir("/tmp/rel", 0777);
    if (chdir("/tmp/rel") != 0) f |= 1;
    if (mkdir("sub", 0755) != 0 && errno != EEXIST) f |= 2;
    int fd = open("sub/new", O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (fd < 0) f |= 4; else close(fd);
    if (link("sub/new", "sub/hard") != 0) f |= 8;
    if (symlink("new", "sub/soft") != 0) f |= 16;
    if (rename("sub/hard", "sub/moved") != 0) f |= 32;
    if (unlink("sub/soft") != 0) f |= 64;
    struct stat st;
    if (stat("/tmp/rel/sub/new", &st) != 0) f |= 128;
    if (stat("/tmp/rel/sub/moved", &st) != 0) f |= 256;
    if (lstat("/tmp/rel/sub/soft", &st) == 0) f |= 512;
    if (stat("/sub", &st) == 0) f |= 1024; /* nothing landed under "/" */
    unlink("/tmp/rel/sub/new");
    unlink("/tmp/rel/sub/moved");
    rmdir("/tmp/rel/sub");
    chdir(old_cwd);
    static char why[48];
    snprintf(why, sizeof(why), "fail=%#x", f);
    report("musl_relative_entry_paths", f == 0, why);
}

/* A signal for a thread running user code on another CPU arrives at once
 * (the sender interrupts that CPU), not at its next timer tick (1 ms):
 * over 50 signals the mean latency stays well under half a tick. */
static volatile int kick_count, kick_stop;
static volatile long long kick_seen_ns;

static void kick_handler(int sig)
{
    (void)sig;
    kick_seen_ns = mono_ns();
    kick_count++;
}

static void *kick_spinner(void *arg)
{
    sched_setaffinity(0, sizeof(cpu_set_t), (cpu_set_t *)arg);
    while (!kick_stop)
        ; /* user mode only: no system call to notice a signal in */
    return 0;
}

static void test_signal_kick(void)
{
    long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
    if (ncpu < 2) {
        report("musl_signal_kick", 1, "one CPU");
        return;
    }
    cpu_set_t all, mine, theirs;
    sched_getaffinity(0, sizeof(all), &all);
    CPU_ZERO(&mine);
    CPU_SET(0, &mine);
    CPU_ZERO(&theirs);
    CPU_SET(1, &theirs);
    sched_setaffinity(0, sizeof(mine), &mine);
    struct sigaction sa, old;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = kick_handler;
    sigaction(SIGUSR1, &sa, &old);
    kick_count = 0;
    kick_stop = 0;
    pthread_t t;
    int ok = pthread_create(&t, 0, kick_spinner, &theirs) == 0;
    long long total = 0, worst = 0;
    int n = 0;
    struct timespec settle = {0, 20 * 1000000};
    nanosleep(&settle, 0); /* the spinner is on CPU 1 */
    for (int i = 0; ok && i < 50; i++) {
        long long t0 = mono_ns();
        if (pthread_kill(t, SIGUSR1) != 0) {
            ok = 0;
            break;
        }
        while (kick_count == i && mono_ns() - t0 < 100000000LL)
            ;
        if (kick_count != i + 1) {
            ok = 0;
            break;
        }
        long long lat = kick_seen_ns - t0;
        total += lat;
        if (lat > worst) worst = lat;
        n++;
    }
    kick_stop = 1;
    if (ok || n) pthread_join(t, 0);
    sigaction(SIGUSR1, &old, 0);
    sched_setaffinity(0, sizeof(all), &all);
    long mean_us = n ? (long)(total / n / 1000) : -1;
    static char why[80];
    snprintf(why, sizeof(why), "ok=%d signals=%d mean=%ldus worst=%ldus", ok, n, mean_us,
             (long)(worst / 1000));
    report("musl_signal_kick", ok && n == 50 && mean_us < 200, why);
}

/* SMP (D5e): a thread restricted to one CPU runs there (getcpu says so)
 * before sched_setaffinity returns, for every CPU in turn; and threads
 * started while the other CPUs are idle are spread over all of them. */
static volatile int spread_stop;
static unsigned long spread_seen;

static void *spread_spin(void *arg)
{
    (void)arg;
    while (!spread_stop) {
        int c = sched_getcpu();
        if (c >= 0 && c < 64) __atomic_fetch_or(&spread_seen, 1UL << c, __ATOMIC_RELAXED);
    }
    return 0;
}

#ifdef SYS_arch_prctl
/* The user GS base belongs to the thread (ARCH_SET_GS/ARCH_GET_GS): two
 * threads sharing one CPU keep their own across switches, and a fork child
 * inherits it. (While the kernel runs, the base sits in a per-CPU register
 * that was not switched with the task.) */
static volatile int gs_go;

static int gs_check(unsigned long mine)
{
    unsigned long got = 0;
    if (syscall(SYS_arch_prctl, 0x1001 /* ARCH_SET_GS */, mine) != 0) return 0;
    for (int i = 0; i < 2000; i++) {
        sched_yield();
        if (syscall(SYS_arch_prctl, 0x1004 /* ARCH_GET_GS */, &got) != 0 || got != mine)
            return 0;
    }
    return 1;
}

static void *gs_thread(void *arg)
{
    (void)arg;
    while (!gs_go) sched_yield();
    return (void *)(long)gs_check(0x7000ab000UL);
}

static void test_gs_base(void)
{
    cpu_set_t one, all;
    sched_getaffinity(0, sizeof(all), &all);
    CPU_ZERO(&one);
    CPU_SET(0, &one);
    sched_setaffinity(0, sizeof(one), &one); /* inherited: both on CPU 0 */
    pthread_t t;
    gs_go = 0;
    int made = pthread_create(&t, 0, gs_thread, 0) == 0;
    gs_go = 1;
    int mine = gs_check(0x7000cd000UL);
    void *theirs = 0;
    if (made) pthread_join(t, &theirs);
    sched_setaffinity(0, sizeof(all), &all);
    unsigned long base = 0;
    int status = 0;
    pid_t pid = fork();
    if (pid == 0) {
        unsigned long b = 0;
        _exit(syscall(SYS_arch_prctl, 0x1004, &b) == 0 && b == 0x7000cd000UL ? 0 : 1);
    }
    int inherited = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status)
                    && WEXITSTATUS(status) == 0;
    errno = 0;
    int eperm = syscall(SYS_arch_prctl, 0x1001, 0x800000000000UL) == -1 && errno == EPERM;
    int kept = syscall(SYS_arch_prctl, 0x1004, &base) == 0 && base == 0x7000cd000UL;
    syscall(SYS_arch_prctl, 0x1001, 0UL);
    static char why[96];
    snprintf(why, sizeof(why), "made=%d mine=%d theirs=%ld inherited=%d eperm=%d kept=%d", made,
             mine, (long)theirs, inherited, eperm, kept);
    report("musl_gs_base_per_thread", made && mine && theirs == (void *)1L && inherited
                                          && eperm && kept, why);
}
#else
/* The GS base is x86_64's. */
static void test_gs_base(void) {}
#endif

/* A new thread gets its creator's mask (it used to get every CPU). */
static void *mask_reader(void *arg)
{
    cpu_set_t got;
    CPU_ZERO(&got);
    long want = (long)arg;
    return (void *)(long)(sched_getaffinity(0, sizeof(got), &got) == 0 && CPU_COUNT(&got) == 1
                          && CPU_ISSET(want, &got) && sched_getcpu() == want);
}

static void test_smp_placement(void)
{
    long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
    int pinned = ncpu >= 1 && ncpu <= 64, where = -1;
    cpu_set_t set;
    for (long c = 0; pinned && c < ncpu; c++) {
        CPU_ZERO(&set);
        CPU_SET(c, &set);
        unsigned cpu = 99, node = 99;
        pinned = sched_setaffinity(0, sizeof(set), &set) == 0
                 && syscall(SYS_getcpu, &cpu, &node, 0) == 0 && cpu == (unsigned)c
                 && node == 0 && sched_getcpu() == c;
        if (!pinned) where = (int)c;
    }
    /* Still pinned to the last CPU: a thread created now is too. */
    pthread_t reader;
    void *inherited = 0;
    if (pthread_create(&reader, 0, mask_reader, (void *)(ncpu - 1)) == 0)
        pthread_join(reader, &inherited);
    CPU_ZERO(&set);
    for (long c = 0; c < ncpu && c < 64; c++) CPU_SET(c, &set);
    sched_setaffinity(0, sizeof(set), &set);

    /* One spinning thread per other CPU, plus this one spinning too. */
    pthread_t t[64];
    int made = 0;
    spread_stop = 0;
    spread_seen = 0;
    for (long i = 1; i < ncpu && i < 64; i++)
        if (pthread_create(&t[made], 0, spread_spin, 0) == 0) made++;
    struct timespec t0, now;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    do {
        int c = sched_getcpu();
        if (c >= 0 && c < 64) __atomic_fetch_or(&spread_seen, 1UL << c, __ATOMIC_RELAXED);
        clock_gettime(CLOCK_MONOTONIC, &now);
    } while ((now.tv_sec - t0.tv_sec) * 1000000000L + (now.tv_nsec - t0.tv_nsec) < 200000000L);
    spread_stop = 1;
    for (int i = 0; i < made; i++) pthread_join(t[i], 0);
    int used = __builtin_popcountl(spread_seen);
    int spread = made == ncpu - 1 && used == ncpu;
    static char why[96];
    snprintf(why, sizeof(why), "cpus=%ld pinned=%d (failed at %d) inherited=%ld threads=%d used=%d",
             ncpu, pinned, where, (long)inherited, made, used);
    report("musl_smp_placement", pinned && inherited == (void *)1L && spread, why);
}

struct test_sched_attr {
    uint32_t size, policy;
    uint64_t flags;
    int32_t nice;
    uint32_t priority;
    uint64_t runtime, deadline, period;
};

/* SMP: busy deadline tasks spread over the CPUs. Each thread reserves
 * 45 ms every 100 ms and never sleeps; spread out, each gets its 45%, and
 * sharing one CPU three or four of them could not (135% or 180%). The
 * placement at wakeup and the pull of waiting deadline tasks spread them.
 * A deadline thread must allow every online CPU, as on Linux: admission
 * counts all of them, so restricted deadline tasks could overload one. */
static volatile int dlb_phase;
static long long dlb_used[4];
static int dlb_ok[4];

static long long thread_cpu_ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &t);
    return (long long)t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void *dlb_thread(void *arg)
{
    long i = (long)arg;
    while (dlb_phase < 1) sched_yield();
    struct test_sched_attr a = {sizeof(a), 6, 0, 0, 0, 45000000, 100000000, 100000000};
    dlb_ok[i] = syscall(SYS_sched_setattr, 0, &a, 0) == 0;
    while (dlb_phase < 2) ;
    /* The clock stands still while the thread does not run, so reading it
     * late (when next scheduled) still reads it at the phase change. */
    long long start = thread_cpu_ns();
    while (dlb_phase < 3) ;
    dlb_used[i] = thread_cpu_ns() - start;
    struct test_sched_attr n = {sizeof(n), SCHED_OTHER, 0, 0, 0, 0, 0, 0};
    syscall(SYS_sched_setattr, 0, &n, 0);
    return 0;
}

/* In a child, so a failure leaves this process's thread as it was: 0 if
 * a restricted thread may not become a deadline one (EPERM), a deadline
 * one may not be restricted (EBUSY), and with every CPU both work. */
static int dlb_mask_rules(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        cpu_set_t all, zero;
        sched_getaffinity(0, sizeof(all), &all);
        CPU_ZERO(&zero);
        CPU_SET(0, &zero);
        struct test_sched_attr a = {sizeof(a), 6, 0, 0, 0, 1000000, 10000000, 10000000};
        int r = 0;
        if (sched_setaffinity(0, sizeof(zero), &zero) != 0) r |= 1;
        errno = 0;
        if (!(syscall(SYS_sched_setattr, 0, &a, 0) == -1 && errno == EPERM)) r |= 2;
        if (sched_setaffinity(0, sizeof(all), &all) != 0) r |= 4;
        if (syscall(SYS_sched_setattr, 0, &a, 0) != 0) r |= 8;
        errno = 0;
        if (!(sched_setaffinity(0, sizeof(zero), &zero) == -1 && errno == EBUSY)) r |= 16;
        if (sched_setaffinity(0, sizeof(all), &all) != 0) r |= 32;
        _exit(r);
    }
    int status = -1;
    if (pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status)) return 64;
    return WEXITSTATUS(status);
}

static void test_dl_balance(void)
{
    long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
    if (ncpu < 2) {
        report("musl_dl_balance", 1, "one CPU");
        return;
    }
    int rules = dlb_mask_rules();
    /* At least three, so that one CPU cannot fit them; at most two per
     * CPU, so that spread out they fit (90%). */
    int want = ncpu >= 4 ? 4 : 3, made = 0;
    pthread_t t[4];
    dlb_phase = 0;
    for (long i = 0; i < want; i++) {
        dlb_used[i] = -1;
        dlb_ok[i] = 0;
        if (pthread_create(&t[made], 0, dlb_thread, (void *)i) == 0) made++;
    }
    dlb_phase = 1;
    struct timespec settle = {0, 100 * 1000000}, window = {0, 600 * 1000000};
    nanosleep(&settle, 0);
    dlb_phase = 2;
    nanosleep(&window, 0);
    dlb_phase = 3;
    for (int i = 0; i < made; i++) pthread_join(t[i], 0);
    int ok = made == want;
    long long least = -1;
    for (int i = 0; i < made; i++) {
        ok = ok && dlb_ok[i];
        if (least < 0 || dlb_used[i] < least) least = dlb_used[i];
    }
    /* 45% of 600 ms is 270 ms. */
    static char why[112];
    snprintf(why, sizeof(why), "cpus=%ld mask rules=%d threads=%d admitted=%d least=%lldms (want >=230)",
             ncpu, rules, made, ok, least / 1000000);
    report("musl_dl_balance", rules == 0 && ok && least >= 230000000LL, why);
}

static void test_sched(void)
{
    static char why[256];
    struct sched_param sp = {0};
    int normal = (int)syscall(SYS_sched_getscheduler, 0) == SCHED_OTHER && (int)syscall(SYS_sched_getparam, 0, &sp) == 0
                 && sp.sched_priority == 0;
    int range = sched_get_priority_max(SCHED_FIFO) == 99 && sched_get_priority_min(SCHED_RR) == 1
                && sched_get_priority_max(SCHED_OTHER) == 0;
    errno = 0;
    int bad_policy = sched_get_priority_max(42) == -1 && errno == EINVAL;
    sp.sched_priority = 0;
    errno = 0;
    int bad_prio = (int)syscall(SYS_sched_setscheduler, 0, SCHED_FIFO, &sp) == -1 && errno == EINVAL;

    sp.sched_priority = 10;
    int fifo = (int)syscall(SYS_sched_setscheduler, 0, SCHED_FIFO, &sp) == 0 && (int)syscall(SYS_sched_getscheduler, 0) == SCHED_FIFO
               && (int)syscall(SYS_sched_getparam, 0, &sp) == 0 && sp.sched_priority == 10;
    sp.sched_priority = 20;
    int setparam = (int)syscall(SYS_sched_setparam, 0, &sp) == 0 && (int)syscall(SYS_sched_getparam, 0, &sp) == 0
                   && sp.sched_priority == 20;

    /* SCHED_RR with reset-on-fork: the child starts as SCHED_OTHER. */
    sp.sched_priority = 5;
    int rr = (int)syscall(SYS_sched_setscheduler, 0, SCHED_RR | SCHED_RESET_ON_FORK, &sp) == 0
             && (int)syscall(SYS_sched_getscheduler, 0) == (SCHED_RR | SCHED_RESET_ON_FORK);
    struct timespec slice = {0, 0};
    int quantum = sched_rr_get_interval(0, &slice) == 0 && slice.tv_sec == 0
                  && slice.tv_nsec == 100000000;
    pid_t pid = fork();
    if (pid == 0)
        _exit((int)syscall(SYS_sched_getscheduler, 0) == SCHED_OTHER ? 0 : 1);
    int status = -1;
    int reset = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status)
                && WEXITSTATUS(status) == 0;

    /* SCHED_DEADLINE through sched_setattr, then admission control. */
    struct test_sched_attr a = {sizeof(a), 6, 0, 0, 0, 1000000, 10000000, 10000000};
    int dl = syscall(SYS_sched_setattr, 0, &a, 0) == 0;
    struct test_sched_attr g;
    memset(&g, 0, sizeof(g));
    dl = dl && syscall(SYS_sched_getattr, 0, &g, sizeof(g), 0) == 0 && g.policy == 6
         && g.runtime == 1000000 && g.deadline == 10000000 && g.period == 10000000;
    pid = fork();
    if (pid == 0) {
        /* Admission keeps the reserved total under 95% of every CPU, and
         * the parent holds 10%: processes reserving 90% each are refused
         * (EBUSY) by the time there is one more of them than CPUs. Each
         * reports its result through a pipe and keeps its reservation
         * until killed. */
        long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
        pid_t kids[64];
        int n = 0, busy = 0;
        for (long i = 0; i <= ncpu && n < 64 && busy == 0; i++) {
            int fd[2];
            if (pipe(fd) != 0) {
                busy = -1;
                break;
            }
            pid_t k = fork();
            if (k == 0) {
                struct test_sched_attr big = {sizeof(big), 6, 0, 0, 0,
                                              9000000, 10000000, 10000000};
                errno = 0;
                char r = syscall(SYS_sched_setattr, 0, &big, 0) == 0 ? 'y'
                         : errno == EBUSY                            ? 'b'
                                                                     : 'e';
                write(fd[1], &r, 1);
                for (;;) pause();
            }
            char r = 'e';
            if (k < 0 || read(fd[0], &r, 1) != 1) r = 'e';
            close(fd[0]);
            close(fd[1]);
            if (k > 0) kids[n++] = k;
            busy = r == 'b' ? 1 : r == 'y' ? 0 : -1;
        }
        for (int i = 0; i < n; i++) {
            kill(kids[i], SIGKILL);
            waitpid(kids[i], 0, 0);
        }
        _exit(busy == 1 ? 0 : 1);
    }
    int admission = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status)
                    && WEXITSTATUS(status) == 0;

    /* Back to SCHED_OTHER with a nice value, read through getpriority. */
    struct test_sched_attr n = {sizeof(n), SCHED_OTHER, 0, 5, 0, 0, 0, 0};
    errno = 0;
    int nice5 = syscall(SYS_sched_setattr, 0, &n, 0) == 0 && getpriority(PRIO_PROCESS, 0) == 5
                && errno == 0 && setpriority(PRIO_PROCESS, 0, 3) == 0
                && getpriority(PRIO_PROCESS, 0) == 3;

    /* The mask holds every CPU that runs tasks, as many as the processor
     * count, and a mask of offline CPUs only is refused. */
    cpu_set_t set;
    CPU_ZERO(&set);
    long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
    int aff = ncpu >= 1 && ncpu < 63 && sched_getaffinity(0, sizeof(set), &set) == 0
              && CPU_COUNT(&set) == ncpu && CPU_ISSET(0, &set);
    CPU_ZERO(&set);
    CPU_SET(63, &set);
    errno = 0;
    int aff_einval = sched_setaffinity(0, sizeof(set), &set) == -1 && errno == EINVAL;
    CPU_ZERO(&set);
    CPU_SET(0, &set);
    int aff_set = sched_setaffinity(0, sizeof(set), &set) == 0;

    /* Unprivileged: no real-time policy (EPERM), no lower nice (EACCES). */
    pid = fork();
    if (pid == 0) {
        if (setgid(1000) != 0 || setuid(1000) != 0)
            _exit(2);
        struct sched_param p;
        memset(&p, 0, sizeof(p));
        p.sched_priority = 1;
        errno = 0;
        int eperm = (int)syscall(SYS_sched_setscheduler, 0, SCHED_FIFO, &p) == -1 && errno == EPERM;
        errno = 0;
        int eacces = setpriority(PRIO_PROCESS, 0, -5) == -1 && errno == EACCES;
        int higher = setpriority(PRIO_PROCESS, 0, 10) == 0;
        _exit(eperm && eacces && higher ? 0 : 1);
    }
    int unpriv = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status)
                 && WEXITSTATUS(status) == 0;
    setpriority(PRIO_PROCESS, 0, 0);

    snprintf(why, sizeof(why),
             "normal=%d range=%d einval=%d/%d fifo=%d setparam=%d rr=%d quantum=%d reset=%d "
             "dl=%d admission=%d nice=%d aff=%d/%d/%d unpriv=%d",
             normal, range, bad_policy, bad_prio, fifo, setparam, rr, quantum, reset, dl,
             admission, nice5, aff, aff_einval, aff_set, unpriv);
    report("musl_sched",
           normal && range && bad_policy && bad_prio && fifo && setparam && rr && quantum && reset
               && dl && admission && nice5 && aff && aff_einval && aff_set && unpriv,
           why);
}

/* The process CPU clock never goes back: a thread's time moves to the
 * process when it exits, including what it runs while tearing down (that
 * part was dropped when its task was reaped, so the clock went back). */
static void *short_burn(void *arg)
{
    (void)arg;
    burn_cpu(2);
    return 0;
}

static long long process_cpu_ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t);
    return (long long)t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void test_process_clock_monotonic(void)
{
    long long last = process_cpu_ns(), worst = 0;
    int backwards = 0, create_err = 0, at = -1;
    for (int i = 0; i < 50; i++) {
        pthread_t t;
        int rc = pthread_create(&t, 0, short_burn, 0);
        if (rc != 0) {
            create_err = rc;
            at = i;
            break;
        }
        pthread_join(t, 0);
        for (int j = 0; j < 200; j++) {
            long long now = process_cpu_ns();
            if (now < last) {
                backwards++;
                if (last - now > worst) worst = last - now;
            }
            last = now > last ? now : last;
        }
    }
    static char why[96];
    snprintf(why, sizeof(why), "went back %d times, by up to %lld ns; create error %d at %d",
             backwards, worst, create_err, at);
    report("musl_process_clock_monotonic", backwards == 0 && create_err == 0, why);
}

/* A process can create and join more threads over its life than it may
 * have at once (256): an exited, joined thread's kernel state is freed.
 * It was kept until the process exited, so the 256th create failed with
 * EAGAIN. */
static void *nop_thread(void *arg)
{
    return arg;
}

static void test_many_threads(void)
{
    int made = 0, err = 0;
    for (int i = 0; i < 600; i++) {
        pthread_t t;
        int rc = pthread_create(&t, 0, nop_thread, 0);
        if (rc != 0) {
            err = rc;
            break;
        }
        pthread_join(t, 0);
        made++;
    }
    static char why[48];
    snprintf(why, sizeof(why), "made %d of 600, error %d", made, err);
    report("musl_many_threads", made == 600, why);
}

/* Fixes from the review of v0.27.0 (PRs #20-#25) that need a real process
 * to show: each check failed, hung or crashed the kernel before its fix. */
static volatile int review_spin_stop;
static volatile unsigned long review_spins;

static void *review_spinner(void *arg)
{
    (void)arg;
    while (!review_spin_stop)
        review_spins++;
    return NULL;
}

static int review_futex_word;

static void *review_futex_waiter(void *arg)
{
    (void)arg;
    struct timespec two = {2, 0};
    syscall(SYS_futex, &review_futex_word, FUTEX_WAIT_PRIVATE, 0, &two, NULL, 0);
    return NULL;
}

static void test_review_fixes(void)
{
    static char why[128];
    int st;

    /* A script naming itself as its interpreter: ELOOP (Linux's
     * BINPRM_MAX_RECURSION), not a kernel stack overflow. */
    {
        const char *path = "/tmp/review_loop.sh";
        int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0755);
        int ok = fd >= 0 && write(fd, "#!/tmp/review_loop.sh\n", 22) == 22;
        if (fd >= 0)
            close(fd);
        pid_t pid = fork();
        if (pid == 0) {
            execl(path, "loop", (char *)0);
            _exit(errno == ELOOP ? 0 : 1);
        }
        ok = ok && pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st)
             && WEXITSTATUS(st) == 0;
        unlink(path);
        snprintf(why, sizeof(why), "status=%#x", st);
        report("review_shebang_loop_is_eloop", ok, why);
    }

    /* A failed exec leaves the other threads running: they used to be
     * ended before the program was even found. */
    {
        pid_t pid = fork();
        if (pid == 0) {
            pthread_t t;
            review_spin_stop = 0;
            if (pthread_create(&t, NULL, review_spinner, NULL) != 0)
                _exit(2);
            usleep(20000);
            int failed = execl("/nonexistent/review", "x", (char *)0) == -1 && errno == ENOENT;
            unsigned long before = review_spins;
            usleep(50000);
            int alive = review_spins != before;
            review_spin_stop = 1;
            pthread_join(t, NULL);
            _exit(failed && alive ? 0 : 1);
        }
        int ok = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;
        snprintf(why, sizeof(why), "status=%#x", st);
        report("review_failed_exec_keeps_threads", ok, why);
    }

    /* A read into memory that ends part-way returns what was copied and
     * moves the file position only by that. */
    {
        const char *path = "/tmp/review_read";
        int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
        static char data[8192];
        memset(data, 'r', sizeof(data));
        int ok = fd >= 0 && write(fd, data, sizeof(data)) == (ssize_t)sizeof(data)
                 && lseek(fd, 0, SEEK_SET) == 0;
        char *p = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        ok = ok && p != MAP_FAILED && munmap(p + 4096, 4096) == 0;
        ssize_t n = ok ? read(fd, p, 8192) : -2;
        off_t at = fd >= 0 ? lseek(fd, 0, SEEK_CUR) : -2;
        ok = ok && n == 4096 && at == 4096;
        if (p != MAP_FAILED)
            munmap(p, 4096);
        if (fd >= 0)
            close(fd);
        unlink(path);
        snprintf(why, sizeof(why), "n=%ld at=%ld", (long)n, (long)at);
        report("review_partial_read_keeps_position", ok, why);
    }

    /* Shrinking a memfd under a MAP_SHARED mapping is EBUSY (the mapping
     * would stop sharing with the file); unmapped, it shrinks. */
    {
        int fd = memfd_create("review", 0);
        int ok = fd >= 0 && ftruncate(fd, 8192) == 0;
        void *p = ok ? mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) : MAP_FAILED;
        errno = 0;
        int busy = p != MAP_FAILED && ftruncate(fd, 0) == -1 && errno == EBUSY;
        if (p != MAP_FAILED)
            munmap(p, 8192);
        ok = ok && busy && ftruncate(fd, 0) == 0;
        if (fd >= 0)
            close(fd);
        snprintf(why, sizeof(why), "busy=%d errno=%d", busy, errno);
        report("review_memfd_shrink_mapped_is_ebusy", ok, why);
    }

    /* O_CREAT | O_DIRECTORY is EINVAL and makes nothing (Linux 6.4+). */
    {
        const char *path = "/tmp/review_odir";
        unlink(path);
        errno = 0;
        int fd = open(path, O_CREAT | O_DIRECTORY | O_RDONLY, 0644);
        int err = errno;
        int ok = fd == -1 && err == EINVAL && access(path, F_OK) == -1;
        if (fd >= 0)
            close(fd);
        unlink(path);
        snprintf(why, sizeof(why), "fd=%d errno=%d", fd, err);
        report("review_creat_directory_is_einval", ok, why);
    }

    /* A completed nanosleep leaves rem alone: a bad pointer there is not
     * EFAULT. */
    {
        struct timespec ts = {0, 1000000};
        long r = syscall(SYS_nanosleep, &ts, (void *)8);
        snprintf(why, sizeof(why), "r=%ld errno=%d", r, errno);
        report("review_nanosleep_leaves_rem_alone", r == 0, why);
    }

    /* A non-blocking write larger than PIPE_BUF into a full pipe is EAGAIN,
     * not 0. */
    {
        int p[2];
        static char chunk[4096];
        int ok = pipe2(p, O_NONBLOCK) == 0;
        while (ok && write(p[1], chunk, sizeof(chunk)) > 0)
            ;
        errno = 0;
        static char big[5000];
        ssize_t w = ok ? write(p[1], big, sizeof(big)) : 0;
        int err = errno;
        ok = ok && w == -1 && err == EAGAIN;
        close(p[0]);
        close(p[1]);
        snprintf(why, sizeof(why), "w=%ld errno=%d", (long)w, err);
        report("review_full_pipe_large_write_is_eagain", ok, why);
    }

    /* poll for POLLOUT on an expired, unread timerfd times out: the past
     * expiry was a wake time, and the wait spun in the kernel. */
    {
        int tfd = timerfd_create(CLOCK_MONOTONIC, 0);
        struct itimerspec its = {{0, 0}, {0, 1000000}};
        int ok = tfd >= 0 && timerfd_settime(tfd, 0, &its, NULL) == 0;
        usleep(5000);
        struct pollfd pfd = {tfd, POLLOUT, 0};
        long t0 = now_ms();
        int r = ok ? poll(&pfd, 1, 100) : -2;
        long took = now_ms() - t0;
        ok = ok && r == 0 && took >= 80;
        if (tfd >= 0)
            close(tfd);
        snprintf(why, sizeof(why), "r=%d took=%ldms", r, took);
        report("review_expired_timerfd_poll_times_out", ok, why);
    }

    /* FUTEX_CMP_REQUEUE onto the same word returns (it looped forever in
     * the kernel with the bucket locked). */
    {
        pthread_t t;
        review_futex_word = 0;
        int ok = pthread_create(&t, NULL, review_futex_waiter, NULL) == 0;
        usleep(20000);
        long r = ok ? syscall(SYS_futex, &review_futex_word, FUTEX_CMP_REQUEUE_PRIVATE, 0,
                              (void *)1000, &review_futex_word, 0)
                    : -2;
        syscall(SYS_futex, &review_futex_word, FUTEX_WAKE_PRIVATE, 1, NULL, NULL, 0);
        if (ok)
            pthread_join(t, NULL);
        snprintf(why, sizeof(why), "r=%ld errno=%d", r, errno);
        report("review_requeue_same_word_returns", ok && r >= 0, why);
    }
}

/* /dev/shm is a tmpfs whose files are their own pages, as on Linux. LTP
 * keeps its results in a file there mapped MAP_SHARED, and children that
 * exec open the file again by path, so a mapping made through a second
 * open must be the same memory. Before, /dev/shm did not exist and a
 * MAP_SHARED mapping of a regular file was a copy per mapping. */
static void test_dev_shm(void)
{
    char why[96];
    const char *path = "/dev/shm/musl_runtime_ipc";
    struct stat st;
    int dir = stat("/dev/shm", &st) == 0 && S_ISDIR(st.st_mode) && (st.st_mode & 07777) == 01777;
    unlink(path);
    int fd = open(path, O_CREAT | O_EXCL | O_RDWR, 0600);
    int ok = dir && fd >= 0 && ftruncate(fd, 4096) == 0;
    volatile char *p = ok ? mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) : MAP_FAILED;
    int err = errno;
    if (fd >= 0)
        close(fd);
    ok = ok && p != MAP_FAILED;
    int seen = -1;
    if (ok) {
        pid_t pid = fork();
        if (pid == 0) {
            int cfd = open(path, O_RDWR);
            char *c = cfd < 0 ? MAP_FAILED : mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, cfd, 0);
            if (c == MAP_FAILED)
                _exit(1);
            c[100] = 0x5a;
            _exit(0);
        }
        int status = 0;
        ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0;
        seen = p[100];
        ok = ok && seen == 0x5a;
        /* read() sees the stores made through the mappings. */
        char b = 0;
        int rfd = open(path, O_RDONLY);
        ok = ok && rfd >= 0 && pread(rfd, &b, 1, 100) == 1 && b == 0x5a;
        if (rfd >= 0)
            close(rfd);
        munmap((void *)p, 4096);
    }
    unlink(path);
    snprintf(why, sizeof(why), "dir=%d fd=%d seen=%d errno=%d", dir, fd, seen, err);
    report("dev_shm_maps_the_file_s_pages", ok, why);
}

/* fallocate (N-208): BlockFS allocates (statfs sees the blocks go), keeps
 * the size with KEEP_SIZE and punches holes that read as zero; an
 * allocation larger than the free space is ENOSPC at once, the file
 * unchanged (LTP's 300 MB loop image); tmpfs grows the file; a memfd's
 * GROW seal refuses it; and the VFS checks come in Linux's order. */
#ifndef FALLOC_FL_ZERO_RANGE
#define FALLOC_FL_ZERO_RANGE 0x10 /* <linux/falloc.h>; musl's fcntl.h lacks it */
#endif
static void test_fallocate(void)
{
    char why[200];
    struct statfs a, b;
    struct stat st;
    long before = -1, after = -1;
    int fd = open("/tmp/musl_falloc", O_CREAT | O_RDWR | O_TRUNC, 0600);
    int ok = fd >= 0 && fstatfs(fd, &a) == 0 && write(fd, "abc", 3) == 3 &&
             fallocate(fd, 0, 0, 8 * 4096) == 0 && fstat(fd, &st) == 0 &&
             st.st_size == 8 * 4096 && fstatfs(fd, &b) == 0;
    if (ok) {
        before = (long)a.f_bfree;
        after = (long)b.f_bfree;
    }
    char buf[8] = {1, 1, 1, 1, 1, 1, 1, 1};
    ok = ok && after <= before - 7 && pread(fd, buf, 8, 0) == 8 && memcmp(buf, "abc\0\0\0\0\0", 8) == 0;
    ok = ok && fallocate(fd, FALLOC_FL_KEEP_SIZE, 8 * 4096, 4096) == 0 && fstat(fd, &st) == 0 &&
         st.st_size == 8 * 4096;
    ok = ok && fallocate(fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE, 1, 1) == 0 &&
         pread(fd, buf, 3, 0) == 3 && memcmp(buf, "a\0c", 3) == 0;
    errno = 0;
    int big = fd >= 0 ? fallocate(fd, 0, 0, (off_t)b.f_bfree * 4096 * 2 + (1 << 20)) : 0;
    int e_big = errno;
    ok = ok && big == -1 && e_big == ENOSPC && fstat(fd, &st) == 0 && st.st_size == 8 * 4096;
    if (fd >= 0)
        close(fd);
    unlink("/tmp/musl_falloc");
    snprintf(why, sizeof(why), "bfree %ld -> %ld big=%d errno=%d size=%ld", before, after, big,
             e_big, (long)st.st_size);
    report("fallocate_blockfs", ok, why);

    fd = open("/dev/shm/musl_falloc", O_CREAT | O_RDWR | O_TRUNC, 0600);
    ok = fd >= 0 && fallocate(fd, 0, 4096, 4096) == 0 && fstat(fd, &st) == 0 &&
         st.st_size == 8192;
    int e_zero = 0;
    errno = 0;
    if (fd >= 0 && fallocate(fd, FALLOC_FL_ZERO_RANGE, 0, 1) != 0)
        e_zero = errno;
    ok = ok && e_zero == EOPNOTSUPP;
    if (fd >= 0)
        close(fd);
    unlink("/dev/shm/musl_falloc");
    int mfd = memfd_create("falloc", MFD_ALLOW_SEALING);
    int e_seal = 0;
    ok = ok && mfd >= 0 && fallocate(mfd, 0, 0, 4096) == 0 &&
         fcntl(mfd, F_ADD_SEALS, F_SEAL_GROW) == 0;
    errno = 0;
    if (mfd >= 0 && fallocate(mfd, FALLOC_FL_KEEP_SIZE, 0, 8192) != 0)
        e_seal = errno;
    ok = ok && e_seal == EPERM && fallocate(mfd, 0, 0, 4096) == 0;
    if (mfd >= 0)
        close(mfd);
    snprintf(why, sizeof(why), "zero_range errno=%d grow-sealed errno=%d", e_zero, e_seal);
    report("fallocate_tmpfs_memfd", ok, why);

    /* EINVAL before EOPNOTSUPP before EBADF; then the file type. */
    int ro = open("/tmp", O_RDONLY | O_DIRECTORY), p[2] = {-1, -1};
    int e[6] = {0};
    errno = 0;
    if (fallocate(-1, 0x100, 0, 0) != 0)
        e[0] = errno;
    errno = 0;
    if (fallocate(-1, 0x100, 0, 1) != 0)
        e[1] = errno;
    errno = 0;
    if (fallocate(-1, FALLOC_FL_PUNCH_HOLE, 0, 1) != 0)
        e[2] = errno;
    errno = 0;
    if (fallocate(ro, 0, 0, 1) != 0)
        e[3] = errno;
    if (pipe(p) == 0) {
        errno = 0;
        if (fallocate(p[1], 0, 0, 1) != 0)
            e[4] = errno;
    }
    fd = open("/tmp/musl_falloc", O_CREAT | O_RDWR | O_TRUNC, 0600);
    errno = 0;
    if (fd >= 0 && fallocate(fd, 0, 0x7fffffffffffffffLL, 1) != 0)
        e[5] = errno;
    ok = e[0] == EINVAL && e[1] == EOPNOTSUPP && e[2] == EOPNOTSUPP && e[3] == EBADF &&
         e[4] == ESPIPE && e[5] == EFBIG;
    for (int i = 0; i < 2; i++)
        if (p[i] >= 0)
            close(p[i]);
    if (ro >= 0)
        close(ro);
    if (fd >= 0)
        close(fd);
    unlink("/tmp/musl_falloc");
    snprintf(why, sizeof(why), "errnos %d %d %d %d %d %d", e[0], e[1], e[2], e[3], e[4], e[5]);
    report("fallocate_checks_in_linux_order", ok, why);
}

/* utimensat with two UTIME_OMIT times returns 0 without looking at the
 * path, and on a descriptor (NULL path) takes no flags. x86_64 also has
 * utime, utimes and futimesat, which musl routes through utimensat, so
 * they are called directly: whole seconds, microseconds (EINVAL outside
 * 0..999999, UTIME_NOW included), a path under a directory fd, and a NULL
 * path acting on the fd. */
static void test_utimes(void)
{
    char why[200];
    struct timespec omit[2] = {{0, UTIME_OMIT}, {0, UTIME_OMIT}};
    int fd = open("/tmp/musl_utimes", O_CREAT | O_RDWR | O_TRUNC, 0600);
    errno = 0;
    int omit_ok = utimensat(AT_FDCWD, "/tmp/no/such/file", omit, 0) == 0;
    errno = 0;
    long r = fd >= 0 ? syscall(SYS_utimensat, fd, NULL, NULL, AT_SYMLINK_NOFOLLOW) : 0;
    int e_flags = errno;
    snprintf(why, sizeof(why), "omit=%d fd+flags=%ld errno=%d", omit_ok, r, e_flags);
    report("utimensat_omit_and_fd_flags", fd >= 0 && omit_ok && r == -1 && e_flags == EINVAL,
           why);
#if defined(__x86_64__)
    struct stat st;
    long t[2] = {100, 200}; /* struct utimbuf: actime, modtime */
    int ok = fd >= 0 && syscall(SYS_utime, "/tmp/musl_utimes", t) == 0 &&
             stat("/tmp/musl_utimes", &st) == 0 && st.st_atime == 100 && st.st_mtime == 200;
    int step = ok ? 1 : 0;
    struct timeval tv[2] = {{300, 5}, {400, 999999}};
    ok = ok && syscall(SYS_utimes, "/tmp/musl_utimes", tv) == 0 &&
         stat("/tmp/musl_utimes", &st) == 0 && st.st_atime == 300 && st.st_mtime == 400;
    step += ok;
    int dir = open("/tmp", O_RDONLY | O_DIRECTORY);
    tv[0].tv_sec = 500;
    tv[1].tv_sec = 600;
    ok = ok && dir >= 0 && syscall(SYS_futimesat, dir, "musl_utimes", tv) == 0 &&
         fstat(fd, &st) == 0 && st.st_atime == 500 && st.st_mtime == 600;
    step += ok;
    tv[0].tv_sec = 700;
    tv[1].tv_sec = 800;
    ok = ok && syscall(SYS_futimesat, fd, NULL, tv) == 0 && fstat(fd, &st) == 0 &&
         st.st_atime == 700 && st.st_mtime == 800;
    step += ok;
    int e[2] = {0, 0};
    tv[1].tv_usec = 1000000;
    errno = 0;
    if (syscall(SYS_utimes, "/tmp/musl_utimes", tv) != 0)
        e[0] = errno;
    tv[1].tv_usec = UTIME_NOW;
    errno = 0;
    if (syscall(SYS_futimesat, fd, NULL, tv) != 0)
        e[1] = errno;
    ok = ok && e[0] == EINVAL && e[1] == EINVAL && fstat(fd, &st) == 0 && st.st_mtime == 800 &&
         syscall(SYS_utime, "/tmp/musl_utimes", NULL) == 0 &&
         syscall(SYS_utimes, "/tmp/musl_utimes", NULL) == 0;
    if (dir >= 0)
        close(dir);
    snprintf(why, sizeof(why), "steps=%d einval=%d,%d atime=%ld mtime=%ld", step, e[0], e[1],
             (long)st.st_atime, (long)st.st_mtime);
    report("legacy_utime_utimes_futimesat", ok, why);
#endif
    if (fd >= 0)
        close(fd);
    unlink("/tmp/musl_utimes");
}

/* statfs and fstatfs: each filesystem reports Linux's magic number, tmpfs
 * its page counts against the size limit, and pipes, sockets and
 * anonymous-inode files their pseudo filesystems (fs/statfs.rs). */
static void test_statfs(void)
{
    char why[160];
    struct statfs a, b;
    int ok = statfs("/dev/shm", &a) == 0 && a.f_type == 0x01021994 && a.f_bsize == 4096 &&
             a.f_blocks > 0 && a.f_bavail == a.f_bfree && a.f_namelen == 255 &&
             a.f_frsize == 4096 && (a.f_flags & 0x20);
    long before = ok ? (long)a.f_bfree : -1, after = -1;
    int fd = open("/dev/shm/musl_statfs", O_CREAT | O_RDWR | O_TRUNC, 0600);
    static char chunk[16 * 4096];
    memset(chunk, 1, sizeof(chunk));
    ok = ok && fd >= 0 && write(fd, chunk, sizeof(chunk)) == (ssize_t)sizeof(chunk) &&
         fstatfs(fd, &b) == 0;
    if (ok)
        after = (long)b.f_bfree;
    ok = ok && b.f_type == 0x01021994 && after <= before - 16;
    if (fd >= 0)
        close(fd);
    unlink("/dev/shm/musl_statfs");
    snprintf(why, sizeof(why), "type=%#lx bfree %ld -> %ld errno=%d", (unsigned long)a.f_type,
             before, after, errno);
    report("statfs_tmpfs_counts_pages", ok, why);

    memset(&a, 0, sizeof(a));
    memset(&b, 0, sizeof(b));
    ok = statfs("/proc", &a) == 0 && a.f_type == 0x9fa0 && a.f_blocks == 0 &&
         statfs("/", &b) == 0 && b.f_type == 0x424C4B46 && b.f_blocks > 0 &&
         b.f_bfree <= b.f_blocks && b.f_files > 0;
    snprintf(why, sizeof(why), "proc=%#lx root=%#lx blocks=%ld errno=%d",
             (unsigned long)a.f_type, (unsigned long)b.f_type, (long)b.f_blocks, errno);
    report("statfs_names_each_filesystem", ok, why);

    int p[2] = {-1, -1}, sv[2] = {-1, -1};
    int efd = eventfd(0, 0);
    unsigned long pt = 0, st = 0, et = 0;
    ok = pipe(p) == 0 && socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0 && efd >= 0;
    if (ok && fstatfs(p[0], &a) == 0)
        pt = a.f_type;
    if (ok && fstatfs(sv[0], &a) == 0)
        st = a.f_type;
    if (ok && fstatfs(efd, &a) == 0)
        et = a.f_type;
    ok = ok && pt == 0x50495045 && st == 0x534F434B && et == 0x09041934;
    for (int i = 0; i < 2; i++) {
        if (p[i] >= 0)
            close(p[i]);
        if (sv[i] >= 0)
            close(sv[i]);
    }
    if (efd >= 0)
        close(efd);
    snprintf(why, sizeof(why), "pipe=%#lx socket=%#lx eventfd=%#lx", pt, st, et);
    report("fstatfs_pseudo_filesystems", ok, why);

    int e1 = 0, e2 = 0, e3 = 0;
    errno = 0;
    if (statfs("/no/such/path", &a) != 0)
        e1 = errno;
    /* musl's statfs clears *buf before the call, so a bad buffer goes to
     * the kernel through syscall(). */
    errno = 0;
    if (syscall(SYS_statfs, "/", (struct statfs *)8) != 0)
        e2 = errno;
    errno = 0;
    if (fstatfs(-1, &a) != 0)
        e3 = errno;
    /* A trailing slash needs a directory, as for stat (PR #34 review). */
    int e4 = 0;
    errno = 0;
    if (statfs("/bin/sh/", &a) != 0)
        e4 = errno;
    ok = e1 == ENOENT && e2 == EFAULT && e3 == EBADF && e4 == ENOTDIR;
    snprintf(why, sizeof(why), "enoent=%d efault=%d ebadf=%d enotdir=%d", e1, e2, e3, e4);
    report("statfs_errors", ok, why);
}

static void test_record_locks(void);
static void test_fcntl_flags(void);
static void test_vectored_io(void);
static void test_link_errors(void);
static void test_stat_numbers(void);
static void test_open_flags(void);
static void test_text_busy(void);
static void test_inotify_events(void);
static void test_pidfd(void);
static void test_clone3(void);
static void test_clone_share(void);
static void test_kcmp_process_vm(void);
static void test_personality(void);
static void test_namespaces_uts(void);
static void test_namespaces_time(void);
static void test_namespaces_mnt(void);
static void test_mount_calls(void);
static void test_mount_flags(void);
static void test_mount_no_escape(void);
static void test_pivot_root(void);
static void test_user_ns(void);
static void test_pid_ns(void);
static void test_net_ns(void);

/* Tests `--only NAME` can run alone (for debugging a single test). */
static const struct {
    const char *name;
    void (*fn)(void);
} only_tests[] = {
    {"review_fixes", test_review_fixes},
    {"record_locks", test_record_locks},
    {"fcntl_flags", test_fcntl_flags},
    {"vectored_io", test_vectored_io},
    {"link_errors", test_link_errors},
    {"stat_numbers", test_stat_numbers},
    {"open_flags", test_open_flags},
    {"text_busy", test_text_busy},
    {"inotify_events", test_inotify_events},
    {"pidfd", test_pidfd},
    {"clone3", test_clone3},
    {"clone_share", test_clone_share},
    {"kcmp_process_vm", test_kcmp_process_vm},
    {"personality", test_personality},
    {"namespaces_uts", test_namespaces_uts},
    {"namespaces_time", test_namespaces_time},
    {"namespaces_mnt", test_namespaces_mnt},
    {"mount_calls", test_mount_calls},
    {"mount_flags", test_mount_flags},
    {"pivot_root", test_pivot_root},
    {"user_ns", test_user_ns},
    {"pid_ns", test_pid_ns},
    {"net_ns", test_net_ns},
    {"threads", test_threads},
    {"cow_race", test_cow_race},
    {"signals", test_signals},
    {"futex_queues", test_futex_queues},
    {"blocking_io", test_blocking_io},
    {"unix_socket_wakeups", test_unix_socket_wakeups},
    {"robust_futex", test_robust_futex},
    {"sched", test_sched},
    {"smp_placement", test_smp_placement},
    {"dl_balance", test_dl_balance},
    {"gs_base", test_gs_base},
    {"tlb_remote_flush", test_tlb_remote_flush},
    {"clocks_and_usage", test_clocks_and_usage},
    {"process_clock_monotonic", test_process_clock_monotonic},
    {"many_threads", test_many_threads},
    {"signal_kick", test_signal_kick},
    {"dir_check_race", test_dir_check_race},
    {"relative_entry_paths", test_relative_entry_paths},
    {"priority_inheritance", test_priority_inheritance},
    {"unix_accept_blocks", test_unix_accept_blocks},
    {"fs_ids", test_fs_ids},
    {"dev_shm_maps_the_file_s_pages", test_dev_shm},
    {"statfs", test_statfs},
};

/* Interval timers (setitimer, getitimer, alarm): ITIMER_REAL sends SIGALRM
 * and an interval one keeps going (re-armed as each SIGALRM is taken);
 * getitimer reads back what was set; ITIMER_PROF fires on CPU time; bad
 * arguments are EINVAL; alarm returns the seconds left of the timer it
 * replaces; a fork child starts with none. */
static volatile int alrm_count, prof_count;
static void count_alrm(int sig)
{
    (void)sig;
    alrm_count++;
}
static void count_prof(int sig)
{
    (void)sig;
    prof_count++;
}

static void test_itimers(void)
{
    char why[200];
    struct sigaction sa, old_alrm, old_prof;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = count_alrm;
    sigaction(SIGALRM, &sa, &old_alrm);
    sa.sa_handler = count_prof;
    sigaction(SIGPROF, &sa, &old_prof);

    /* A 20 ms interval timer: three SIGALRMs, then disarmed. */
    struct itimerval it = {{0, 20000}, {0, 20000}}, got;
    int set = setitimer(ITIMER_REAL, &it, NULL) == 0;
    int got_ok = getitimer(ITIMER_REAL, &got) == 0 && got.it_interval.tv_sec == 0 &&
               got.it_interval.tv_usec == 20000 && got.it_value.tv_sec == 0 &&
               got.it_value.tv_usec > 0 && got.it_value.tv_usec <= 20000;
    for (int i = 0; i < 200 && alrm_count < 3; i++)
        usleep(10000);
    struct itimerval zero = {{0, 0}, {0, 0}};
    int disarmed = setitimer(ITIMER_REAL, &zero, &got) == 0 && got.it_interval.tv_usec == 20000;
    int count = alrm_count;
    usleep(60000);
    int stopped = alrm_count == count && getitimer(ITIMER_REAL, &got) == 0 &&
                  got.it_value.tv_sec == 0 && got.it_value.tv_usec == 0 &&
                  got.it_interval.tv_usec == 0;
    snprintf(why, sizeof(why), "set=%d read=%d count=%d disarmed=%d stopped=%d", set, got_ok, count,
             disarmed, stopped);
    report("itimer_real_interval", set && got_ok && count >= 3 && disarmed && stopped, why);

    /* ITIMER_PROF on CPU time: a busy loop gets SIGPROF. */
    struct itimerval prof = {{0, 0}, {0, 10000}};
    int pset = setitimer(ITIMER_PROF, &prof, NULL) == 0;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    volatile unsigned long spin = 0;
    do {
        for (int i = 0; i < 100000; i++)
            spin++;
        clock_gettime(CLOCK_MONOTONIC, &t1);
    } while (prof_count == 0 && t1.tv_sec - t0.tv_sec < 5);
    (void)spin;
    int pread = getitimer(ITIMER_PROF, &got) == 0 && got.it_value.tv_sec == 0 &&
                got.it_value.tv_usec == 0;
    snprintf(why, sizeof(why), "set=%d fired=%d disarmed=%d", pset, prof_count, pread);
    report("itimer_prof_counts_cpu_time", pset && prof_count == 1 && pread, why);

    /* EINVAL: a bad timer, a bad tv_usec, a negative tv_sec. */
    int e[3] = {0, 0, 0};
    struct itimerval bad = {{0, 0}, {0, 1000000}};
    errno = 0;
    if (setitimer(3, &zero, NULL) != 0)
        e[0] = errno;
    errno = 0;
    if (setitimer(ITIMER_REAL, &bad, NULL) != 0)
        e[1] = errno;
    bad.it_value.tv_usec = 0;
    bad.it_value.tv_sec = -1;
    errno = 0;
    if (setitimer(ITIMER_VIRTUAL, &bad, NULL) != 0)
        e[2] = errno;
    errno = 0;
    int gbad = getitimer(3, &got) == -1 && errno == EINVAL;
    int einval = e[0] == EINVAL && e[1] == EINVAL && e[2] == EINVAL && gbad;

    /* alarm: seconds left of the timer it replaces, rounded up. */
    unsigned a0 = alarm(10);
    unsigned a1 = alarm(0);
    struct itimerval half = {{0, 0}, {2, 600000}};
    setitimer(ITIMER_REAL, &half, NULL);
    unsigned a2 = alarm(0);
    int alarms = a0 == 0 && a1 == 10 && a2 == 3;

    /* A fork child has no timers. */
    struct itimerval ten = {{0, 0}, {10, 0}};
    setitimer(ITIMER_REAL, &ten, NULL);
    int fds[2] = {-1, -1};
    pipe(fds);
    pid_t pid = fork();
    if (pid == 0) {
        struct itimerval c;
        char r = getitimer(ITIMER_REAL, &c) == 0 && c.it_value.tv_sec == 0 && c.it_value.tv_usec == 0;
        write(fds[1], &r, 1);
        _exit(0);
    }
    char child = 0;
    if (pid > 0) {
        read(fds[0], &child, 1);
        waitpid(pid, NULL, 0);
    }
    alarm(0);
    close(fds[0]);
    close(fds[1]);
    sigaction(SIGALRM, &old_alrm, NULL);
    sigaction(SIGPROF, &old_prof, NULL);
    snprintf(why, sizeof(why), "einval=%d,%d,%d,%d alarm=%u,%u,%u child=%d", e[0], e[1], e[2],
             gbad, a0, a1, a2, child);
    report("itimer_checks_alarm_and_fork", einval && alarms && child == 1, why);
}

/* POSIX timers (timer_create, timer_settime, timer_gettime,
 * timer_getoverrun, timer_delete): a periodic timer's signal carries
 * SI_TIMER and its value; a blocked periodic timer is queued once and
 * reports the expiries missed as its overrun; SIGEV_NONE counts down;
 * TIMER_ABSTIME in the past fires at once; a deleted timer's pending
 * signal is dropped; an ignored periodic timer is parked and resumes when
 * caught again; CPU-clock timers fire on CPU time; a fork child has none. */
static volatile int ptimer_count, ptimer_code, ptimer_value;
static void ptimer_handler(int sig, siginfo_t *si, void *uc)
{
    (void)sig;
    (void)uc;
    ptimer_code = si->si_code;
    ptimer_value = si->si_value.sival_int;
    ptimer_count++;
}

static int ptimer_make(clockid_t clock, int notify, int sig, int value, timer_t *t)
{
    struct sigevent sev;
    memset(&sev, 0, sizeof(sev));
    sev.sigev_notify = notify;
    sev.sigev_signo = sig;
    sev.sigev_value.sival_int = value;
    return timer_create(clock, &sev, t);
}

static int ptimer_wait_count(int n)
{
    for (int i = 0; i < 300 && ptimer_count < n; i++)
        usleep(10000);
    return ptimer_count >= n;
}

static void test_posix_timers(void)
{
    char why[256];
    int sig = SIGRTMIN + 3;
    struct sigaction sa, old_sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_sigaction = ptimer_handler;
    sa.sa_flags = SA_SIGINFO;
    sigaction(sig, &sa, &old_sa);

    /* Periodic, caught: SI_TIMER with the timer's value. */
    timer_t t;
    int made = ptimer_make(CLOCK_MONOTONIC, SIGEV_SIGNAL, sig, 4242, &t) == 0;
    struct itimerspec ten_ms = {{0, 10000000}, {0, 10000000}}, cur;
    int set = made && timer_settime(t, 0, &ten_ms, NULL) == 0;
    int fired = set && ptimer_wait_count(3);
    int got = timer_gettime(t, &cur) == 0 && cur.it_interval.tv_nsec == 10000000 &&
              cur.it_value.tv_sec == 0 && cur.it_value.tv_nsec > 0 &&
              cur.it_value.tv_nsec <= 10000000;
    int del = timer_delete(t) == 0;
    snprintf(why, sizeof(why), "made=%d set=%d fired=%d(%d) code=%d value=%d get=%d del=%d", made,
             set, fired, ptimer_count, ptimer_code, ptimer_value, got, del);
    report("posix_timer_periodic_siginfo",
           made && set && fired && ptimer_code == SI_TIMER && ptimer_value == 4242 && got && del,
           why);

    /* Blocked periodic: queued once, the missed expiries as its overrun,
     * and the same through signalfd. */
    sigset_t set1, old_mask;
    sigemptyset(&set1);
    sigaddset(&set1, sig);
    sigprocmask(SIG_BLOCK, &set1, &old_mask);
    struct itimerspec five_ms = {{0, 5000000}, {0, 5000000}};
    made = ptimer_make(CLOCK_MONOTONIC, SIGEV_SIGNAL, sig, 7, &t) == 0;
    set = made && timer_settime(t, 0, &five_ms, NULL) == 0;
    usleep(60000);
    struct timespec wait = {1, 0};
    siginfo_t si;
    memset(&si, 0, sizeof(si));
    int taken = sigtimedwait(&set1, &si, &wait) == sig;
    /* Taking the signal re-arms the timer, so on a slow machine its next
     * expiry can be queued before the second wait (seen on RISC-V): a
     * second signal is a duplicate only if it came before that expiry.
     * The gap is measured from before timer_gettime, so a stall between
     * the two (seen on AArch64 with four emulated CPUs) is counted. */
    struct itimerspec next;
    struct timespec zero = {0, 0}, w0, w1;
    clock_gettime(CLOCK_MONOTONIC, &w0);
    long next_ns = timer_gettime(t, &next) == 0 ? next.it_value.tv_nsec : 0;
    int overrun = si.si_overrun, getover = timer_getoverrun(t);
    errno = 0;
    int again = sigtimedwait(&set1, &si, &zero);
    int again_errno = errno;
    int eagain = again == -1 && again_errno == EAGAIN;
    clock_gettime(CLOCK_MONOTONIC, &w1);
    long gap = (w1.tv_sec - w0.tv_sec) * 1000000000L + (w1.tv_nsec - w0.tv_nsec);
    int once = eagain || (again == sig && gap + 1000000 >= next_ns);
    int fd = signalfd(-1, &set1, SFD_CLOEXEC);
    struct signalfd_siginfo ssi;
    memset(&ssi, 0, sizeof(ssi));
    usleep(20000);
    int sfd = fd >= 0 && read(fd, &ssi, sizeof(ssi)) == (ssize_t)sizeof(ssi) &&
              ssi.ssi_code == SI_TIMER && ssi.ssi_tid == (unsigned)(intptr_t)t &&
              ssi.ssi_int == 7 && ssi.ssi_overrun >= 1;
    if (fd >= 0)
        close(fd);
    struct itimerspec old;
    struct itimerspec off = {{0, 0}, {0, 0}};
    int oldok = timer_settime(t, 0, &off, &old) == 0 && old.it_interval.tv_nsec == 5000000;
    timer_delete(t);
    snprintf(why, sizeof(why),
             "made=%d set=%d taken=%d overrun=%d getoverrun=%d once=%d(%d,%d,%ld,%ld) "
             "sfd=%d(%d,%u,%u) old=%d",
             made, set, taken, overrun, getover, once, again, again_errno, gap, next_ns, sfd,
             ssi.ssi_code, ssi.ssi_tid, ssi.ssi_overrun, oldok);
    report("posix_timer_overrun_when_blocked",
           made && set && taken && overrun >= 5 && getover == overrun && once && sfd && oldok,
           why);

    /* SIGEV_NONE counts down; TIMER_ABSTIME in the past fires at once; a
     * deleted timer's pending signal is dropped; the default sigevent is
     * SIGALRM with the timer id. */
    timer_t n;
    int none = ptimer_make(CLOCK_MONOTONIC, SIGEV_NONE, 0, 0, &n) == 0;
    struct itimerspec sec = {{0, 0}, {1, 0}}, ms = {{0, 0}, {0, 1000000}};
    none = none && timer_settime(n, 0, &sec, NULL) == 0 && timer_gettime(n, &cur) == 0 &&
           cur.it_value.tv_sec == 0 && cur.it_value.tv_nsec > 500000000;
    none = none && timer_settime(n, 0, &ms, NULL) == 0 && usleep(10000) == 0 &&
           timer_gettime(n, &cur) == 0 && cur.it_value.tv_sec == 0 && cur.it_value.tv_nsec == 0;
    timer_delete(n);
    made = ptimer_make(CLOCK_REALTIME, SIGEV_SIGNAL, sig, 9, &t) == 0;
    struct timespec now;
    clock_gettime(CLOCK_REALTIME, &now);
    struct itimerspec past = {{0, 0}, {now.tv_sec - 1, 0}};
    sigset_t pend;
    int abs = made && timer_settime(t, TIMER_ABSTIME, &past, NULL) == 0 &&
              sigpending(&pend) == 0 && sigismember(&pend, sig);
    int dropped = timer_delete(t) == 0 && sigtimedwait(&set1, &si, &zero) == -1 && errno == EAGAIN;
    sigset_t alrm;
    sigemptyset(&alrm);
    sigaddset(&alrm, SIGALRM);
    sigset_t alrm_old;
    sigprocmask(SIG_BLOCK, &alrm, &alrm_old);
    timer_t d;
    memset(&si, 0, sizeof(si));
    int dflt = timer_create(CLOCK_MONOTONIC, NULL, &d) == 0 && timer_settime(d, 0, &ms, NULL) == 0 &&
               sigtimedwait(&alrm, &si, &wait) == SIGALRM && si.si_code == SI_TIMER &&
               si.si_value.sival_int == (int)(intptr_t)d;
    timer_delete(d);
    sigprocmask(SIG_SETMASK, &alrm_old, NULL);
    snprintf(why, sizeof(why), "none=%d abs=%d dropped=%d dflt=%d", none, abs, dropped, dflt);
    report("posix_timer_none_abstime_delete", none && abs && dropped && dflt, why);
    sigprocmask(SIG_SETMASK, &old_mask, NULL);

    /* Ignored periodic timer: parked, then queued again once caught. */
    struct sigaction ign;
    memset(&ign, 0, sizeof(ign));
    ign.sa_handler = SIG_IGN;
    sigaction(sig, &ign, NULL);
    made = ptimer_make(CLOCK_MONOTONIC, SIGEV_SIGNAL, sig, 11, &t) == 0;
    set = made && timer_settime(t, 0, &five_ms, NULL) == 0;
    usleep(30000);
    ptimer_count = 0;
    sigaction(sig, &sa, NULL);
    int resumed = set && ptimer_wait_count(2) && ptimer_value == 11;
    timer_delete(t);

    /* CPU time: a process CPU-clock timer fires while the process spins. */
    ptimer_count = 0;
    made = ptimer_make(CLOCK_PROCESS_CPUTIME_ID, SIGEV_SIGNAL, sig, 12, &t) == 0;
    struct itimerspec cpu = {{0, 0}, {0, 20000000}};
    set = made && timer_settime(t, 0, &cpu, NULL) == 0;
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    volatile unsigned long spin = 0;
    do {
        for (int i = 0; i < 100000; i++)
            spin++;
        clock_gettime(CLOCK_MONOTONIC, &t1);
    } while (ptimer_count == 0 && t1.tv_sec - t0.tv_sec < 5);
    (void)spin;
    int cpuok = set && ptimer_count == 1 && ptimer_value == 12 && timer_gettime(t, &cur) == 0 &&
                cur.it_value.tv_sec == 0 && cur.it_value.tv_nsec == 0;
    timer_delete(t);

    /* Errors, and a fork child without timers. */
    timer_t bad = (timer_t)(intptr_t)12345;
    errno = 0;
    int e1 = timer_gettime(bad, &cur) == -1 && errno == EINVAL;
    errno = 0;
    int e2 = timer_create(1234, NULL, &t) == -1 && errno == EINVAL;
    errno = 0;
    int e3 = timer_create(CLOCK_MONOTONIC_RAW, NULL, &t) == -1 && errno == EOPNOTSUPP;
    made = timer_create(CLOCK_MONOTONIC, NULL, &t) == 0;
    struct itimerspec badns = {{0, 0}, {0, 1000000000}};
    errno = 0;
    int e4 = made && timer_settime(t, 0, &badns, NULL) == -1 && errno == EINVAL;
    int fds[2] = {-1, -1};
    pipe(fds);
    pid_t pid = fork();
    if (pid == 0) {
        errno = 0;
        char r = timer_gettime(t, &cur) == -1 && errno == EINVAL;
        write(fds[1], &r, 1);
        _exit(0);
    }
    char child = 0;
    if (pid > 0) {
        read(fds[0], &child, 1);
        waitpid(pid, NULL, 0);
    }
    close(fds[0]);
    close(fds[1]);
    timer_delete(t);
    /* musl's timer_delete returns the raw -EINVAL without errno: ask the
     * kernel directly. */
    errno = 0;
    int e5 = syscall(SYS_timer_delete, (int)(intptr_t)t) == -1 && errno == EINVAL;
    sigaction(sig, &old_sa, NULL);
    snprintf(why, sizeof(why), "resumed=%d(%d) cpu=%d errors=%d,%d,%d,%d,%d child=%d", resumed,
             ptimer_count, cpuok, e1, e2, e3, e4, e5, child);
    report("posix_timer_ignore_cpu_errors_fork",
           resumed && cpuok && e1 && e2 && e3 && e4 && e5 && child == 1, why);
}

/* restart_syscall: a relative sleep, or a poll with a timeout, that a stop
 * interrupts carries on to its first deadline once the process continues,
 * instead of starting the whole duration again (Linux's restart block).
 * The child tells the parent it is about to wait, is stopped 50 ms in and
 * continued at 300 ms; it must then return at once, not 300 ms later.
 * Where there is no poll syscall (arm64, riscv64) musl's poll is ppoll,
 * which Linux reruns with the time left when the stop came (fs/select.c
 * poll_select_finish): about 250 ms more after the continue. */
enum { RESTART_SLEEP, RESTART_POLL, RESTART_PPOLL };
static void restart_after_stop(const char *name, int mode)
{
    char why[200];
    int ready[2] = {-1, -1}, result[2] = {-1, -1}, idle[2] = {-1, -1};
    if (pipe(ready) != 0 || pipe(result) != 0 || pipe(idle) != 0) {
        report(name, 0, "pipe failed");
        return;
    }
    pid_t pid = fork();
    if (pid == 0) {
        long long out[4];
        char c = 1;
        write(ready[1], &c, 1);
        out[0] = mono_ns();
        if (mode != RESTART_SLEEP) {
            struct pollfd p = {idle[0], POLLIN, 0};
            out[2] = poll(&p, 1, 300);
        } else {
            struct timespec req = {0, 300000000L}, rem = {0, 0};
            out[2] = nanosleep(&req, &rem);
        }
        out[3] = errno;
        out[1] = mono_ns();
        write(result[1], out, sizeof(out));
        _exit(0);
    }
    char c = 0;
    long long out[4] = {0, 0, -2, 0}, cont = 0;
    int stopped = 0;
    if (pid > 0 && read(ready[0], &c, 1) == 1) {
        usleep(50000);
        int st = 0;
        stopped = kill(pid, SIGSTOP) == 0 && waitpid(pid, &st, WUNTRACED) == pid && WIFSTOPPED(st);
        usleep(250000);
        cont = mono_ns();
        kill(pid, SIGCONT);
        read(result[0], out, sizeof(out));
        waitpid(pid, NULL, 0);
    }
    for (int i = 0; i < 2; i++) {
        close(ready[i]);
        close(result[i]);
        close(idle[i]);
    }
    long long slept = (out[1] - out[0]) / 1000000, after = (out[1] - cont) / 1000000;
    snprintf(why, sizeof(why), "stopped=%d ret=%lld errno=%lld slept=%lldms after_cont=%lldms",
             stopped, out[2], out[3], slept, after);
    if (mode == RESTART_PPOLL)
        report(name, stopped && out[2] == 0 && slept >= 450 && after >= 150 && after < 450, why);
    else
        /* Its deadline passed while it was stopped: it ends soon after
         * SIGCONT. A sleep started again would take 300 ms more; the margin
         * below that is for slow CI guests (200 ms seen on GitHub's 4-CPU
         * runner, PR #35). */
        report(name, stopped && out[2] == 0 && slept >= 300 && after < 280, why);
}

/* adjtimex and clock_adjtime: Linux's NTP state is read by everyone and
 * changed only with CAP_SYS_TIME; ADJ_SETOFFSET steps CLOCK_REALTIME and
 * ADJ_TAI sets CLOCK_TAI's offset. */
static long long ts_ns(struct timespec t)
{
    return (long long)t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void test_adjtimex(void)
{
    char why[200];
    struct timex tx;
    memset(&tx, 0, sizeof(tx));
    int state = adjtimex(&tx);
    memset(&tx, 0, sizeof(tx));
    int raw = syscall(SYS_adjtimex, &tx);
    int defaults = tx.tick == 10000 && tx.precision == 1 && tx.tolerance == 32768000 &&
                   tx.time.tv_sec >= 1700000000;
    /* An invalid tick is refused before anything is written back. */
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_TICK;
    tx.tick = 1;
    tx.offset = 12345;
    errno = 0;
    int badtick = adjtimex(&tx) == -1 && errno == EINVAL && tx.offset == 12345 && tx.tick == 1;
    int user = 0;
    pid_t pid = fork();
    if (pid == 0) {
        setresuid(1000, 1000, 1000);
        struct timex u;
        memset(&u, 0, sizeof(u));
        int ok = adjtimex(&u) >= 0;
        u.modes = ADJ_MAXERROR;
        errno = 0;
        ok = ok && adjtimex(&u) == -1 && errno == EPERM;
        memset(&u, 0, sizeof(u));
        u.modes = ADJ_OFFSET_SS_READ;
        ok = ok && adjtimex(&u) >= 0;
        _exit(ok ? 0 : 1);
    }
    int st = 0;
    if (pid > 0 && waitpid(pid, &st, 0) == pid)
        user = WIFEXITED(st) && WEXITSTATUS(st) == 0;
    snprintf(why, sizeof(why), "state=%d raw=%d status=%d tick=%ld tol=%ld defaults=%d badtick=%d user=%d",
             state, raw, tx.status, tx.tick, tx.tolerance, defaults, badtick, user);
    report("adjtimex_read_and_permission", state == TIME_ERROR && raw == TIME_ERROR && defaults &&
                                               badtick && user, why);

    /* adjtime's slew is swapped and read back; ADJ_SETOFFSET steps the
     * wall clock; ADJ_TAI moves CLOCK_TAI. Each is put back. */
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_OFFSET_SINGLESHOT;
    tx.offset = 250000;
    int set1 = adjtimex(&tx) >= 0;
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_OFFSET_SS_READ;
    int slew = adjtimex(&tx) >= 0 && tx.offset == 250000;
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_OFFSET_SINGLESHOT;
    slew = slew && set1 && adjtimex(&tx) >= 0 && tx.offset == 250000;

    struct timespec a, b, rt, tai;
    clock_gettime(CLOCK_REALTIME, &a);
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_SETOFFSET | ADJ_NANO;
    tx.time.tv_sec = 1;
    int stepped = adjtimex(&tx) >= 0;
    clock_gettime(CLOCK_REALTIME, &b);
    long long moved = (ts_ns(b) - ts_ns(a)) / 1000000;
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_SETOFFSET | ADJ_NANO;
    tx.time.tv_sec = -1;
    stepped = stepped && adjtimex(&tx) >= 0 && moved >= 1000 && moved < 1500;
    /* The fraction must be within a second. */
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_SETOFFSET;
    tx.time.tv_usec = 1000000;
    errno = 0;
    int badfrac = adjtimex(&tx) == -1 && errno == EINVAL;

    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_TAI;
    tx.constant = 37;
    int taiset = adjtimex(&tx) >= 0 && tx.tai == 37;
    clock_gettime(CLOCK_REALTIME, &rt);
    clock_gettime(CLOCK_TAI, &tai);
    long long tai_ms = (ts_ns(tai) - ts_ns(rt)) / 1000000;
    memset(&tx, 0, sizeof(tx));
    tx.modes = ADJ_TAI;
    tx.constant = 0;
    taiset = taiset && adjtimex(&tx) >= 0 && tx.tai == 0 && tai_ms >= 36900 && tai_ms <= 37100;
    snprintf(why, sizeof(why), "slew=%d stepped=%d moved=%lldms badfrac=%d tai=%d(%lldms)", slew,
             stepped, moved, badfrac, taiset, tai_ms);
    report("adjtimex_adjtime_setoffset_tai", slew && stepped && badfrac && taiset, why);

    /* clock_adjtime adjusts CLOCK_REALTIME only. */
    memset(&tx, 0, sizeof(tx));
    int real = clock_adjtime(CLOCK_REALTIME, &tx) == TIME_ERROR && tx.tick == 10000;
    errno = 0;
    int mono = clock_adjtime(CLOCK_MONOTONIC, &tx) == -1 && errno == EOPNOTSUPP;
    errno = 0;
    int cpu = clock_adjtime(CLOCK_PROCESS_CPUTIME_ID, &tx) == -1 && errno == EOPNOTSUPP;
    errno = 0;
    int none = clock_adjtime(10, &tx) == -1 && errno == EINVAL;
    errno = 0;
    int fault = syscall(SYS_clock_adjtime, CLOCK_REALTIME, (void *)8) == -1 && errno == EFAULT;
    snprintf(why, sizeof(why), "real=%d mono=%d cpu=%d none=%d fault=%d", real, mono, cpu, none,
             fault);
    report("clock_adjtime_clocks", real && mono && cpu && none && fault, why);
}

static volatile int restart_alrm;
static void restart_on_alrm(int sig)
{
    (void)sig;
    restart_alrm++;
}

static void test_restart_syscall(void)
{
    restart_after_stop("nanosleep_stop_keeps_deadline", RESTART_SLEEP);
#ifdef SYS_poll
    restart_after_stop("poll_stop_keeps_deadline", RESTART_POLL);
#else
    restart_after_stop("ppoll_stop_reruns_time_left", RESTART_PPOLL);
#endif

    /* With no restart block it is EINTR; a handler, SA_RESTART or not,
     * makes an interrupted sleep EINTR with the time left in rem, and the
     * block it left is gone after the handler (rt_sigreturn). */
    char why[200];
    errno = 0;
    long none = syscall(SYS_restart_syscall);
    int none_eintr = none == -1 && errno == EINTR;
    struct sigaction sa, old;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = restart_on_alrm;
    sa.sa_flags = SA_RESTART;
    sigaction(SIGALRM, &sa, &old);
    struct itimerval it = {{0, 0}, {0, 50000}};
    setitimer(ITIMER_REAL, &it, NULL);
    struct timespec req = {2, 0}, rem = {0, 0};
    errno = 0;
    int r = nanosleep(&req, &rem);
    int e = errno;
    int left = rem.tv_sec == 1;
    errno = 0;
    long after = syscall(SYS_restart_syscall);
    int after_eintr = after == -1 && errno == EINTR;
    sigaction(SIGALRM, &old, NULL);
    snprintf(why, sizeof(why), "none=%ld,%d handler: ret=%d errno=%d alrm=%d rem=%lld.%09ld after=%ld",
             none, none_eintr, r, e, restart_alrm, (long long)rem.tv_sec, rem.tv_nsec, after);
    report("restart_syscall_eintr_and_handler",
           none_eintr && r == -1 && e == EINTR && restart_alrm == 1 && left && after_eintr, why);
}

#ifndef RWF_NOSIGNAL
#define RWF_NOSIGNAL 0x00000100
#endif

/* Read the file at `path` whole into buf (at most n - 1 bytes, NUL ended). */
static ssize_t slurp(const char *path, char *buf, size_t n)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0)
        return -1;
    ssize_t r = read(fd, buf, n - 1);
    close(fd);
    if (r >= 0)
        buf[r] = 0;
    return r;
}

/*
 * preadv, pwritev, preadv2, pwritev2 (group 3 step D1): positioned vectors
 * leave the file position alone, -1 uses it, and the RWF_* flags are
 * checked as Linux's. musl's pwrite is pwritev2 with RWF_NOAPPEND, so on
 * an O_APPEND file it writes where asked, while the raw pwrite64 and
 * pwritev append, as on Linux. RWF_NOSIGNAL keeps EPIPE quiet.
 */
static void test_preadv_pwritev(void)
{
    const char *path = "/tmp/musl_vec";
    char why[160];
    int f = 0;
    int fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0644);
    if (fd < 0 || write(fd, "0123456789", 10) != 10) f |= 1;
    struct iovec w[2] = {{"ab", 2}, {"cd", 2}};
    if (pwritev2(fd, w, 2, 2, 0) != 4 || lseek(fd, 0, SEEK_CUR) != 10) f |= 2;
    char a[4] = {0}, b[4] = {0};
    struct iovec r[2] = {{a, 3}, {b, 3}};
    if (preadv(fd, r, 2, 1) != 6 || memcmp(a, "1ab", 3) || memcmp(b, "cd6", 3)) f |= 4;
    /* Offset -1: at the position, which then moves. */
    lseek(fd, 8, SEEK_SET);
    memset(a, 0, sizeof(a));
    struct iovec one = {a, 3};
    if (preadv2(fd, &one, 1, -1, RWF_HIPRI) != 2 || memcmp(a, "89", 2) ||
        lseek(fd, 0, SEEK_CUR) != 10) f |= 8;
    errno = 0;
    if (preadv2(fd, &one, 1, 0, 0x200) != -1 || errno != EOPNOTSUPP) f |= 16;
    errno = 0;
    if (syscall(SYS_preadv2, fd, &one, 1, -2L, 0L, 0) != -1 || errno != EINVAL) f |= 32;
    errno = 0;
    if (pwritev2(fd, w, 1, 0, RWF_APPEND | RWF_NOAPPEND) != -1 || errno != EINVAL) f |= 64;
    /* RWF_APPEND: at the end, whatever the offset. */
    if (pwritev2(fd, w, 1, 0, RWF_APPEND) != 2) f |= 128;
    close(fd);
    /* O_APPEND: musl's pwrite (pwritev2 with RWF_NOAPPEND) writes at 0;
     * the raw pwrite64 and pwritev calls append. */
    fd = open(path, O_RDWR | O_APPEND);
    struct iovec x = {"Y", 1};
    if (fd < 0 || pwrite(fd, "X", 1, 0) != 1 ||
        syscall(SYS_pwrite64, fd, "Z", 1, 0L) != 1 ||
        syscall(SYS_pwritev, fd, &x, 1, 0L, 0L) != 1) f |= 256;
    close(fd);
    char got[32];
    if (slurp(path, got, sizeof(got)) != 14 || strcmp(got, "X1abcd6789abZY")) f |= 512;
    /* A pipe has no offset (ESPIPE); with no reader, EPIPE, and SIGPIPE
     * only without RWF_NOSIGNAL. */
    int p[2];
    sigset_t pipe_set, old, pending;
    sigemptyset(&pipe_set);
    sigaddset(&pipe_set, SIGPIPE);
    sigprocmask(SIG_BLOCK, &pipe_set, &old);
    if (pipe(p) != 0) f |= 1024;
    errno = 0;
    if (preadv(p[0], r, 1, 0) != -1 || errno != ESPIPE) f |= 2048;
    close(p[0]);
    errno = 0;
    if (pwritev2(p[1], w, 1, -1, RWF_NOSIGNAL) != -1 || errno != EPIPE) f |= 4096;
    sigpending(&pending);
    if (sigismember(&pending, SIGPIPE)) f |= 8192;
    errno = 0;
    if (pwritev2(p[1], w, 1, -1, 0) != -1 || errno != EPIPE) f |= 16384;
    sigpending(&pending);
    if (!sigismember(&pending, SIGPIPE)) f |= 32768;
    struct timespec zero = {0, 0};
    sigtimedwait(&pipe_set, NULL, &zero);
    sigprocmask(SIG_SETMASK, &old, NULL);
    close(p[1]);
    unlink(path);
    snprintf(why, sizeof(why), "f=0x%x got=%s errno=%d", f, got, errno);
    report("preadv_pwritev_rwf", f == 0, why);
}

/*
 * creat, lchown, fchmodat2 and the old getdents (group 3 step D1).
 */
static void test_creat_lchown_fchmodat2(void)
{
    const char *path = "/tmp/musl_creat", *link = "/tmp/musl_creat_lnk";
    char why[160];
    int f = 0;
    unlink(link);
    int fd = open(path, O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (fd < 0 || write(fd, "old", 3) != 3) f |= 1;
    close(fd);
    /* creat truncates and opens write-only. */
#ifdef SYS_creat
    fd = syscall(SYS_creat, path, 0600);
#else
    fd = creat(path, 0600);
#endif
    char c;
    struct stat st;
    errno = 0;
    if (fd < 0 || read(fd, &c, 1) != -1 || errno != EBADF || fstat(fd, &st) || st.st_size != 0)
        f |= 2;
    close(fd);
    if (symlink(path, link) != 0) f |= 4;
    /* lchown changes the link, not its target. */
    struct stat ls;
    if (lchown(link, 1000, 1001) != 0 || lstat(link, &ls) || stat(path, &st) ||
        ls.st_uid != 1000 || ls.st_gid != 1001 || st.st_uid != 0) f |= 8;
    errno = 0;
    if (syscall(SYS_fchownat, AT_FDCWD, link, -1, -1, AT_SYMLINK_FOLLOW) != -1 || errno != EINVAL)
        f |= 16;
    /* fchmodat2: a symlink's mode cannot change; bad flags are EINVAL;
     * AT_EMPTY_PATH changes the fd's file. */
    errno = 0;
    if (syscall(SYS_fchmodat2, AT_FDCWD, link, 0600, AT_SYMLINK_NOFOLLOW) != -1 ||
        errno != EOPNOTSUPP) f |= 32;
    errno = 0;
    if (syscall(SYS_fchmodat2, AT_FDCWD, path, 0600, 0x8000) != -1 || errno != EINVAL) f |= 64;
    fd = open(path, O_RDONLY);
    if (syscall(SYS_fchmodat2, fd, "", 0640, AT_EMPTY_PATH) != 0 || fstat(fd, &st) ||
        (st.st_mode & 07777) != 0640) f |= 128;
    if (syscall(SYS_fchmodat2, AT_FDCWD, link, 0604, 0) != 0 || stat(path, &st) ||
        (st.st_mode & 07777) != 0604) f |= 256;
    close(fd);
#ifdef SYS_getdents
    /* The old record: d_type in its last byte. */
    mkdir("/tmp/musl_gd", 0755);
    close(open("/tmp/musl_gd/regular", O_CREAT | O_WRONLY, 0644));
    int dfd = open("/tmp/musl_gd", O_RDONLY | O_DIRECTORY);
    char buf[1024];
    long n = syscall(SYS_getdents, dfd, buf, sizeof(buf));
    int found = 0;
    for (long off = 0; off < n;) {
        unsigned short reclen;
        memcpy(&reclen, buf + off + 16, 2);
        if (reclen == 0) break;
        if (strcmp(buf + off + 18, "regular") == 0 && buf[off + reclen - 1] == DT_REG) found = 1;
        off += reclen;
    }
    if (n <= 0 || !found) f |= 512;
    close(dfd);
    unlink("/tmp/musl_gd/regular");
    rmdir("/tmp/musl_gd");
#endif
    unlink(link);
    unlink(path);
    snprintf(why, sizeof(why), "f=0x%x errno=%d", f, errno);
    report("creat_lchown_fchmodat2_getdents", f == 0, why);
}

/*
 * close_range, syncfs, sync_file_range, posix_fadvise and readahead
 * (group 3 step D1).
 */
static void test_close_range_and_advice(void)
{
    char why[160];
    int f = 0;
    /* Fixed numbers well above anything the earlier tests left open. */
    int null = open("/dev/null", O_RDONLY);
    int a = dup2(null, 200), b = dup2(null, 201), c = dup2(null, 202);
    close(null);
    if (a != 200 || b != 201 || c != 202) f |= 1;
    if (syscall(SYS_close_range, b, c, 4 /* CLOSE_RANGE_CLOEXEC */) != 0 ||
        fcntl(b, F_GETFD) != FD_CLOEXEC || fcntl(c, F_GETFD) != FD_CLOEXEC ||
        fcntl(a, F_GETFD) != 0) f |= 2;
    errno = 0;
    if (syscall(SYS_close_range, 5, 4, 0) != -1 || errno != EINVAL) f |= 4;
    errno = 0;
    if (syscall(SYS_close_range, 3, ~0U, ~0U) != -1 || errno != EINVAL) f |= 8;
    if (syscall(SYS_close_range, b, ~0U, 2 /* CLOSE_RANGE_UNSHARE */) != 0 ||
        fcntl(b, F_GETFD) != -1 || fcntl(c, F_GETFD) != -1 || fcntl(a, F_GETFD) != 0) f |= 16;
    close(a);
    int fd = open("/tmp/musl_adv", O_CREAT | O_RDWR | O_TRUNC, 0644);
    write(fd, "data", 4);
    errno = 0;
    if (syncfs(fd) != 0 || syncfs(-1) != -1 || errno != EBADF) f |= 32;
    if (sync_file_range(fd, 0, 0, SYNC_FILE_RANGE_WRITE | SYNC_FILE_RANGE_WAIT_AFTER) != 0) f |= 64;
    errno = 0;
    if (sync_file_range(fd, 0, 0, 8) != -1 || errno != EINVAL) f |= 128;
    errno = 0;
    if (sync_file_range(fd, -1, 0, 0) != -1 || errno != EINVAL) f |= 256;
    if (posix_fadvise(fd, 0, 0, POSIX_FADV_DONTNEED) != 0 ||
        posix_fadvise(fd, 0, 0, 99) != EINVAL || posix_fadvise(fd, 0, -1, 0) != EINVAL) f |= 512;
    if (readahead(fd, 0, 4096) != 0) f |= 1024;
    int p[2];
    pipe(p);
    errno = 0;
    if (sync_file_range(p[0], 0, 0, 0) != -1 || errno != ESPIPE) f |= 2048;
    if (posix_fadvise(p[0], 0, 0, 0) != ESPIPE) f |= 4096;
    errno = 0;
    if (readahead(p[0], 0, 1) != -1 || errno != EINVAL) f |= 8192;
    errno = 0;
    if (readahead(p[1], 0, 1) != -1 || errno != EBADF) f |= 16384;
    close(p[0]);
    close(p[1]);
    close(fd);
    unlink("/tmp/musl_adv");
    snprintf(why, sizeof(why), "f=0x%x errno=%d", f, errno);
    report("close_range_sync_advice", f == 0, why);
}

/*
 * sendfile, copy_file_range, splice, tee and vmsplice (group 3 step D2).
 */
static void test_splice_family(void)
{
    const char *src = "/tmp/musl_sf_src", *dst = "/tmp/musl_sf_dst";
    char why[160], got[64] = "";
    int f = 0;
    int in = open(src, O_CREAT | O_RDWR | O_TRUNC, 0644);
    int out = open(dst, O_CREAT | O_RDWR | O_TRUNC, 0644);
    if (in < 0 || out < 0 || write(in, "0123456789", 10) != 10) f |= 1;
    /* sendfile with an offset reads from it and moves it, not the position;
     * without one, from the position, which moves. */
    off_t off = 2;
    if (sendfile(out, in, &off, 5) != 5 || off != 7 || lseek(in, 0, SEEK_CUR) != 10) f |= 2;
    lseek(in, 8, SEEK_SET);
    if (sendfile(out, in, NULL, 100) != 2 || lseek(in, 0, SEEK_CUR) != 10) f |= 4;
    if (slurp(dst, got, sizeof(got)) != 7 || strcmp(got, "2345689")) f |= 8;
    /* copy_file_range: the offsets move, the positions do not. */
    off_t oi = 0, oo = 7;
    if (copy_file_range(in, &oi, out, &oo, 3, 0) != 3 || oi != 3 || oo != 10 ||
        lseek(out, 0, SEEK_CUR) != 7) f |= 16;
    /* Overlap in one file, flags, a directory, O_APPEND, two filesystems. */
    off_t a = 0, b = 5;
    errno = 0;
    if (copy_file_range(out, &a, out, &b, 10, 0) != -1 || errno != EINVAL) f |= 32;
    errno = 0;
    if (copy_file_range(in, &a, out, &oo, 1, 1) != -1 || errno != EINVAL) f |= 64;
    int dir = open("/tmp", O_RDONLY | O_DIRECTORY);
    errno = 0;
    if (copy_file_range(dir, NULL, out, NULL, 1, 0) != -1 || errno != EISDIR) f |= 128;
    close(dir);
    int app = open(dst, O_WRONLY | O_APPEND);
    errno = 0;
    if (copy_file_range(in, &a, app, NULL, 1, 0) != -1 || errno != EBADF) f |= 256;
    close(app);
    int shm = open("/dev/shm/musl_sf", O_CREAT | O_RDWR | O_TRUNC, 0644);
    errno = 0;
    if (shm < 0 || copy_file_range(in, &a, shm, NULL, 1, 0) != -1 || errno != EXDEV) f |= 512;
    close(shm);
    unlink("/dev/shm/musl_sf");
    /* splice a file into a pipe, tee it into another, read that one back
     * with vmsplice, then splice the first into the file. */
    int p[2], q[2];
    if (pipe(p) || pipe(q)) f |= 1024;
    off_t so = 1;
    if (splice(in, &so, p[1], NULL, 4, 0) != 4 || so != 5) f |= 2048;
    if (tee(p[0], q[1], 100, 0) != 4) f |= 4096;
    char buf[16] = {0};
    struct iovec v = {buf, sizeof(buf)};
    if (vmsplice(q[0], &v, 1, 0) != 4 || memcmp(buf, "1234", 4)) f |= 8192;
    off_t to = 0;
    if (splice(p[0], NULL, out, &to, 100, 0) != 4 || to != 4) f |= 16384;
    if (slurp(dst, got, sizeof(got)) != 10 || strcmp(got, "1234689012")) f |= 32768;
    /* A pipe takes no offset; a pipe into itself; an empty pipe without
     * waiting; a pipe is no sendfile source. */
    errno = 0;
    if (splice(p[0], &so, out, NULL, 1, 0) != -1 || errno != ESPIPE) f |= 65536;
    errno = 0;
    if (splice(p[0], NULL, p[1], NULL, 1, 0) != -1 || errno != EINVAL) f |= 131072;
    errno = 0;
    if (splice(p[0], NULL, q[1], NULL, 1, SPLICE_F_NONBLOCK) != -1 || errno != EAGAIN)
        f |= 262144;
    struct iovec w = {"xy", 2};
    if (vmsplice(p[1], &w, 1, 0) != 2 || read(p[0], buf, sizeof(buf)) != 2 ||
        memcmp(buf, "xy", 2)) f |= 524288;
    /* Empty entries are skipped, not the end (PR #35 review). */
    struct iovec ws[3] = {{"", 0}, {"ab", 2}, {"c", 1}};
    if (vmsplice(p[1], ws, 3, 0) != 3 || read(p[0], buf, sizeof(buf)) != 3 ||
        memcmp(buf, "abc", 3)) f |= 524288;
    errno = 0;
    if (sendfile(out, p[0], NULL, 1) != -1 || errno != EINVAL) f |= 1048576;
    /* Into a pipe nobody reads: EPIPE, and SIGPIPE. */
    sigset_t pipe_set, old, pending;
    sigemptyset(&pipe_set);
    sigaddset(&pipe_set, SIGPIPE);
    sigprocmask(SIG_BLOCK, &pipe_set, &old);
    close(q[0]);
    so = 0;
    errno = 0;
    if (splice(in, &so, q[1], NULL, 1, 0) != -1 || errno != EPIPE) f |= 2097152;
    sigpending(&pending);
    if (!sigismember(&pending, SIGPIPE)) f |= 4194304;
    struct timespec zero = {0, 0};
    sigtimedwait(&pipe_set, NULL, &zero);
    sigprocmask(SIG_SETMASK, &old, NULL);
    close(p[0]);
    close(p[1]);
    close(q[1]);
    close(in);
    close(out);
    unlink(src);
    unlink(dst);
    snprintf(why, sizeof(why), "f=0x%x got=%s errno=%d", f, got, errno);
    report("sendfile_copy_splice_tee", f == 0, why);
}

#if defined(__x86_64__)
/* ustat and sysfs, x86_64 only (group 3 step D1). */
static void test_ustat_sysfs(void)
{
    char why[160];
    int f = 0;
    struct stat st;
    char ubuf[32];
    if (stat("/", &st) != 0 || syscall(SYS_ustat, (unsigned)st.st_dev, ubuf) != 0) f |= 1;
    errno = 0;
    if (syscall(SYS_ustat, (unsigned)-1, ubuf) != -1 || errno != EINVAL) f |= 2;
    errno = 0;
    if (syscall(SYS_ustat, (unsigned)st.st_dev, (void *)-1) != -1 || errno != EFAULT) f |= 4;
    long idx = syscall(SYS_sysfs, 1, "proc");
    char name[32] = {0};
    if (idx < 0 || syscall(SYS_sysfs, 2, idx, name) != 0 || strcmp(name, "proc")) f |= 8;
    if (syscall(SYS_sysfs, 3) <= idx) f |= 16;
    errno = 0;
    if (syscall(SYS_sysfs, 1, "ext0") != -1 || errno != EINVAL) f |= 32;
    errno = 0;
    if (syscall(SYS_sysfs, 4) != -1 || errno != EINVAL) f |= 64;
    errno = 0;
    if (syscall(SYS_sysfs, 2, 1000, name) != -1 || errno != EINVAL) f |= 128;
    errno = 0;
    if (syscall(SYS_sysfs, 1, NULL) != -1 || errno != EFAULT) f |= 256;
    snprintf(why, sizeof(why), "f=0x%x errno=%d", f, errno);
    report("ustat_sysfs", f == 0, why);
}
#endif

/*
 * statx (group 3 step D3a): the numbers stat reports, the mount id and
 * mount root, AT_EMPTY_PATH on a descriptor and on the cwd, and the
 * argument errors in Linux's order.
 */
static void test_statx(void)
{
    const char *path = "/tmp/musl_statx", *link_path = "/tmp/musl_statx_ln";
    char why[200];
    int f = 0;
    unlink(path);
    unlink(link_path);
    int fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0640);
    if (fd < 0 || write(fd, "0123456789", 10) != 10) f |= 1;
    struct stat st;
    struct statx sx;
    memset(&sx, 0xa5, sizeof(sx));
    if (stat(path, &st) || statx(AT_FDCWD, path, 0, STATX_BASIC_STATS, &sx)) f |= 2;
    if ((sx.stx_mask & STATX_BASIC_STATS) != STATX_BASIC_STATS || !(sx.stx_mask & STATX_MNT_ID) ||
        (sx.stx_mask & STATX_BTIME))
        f |= 4;
    if (sx.stx_ino != st.st_ino || sx.stx_size != 10 || sx.stx_mode != st.st_mode ||
        sx.stx_uid != st.st_uid || sx.stx_nlink != st.st_nlink ||
        sx.stx_blocks != (uint64_t)st.st_blocks || sx.stx_blksize != (uint32_t)st.st_blksize ||
        sx.stx_mtime.tv_sec != st.st_mtime ||
        makedev(sx.stx_dev_major, sx.stx_dev_minor) != st.st_dev)
        f |= 8;
    if ((sx.stx_attributes & STATX_ATTR_MOUNT_ROOT) ||
        !(sx.stx_attributes_mask & STATX_ATTR_MOUNT_ROOT))
        f |= 16;
    /* The rest of the struct is written as zeroes, not left as it was. */
    if (sx.stx_btime.tv_sec != 0 || sx.__pad2[8] != 0) f |= 32;
    /* AT_EMPTY_PATH with NULL or "" is the descriptor; with AT_FDCWD the
     * cwd. */
    struct statx byfd, empty, cwd;
    if (statx(fd, NULL, AT_EMPTY_PATH, STATX_BASIC_STATS, &byfd) ||
        statx(fd, "", AT_EMPTY_PATH, STATX_BASIC_STATS, &empty) ||
        byfd.stx_ino != sx.stx_ino || empty.stx_ino != sx.stx_ino ||
        byfd.stx_mnt_id != sx.stx_mnt_id)
        f |= 64;
    struct stat dot;
    if (stat(".", &dot) || statx(AT_FDCWD, "", AT_EMPTY_PATH, 0, &cwd) ||
        cwd.stx_ino != dot.st_ino)
        f |= 128;
    /* Mount roots: "/", /proc and /dev/shm, each with its own id. */
    struct statx root, proc, shm;
    if (statx(AT_FDCWD, "/", 0, 0, &root) || statx(AT_FDCWD, "/proc", 0, 0, &proc) ||
        statx(AT_FDCWD, "/dev/shm", 0, 0, &shm))
        f |= 256;
    if (!(root.stx_attributes & STATX_ATTR_MOUNT_ROOT) ||
        !(proc.stx_attributes & STATX_ATTR_MOUNT_ROOT) ||
        !(shm.stx_attributes & STATX_ATTR_MOUNT_ROOT) || proc.stx_mnt_id == root.stx_mnt_id ||
        shm.stx_mnt_id == proc.stx_mnt_id)
        f |= 512;
    /* STATX_MNT_ID_UNIQUE instead of STATX_MNT_ID, above 1 << 31. */
    struct statx uniq;
    if (statx(AT_FDCWD, "/proc", 0, STATX_MNT_ID_UNIQUE, &uniq) ||
        !(uniq.stx_mask & STATX_MNT_ID_UNIQUE) || (uniq.stx_mask & STATX_MNT_ID) ||
        uniq.stx_mnt_id <= (1ULL << 31))
        f |= 1024;
    /* AT_SYMLINK_NOFOLLOW stats the link; without it, the target. */
    struct statx ln, target;
    if (symlink(path, link_path) ||
        statx(AT_FDCWD, link_path, AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS, &ln) ||
        !S_ISLNK(ln.stx_mode) || statx(AT_FDCWD, link_path, 0, STATX_BASIC_STATS, &target) ||
        target.stx_ino != sx.stx_ino)
        f |= 2048;
    /* A pipe has no path: still a FIFO, with a mount id. */
    int p[2];
    struct statx pipe_sx;
    if (pipe(p) || statx(p[0], NULL, AT_EMPTY_PATH, STATX_BASIC_STATS, &pipe_sx) ||
        !S_ISFIFO(pipe_sx.stx_mode) || !(pipe_sx.stx_mask & STATX_MNT_ID))
        f |= 4096;
    close(p[0]);
    close(p[1]);
    /* Errors: the reserved mask bit and both sync types are EINVAL, an
     * unknown flag on a lookup EINVAL, a negative dirfd with no name EBADF,
     * a bad name EFAULT before EINVAL, no name without AT_EMPTY_PATH
     * ENOENT. */
    struct statx junk;
    errno = 0;
    if (statx(AT_FDCWD, path, 0, 0x80000000U, &junk) != -1 || errno != EINVAL) f |= 8192;
    errno = 0;
    if (statx(AT_FDCWD, path, AT_STATX_FORCE_SYNC | AT_STATX_DONT_SYNC, 0, &junk) != -1 ||
        errno != EINVAL)
        f |= 16384;
    errno = 0;
    if (statx(AT_FDCWD, path, 0x8000, 0, &junk) != -1 || errno != EINVAL) f |= 32768;
    errno = 0;
    if (statx(-1, NULL, AT_EMPTY_PATH, 0, &junk) != -1 || errno != EBADF) f |= 65536;
    errno = 0;
    if (statx(AT_FDCWD, (const char *)1, 0, 0x80000000U, &junk) != -1 || errno != EFAULT)
        f |= 131072;
    errno = 0;
    if (statx(AT_FDCWD, "", 0, 0, &junk) != -1 || errno != ENOENT) f |= 262144;
    /* Either sync type alone is fine. */
    if (statx(AT_FDCWD, path, AT_STATX_DONT_SYNC, 0, &junk)) f |= 524288;
    close(fd);
    unlink(link_path);
    unlink(path);
    snprintf(why, sizeof(why), "f=0x%x mask=0x%x attrs=0x%llx mnt=%llu/%llu/%llu errno=%d", f,
             sx.stx_mask, (unsigned long long)sx.stx_attributes,
             (unsigned long long)root.stx_mnt_id, (unsigned long long)proc.stx_mnt_id,
             (unsigned long long)shm.stx_mnt_id, errno);
    report("statx", f == 0, why);
}

/*
 * openat2 (group 3 step D3b): the extensible struct, build_open_flags'
 * checks in Linux's order, OPENAT2_REGULAR, and every RESOLVE_* flag.
 * musl has no wrapper; struct open_how is <linux/openat2.h>'s.
 */
struct musl_open_how {
    uint64_t flags, mode, resolve;
};
#define MUSL_RESOLVE_NO_XDEV 0x01
#define MUSL_RESOLVE_NO_MAGICLINKS 0x02
#define MUSL_RESOLVE_NO_SYMLINKS 0x04
#define MUSL_RESOLVE_BENEATH 0x08
#define MUSL_RESOLVE_IN_ROOT 0x10
#define MUSL_RESOLVE_CACHED 0x20
#define MUSL_OPENAT2_REGULAR (1ULL << 32)
#define MUSL_EFTYPE 134

static long o2(int dfd, const char *path, uint64_t flags, uint64_t mode, uint64_t resolve)
{
    struct musl_open_how how = {flags, mode, resolve};
    return syscall(SYS_openat2, dfd, path, &how, sizeof(how));
}

/* -1 and errno == want, or a descriptor (closed) when want is 0. */
static int o2_is(int dfd, const char *path, uint64_t flags, uint64_t mode, uint64_t resolve,
                 int want)
{
    errno = 0;
    long fd = o2(dfd, path, flags, mode, resolve);
    if (fd >= 0) close((int)fd);
    return want ? (fd == -1 && errno == want) : fd >= 0;
}

static void test_openat2(void)
{
    const char *base = "/tmp/musl_o2";
    char why[200];
    int f = 0, step = 0;
    unlink("/tmp/musl_o2/sub/file");
    unlink("/tmp/musl_o2/abs");
    unlink("/tmp/musl_o2/rel");
    unlink("/tmp/musl_o2/up");
    unlink("/tmp/musl_o2/fifo");
    unlink("/tmp/musl_o2/new");
    unlink("/tmp/musl_o2/sub/new");
    rmdir("/tmp/musl_o2/sub");
    rmdir(base);
    unlink("/dev/shm/musl_o2_new");
    unlink("/dev/shm/musl_o2_abs");
    if (mkdir(base, 0755) || mkdir("/tmp/musl_o2/sub", 0755)) f |= 1;
    int w = open("/tmp/musl_o2/sub/file", O_CREAT | O_WRONLY, 0644);
    if (w < 0 || write(w, "x", 1) != 1) f |= 1;
    close(w);
    if (symlink("/tmp/musl_o2/sub/file", "/tmp/musl_o2/abs") ||
        symlink("sub/file", "/tmp/musl_o2/rel") || symlink("../musl_o2/sub/file", "/tmp/musl_o2/up") ||
        mkfifo("/tmp/musl_o2/fifo", 0644))
        f |= 1;
    int dir = open(base, O_RDONLY | O_DIRECTORY);
    if (dir < 0) f |= 1;

    /* The struct: below its first version EINVAL, above a page E2BIG, a
     * zero tail fine, a nonzero one E2BIG, an unreadable one EFAULT. */
    step = 1;
    struct {
        struct musl_open_how how;
        uint64_t pad;
    } big = {{O_RDONLY, 0, 0}, 0};
    errno = 0;
    if (syscall(SYS_openat2, dir, "sub/file", &big, 0) != -1 || errno != EINVAL) f |= 2;
    errno = 0;
    if (syscall(SYS_openat2, dir, "sub/file", &big, 23) != -1 || errno != EINVAL) f |= 2;
    errno = 0;
    if (syscall(SYS_openat2, dir, "sub/file", &big, 4097) != -1 || errno != E2BIG) f |= 2;
    long fd = syscall(SYS_openat2, dir, "sub/file", &big, sizeof(big));
    if (fd < 0) f |= 4;
    else close((int)fd);
    big.pad = 1;
    errno = 0;
    if (syscall(SYS_openat2, dir, "sub/file", &big, sizeof(big)) != -1 || errno != E2BIG) f |= 4;
    char *pages = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pages == MAP_FAILED || munmap(pages + 4096, 4096)) f |= 1;
    else {
        struct musl_open_how *edge = (struct musl_open_how *)(pages + 4096 - sizeof(*edge));
        memset(edge, 0, sizeof(*edge));
        errno = 0;
        if (syscall(SYS_openat2, dir, "sub/file", edge, sizeof(*edge) + 1) != -1 || errno != EFAULT)
            f |= 4;
        munmap(pages, 4096);
    }

    /* build_open_flags, in order. */
    step = 2;
    if (!o2_is(dir, "sub/file", 1ULL << 33, 0, 0, EINVAL)) f |= 8;
    if (!o2_is(dir, "sub/file", O_RDONLY, 0, 0x40, EINVAL)) f |= 8;
    if (!o2_is(dir, "sub/file", O_RDONLY, 0, MUSL_RESOLVE_BENEATH | MUSL_RESOLVE_IN_ROOT, EINVAL))
        f |= 8;
    if (!o2_is(dir, "sub/file", O_RDONLY, 0644, 0, EINVAL)) f |= 16;
    if (!o2_is(dir, "new", O_CREAT | O_WRONLY, 010000, 0, EINVAL)) f |= 16;
    if (!o2_is(dir, "new", O_CREAT | O_DIRECTORY | O_RDONLY, 0644, 0, EINVAL)) f |= 16;
    if (!o2_is(dir, "sub", O_TMPFILE | O_RDONLY, 0600, 0, EINVAL)) f |= 32;
    if (!o2_is(dir, "sub", (O_TMPFILE & ~O_DIRECTORY) | O_RDWR, 0600, 0, EINVAL)) f |= 32;
    if (!o2_is(dir, "sub", O_DIRECTORY | MUSL_OPENAT2_REGULAR, 0, 0, EINVAL)) f |= 32;
    if (!o2_is(dir, "sub/file", O_PATH | O_WRONLY, 0, 0, EINVAL)) f |= 32;
    if (!o2_is(dir, "new", O_CREAT | O_WRONLY, 0644, MUSL_RESOLVE_CACHED, EAGAIN)) f |= 64;
    if (!o2_is(dir, "sub/file", O_WRONLY | O_TRUNC, 0, MUSL_RESOLVE_CACHED, EAGAIN)) f |= 64;
    if (!o2_is(dir, "sub/file", O_RDONLY, 0, MUSL_RESOLVE_CACHED, 0)) f |= 64;
    /* Then the name, then the directory. */
    if (!o2_is(dir, NULL, O_RDONLY, 0, 0, EFAULT) || !o2_is(dir, "", O_RDONLY, 0, 0, ENOENT))
        f |= 128;
    if (!o2_is(-1, "sub/file", O_RDONLY, 0, 0, EBADF) ||
        !o2_is(-1, "sub/file", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, EBADF))
        f |= 128;
    int file_fd = open("/tmp/musl_o2/sub/file", O_RDONLY);
    if (!o2_is(file_fd, "x", O_RDONLY, 0, MUSL_RESOLVE_NO_XDEV, ENOTDIR)) f |= 128;
    close(file_fd);

    /* OPENAT2_REGULAR: EFTYPE for a directory, a FIFO or a device. */
    step = 3;
    if (!o2_is(dir, "sub/file", O_RDONLY | MUSL_OPENAT2_REGULAR, 0, 0, 0) ||
        !o2_is(dir, "sub", O_RDONLY | MUSL_OPENAT2_REGULAR, 0, 0, MUSL_EFTYPE) ||
        !o2_is(dir, "fifo", O_RDONLY | O_NONBLOCK | MUSL_OPENAT2_REGULAR, 0, 0, MUSL_EFTYPE) ||
        !o2_is(AT_FDCWD, "/dev/null", O_RDONLY | MUSL_OPENAT2_REGULAR, 0, 0, MUSL_EFTYPE))
        f |= 256;

    /* RESOLVE_NO_SYMLINKS refuses every symlink, RESOLVE_NO_MAGICLINKS only
     * /proc/self/exe. */
    step = 4;
    if (!o2_is(dir, "rel", O_RDONLY, 0, 0, 0) ||
        !o2_is(dir, "rel", O_RDONLY, 0, MUSL_RESOLVE_NO_SYMLINKS, ELOOP) ||
        !o2_is(dir, "rel", O_RDONLY, 0, MUSL_RESOLVE_NO_MAGICLINKS, 0) ||
        !o2_is(AT_FDCWD, "/proc/self/exe", O_RDONLY, 0, 0, 0) ||
        !o2_is(AT_FDCWD, "/proc/self/exe", O_RDONLY, 0, MUSL_RESOLVE_NO_MAGICLINKS, ELOOP))
        f |= 512;

    /* RESOLVE_NO_XDEV: entering a mount, leaving one by "..", an absolute
     * symlink read on another mount than the root's. */
    step = 5;
    int shm = open("/dev/shm", O_RDONLY | O_DIRECTORY);
    if (shm < 0 || symlink("/tmp/musl_o2/sub/file", "/dev/shm/musl_o2_abs")) f |= 1;
    if (!o2_is(AT_FDCWD, "/proc/version", O_RDONLY, 0, MUSL_RESOLVE_NO_XDEV, EXDEV) ||
        !o2_is(dir, "abs", O_RDONLY, 0, MUSL_RESOLVE_NO_XDEV, 0) ||
        !o2_is(shm, "musl_o2_new", O_CREAT | O_WRONLY, 0600, MUSL_RESOLVE_NO_XDEV, 0) ||
        !o2_is(shm, "..", O_RDONLY | O_DIRECTORY, 0, MUSL_RESOLVE_NO_XDEV, EXDEV) ||
        !o2_is(shm, "musl_o2_abs", O_RDONLY, 0, MUSL_RESOLVE_NO_XDEV, EXDEV))
        f |= 1024;

    /* RESOLVE_BENEATH: no absolute path or symlink, no ".." above the
     * start, no magic link; a create stays inside. */
    step = 6;
    int proc_self = open("/proc/self", O_RDONLY | O_DIRECTORY);
    if (!o2_is(dir, "sub/../sub/file", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, 0) ||
        !o2_is(dir, "rel", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, 0) ||
        !o2_is(dir, "/tmp/musl_o2/sub/file", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, EXDEV) ||
        !o2_is(dir, "../musl_o2/sub/file", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, EXDEV) ||
        !o2_is(dir, "abs", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, EXDEV) ||
        !o2_is(dir, "up", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, EXDEV) ||
        !o2_is(proc_self, "exe", O_RDONLY, 0, MUSL_RESOLVE_BENEATH, EXDEV) ||
        !o2_is(dir, "sub/new", O_CREAT | O_WRONLY, 0600, MUSL_RESOLVE_BENEATH, 0) ||
        !o2_is(dir, "../musl_o2/new", O_CREAT | O_WRONLY, 0600, MUSL_RESOLVE_BENEATH, EXDEV))
        f |= 2048;
    struct stat st;
    if (stat("/tmp/musl_o2/sub/new", &st) || stat("/tmp/musl_o2/new", &st) == 0) f |= 2048;

    /* RESOLVE_IN_ROOT: "/" and ".." stay at the start, as under chroot. */
    step = 7;
    if (!o2_is(dir, "/sub/file", O_RDONLY, 0, MUSL_RESOLVE_IN_ROOT, 0) ||
        !o2_is(dir, "../../sub/file", O_RDONLY, 0, MUSL_RESOLVE_IN_ROOT, 0) ||
        !o2_is(dir, "/proc/version", O_RDONLY, 0, MUSL_RESOLVE_IN_ROOT, ENOENT) ||
        !o2_is(dir, "abs", O_RDONLY, 0, MUSL_RESOLVE_IN_ROOT, ENOENT) ||
        !o2_is(proc_self, "exe", O_RDONLY, 0, MUSL_RESOLVE_IN_ROOT, EXDEV))
        f |= 4096;

    /* openat's behaviour without resolve flags; O_CLOEXEC is kept. */
    step = 8;
    fd = o2(dir, "sub/file", O_RDONLY | O_CLOEXEC, 0, 0);
    if (fd < 0 || !(fcntl((int)fd, F_GETFD) & FD_CLOEXEC)) f |= 8192;
    if (fd >= 0) close((int)fd);

    close(proc_self);
    close(shm);
    close(dir);
    unlink("/dev/shm/musl_o2_new");
    unlink("/dev/shm/musl_o2_abs");
    unlink("/tmp/musl_o2/sub/new");
    unlink("/tmp/musl_o2/sub/file");
    unlink("/tmp/musl_o2/abs");
    unlink("/tmp/musl_o2/rel");
    unlink("/tmp/musl_o2/up");
    unlink("/tmp/musl_o2/fifo");
    rmdir("/tmp/musl_o2/sub");
    rmdir(base);
    snprintf(why, sizeof(why), "f=0x%x step=%d errno=%d", f, step, errno);
    report("openat2", f == 0, why);
}

#ifndef AT_EXECVE_CHECK
#define AT_EXECVE_CHECK 0x10000
#endif

/*
 * `--exe EXE EXECFN`: the exec under test landed here; exit 0 when
 * /proc/self/exe is EXE and AT_EXECFN is EXECFN, else which differed.
 */
static int print_exe_check(const char *exe, const char *execfn)
{
    char buf[512];
    ssize_t n = readlink("/proc/self/exe", buf, sizeof(buf) - 1);
    const char *fn = (const char *)getauxval(AT_EXECFN);
    int bad = 0;
    if (n < 0 || (buf[n] = 0, strcmp(buf, exe) != 0)) bad |= 1;
    if (!fn || strcmp(fn, execfn) != 0) bad |= 2;
    return bad;
}

static long musl_execveat(int dfd, const char *path, char *const *argv, char *const *envp,
                          int flags)
{
    return syscall(SYS_execveat, dfd, path, argv, envp, flags);
}

/* The exit status a child gives when its exec failed with `e`. */
#define EXEC_ERR(e) (200 + ((e) & 0x3f))

/* Run execveat in a child: its exit status, EXEC_ERR(errno) when the call
 * failed, or -1 when the child died otherwise. */
static int execveat_status(int dfd, const char *path, char *const *argv, int flags)
{
    char *env[] = {NULL};
    pid_t pid = fork();
    if (pid == 0) {
        musl_execveat(dfd, path, argv, env, flags);
        _exit(EXEC_ERR(errno));
    }
    int status;
    if (pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status)) return -1;
    return WEXITSTATUS(status);
}

/*
 * execveat (group 3 step D3c): a dirfd, AT_FDCWD, AT_EMPTY_PATH on a
 * descriptor, the names the program gets (/proc/self/exe, AT_EXECFN),
 * AT_EXECVE_CHECK, a script from a close-on-exec descriptor, and the errors
 * in Linux's order. execve of a relative name runs it from the working
 * directory, without a PATH search.
 */
static void test_execveat(void)
{
    const char *plain = "/tmp/musl_execveat_plain", *script = "/tmp/musl_execveat.sh";
    const char *junk = "/tmp/musl_execveat_junk", *link_path = "/tmp/musl_execveat_ln";
    char why[240], self[512], dir[512], fdname[64], fdexecfn[600];
    int f = 0;
    char *t_argv[] = {"true", NULL}, *f_argv[] = {"false", NULL};
    char *env[] = {NULL};

    ssize_t n = readlink("/proc/self/exe", self, sizeof(self) - 1);
    if (n <= 0) {
        report("execveat", 0, "readlink /proc/self/exe");
        return;
    }
    self[n] = 0;
    /* The kernel's launcher records the name it was given, which may run
     * through a symlink; exec records the file's own path. */
    char real[512];
    if (realpath(self, real)) strcpy(self, real);
    strcpy(dir, self);
    *strrchr(dir, '/') = 0;
    const char *base = strrchr(self, '/') + 1;

    /* AT_FDCWD with an absolute name; a dirfd with a relative one. */
    if (execveat_status(AT_FDCWD, "/bin/true", t_argv, 0) != 0) f |= 1;
    int bin = open("/bin", O_RDONLY | O_DIRECTORY);
    if (bin < 0 || execveat_status(bin, "true", t_argv, 0) != 0 ||
        execveat_status(bin, "false", f_argv, 0) != 1)
        f |= 2;

    /* The program's names: a dirfd-relative exec is /dev/fd/N/name to
     * AT_EXECFN and the file itself to /proc/self/exe; AT_EMPTY_PATH is
     * /dev/fd/N. */
    int dfd = open(dir, O_RDONLY | O_DIRECTORY);
    snprintf(fdexecfn, sizeof(fdexecfn), "/dev/fd/%d/%s", dfd, base);
    char *names_argv[] = {"x", "--exe", self, fdexecfn, NULL};
    if (dfd < 0 || execveat_status(dfd, base, names_argv, 0) != 0) f |= 4;
    int exe = open(self, O_RDONLY);
    snprintf(fdname, sizeof(fdname), "/dev/fd/%d", exe);
    char *empty_argv[] = {"x", "--exe", self, fdname, NULL};
    if (exe < 0 || execveat_status(exe, "", empty_argv, AT_EMPTY_PATH) != 0) f |= 8;
    /* An absolute name is itself, whatever the dirfd. */
    char *abs_argv[] = {"x", "--exe", self, self, NULL};
    if (execveat_status(bin, self, abs_argv, 0) != 0) f |= 16;

    /* execve("name") runs ./name: no PATH search, and /proc/self/exe is
     * the canonical path. */
    char *rel_argv[] = {"x", "--exe", self, (char *)base, NULL};
    pid_t pid = fork();
    if (pid == 0) {
        if (chdir(dir) == 0) execve(base, rel_argv, env);
        _exit(EXEC_ERR(errno));
    }
    int status = -1;
    if (pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status) ||
        WEXITSTATUS(status) != 0)
        f |= 32;
    pid = fork();
    if (pid == 0) {
        if (chdir("/tmp") == 0) execve("true", t_argv, env);
        _exit(EXEC_ERR(errno));
    }
    if (pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status) ||
        WEXITSTATUS(status) != EXEC_ERR(ENOENT))
        f |= 64;

    /* A script run from a close-on-exec descriptor: its interpreter could
     * not open /dev/fd/N after the exec, so ENOENT up front. */
    int sfd = open(script, O_CREAT | O_WRONLY | O_TRUNC, 0755);
    if (sfd < 0 || write(sfd, "#!/bin/sh\nexit 3\n", 17) != 17) f |= 128;
    close(sfd);
    if (execveat_status(AT_FDCWD, script, t_argv, 0) != 3) f |= 128;
    sfd = open(script, O_RDONLY | O_CLOEXEC);
    if (sfd < 0 || execveat_status(sfd, "", t_argv, AT_EMPTY_PATH) != EXEC_ERR(ENOENT))
        f |= 256;

    /* AT_EXECVE_CHECK: the checks and nothing else, so it returns here,
     * also for a file that is not a program; no execute bit is EACCES. */
    int jfd = open(junk, O_CREAT | O_WRONLY | O_TRUNC, 0755);
    if (jfd < 0 || write(jfd, "junk", 4) != 4) f |= 512;
    close(jfd);
    errno = 0;
    if (musl_execveat(AT_FDCWD, "/bin/true", t_argv, env, AT_EXECVE_CHECK) != 0 ||
        musl_execveat(AT_FDCWD, junk, t_argv, env, AT_EXECVE_CHECK) != 0 ||
        musl_execveat(exe, "", NULL, NULL, AT_EMPTY_PATH | AT_EXECVE_CHECK) != 0)
        f |= 1024;
    int pfd = open(plain, O_CREAT | O_WRONLY | O_TRUNC, 0644);
    close(pfd);
    errno = 0;
    if (musl_execveat(AT_FDCWD, plain, t_argv, env, AT_EXECVE_CHECK) != -1 || errno != EACCES)
        f |= 2048;

    /* Errors: the name first (EFAULT, ENOENT for an empty one without
     * AT_EMPTY_PATH), then unknown flags (EINVAL), then the lookup (EBADF,
     * ENOTDIR, ELOOP, ENOENT), then the file (EACCES for a directory, a
     * file without x), then argv (EFAULT). */
    struct {
        int dfd;
        const char *path;
        char *const *argv;
        int flags, err;
    } cases[] = {
        {AT_FDCWD, (const char *)1, t_argv, 0x1, EFAULT},
        {AT_FDCWD, "", t_argv, 0x1, ENOENT},
        {AT_FDCWD, "/bin/true", t_argv, 0x1, EINVAL},
        {AT_FDCWD, "/bin/true", t_argv, AT_SYMLINK_FOLLOW, EINVAL},
        {-1, "", t_argv, AT_EMPTY_PATH, EBADF},
        {-1, "true", t_argv, 0, EBADF},
        {exe, "true", t_argv, 0, ENOTDIR},
        {AT_FDCWD, link_path, t_argv, AT_SYMLINK_NOFOLLOW, ELOOP},
        {AT_FDCWD, "/tmp/musl_execveat_none", (char *const *)1, 0, ENOENT},
        {AT_FDCWD, "/tmp", t_argv, 0, EACCES},
        {AT_FDCWD, plain, t_argv, 0, EACCES},
        {AT_FDCWD, "/bin/true/", t_argv, 0, ENOTDIR},
        {AT_FDCWD, "/bin/true", (char *const *)1, 0, EFAULT},
    };
    unlink(link_path);
    if (symlink("/bin/true", link_path)) f |= 4096;
    int bad_case = -1, bad_errno = 0;
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        errno = 0;
        long r = musl_execveat(cases[i].dfd, cases[i].path, cases[i].argv, env, cases[i].flags);
        if (r != -1 || errno != cases[i].err) {
            if (bad_case < 0) bad_case = (int)i, bad_errno = errno;
            f |= 8192;
        }
    }
    /* Following the symlink is fine. */
    if (execveat_status(AT_FDCWD, link_path, t_argv, 0) != 0) f |= 16384;

    close(bin);
    close(dfd);
    close(exe);
    close(sfd);
    unlink(script);
    unlink(junk);
    unlink(plain);
    unlink(link_path);
    snprintf(why, sizeof(why), "f=0x%x case=%d errno=%d", f, bad_case, bad_errno);
    report("execveat", f == 0, why);
}

#ifndef AT_HANDLE_FID
#define AT_HANDLE_FID 0x200
#endif
#ifndef AT_HANDLE_MNT_ID_UNIQUE
#define AT_HANDLE_MNT_ID_UNIQUE 0x1
#endif
#ifndef AT_HANDLE_CONNECTABLE
#define AT_HANDLE_CONNECTABLE 0x2
#endif

/* A file handle with room for MAX_HANDLE_SZ bytes. */
union fh_buf {
    struct file_handle fh;
    unsigned char bytes[sizeof(struct file_handle) + MAX_HANDLE_SZ];
};

/* name_to_handle_at into `b` with all the room there is. */
static int fh_get(int dfd, const char *name, union fh_buf *b, int *mnt, int flags)
{
    b->fh.handle_bytes = MAX_HANDLE_SZ;
    return name_to_handle_at(dfd, name, &b->fh, mnt, flags);
}

/* Whether a call returned -1 with errno `e`. */
static int fails_with(long r, int e)
{
    return r == -1 && errno == e;
}

/*
 * name_to_handle_at and open_by_handle_at (group 3 step D4): the size
 * probe, a handle that opens the same file (by path, by AT_EMPTY_PATH on a
 * descriptor, after following a symlink or not), the mount id statx
 * reports, AT_HANDLE_FID on procfs, connectable handles below their anchor
 * only, and the errors in Linux's order, EPERM without CAP_DAC_READ_SEARCH
 * and ESTALE once the file is gone.
 */
static void test_fhandle(void)
{
    const char *dir = "/tmp/musl_fh", *file = "/tmp/musl_fh/f", *link_path = "/tmp/musl_fh/ln";
    const char *other = "/tmp/musl_fh_other";
    union fh_buf b, d, l, c;
    char why[160], buf[8];
    int f = 0, mnt = -1, mnt2 = -1;

    mkdir(dir, 0755);
    mkdir(other, 0755);
    int wfd = open(file, O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (wfd < 0 || write(wfd, "hi", 2) != 2) f |= 1;
    close(wfd);
    unlink(link_path);
    if (symlink("f", link_path)) f |= 1;

    /* The size probe: EOVERFLOW, with the size needed written back. */
    struct file_handle probe = {0};
    errno = 0;
    if (!fails_with(name_to_handle_at(AT_FDCWD, file, &probe, &mnt, 0), EOVERFLOW) ||
        probe.handle_bytes == 0 || probe.handle_bytes > MAX_HANDLE_SZ)
        f |= 2;

    /* The handle opens the same file; the mount id is statx's. */
    struct statx sx;
    struct stat st, ost;
    if (fh_get(AT_FDCWD, file, &b, &mnt, 0) != 0 || b.fh.handle_bytes != probe.handle_bytes ||
        statx(AT_FDCWD, file, 0, STATX_MNT_ID, &sx) != 0 || (unsigned)mnt != sx.stx_mnt_id)
        f |= 4;
    uint64_t umnt = 0;
    if (fh_get(AT_FDCWD, file, &c, (int *)&umnt, AT_HANDLE_MNT_ID_UNIQUE) != 0 ||
        umnt <= (1ULL << 31))
        f |= 4;
    int fd = open_by_handle_at(AT_FDCWD, &b.fh, O_RDONLY);
    if (fd < 0 || read(fd, buf, sizeof(buf)) != 2 || memcmp(buf, "hi", 2) != 0 ||
        fstat(fd, &ost) != 0 || stat(file, &st) != 0 || ost.st_ino != st.st_ino)
        f |= 8;
    close(fd);

    /* AT_EMPTY_PATH on a directory descriptor, opened as a directory;
     * writing a directory is EISDIR. */
    int dfd = open(dir, O_RDONLY | O_DIRECTORY);
    if (dfd < 0 || fh_get(dfd, "", &d, &mnt2, AT_EMPTY_PATH) != 0 || mnt2 != mnt) f |= 16;
    fd = open_by_handle_at(dfd, &d.fh, O_RDONLY | O_DIRECTORY);
    if (fd < 0 || fstat(fd, &ost) != 0 || !S_ISDIR(ost.st_mode)) f |= 16;
    close(fd);
    errno = 0;
    if (!fails_with(open_by_handle_at(dfd, &d.fh, O_RDWR), EISDIR)) f |= 16;

    /* A symlink's own handle opens as ELOOP; following it, the target. */
    if (fh_get(AT_FDCWD, link_path, &l, &mnt, 0) != 0) f |= 32;
    errno = 0;
    if (!fails_with(open_by_handle_at(AT_FDCWD, &l.fh, O_RDONLY), ELOOP)) f |= 32;
    if (fh_get(AT_FDCWD, link_path, &l, &mnt, AT_SYMLINK_FOLLOW) != 0 ||
        l.fh.handle_type != b.fh.handle_type ||
        memcmp(l.fh.f_handle, b.fh.f_handle, b.fh.handle_bytes) != 0)
        f |= 32;

    /* procfs cannot decode handles: EOPNOTSUPP, unless AT_HANDLE_FID. */
    errno = 0;
    if (!fails_with(fh_get(AT_FDCWD, "/proc/self/status", &c, &mnt, 0), EOPNOTSUPP) ||
        fh_get(AT_FDCWD, "/proc/self/status", &c, &mnt, AT_HANDLE_FID) != 0)
        f |= 64;

    /* A connectable handle opens from its directory, not from elsewhere. */
    int ofd = open(other, O_RDONLY | O_DIRECTORY);
    if (fh_get(AT_FDCWD, file, &c, &mnt, AT_HANDLE_CONNECTABLE) != 0) f |= 128;
    fd = open_by_handle_at(dfd, &c.fh, O_RDONLY);
    if (fd < 0) f |= 128;
    close(fd);
    errno = 0;
    if (ofd < 0 || !fails_with(open_by_handle_at(ofd, &c.fh, O_RDONLY), ESTALE)) f |= 128;

    /* Errors: flags, then the name, then the lookup, then the header. */
    struct file_handle big = {.handle_bytes = MAX_HANDLE_SZ + 1};
    int bad_case = -1, bad_errno = 0;
    int idx = 0;
#define FH_CASE(call, e)                                   \
    do {                                                   \
        errno = 0;                                         \
        long r_ = (call);                                  \
        if (!fails_with(r_, e) && bad_case < 0) {          \
            bad_case = idx;                                \
            bad_errno = errno;                             \
        }                                                  \
        idx++;                                             \
    } while (0)
    FH_CASE(fh_get(AT_FDCWD, file, &c, &mnt, -1), EINVAL);
    FH_CASE(fh_get(AT_FDCWD, file, &c, &mnt, AT_HANDLE_FID | AT_HANDLE_CONNECTABLE), EINVAL);
    FH_CASE(fh_get(AT_FDCWD, "", &c, &mnt, 0), ENOENT);
    FH_CASE(fh_get(-1, "f", &c, &mnt, 0), EBADF);
    FH_CASE(fh_get(AT_FDCWD, (const char *)1, &c, &mnt, 0), EFAULT);
    FH_CASE(name_to_handle_at(AT_FDCWD, file, (struct file_handle *)1, &mnt, 0), EFAULT);
    FH_CASE(name_to_handle_at(AT_FDCWD, file, &big, &mnt, 0), EINVAL);
    FH_CASE(fh_get(AT_FDCWD, file, &c, (int *)1, 0), EFAULT);
    FH_CASE(open_by_handle_at(AT_FDCWD, (struct file_handle *)1, O_RDONLY), EFAULT);
    FH_CASE(open_by_handle_at(AT_FDCWD, &big, O_RDONLY), EINVAL);
    FH_CASE(open_by_handle_at(-1, &b.fh, O_RDONLY), EBADF);
#undef FH_CASE
    if (bad_case >= 0) f |= 256;

    /* Without CAP_DAC_READ_SEARCH (an unprivileged child): EPERM. */
    pid_t pid = fork();
    if (pid == 0) {
        if (setuid(65534) != 0) _exit(2);
        errno = 0;
        _exit(fails_with(open_by_handle_at(AT_FDCWD, &b.fh, O_RDONLY), EPERM) ? 0 : 1);
    }
    int status = -1;
    if (pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status) ||
        WEXITSTATUS(status) != 0)
        f |= 512;

    /* Gone, or decoded against a pipe: ESTALE. */
    int p[2];
    if (pipe(p) == 0) {
        errno = 0;
        if (!fails_with(open_by_handle_at(p[0], &b.fh, O_RDONLY), ESTALE)) f |= 1024;
        close(p[0]);
        close(p[1]);
    }
    unlink(link_path);
    unlink(file);
    errno = 0;
    if (!fails_with(open_by_handle_at(AT_FDCWD, &b.fh, O_RDONLY), ESTALE)) f |= 1024;

    close(dfd);
    close(ofd);
    rmdir(dir);
    rmdir(other);
    snprintf(why, sizeof(why), "f=0x%x case=%d errno=%d", f, bad_case, bad_errno);
    report("fhandle", f == 0, why);
}

/*
 * ioctl (group 3 step E1, N-198): a descriptor is a terminal by what it
 * is, not by its number -- a file is not one even as fd 0, /dev/console
 * is one above 2; an unknown request is ENOTTY, a closed fd EBADF before
 * anything else; FIONREAD, FIONBIO and FIOCLEX/FIONCLEX work on any file.
 */
static void test_ioctl(void)
{
    const char *path = "/tmp/musl_ioctl";
    struct termios t;
    char why[96], buf[4];
    int f = 0, n = -1;

    int fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0644);
    if (fd < 0 || write(fd, "abcdef", 6) != 6 || lseek(fd, 2, SEEK_SET) != 2) f |= 1;
    /* A file: FIONREAD is size minus position; no terminal calls. */
    if (ioctl(fd, FIONREAD, &n) != 0 || n != 4) f |= 2;
    errno = 0;
    if (!fails_with(ioctl(fd, TCGETS, &t), ENOTTY) || isatty(fd)) f |= 4;
    errno = 0;
    if (!fails_with(ioctl(fd, 0x7fff, 0), ENOTTY)) f |= 4;
    /* The same file as fd 0 is still not a terminal. */
    int saved = dup(0);
    if (saved < 0 || dup2(fd, 0) != 0) f |= 8;
    errno = 0;
    if (!fails_with(ioctl(0, TCGETS, &t), ENOTTY)) f |= 8;
    if (saved >= 0) {
        dup2(saved, 0);
        close(saved);
    }

    /* FIOCLEX and FIONCLEX are F_SETFD's close-on-exec. */
    if (ioctl(fd, FIOCLEX) != 0 || fcntl(fd, F_GETFD) != FD_CLOEXEC) f |= 16;
    if (ioctl(fd, FIONCLEX) != 0 || fcntl(fd, F_GETFD) != 0) f |= 16;
    close(fd);
    unlink(path);

    /* A closed descriptor: EBADF, whatever the request. */
    errno = 0;
    if (!fails_with(ioctl(fd, TCGETS, &t), EBADF)) f |= 32;
    errno = 0;
    if (!fails_with(ioctl(fd, FIONREAD, &n), EBADF)) f |= 32;

    /* A pipe: FIONREAD on either end; FIONBIO is O_NONBLOCK. */
    int p[2];
    if (pipe(p) != 0 || write(p[1], "hello", 5) != 5) f |= 64;
    if (ioctl(p[0], FIONREAD, &n) != 0 || n != 5) f |= 64;
    if (ioctl(p[1], FIONREAD, &n) != 0 || n != 5) f |= 64;
    if (read(p[0], buf, 2) != 2 || ioctl(p[0], FIONREAD, &n) != 0 || n != 3) f |= 64;
    int on = 1, off = 0;
    if (read(p[0], buf, 3) != 3 || ioctl(p[0], FIONBIO, &on) != 0 ||
        !(fcntl(p[0], F_GETFL) & O_NONBLOCK))
        f |= 128;
    errno = 0;
    if (!fails_with(read(p[0], buf, 1), EAGAIN)) f |= 128;
    if (ioctl(p[0], FIONBIO, &off) != 0 || (fcntl(p[0], F_GETFL) & O_NONBLOCK)) f |= 128;
    if (ioctl(p[0], FIOASYNC, &off) != 0) f |= 128;
    errno = 0;
    if (!fails_with(ioctl(p[0], TCGETS, &t), ENOTTY)) f |= 128;
    close(p[0]);
    close(p[1]);

    /* /dev/console is a terminal at any fd number; /dev/null is not. */
    int con = open("/dev/console", O_RDWR | O_NOCTTY);
    if (con < 3 || ioctl(con, TCGETS, &t) != 0 || !isatty(con)) f |= 256;
    close(con);
    int null = open("/dev/null", O_RDWR);
    errno = 0;
    if (null < 0 || !fails_with(ioctl(null, TCGETS, &t), ENOTTY) ||
        !fails_with(ioctl(null, FIONREAD, &n), ENOTTY))
        f |= 512;
    close(null);

    snprintf(why, sizeof(why), "f=0x%x n=%d errno=%d", f, n, errno);
    report("ioctl", f == 0, why);
}

/* One fcntl record-lock request on [start, start + len). */
static int setlk(int fd, int cmd, short type, off_t start, off_t len)
{
    struct flock fl = {.l_type = type, .l_whence = SEEK_SET, .l_start = start, .l_len = len};
    return fcntl(fd, cmd, &fl);
}

/* The lock F_GETLK (or F_OFD_GETLK) finds in the way of a write lock on
 * [start, start + len), in *fl. */
static int getlk(int fd, int cmd, off_t start, off_t len, struct flock *fl)
{
    memset(fl, 0, sizeof(*fl));
    fl->l_type = F_WRLCK;
    fl->l_whence = SEEK_SET;
    fl->l_start = start;
    fl->l_len = len;
    return fcntl(fd, cmd, fl);
}

/* fcntl record locks (N-199): POSIX locks belong to the descriptor table
 * and go with any close of the file, OFD locks to the open file; a cycle
 * of F_SETLKW waiters is EDEADLK for the one that would close it. */
static void test_record_locks(void)
{
    const char *path = "/tmp/musl_reclock";
    struct flock fl;
    char why[96];
    int f = 0, child = -1;

    int fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0644);
    /* [0, 100) less [40, 60): two locks with a hole. */
    if (fd < 0 || setlk(fd, F_SETLK, F_WRLCK, 0, 100) != 0 ||
        setlk(fd, F_SETLK, F_UNLCK, 40, 20) != 0)
        f |= 1;
    /* An owner's own locks are never in its way. */
    if (getlk(fd, F_GETLK, 0, 100, &fl) != 0 || fl.l_type != F_UNLCK) f |= 1;
    pid_t parent = getpid();
    pid_t pid = fork();
    if (pid == 0) {
        int g = 0;
        /* The inherited descriptor is in another table: the locks are
         * the parent's. */
        if (getlk(fd, F_GETLK, 10, 1, &fl) != 0 || fl.l_type != F_WRLCK || fl.l_pid != parent ||
            fl.l_start != 0 || fl.l_len != 40)
            g |= 1;
        errno = 0;
        if (!fails_with(setlk(fd, F_SETLK, F_RDLCK, 70, 1), EAGAIN)) g |= 2;
        if (setlk(fd, F_SETLK, F_WRLCK, 45, 10) != 0) g |= 4;
        _exit(g);
    }
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) {
        f |= 2;
        child = WIFEXITED(st) ? WEXITSTATUS(st) : -1;
    }
    /* The child's lock went with it. */
    if (getlk(fd, F_GETLK, 40, 20, &fl) != 0 || fl.l_type != F_UNLCK) f |= 4;

    /* Closing any descriptor of the file drops the table's locks. */
    int other = open(path, O_RDONLY);
    if (other < 0 || close(other) != 0) f |= 8;
    pid = fork();
    if (pid == 0)
        _exit(setlk(fd, F_SETLK, F_WRLCK, 0, 0) == 0 ? 0 : 1);
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 8;

    /* An OFD lock conflicts with the same process's POSIX lock, and the
     * POSIX lock's holder is reported; its own holder is -1. */
    int ofd = open(path, O_RDWR);
    if (ofd < 0 || setlk(fd, F_SETLK, F_WRLCK, 0, 10) != 0) f |= 16;
    errno = 0;
    if (!fails_with(setlk(ofd, F_OFD_SETLK, F_WRLCK, 5, 1), EAGAIN)) f |= 16;
    if (getlk(ofd, F_OFD_GETLK, 5, 1, &fl) != 0 || fl.l_type != F_WRLCK || fl.l_pid != parent)
        f |= 16;
    if (setlk(ofd, F_OFD_SETLK, F_RDLCK, 200, 0) != 0) f |= 32;
    errno = 0;
    if (!fails_with(setlk(fd, F_SETLK, F_WRLCK, 300, 1), EAGAIN)) f |= 32;
    if (getlk(fd, F_GETLK, 300, 1, &fl) != 0 || fl.l_type != F_RDLCK || fl.l_pid != -1 ||
        fl.l_start != 200 || fl.l_len != 0)
        f |= 32;
    /* An OFD request naming a pid is EINVAL; closing the open file drops
     * its lock. */
    struct flock bad = {.l_type = F_RDLCK, .l_whence = SEEK_SET, .l_pid = 1};
    errno = 0;
    if (!fails_with(fcntl(ofd, F_OFD_SETLK, &bad), EINVAL)) f |= 64;
    close(ofd);
    if (setlk(fd, F_SETLK, F_WRLCK, 300, 1) != 0) f |= 64;

    /* The descriptor must be open for the lock's kind, and F_GETLK asks
     * about a read or a write lock. */
    int ro = open(path, O_RDONLY);
    errno = 0;
    if (ro < 0 || !fails_with(setlk(ro, F_SETLK, F_WRLCK, 500, 1), EBADF)) f |= 128;
    close(ro);
    if (setlk(fd, F_SETLK, F_UNLCK, 0, 0) != 0) f |= 128;
    struct flock unl = {.l_type = F_UNLCK, .l_whence = SEEK_SET};
    errno = 0;
    if (!fails_with(fcntl(fd, F_GETLK, &unl), EINVAL)) f |= 128;

    /* Deadlock: the parent holds byte 0, the child byte 1, and each asks
     * for the other's. Whichever asks second closes the cycle and gets
     * EDEADLK, then gives its byte up so the other is granted. */
    int sync[2];
    if (pipe(sync) != 0 || setlk(fd, F_SETLK, F_WRLCK, 0, 1) != 0) f |= 256;
    pid = fork();
    if (pid == 0) {
        char c = 'r';
        if (setlk(fd, F_SETLK, F_WRLCK, 1, 1) != 0 || write(sync[1], &c, 1) != 1) _exit(2);
        if (setlk(fd, F_SETLKW, F_WRLCK, 0, 1) == 0) _exit(0);
        int e = errno;
        setlk(fd, F_SETLK, F_UNLCK, 1, 1);
        _exit(e == EDEADLK ? 1 : 3);
    }
    char c;
    int parent_deadlk = 0;
    if (pid < 0 || read(sync[0], &c, 1) != 1) f |= 256;
    if (setlk(fd, F_SETLKW, F_WRLCK, 1, 1) != 0) {
        parent_deadlk = errno == EDEADLK ? 1 : 2;
        setlk(fd, F_SETLK, F_UNLCK, 0, 1);
    }
    st = 0;
    int child_deadlk = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) ? WEXITSTATUS(st) : 9;
    if (!((parent_deadlk == 1 && child_deadlk == 0) || (parent_deadlk == 0 && child_deadlk == 1)))
        f |= 512;
    close(sync[0]);
    close(sync[1]);
    close(fd);
    unlink(path);

    snprintf(why, sizeof(why), "f=0x%x child=%d deadlk=%d/%d errno=%d", f, child, parent_deadlk,
             child_deadlk, errno);
    report("record_locks", f == 0, why);
}

#ifndef F_DUPFD_QUERY
#define F_DUPFD_QUERY 1027
#endif

/* fcntl (N-199): every command on a closed descriptor is EBADF and an
 * unknown one EINVAL; F_SETFL changes O_APPEND (which writes then honour)
 * and the other status flags, never the access mode. */
static void test_fcntl_flags(void)
{
    const char *path = "/tmp/musl_fcntl_fl";
    struct flock fl = {.l_type = F_RDLCK, .l_whence = SEEK_SET};
    char why[96], buf[8] = {0};
    int f = 0;

    int fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0644);
    if (fd < 0 || write(fd, "abc", 3) != 3 || lseek(fd, 0, SEEK_SET) != 0) f |= 1;
    int fl0 = fcntl(fd, F_GETFL);
    if (fl0 < 0 || (fl0 & O_ACCMODE) != O_RDWR || (fl0 & O_APPEND)) f |= 1;
    /* O_APPEND on: the write goes to the end. */
    if (fcntl(fd, F_SETFL, fl0 | O_APPEND) != 0 || !(fcntl(fd, F_GETFL) & O_APPEND)) f |= 2;
    if (write(fd, "d", 1) != 1 || lseek(fd, 0, SEEK_CUR) != 4) f |= 2;
    /* Off again: the write goes to the position. */
    if (fcntl(fd, F_SETFL, fl0) != 0 || lseek(fd, 0, SEEK_SET) != 0 || write(fd, "X", 1) != 1)
        f |= 4;
    if (pread(fd, buf, sizeof(buf), 0) != 4 || memcmp(buf, "Xbcd", 4) != 0) f |= 4;
    /* The access mode is not F_SETFL's to change. */
    if (fcntl(fd, F_SETFL, O_WRONLY | O_ASYNC) != 0 || (fcntl(fd, F_GETFL) & O_ACCMODE) != O_RDWR ||
        !(fcntl(fd, F_GETFL) & O_ASYNC))
        f |= 8;
    fcntl(fd, F_SETFL, fl0);
    /* F_DUPFD_QUERY: the same open file or not. */
    int d = dup(fd), other = open(path, O_RDONLY);
    if (fcntl(fd, F_DUPFD_QUERY, d) != 1 || fcntl(fd, F_DUPFD_QUERY, other) != 0) f |= 16;
    errno = 0;
    if (!fails_with(fcntl(fd, 0x7fff, 0), EINVAL)) f |= 32;
    close(d);
    close(other);

    /* O_NOATIME is the owner's: another user is refused. */
    pid_t pid = fork();
    if (pid == 0)
        _exit(setresuid(65534, 65534, 65534) == 0 && fcntl(fd, F_SETFL, O_NOATIME) == -1 &&
                      errno == EPERM
                  ? 0
                  : 1);
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 64;
    if (fcntl(fd, F_SETFL, O_NOATIME) != 0 || !(fcntl(fd, F_GETFL) & O_NOATIME)) f |= 64;

    /* A closed descriptor: EBADF whatever the command. */
    close(fd);
    unlink(path);
    static const int cmds[] = {F_DUPFD, F_GETFD, F_SETFD,       F_GETFL, F_SETFL,
                               F_GETLK, F_SETLK, F_DUPFD_QUERY, 0x7fff};
    for (size_t i = 0; i < sizeof(cmds) / sizeof(cmds[0]); i++) {
        errno = 0;
        if (!fails_with(fcntl(fd, cmds[i], &fl), EBADF)) f |= 128;
    }

    snprintf(why, sizeof(why), "f=0x%x fl0=0x%x errno=%d", f, fl0, errno);
    report("fcntl_flags", f == 0, why);
}

/* readv and writev are one transfer each (N-200): a readv of a pipe takes
 * what is there instead of waiting again for a later buffer, a writev of
 * at most PIPE_BUF bytes to a pipe goes in whole or not at all, buffers
 * are filled and drained in order, and a buffer outside user space is
 * EFAULT before anything is read. The descriptor is checked first. */
static void test_vectored_io(void)
{
    static char page[4096];
    char why[96], a[8], b[128], x[50], y[100];
    int f = 0, p[2];

    /* Five bytes in the pipe, buffers of 5 and 100, a writer still open:
     * readv returns 5 instead of waiting for the second buffer. */
    if (pipe(p) != 0) f |= 1;
    pid_t pid = fork();
    if (pid == 0) {
        alarm(5);
        struct iovec r[2] = {{a, 5}, {b, 100}};
        if (write(p[1], "hello", 5) != 5) _exit(90);
        ssize_t n = readv(p[0], r, 2);
        _exit(n == 5 && memcmp(a, "hello", 5) == 0 ? 0 : 91);
    }
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0)
        f |= 2;

    /* In order across buffers, both ways. */
    struct iovec w2[2] = {{"ab", 2}, {"cde", 3}};
    struct iovec r2[2] = {{a, 3}, {b, 10}};
    if (writev(p[1], w2, 2) != 5 || readv(p[0], r2, 2) != 5 || memcmp(a, "abc", 3) ||
        memcmp(b, "de", 2))
        f |= 4;

    /* A buffer in kernel space: EFAULT, and the data stays in the pipe. */
    struct iovec bad[2] = {{a, 2}, {(void *)(uintptr_t)-4096, 3}};
    errno = 0;
    if (write(p[1], "hello", 5) != 5 || readv(p[0], bad, 2) != -1 || errno != EFAULT ||
        read(p[0], b, sizeof(b)) != 5)
        f |= 8;
    close(p[0]);
    close(p[1]);

    /* A full O_NONBLOCK pipe with 100 bytes read out: a writev of 150
     * bytes (under PIPE_BUF) is EAGAIN, not the first buffer's 50. */
    int full = 0;
    if (pipe2(p, O_NONBLOCK) != 0) f |= 1;
    for (int i = 0; i < 1024 && !full; i++)
        if (write(p[1], page, sizeof(page)) < 0) full = errno == EAGAIN;
    memset(x, 'x', sizeof(x));
    memset(y, 'y', sizeof(y));
    struct iovec w[2] = {{x, sizeof(x)}, {y, sizeof(y)}};
    errno = 0;
    ssize_t n = 0;
    if (!full || read(p[0], b, 100) != 100 || (n = writev(p[1], w, 2)) != -1 || errno != EAGAIN)
        f |= 16;
    close(p[0]);
    close(p[1]);

    /* A bad descriptor is EBADF even with no buffers. */
    errno = 0;
    if (readv(-1, NULL, 0) != -1 || errno != EBADF) f |= 32;
    errno = 0;
    if (writev(-1, NULL, 0) != -1 || errno != EBADF) f |= 32;

    snprintf(why, sizeof(why), "f=%#x st=%#x n=%zd errno=%d", f, st, n, errno);
    report("vectored_io", f == 0, why);
}

/* Hard and symbolic links fail as on Linux (N-202, N-203, N-206): an
 * existing new name is EEXIST (with or without a trailing slash), a
 * missing one with a slash ENOENT, a directory EPERM, unknown linkat flags
 * EINVAL; linkat follows a final symlink only with AT_SYMLINK_FOLLOW and
 * links an open file with AT_EMPTY_PATH; a dirfd whose directory was
 * removed is ENOENT. A trailing slash on a non-directory is ENOTDIR, and
 * follows a symlink to a directory. fchown of a bad descriptor is EBADF
 * and getdents64 of a file ENOTDIR. */
static int errno_is(int ret, int want)
{
    return ret == -1 && errno == want;
}

static void test_link_errors(void)
{
    const char *d = "/tmp/musl_links";
    char why[96];
    struct stat a, b;
    int f = 0;

    mkdir(d, 0755);
    if (chdir(d) != 0) f |= 1;
    unlink("file");
    unlink("other");
    unlink("ln");
    unlink("sl");
    unlink("sl2");
    unlink("viafollow");
    unlink("vianofollow");
    unlink("viafd");
    unlink("dsl");
    rmdir("sub");
    rmdir("gone");
    int fd = open("file", O_CREAT | O_RDWR, 0644);
    int fd2 = open("other", O_CREAT | O_RDWR, 0644);
    if (fd < 0 || fd2 < 0 || mkdir("sub", 0755) != 0 || symlink("file", "sl") != 0 ||
        symlink("sub", "dsl") != 0)
        f |= 1;

    /* link */
    if (!errno_is(link("file", "other"), EEXIST)) f |= 2;
    if (!errno_is(link("file", "other/"), EEXIST)) f |= 2;
    if (!errno_is(link("file", "ln/"), ENOENT)) f |= 4;
    if (!errno_is(link("sub", "ln"), EPERM)) f |= 8;
    if (!errno_is(link("missing", "ln"), ENOENT)) f |= 4;
    if (link("file", "ln") != 0 || stat("file", &a) != 0 || stat("ln", &b) != 0 ||
        a.st_ino != b.st_ino)
        f |= 16;

    /* linkat */
    if (!errno_is(linkat(AT_FDCWD, "file", AT_FDCWD, "x", 0x1), EINVAL)) f |= 32;
    if (linkat(AT_FDCWD, "sl", AT_FDCWD, "vianofollow", 0) != 0 ||
        lstat("vianofollow", &b) != 0 || !S_ISLNK(b.st_mode))
        f |= 64;
    if (linkat(AT_FDCWD, "sl", AT_FDCWD, "viafollow", AT_SYMLINK_FOLLOW) != 0 ||
        lstat("viafollow", &b) != 0 || !S_ISREG(b.st_mode) || b.st_ino != a.st_ino)
        f |= 128;
    if (linkat(fd, "", AT_FDCWD, "viafd", AT_EMPTY_PATH) != 0 || stat("viafd", &b) != 0 ||
        b.st_ino != a.st_ino)
        f |= 256;

    /* A directory descriptor whose directory is gone. */
    int gone = -1;
    if (mkdir("gone", 0755) != 0 || (gone = open("gone", O_DIRECTORY)) < 0 || rmdir("gone") != 0)
        f |= 1;
    if (!errno_is(linkat(gone, "x", AT_FDCWD, "y", 0), ENOENT)) f |= 512;
    if (!errno_is(linkat(AT_FDCWD, "file", gone, "y", 0), ENOENT)) f |= 512;
    if (!errno_is(symlinkat("file", gone, "y"), ENOENT)) f |= 512;
    if (!errno_is(linkat(gone, "sub/file", AT_FDCWD, "y", 0), ENOENT)) f |= 512;
    if (!errno_is(symlinkat("../file", gone, "sub/y"), ENOENT)) f |= 512;
    close(gone);

    /* symlink */
    if (!errno_is(symlink("file", "other"), EEXIST)) f |= 1024;
    if (!errno_is(symlink("", "sl2"), ENOENT)) f |= 1024;

    /* Trailing slashes. */
    if (!errno_is(stat("file/", &b), ENOTDIR) || !errno_is(open("file/", O_RDONLY), ENOTDIR) ||
        !errno_is(lstat("sl/", &b), ENOTDIR))
        f |= 2048;
    if (lstat("dsl/", &b) != 0 || !S_ISDIR(b.st_mode)) f |= 4096;

    /* fchown and getdents64. */
    char buf[256];
    if (!errno_is(fchown(-1, 0, 0), EBADF)) f |= 8192;
    if (!errno_is((int)syscall(SYS_getdents64, fd, buf, sizeof(buf)), ENOTDIR)) f |= 16384;

    close(fd);
    close(fd2);
    const char *names[] = {"file", "other", "ln", "sl", "viafollow", "vianofollow", "viafd", "dsl"};
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); i++)
        unlink(names[i]);
    rmdir("sub");
    if (chdir("/") != 0) f |= 1;
    rmdir(d);
    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("link_errors", f == 0, why);
}

/* stat's numbers (N-205): st_nlink counts a file's names and a
 * directory's subdirectories (2 + n, 0 once removed), and st_dev is the
 * same for files on one mount and differs between mounts (the root and
 * /proc), fstat and fstatat agreeing. */
static void test_stat_numbers(void)
{
    const char *d = "/tmp/musl_statnum";
    char why[120];
    struct stat a, b, c, r, p;
    int f = 0;

    unlink("/tmp/musl_statnum/f");
    unlink("/tmp/musl_statnum/g");
    rmdir("/tmp/musl_statnum/s1");
    rmdir("/tmp/musl_statnum/s2");
    rmdir(d);
    if (mkdir(d, 0755) != 0 || chdir(d) != 0) f |= 1;
    int fd = open("f", O_CREAT | O_RDWR, 0644);
    if (fd < 0 || fstat(fd, &a) != 0 || a.st_nlink != 1) f |= 2;
    if (link("f", "g") != 0 || stat("f", &a) != 0 || a.st_nlink != 2 || fstat(fd, &b) != 0 ||
        b.st_nlink != 2)
        f |= 4;
    if (unlink("f") != 0 || fstat(fd, &b) != 0 || b.st_nlink != 1) f |= 8;
    if (unlink("g") != 0 || fstat(fd, &b) != 0 || b.st_nlink != 0) f |= 16;
    /* No new name for a file with none left. */
    if (!(linkat(fd, "", AT_FDCWD, "f", AT_EMPTY_PATH) == -1 && errno == ENOENT)) f |= 16;
    if (mkdir("s1", 0755) != 0 || mkdir("s2", 0755) != 0 || stat(".", &c) != 0 ||
        c.st_nlink != 4)
        f |= 32;
    int dfd = open("s2", O_DIRECTORY);
    if (dfd < 0 || rmdir("s2") != 0 || fstat(dfd, &c) != 0 || c.st_nlink != 0 ||
        stat(".", &c) != 0 || c.st_nlink != 3)
        f |= 64;
    close(dfd);
    /* One device per mount. */
    if (stat("/", &r) != 0 || stat("/proc", &p) != 0 || r.st_dev == p.st_dev) f |= 128;
    struct stat pa, pb;
    if (stat("/proc/self", &pa) != 0 || fstatat(AT_FDCWD, "/proc/self", &pb, 0) != 0 ||
        pa.st_dev != pb.st_dev || pa.st_dev != p.st_dev)
        f |= 256;
    if (stat(".", &c) != 0 || fstat(fd, &b) != 0 || c.st_dev != b.st_dev) f |= 512;
    close(fd);
    rmdir("s1");
    if (chdir("/") != 0) f |= 1;
    rmdir(d);
    snprintf(why, sizeof(why), "f=%#x nlink=%lu dev=%#lx/%#lx", f, (unsigned long)c.st_nlink,
             (unsigned long)r.st_dev, (unsigned long)p.st_dev);
    report("stat_numbers", f == 0, why);
}

/* Open flags as Linux 7.3 handles them (N-206): an O_PATH descriptor names
 * a file without opening it -- no permission check, truncation or FIFO
 * wait, other flags dropped by open(2) -- and serves only path operations
 * (fstat, *at directories and AT_EMPTY_PATH, fchdir, dup, F_GETFD/F_SETFD/
 * F_GETFL); read, write, lseek, ioctl, mmap, getdents64, fsync, fchmod and
 * F_SETFL are EBADF (fchmod by its system call: musl's falls back to
 * /proc/self/fd for an O_PATH descriptor, and musl's O_ACCMODE includes
 * O_PATH, so the access mode is the low two bits). O_TMPFILE makes an
 * unnamed file in a directory (no links) that linkat with AT_EMPTY_PATH can
 * name unless O_EXCL was given; it needs O_DIRECTORY and write access
 * (EINVAL) and a directory (ENOTDIR). O_NOATIME needs the file's owner
 * (EPERM). */
static void test_open_flags(void)
{
    const char *d = "/tmp/musl_oflags";
    char why[96], buf[64];
    struct stat st;
    int f = 0;

    unlink("/tmp/musl_oflags/file");
    unlink("/tmp/musl_oflags/sl");
    unlink("/tmp/musl_oflags/fifo");
    unlink("/tmp/musl_oflags/viapath");
    rmdir("/tmp/musl_oflags/sub");
    rmdir(d);
    if (mkdir(d, 0755) != 0 || chdir(d) != 0) f |= 1;
    int w = open("file", O_CREAT | O_WRONLY, 0644);
    if (w < 0 || write(w, "hello", 5) != 5) f |= 1;
    close(w);
    if (mkdir("sub", 0755) != 0 || symlink("file", "sl") != 0 || mkfifo("fifo", 0644) != 0)
        f |= 1;

    /* O_PATH: other flags dropped (no truncation), no I/O. */
    int p = open("file", O_PATH | O_RDWR | O_TRUNC);
    int fl = p < 0 ? 0 : fcntl(p, F_GETFL);
    if (p < 0 || stat("file", &st) != 0 || st.st_size != 5 || !(fl & O_PATH) ||
        (fl & 3) != 0)
        f |= 2;
    if (!errno_is((int)read(p, buf, 1), EBADF) || !errno_is((int)write(p, "x", 1), EBADF) ||
        !errno_is((int)lseek(p, 0, SEEK_SET), EBADF) || !errno_is((int)fsync(p), EBADF) ||
        !errno_is((int)syscall(SYS_fchmod, p, 0600), EBADF) || !errno_is((int)ioctl(p, FIONREAD, &fl), EBADF))
        f |= 4;
    if (mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, p, 0) != MAP_FAILED || errno != EBADF) f |= 4;
    if (!errno_is((int)fcntl(p, F_SETFL, O_NONBLOCK), EBADF) || fcntl(p, F_GETFD) != 0 ||
        fcntl(p, F_SETFD, FD_CLOEXEC) != 0 || fcntl(p, F_GETFD) != FD_CLOEXEC)
        f |= 8;
    /* Path operations work. */
    int p2 = dup(p);
    if (p2 < 0 || fstat(p2, &st) != 0 || !S_ISREG(st.st_mode) || st.st_size != 5) f |= 16;
    if (fstatat(p, "", &st, AT_EMPTY_PATH) != 0 || st.st_size != 5) f |= 16;
    if (linkat(p, "", AT_FDCWD, "viapath", AT_EMPTY_PATH) != 0 || stat("viapath", &st) != 0 ||
        st.st_nlink != 2)
        f |= 32;
    close(p2);
    close(p);

    /* An O_PATH directory: a dirfd and fchdir, but no getdents64. */
    int pd = open("sub", O_PATH | O_DIRECTORY);
    int made = pd < 0 ? -1 : openat(pd, "inner", O_CREAT | O_WRONLY, 0644);
    if (made < 0 || fstatat(pd, "inner", &st, 0) != 0) f |= 64;
    close(made);
    if (!errno_is((int)syscall(SYS_getdents64, pd, buf, sizeof(buf)), EBADF)) f |= 64;
    if (fchdir(pd) != 0 || stat("inner", &st) != 0 || chdir(d) != 0) f |= 64;
    unlinkat(pd, "inner", 0);
    close(pd);
    if (!errno_is((int)open("file", O_PATH | O_DIRECTORY), ENOTDIR)) f |= 64;

    /* O_PATH|O_NOFOLLOW names a symlink itself; a FIFO opens at once. */
    int ps = open("sl", O_PATH | O_NOFOLLOW);
    long n = ps < 0 ? -1 : readlinkat(ps, "", buf, sizeof(buf));
    if (ps < 0 || fstat(ps, &st) != 0 || !S_ISLNK(st.st_mode) || n != 4 ||
        memcmp(buf, "file", 4) != 0)
        f |= 128;
    close(ps);
    int pf = open("fifo", O_PATH);
    if (pf < 0 || fstat(pf, &st) != 0 || !S_ISFIFO(st.st_mode)) f |= 128;
    close(pf);

    /* O_TMPFILE, in a tmpfs. */
    int t = open("/dev/shm", O_TMPFILE | O_RDWR, 0600);
    if (t < 0 || write(t, "tmp", 3) != 3 || fstat(t, &st) != 0 || st.st_nlink != 0 ||
        !S_ISREG(st.st_mode))
        f |= 256;
    unlink("/dev/shm/musl_oflags_tmp");
    if (linkat(t, "", AT_FDCWD, "/dev/shm/musl_oflags_tmp", AT_EMPTY_PATH) != 0 ||
        fstat(t, &st) != 0 || st.st_nlink != 1)
        f |= 512;
    int r = open("/dev/shm/musl_oflags_tmp", O_RDONLY);
    if (r < 0 || read(r, buf, sizeof(buf)) != 3 || memcmp(buf, "tmp", 3) != 0) f |= 512;
    close(r);
    close(t);
    unlink("/dev/shm/musl_oflags_tmp");
    int te = open("/dev/shm", O_TMPFILE | O_RDWR | O_EXCL, 0600);
    if (te < 0 || !errno_is((int)linkat(te, "", AT_FDCWD, "/dev/shm/musl_oflags_tmp",
                                      AT_EMPTY_PATH),
                               ENOENT))
        f |= 1024;
    close(te);
    if (!errno_is((int)open("/dev/shm", O_TMPFILE | O_RDONLY, 0600), EINVAL) ||
        !errno_is((int)open("file", O_TMPFILE | O_RDWR, 0600), ENOTDIR))
        f |= 1024;

    /* O_NOATIME: the owner may, another user may not. Without
     * CAP_DAC_READ_SEARCH, linkat with AT_EMPTY_PATH links what the caller
     * opened itself, not a descriptor opened with other credentials. */
    int na = open("file", O_RDONLY | O_NOATIME);
    if (na < 0 || !(fcntl(na, F_GETFL) & O_NOATIME)) f |= 2048;
    close(na);
    int tp = open("/dev/shm", O_TMPFILE | O_RDWR, 0600);
    pid_t pid = fork();
    if (pid == 0) {
        int dropped = getuid() == 0;
        if (dropped && setuid(65534) != 0) _exit(2);
        int o = open("/", O_RDONLY | O_NOATIME);
        if (!(o == -1 && errno == EPERM)) _exit(1);
        char nm[48];
        snprintf(nm, sizeof(nm), "/dev/shm/musl_oflags_u%d", (int)getpid());
        int u = open("/dev/shm", O_TMPFILE | O_RDWR, 0600);
        if (u < 0 || linkat(u, "", AT_FDCWD, nm, AT_EMPTY_PATH) != 0) _exit(3);
        unlink(nm);
        if (dropped && !(linkat(tp, "", AT_FDCWD, nm, AT_EMPTY_PATH) == -1 && errno == ENOENT))
            _exit(4);
        _exit(0);
    }
    int status = 0;
    if (tp < 0 || pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status) ||
        WEXITSTATUS(status) != 0)
        f |= 4096;
    close(tp);

    unlink("file");
    unlink("viapath");
    unlink("sl");
    unlink("fifo");
    rmdir("sub");
    if (chdir("/") != 0) f |= 1;
    rmdir(d);
    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("open_flags", f == 0, why);
}

/* ETXTBSY (Linux's i_writecount): a program that is
 * running cannot be opened for writing or truncated, and a file open for
 * writing cannot be executed; reading it, O_PATH and writing it once the
 * process is gone are fine. The program is a copy of sleep, and the
 * checks wait until it runs. */
static int txtbsy_copy(const char *from, const char *to)
{
    char buf[4096];
    int in = open(from, O_RDONLY), ok = in >= 0;
    int out = open(to, O_CREAT | O_TRUNC | O_WRONLY, 0755);
    ok = ok && out >= 0;
    ssize_t n;
    while (ok && (n = read(in, buf, sizeof(buf))) > 0)
        ok = write(out, buf, (size_t)n) == n;
    if (in >= 0) close(in);
    if (out >= 0) close(out);
    return ok;
}

static void test_text_busy(void)
{
    const char *prog = "/tmp/musl_txtbsy";
    char why[128];
    int f = 0;

    unlink(prog);
    if (!txtbsy_copy("/bin/sleep", prog)) f |= 1;

    /* Open for writing: exec is ETXTBSY. */
    int w = open(prog, O_WRONLY);
    pid_t pid = fork();
    if (pid == 0) {
        char *argv[] = {"sleep", "0", NULL};
        execv(prog, argv);
        _exit(errno == ETXTBSY ? 0 : 1);
    }
    int status = 0;
    if (w < 0 || pid < 0 || waitpid(pid, &status, 0) != pid || !WIFEXITED(status) ||
        WEXITSTATUS(status) != 0)
        f |= 2;
    close(w);

    /* Running: writing and truncating are ETXTBSY. The child's
     * close-on-exec pipe end closes when its exec succeeds, so the parent
     * reads end of file then, or the exec's errno. */
    int ready[2];
    if (pipe2(ready, O_CLOEXEC) != 0) f |= 1;
    pid = fork();
    if (pid == 0) {
        char *argv[] = {"sleep", "10", NULL};
        close(ready[0]);
        execv(prog, argv);
        int e = errno;
        if (write(ready[1], &e, sizeof(e)) < 0) _exit(126);
        _exit(127);
    }
    close(ready[1]);
    int exec_errno = 0;
    int running = pid > 0 && read(ready[0], &exec_errno, sizeof(exec_errno)) == 0;
    close(ready[0]);
    if (!running || !(open(prog, O_WRONLY) == -1 && errno == ETXTBSY)) f |= 4;
    if (!(truncate(prog, 0) == -1 && errno == ETXTBSY)) f |= 8;
    if (!(open(prog, O_RDWR | O_TRUNC) == -1 && errno == ETXTBSY)) f |= 8;
    int r = open(prog, O_RDONLY), p = open(prog, O_PATH);
    if (r < 0 || p < 0) f |= 16;
    close(r);
    close(p);
    int ran = 0;
    if (pid > 0) {
        kill(pid, SIGKILL);
        waitpid(pid, &ran, 0);
    }
    /* The process is gone: the file can be written again. */
    w = open(prog, O_WRONLY);
    if (w < 0) f |= 32;
    close(w);

    /* A shared mapping keeps the write access of the file it was made
     * from after the descriptor is closed (Linux's vm_file); unmapped, the
     * file runs again (PR #35 review). */
    w = open(prog, O_RDWR);
    void *map = w >= 0 ? mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, w, 0) : MAP_FAILED;
    if (w >= 0) close(w);
    for (int unmapped = 0; unmapped < 2; unmapped++) {
        if (unmapped && map != MAP_FAILED) munmap(map, 4096);
        pid = fork();
        if (pid == 0) {
            char *argv[] = {"sleep", "0", NULL};
            execv(prog, argv);
            _exit(errno == ETXTBSY ? 3 : 4);
        }
        status = 0;
        int want = unmapped ? 0 : 3;
        if (map == MAP_FAILED || pid < 0 || waitpid(pid, &status, 0) != pid ||
            !WIFEXITED(status) || WEXITSTATUS(status) != want)
            f |= 64;
    }

    unlink(prog);
    snprintf(why, sizeof(why), "f=%#x errno=%d exec=%d status=%#x", f, errno, exec_errno, ran);
    report("text_busy", f == 0, why);
}

/* inotify as Linux 7.3's fs/notify: a watch on a directory reports its
 * entries' events with their names, one on a file the file's own; a move
 * is MOVED_FROM and MOVED_TO with one cookie, then MOVE_SELF; removing the
 * last name is ATTRIB, DELETE_SELF and IGNORED as the watch goes, before
 * the directory's DELETE; an event
 * identical to the one last queued merges into it; O_PATH opens report
 * nothing. Events are read as "wd:mask:name" lines. */
static int inotify_drain(int fd, char *out, size_t len, unsigned *cookies)
{
    char buf[4096] __attribute__((aligned(8)));
    size_t used = 0;
    int ncookie = 0;
    out[0] = 0;
    for (;;) {
        ssize_t n = read(fd, buf, sizeof(buf));
        if (n <= 0) return n < 0 && errno == EAGAIN ? 0 : -1;
        for (char *p = buf; p < buf + n;) {
            struct inotify_event *ev = (struct inotify_event *)p;
            int w = snprintf(out + used, len - used, "%d:%x:%s ", ev->wd, ev->mask,
                             ev->len ? ev->name : "");
            if (w < 0 || (size_t)w >= len - used) return -1;
            used += (size_t)w;
            if (ev->cookie && ncookie < 4) cookies[ncookie++] = ev->cookie;
            p += sizeof(*ev) + ev->len;
        }
    }
}

static void test_inotify_events(void)
{
    const char *d = "/tmp/musl_inotify";
    char why[160], got[1024], want[512];
    unsigned ck[4] = {0, 0, 0, 0};
    int f = 0;

    unlink("/tmp/musl_inotify/a");
    unlink("/tmp/musl_inotify/b");
    rmdir("/tmp/musl_inotify/sub");
    rmdir(d);
    if (mkdir(d, 0755) != 0) f |= 1;

    /* Arguments. */
    int in = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
    if (in < 0 || !(fcntl(in, F_GETFL) & O_NONBLOCK) || fcntl(in, F_GETFD) != FD_CLOEXEC)
        f |= 2;
    char one;
    if (!(read(in, &one, 1) == -1 && errno == EAGAIN)) f |= 2;
    if (!(inotify_init1(0x1234) == -1 && errno == EINVAL)) f |= 2;
    if (!(inotify_add_watch(in, d, 0) == -1 && errno == EINVAL) ||
        !(inotify_add_watch(in, d, IN_MASK_ADD | IN_MASK_CREATE | IN_CREATE) == -1 &&
          errno == EINVAL) ||
        !(inotify_add_watch(0x7fff, d, IN_CREATE) == -1 && errno == EBADF) ||
        !(inotify_add_watch(1, d, IN_CREATE) == -1 && errno == EINVAL) ||
        !(inotify_add_watch(in, "/tmp/musl_inotify/none", IN_CREATE) == -1 && errno == ENOENT) ||
        !(inotify_rm_watch(in, 12345) == -1 && errno == EINVAL))
        f |= 2;

    int wd = inotify_add_watch(in, d, IN_ALL_EVENTS);
    if (wd < 1 || inotify_add_watch(in, d, IN_ALL_EVENTS) != wd ||
        !(inotify_add_watch(in, d, IN_CREATE | IN_MASK_CREATE) == -1 && errno == EEXIST))
        f |= 4;

    /* A new file: CREATE, OPEN, one MODIFY for two writes, CLOSE_WRITE. */
    int w = open("/tmp/musl_inotify/a", O_CREAT | O_WRONLY, 0644);
    if (w < 0 || write(w, "x", 1) != 1 || write(w, "y", 1) != 1) f |= 8;
    close(w);
    int r = open("/tmp/musl_inotify/a", O_RDONLY);
    if (r < 0 || read(r, &one, 1) != 1) f |= 8;
    close(r);
    int p = open("/tmp/musl_inotify/a", O_PATH);
    close(p);
    struct pollfd pfd = {in, POLLIN, 0};
    int avail = 0;
    if (poll(&pfd, 1, 0) != 1 || !(pfd.revents & POLLIN) || ioctl(in, FIONREAD, &avail) != 0 ||
        avail != 7 * 32)
        f |= 16;
    /* A buffer too small for the next event. */
    char small[8];
    if (!(read(in, small, sizeof(small)) == -1 && errno == EINVAL)) f |= 16;
    if (inotify_drain(in, got, sizeof(got), ck) != 0) f |= 8;
    snprintf(want, sizeof(want), "%d:100:a %d:20:a %d:2:a %d:8:a %d:20:a %d:1:a %d:10:a ", wd,
             wd, wd, wd, wd, wd, wd);
    if (strcmp(got, want) != 0) f |= 8;

    /* A file's own watch: no names. chmod, rename, unlink. */
    int wf = inotify_add_watch(in, "/tmp/musl_inotify/a", IN_ATTRIB | IN_MOVE_SELF |
                                                            IN_DELETE_SELF);
    if (wf < 1 || wf == wd || chmod("/tmp/musl_inotify/a", 0600) != 0 ||
        rename("/tmp/musl_inotify/a", "/tmp/musl_inotify/b") != 0 ||
        unlink("/tmp/musl_inotify/b") != 0)
        f |= 32;
    if (inotify_drain(in, got, sizeof(got), ck) != 0) f |= 32;
    snprintf(want, sizeof(want),
             "%d:4:a %d:4: %d:40:a %d:80:b %d:800: %d:4: %d:400: %d:8000: %d:200:b ", wd, wf,
             wd, wd, wf, wf, wf, wf, wd);
    if (strcmp(got, want) != 0 || ck[0] == 0 || ck[0] != ck[1]) f |= 32;

    /* Directories carry IN_ISDIR. */
    if (mkdir("/tmp/musl_inotify/sub", 0755) != 0 || rmdir("/tmp/musl_inotify/sub") != 0)
        f |= 64;
    if (inotify_drain(in, got, sizeof(got), ck) != 0) f |= 64;
    snprintf(want, sizeof(want), "%d:40000100:sub %d:40000200:sub ", wd, wd);
    if (strcmp(got, want) != 0) f |= 64;

    /* IN_ONESHOT reports once; inotify_rm_watch queues IN_IGNORED. */
    int wo = inotify_add_watch(in, d, IN_CREATE | IN_ONESHOT);
    w = open("/tmp/musl_inotify/a", O_CREAT | O_WRONLY, 0644);
    close(w);
    unlink("/tmp/musl_inotify/a");
    if (wo != wd || inotify_drain(in, got, sizeof(got), ck) != 0) f |= 128;
    snprintf(want, sizeof(want), "%d:100:a %d:8000: ", wd, wd);
    if (strcmp(got, want) != 0) f |= 128;
    int wr = inotify_add_watch(in, d, IN_CREATE);
    if (wr < 1 || wr == wd || inotify_rm_watch(in, wr) != 0 ||
        !(inotify_rm_watch(in, wr) == -1 && errno == EINVAL) ||
        inotify_drain(in, got, sizeof(got), ck) != 0)
        f |= 256;
    snprintf(want, sizeof(want), "%d:8000: ", wr);
    if (strcmp(got, want) != 0) f |= 256;

    /* An attribute set through a symlink changes the target, so the event
     * is the target's directory's, not the link's (PR #35 review). */
    int ix = inotify_init1(IN_NONBLOCK);
    mkdir("/tmp/musl_inotify/data", 0755);
    mkdir("/tmp/musl_inotify/links", 0755);
    w = open("/tmp/musl_inotify/data/file", O_CREAT | O_WRONLY, 0644);
    close(w);
    symlink("../data/file", "/tmp/musl_inotify/links/alias");
    int wda = inotify_add_watch(ix, "/tmp/musl_inotify/data", IN_ATTRIB);
    int wdl = inotify_add_watch(ix, "/tmp/musl_inotify/links", IN_ATTRIB);
    if (ix < 0 || wda < 1 || wdl < 1 ||
        setxattr("/tmp/musl_inotify/links/alias", "user.t", "1", 1, 0) != 0 ||
        inotify_drain(ix, got, sizeof(got), ck) != 0)
        f |= 1024;
    snprintf(want, sizeof(want), "%d:4:file ", wda);
    if (strcmp(got, want) != 0) f |= 1024;
    close(ix);
    unlink("/tmp/musl_inotify/links/alias");
    unlink("/tmp/musl_inotify/data/file");
    rmdir("/tmp/musl_inotify/links");
    rmdir("/tmp/musl_inotify/data");
    inotify_drain(in, got, sizeof(got), ck);

    /* The limits are where Linux keeps them. */
    char lim[32] = "";
    int l = open("/proc/sys/fs/inotify/max_queued_events", O_RDONLY);
    if (l < 0 || read(l, lim, sizeof(lim) - 1) <= 0 || strcmp(lim, "16384\n") != 0) f |= 512;
    close(l);
    l = open("/proc/sys/fs/inotify/max_user_instances", O_RDONLY);
    if (l < 0) f |= 512;
    close(l);

    close(in);
    rmdir(d);
    snprintf(why, sizeof(why), "f=%#x errno=%d got=%.100s", f, errno, got);
    report("inotify_events", f == 0, why);
}

/* pidfd_open, pidfd_send_signal, pidfd_getfd, waitid(P_PIDFD) and
 * clone(CLONE_PIDFD), as Linux 7.3: a pidfd is close-on-exec, polls
 * readable once its process has exited, signals and waits for that process
 * only, and copies a descriptor from a process the caller may trace. */
#ifndef P_PIDFD
#define P_PIDFD 3
#endif
#ifndef PIDFD_NONBLOCK
#define PIDFD_NONBLOCK O_NONBLOCK
#endif
#ifndef PIDFD_THREAD
#define PIDFD_THREAD O_EXCL
#endif
#ifndef CLONE_PIDFD
#define CLONE_PIDFD 0x1000
#endif
static int sys_pidfd_open(pid_t pid, unsigned flags)
{
    return (int)syscall(SYS_pidfd_open, pid, flags);
}

static int sys_pidfd_send_signal(int pidfd, int sig, siginfo_t *info, unsigned flags)
{
    return (int)syscall(SYS_pidfd_send_signal, pidfd, sig, info, flags);
}

static int sys_pidfd_getfd(int pidfd, int fd, unsigned flags)
{
    return (int)syscall(SYS_pidfd_getfd, pidfd, fd, flags);
}

static void test_pidfd(void)
{
    char why[160];
    int f = 0;

    /* Arguments. */
    if (!(sys_pidfd_open(getpid(), 0x4) == -1 && errno == EINVAL) ||
        !(sys_pidfd_open(0, 0) == -1 && errno == EINVAL) ||
        !(sys_pidfd_open(-1, 0) == -1 && errno == EINVAL) ||
        !(sys_pidfd_open(0x3ffffff0, 0) == -1 && errno == ESRCH))
        f |= 1;
    int self = sys_pidfd_open(getpid(), 0);
    if (self < 0 || fcntl(self, F_GETFD) != FD_CLOEXEC || (fcntl(self, F_GETFL) & O_ACCMODE) != O_RDWR)
        f |= 2;
    char c;
    if (!(read(self, &c, 1) == -1 && errno == EINVAL)) f |= 2;
    if (!(sys_pidfd_send_signal(self, 0, NULL, 0x8) == -1 && errno == EINVAL) ||
        !(sys_pidfd_send_signal(self, 0, NULL, 0x3) == -1 && errno == EINVAL) ||
        !(sys_pidfd_send_signal(0x7fff, 0, NULL, 0) == -1 && errno == EBADF) ||
        !(sys_pidfd_send_signal(1, 0, NULL, 0) == -1 && errno == EBADF) ||
        sys_pidfd_send_signal(self, 0, NULL, 0) != 0)
        f |= 4;
    struct pollfd pfd = {self, POLLIN, 0};
    if (poll(&pfd, 1, 0) != 0) f |= 8;

    /* A child that waits for a signal: it is signalled through its pidfd,
     * which then polls readable, and waitid reaps it through the pidfd. */
    int pipefd[2];
    if (pipe(pipefd) != 0) f |= 16;
    pid_t pid = fork();
    if (pid == 0) {
        close(pipefd[0]);
        pause();
        _exit(1);
    }
    close(pipefd[1]);
    int pfd_child = sys_pidfd_open(pid, PIDFD_NONBLOCK);
    if (pfd_child < 0 || !(fcntl(pfd_child, F_GETFL) & O_NONBLOCK)) f |= 32;
    /* Copy its write end of the pipe: the copy is close-on-exec. */
    int got = sys_pidfd_getfd(pfd_child, pipefd[1], 0);
    if (got < 0 || fcntl(got, F_GETFD) != FD_CLOEXEC) f |= 64;
    if (!(sys_pidfd_getfd(pfd_child, pipefd[1], 1) == -1 && errno == EINVAL) ||
        !(sys_pidfd_getfd(pfd_child, 0x7ffe, 0) == -1 && errno == EBADF))
        f |= 64;
    if (got >= 0) {
        if (write(got, "x", 1) != 1) f |= 64;
        close(got);
        if (read(pipefd[0], &c, 1) != 1 || c != 'x') f |= 64;
    }
    siginfo_t si;
    memset(&si, 0, sizeof(si));
    /* Nothing to report yet on a nonblocking pidfd: EAGAIN. */
    if (!(waitid(P_PIDFD, pfd_child, &si, WEXITED) == -1 && errno == EAGAIN)) f |= 128;
    if (sys_pidfd_send_signal(pfd_child, SIGTERM, NULL, 0) != 0) f |= 256;
    pfd.fd = pfd_child;
    pfd.revents = 0;
    if (poll(&pfd, 1, 5000) != 1 || !(pfd.revents & POLLIN)) f |= 512;
    memset(&si, 0, sizeof(si));
    if (waitid(P_PIDFD, pfd_child, &si, WEXITED) != 0 || si.si_pid != pid ||
        si.si_code != CLD_KILLED || si.si_status != SIGTERM)
        f |= 1024;
    /* Reaped: polls readable and hung up; signals are ESRCH. */
    pfd.revents = 0;
    if (poll(&pfd, 1, 0) != 1 || !(pfd.revents & POLLHUP)) f |= 2048;
    if (!(sys_pidfd_send_signal(pfd_child, SIGTERM, NULL, 0) == -1 && errno == ESRCH)) f |= 2048;
    close(pfd_child);
    close(pipefd[0]);

    /* clone(CLONE_PIDFD): the pidfd comes back through parent_tid. */
    int cfd = -1;
    long cpid = syscall(SYS_clone, CLONE_PIDFD | SIGCHLD, 0, &cfd, 0, 0);
    if (cpid == 0)
        _exit(7);
    if (cpid < 0 || cfd < 0 || fcntl(cfd, F_GETFD) != FD_CLOEXEC) f |= 4096;
    memset(&si, 0, sizeof(si));
    if (cfd >= 0 && (waitid(P_PIDFD, cfd, &si, WEXITED) != 0 || si.si_pid != cpid ||
                     si.si_code != CLD_EXITED || si.si_status != 7))
        f |= 4096;
    if (cfd >= 0) close(cfd);
    if (cpid > 0) waitpid((pid_t)cpid, NULL, WNOHANG);
    close(self);

    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("pidfd", f == 0, why);
}

/* clone3, as Linux 7.3: struct clone_args read as an extensible struct and
 * checked in Linux's order, the pidfd stored through its own pointer, and
 * CLONE_CLEAR_SIGHAND, CLONE_NNP, CLONE_AUTOREAP, CLONE_PIDFD_AUTOKILL,
 * set_tid and CLONE_INTO_CGROUP as Linux takes them. */
#ifndef SYS_clone3
#define SYS_clone3 435
#endif
#ifndef SYS_pidfd_open
#define SYS_pidfd_open 434
#endif
#ifndef CLONE_PIDFD
#define CLONE_PIDFD 0x1000
#endif
#ifndef P_PIDFD
#define P_PIDFD 3
#endif
#define T_CLONE_CLEAR_SIGHAND (1ULL << 32)
#define T_CLONE_INTO_CGROUP (1ULL << 33)
#define T_CLONE_AUTOREAP (1ULL << 34)
#define T_CLONE_NNP (1ULL << 35)
#define T_CLONE_PIDFD_AUTOKILL (1ULL << 36)
struct t_clone_args {
    uint64_t flags, pidfd, child_tid, parent_tid, exit_signal, stack, stack_size, tls,
        set_tid, set_tid_size, cgroup;
};

static long t_clone3(struct t_clone_args *a, size_t size)
{
    return syscall(SYS_clone3, a, size);
}

/* Reap `pid` and give its exit status (-1 if it did not exit normally). */
static int t_exit_status(pid_t pid)
{
    int st;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st))
        return -1;
    return WEXITSTATUS(st);
}

static void t_handler(int sig) { (void)sig; }

static void test_clone3(void)
{
    char why[160];
    int f = 0;
    struct t_clone_args a;
    unsigned char big[96];
    long r;

    /* The struct's size: below its first version EINVAL, above a page or
     * with nonzero bytes past the known fields E2BIG. */
    memset(&a, 0, sizeof(a));
    a.exit_signal = SIGCHLD;
    if (!(t_clone3(&a, 0) == -1 && errno == EINVAL) ||
        !(t_clone3(&a, 63) == -1 && errno == EINVAL) ||
        !(t_clone3(&a, 4097) == -1 && errno == E2BIG))
        f |= 1;
    memset(big, 0, sizeof(big));
    memcpy(big, &a, sizeof(a));
    big[90] = 1;
    if (!(t_clone3((struct t_clone_args *)big, sizeof(big)) == -1 && errno == E2BIG))
        f |= 1;
    big[90] = 0;
    r = t_clone3((struct t_clone_args *)big, sizeof(big));
    if (r == 0)
        _exit(5);
    if (r < 0 || t_exit_status((pid_t)r) != 5)
        f |= 1;

    /* Flags and fields clone3 refuses. */
    memset(&a, 0, sizeof(a));
    a.flags = 1ULL << 40;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 2;
    a.flags = 0x00400000; /* CLONE_DETACHED */
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 2;
    a.flags = 0;
    a.exit_signal = 0x100;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 2;
    a.exit_signal = 65;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 2;
    a.exit_signal = SIGCHLD;
    a.stack_size = 4096;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 2;
    a.stack_size = 0;
    a.flags = CLONE_THREAD | CLONE_SIGHAND | CLONE_VM;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 2;
    a.flags = CLONE_SIGHAND;
    a.exit_signal = 0;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 2;

    /* CLONE_PIDFD stores through `pidfd`, CLONE_PARENT_SETTID through
     * `parent_tid`; one address for both is EINVAL. */
    int pidfd = -1, ptid = 0;
    memset(&a, 0, sizeof(a));
    a.flags = CLONE_PIDFD | CLONE_PARENT_SETTID;
    a.pidfd = (uintptr_t)&pidfd;
    a.parent_tid = (uintptr_t)&pidfd;
    a.exit_signal = SIGCHLD;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 4;
    a.parent_tid = (uintptr_t)&ptid;
    r = t_clone3(&a, sizeof(a));
    if (r == 0)
        _exit(6);
    if (r < 0 || ptid != r || pidfd < 0 || fcntl(pidfd, F_GETFD) != FD_CLOEXEC)
        f |= 4;
    siginfo_t si;
    memset(&si, 0, sizeof(si));
    if (pidfd >= 0 && (waitid(P_PIDFD, pidfd, &si, WEXITED) != 0 || si.si_pid != r ||
                       si.si_status != 6))
        f |= 4;
    if (pidfd >= 0) close(pidfd);

    /* CLONE_CLEAR_SIGHAND: a handler goes back to SIG_DFL, SIG_IGN stays;
     * not with CLONE_SIGHAND. CLONE_NNP: the child alone has
     * no_new_privs. */
    struct sigaction sa, old1, old2;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = t_handler;
    sigaction(SIGUSR1, &sa, &old1);
    sa.sa_handler = SIG_IGN;
    sigaction(SIGUSR2, &sa, &old2);
    memset(&a, 0, sizeof(a));
    a.flags = T_CLONE_CLEAR_SIGHAND | T_CLONE_NNP;
    a.exit_signal = SIGCHLD;
    r = t_clone3(&a, sizeof(a));
    if (r == 0) {
        struct sigaction c1, c2;
        sigaction(SIGUSR1, NULL, &c1);
        sigaction(SIGUSR2, NULL, &c2);
        _exit((c1.sa_handler == SIG_DFL ? 0 : 1) | (c2.sa_handler == SIG_IGN ? 0 : 2) |
              (prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) == 1 ? 0 : 4));
    }
    if (r < 0 || t_exit_status((pid_t)r) != 0 || prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 0)
        f |= 8;
    sigaction(SIGUSR1, &old1, NULL);
    sigaction(SIGUSR2, &old2, NULL);

    /* CLONE_AUTOREAP: no exit signal allowed, and the child is gone once
     * it exits -- its pidfd hangs up and there is nothing to wait for. */
    memset(&a, 0, sizeof(a));
    a.flags = T_CLONE_AUTOREAP;
    a.exit_signal = SIGCHLD;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 16;
    pidfd = -1;
    a.flags = T_CLONE_AUTOREAP | CLONE_PIDFD;
    a.pidfd = (uintptr_t)&pidfd;
    a.exit_signal = 0;
    r = t_clone3(&a, sizeof(a));
    if (r == 0)
        _exit(0);
    struct pollfd p = {pidfd, POLLIN, 0};
    if (r < 0 || pidfd < 0 || poll(&p, 1, 5000) != 1)
        f |= 16;
    /* Reaped: no zombie is left to wait for. */
    for (int i = 0; i < 100 && !(p.revents & POLLHUP); i++) {
        usleep(10000);
        p.revents = 0;
        poll(&p, 1, 0);
    }
    if (!(p.revents & POLLHUP) || !(waitpid((pid_t)r, NULL, WNOHANG) == -1 && errno == ECHILD))
        f |= 16;
    if (pidfd >= 0) close(pidfd);

    /* CLONE_PIDFD_AUTOKILL: the last close of the clone pidfd SIGKILLs the
     * child; it needs CLONE_PIDFD and CLONE_AUTOREAP, and CLONE_NNP or
     * CAP_SYS_ADMIN. */
    memset(&a, 0, sizeof(a));
    a.flags = T_CLONE_AUTOREAP | T_CLONE_PIDFD_AUTOKILL;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 32;
    pidfd = -1;
    a.flags = CLONE_PIDFD | T_CLONE_AUTOREAP | T_CLONE_PIDFD_AUTOKILL | T_CLONE_NNP;
    a.pidfd = (uintptr_t)&pidfd;
    r = t_clone3(&a, sizeof(a));
    if (r == 0) {
        pause();
        _exit(1);
    }
    int watch = r > 0 ? (int)syscall(SYS_pidfd_open, (pid_t)r, 0) : -1;
    if (r < 0 || pidfd < 0 || watch < 0)
        f |= 32;
    if (pidfd >= 0) close(pidfd);
    p.fd = watch;
    p.revents = 0;
    if (watch >= 0 && poll(&p, 1, 5000) != 1)
        f |= 32;
    if (watch >= 0) close(watch);

    /* set_tid: one entry (one PID namespace), from 1 below pid_max, and
     * only with CAP_CHECKPOINT_RESTORE or CAP_SYS_ADMIN; a taken ID is
     * EEXIST. */
    pid_t ids[2] = {0, 0};
    memset(&a, 0, sizeof(a));
    a.exit_signal = SIGCHLD;
    a.set_tid = (uintptr_t)ids;
    a.set_tid_size = 1;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 64;
    ids[0] = getpid() + 7000;
    a.set_tid_size = 2;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 64;
    a.set_tid_size = 1;
    if (geteuid() != 0) {
        if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EPERM)) f |= 64;
    } else {
        ids[0] = getpid();
        if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EEXIST)) f |= 64;
        ids[0] = getpid() + 7000;
        r = t_clone3(&a, sizeof(a));
        if (r == 0)
            _exit(getpid() == ids[0] ? 0 : 1);
        if (r != ids[0] || t_exit_status((pid_t)r) != 0)
            f |= 64;
    }

    /* CLONE_INTO_CGROUP: a descriptor past INT_MAX or a struct too short
     * to have one is EINVAL; a descriptor that is not a cgroup EBADF. */
    memset(&a, 0, sizeof(a));
    a.flags = T_CLONE_INTO_CGROUP;
    a.exit_signal = SIGCHLD;
    a.cgroup = 1ULL << 31;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EINVAL)) f |= 128;
    int devnull = open("/dev/null", O_RDONLY | O_CLOEXEC);
    a.cgroup = devnull;
    if (!(t_clone3(&a, 80) == -1 && errno == EINVAL)) f |= 128;
    if (!(t_clone3(&a, sizeof(a)) == -1 && errno == EBADF)) f |= 128;
    if (devnull >= 0) close(devnull);

    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("clone3", f == 0, why);
}

/* kcmp and process_vm_readv/writev, as Linux 7.3: kcmp says 0 for a shared
 * object and 1 or 2 otherwise, by an order the two arguments reverse;
 * process_vm_* copy between this process and one it may ptrace, page by
 * page, returning what was copied before a fault. */
#ifndef SYS_kcmp
#define SYS_kcmp 312
#endif
static long t_kcmp(pid_t a, pid_t b, int type, unsigned long i1, unsigned long i2)
{
    return syscall(SYS_kcmp, a, b, type, i1, i2);
}
static char t_pvm_buf[8192];

/* G2b: a process clone may share the file table (CLONE_FILES) and, with
 * the memory, the signal actions (CLONE_SIGHAND), and may be its caller's
 * sibling (CLONE_PARENT). musl's clone() runs a CLONE_VM child on the given
 * stack as vfork would (it refuses only the thread flags: LTP clone10's
 * EINVAL is musl's); CLONE_VFORK keeps the caller stopped until the child
 * exits or execs. */
static char t_share_stack[65536] __attribute__((aligned(16)));
static volatile long t_share_out[3];
static int t_share_fd;
static void t_share_handler(int sig) { (void)sig; }

/* Report whether kcmp finds the file table and the signal actions shared
 * with the caller, then close the descriptor and catch SIGUSR1. */
static int t_share_child(void *arg)
{
    (void)arg;
    pid_t me = (pid_t)syscall(SYS_getpid), parent = (pid_t)syscall(SYS_getppid);
    t_share_out[0] = t_kcmp(me, parent, 2 /* KCMP_FILES */, 0, 0);
    t_share_out[1] = t_kcmp(me, parent, 4 /* KCMP_SIGHAND */, 0, 0);
    syscall(SYS_close, t_share_fd);
    struct sigaction sa = {.sa_handler = t_share_handler};
    sigaction(SIGUSR1, &sa, 0);
    return 0;
}

static int t_share_exec(void *arg)
{
    (void)arg;
    char *argv[] = {"true", 0};
    execve("/bin/true", argv, 0);
    return 127;
}

static int t_share_sibling(void *arg)
{
    (void)arg;
    t_share_out[2] = syscall(SYS_getppid);
    return 7;
}

/* Run `fn` in a clone with `flags`; its exit code, or -1. */
static int t_share_run(int (*fn)(void *), int flags)
{
    pid_t pid = clone(fn, t_share_stack + sizeof(t_share_stack), flags, 0);
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st))
        return -1;
    return WEXITSTATUS(st);
}

static void test_clone_share(void)
{
    char why[96];
    int f = 0;
    struct sigaction dfl = {.sa_handler = SIG_DFL}, old;
    sigaction(SIGUSR1, &dfl, 0);

    /* Shared: the child's close and handler are the caller's. */
    t_share_fd = open("/dev/null", O_RDONLY);
    t_share_out[0] = t_share_out[1] = -1;
    if (t_share_run(t_share_child, CLONE_VM | CLONE_FILES | CLONE_SIGHAND | CLONE_VFORK | SIGCHLD) != 0)
        f |= 1;
    if (t_share_out[0] != 0 || t_share_out[1] != 0)
        f |= 2;
    if (!(fcntl(t_share_fd, F_GETFD) == -1 && errno == EBADF))
        f |= 4;
    if (sigaction(SIGUSR1, 0, &old) != 0 || old.sa_handler != t_share_handler)
        f |= 8;
    sigaction(SIGUSR1, &dfl, 0);

    /* Copies: the caller keeps its own. */
    t_share_fd = open("/dev/null", O_RDONLY);
    if (t_share_run(t_share_child, CLONE_VM | CLONE_VFORK | SIGCHLD) != 0)
        f |= 16;
    if (t_share_out[0] <= 0 || t_share_out[1] <= 0)
        f |= 32;
    if (fcntl(t_share_fd, F_GETFD) != 0)
        f |= 64;
    if (sigaction(SIGUSR1, 0, &old) != 0 || old.sa_handler != SIG_DFL)
        f |= 128;
    close(t_share_fd);

    /* exec in a sharer copies both first: the caller's close-on-exec
     * descriptor and its handler stay. */
    int cloexec = open("/dev/null", O_RDONLY | O_CLOEXEC);
    struct sigaction sa = {.sa_handler = t_share_handler};
    sigaction(SIGUSR1, &sa, 0);
    if (t_share_run(t_share_exec, CLONE_VM | CLONE_FILES | CLONE_SIGHAND | CLONE_VFORK | SIGCHLD) != 0)
        f |= 256;
    if (fcntl(cloexec, F_GETFD) != FD_CLOEXEC)
        f |= 512;
    if (sigaction(SIGUSR1, 0, &old) != 0 || old.sa_handler != t_share_handler)
        f |= 1024;
    close(cloexec);
    sigaction(SIGUSR1, &dfl, 0);

    /* CLONE_PARENT: a middle process's clone is this process's child, with
     * the middle one's exit signal, and the middle one has no child. */
    pid_t me = getpid();
    int pfd[2];
    if (pipe(pfd) != 0)
        f |= 2048;
    pid_t mid = fork();
    if (mid == 0) {
        close(pfd[0]);
        t_share_out[2] = 0;
        pid_t sib = clone(t_share_sibling, t_share_stack + sizeof(t_share_stack),
                          CLONE_VM | CLONE_PARENT | CLONE_VFORK | SIGCHLD, 0);
        int code = 0;
        if (sib < 0)
            code |= 1;
        if (t_share_out[2] != me)
            code |= 2;
        errno = 0;
        if (!(waitpid(-1, 0, WNOHANG) == -1 && errno == ECHILD))
            code |= 4;
        if (write(pfd[1], &sib, sizeof(sib)) != (ssize_t)sizeof(sib))
            code |= 8;
        _exit(code);
    }
    close(pfd[1]);
    pid_t sib = -1;
    int st = 0, sst = 0;
    if (mid < 0 || waitpid(mid, &st, 0) != mid || !WIFEXITED(st) || WEXITSTATUS(st) != 0)
        f |= 4096;
    if (read(pfd[0], &sib, sizeof(sib)) != (ssize_t)sizeof(sib) || sib <= 0 ||
        waitpid(sib, &sst, 0) != sib || !WIFEXITED(sst) || WEXITSTATUS(sst) != 7)
        f |= 8192;
    close(pfd[0]);

    snprintf(why, sizeof(why), "f=%#x kcmp=%ld/%ld mid=%#x", f, t_share_out[0], t_share_out[1], st);
    report("clone_share", f == 0, why);
}

static void test_kcmp_process_vm(void)
{
    char why[160];
    int f = 0;
    pid_t me = getpid();
    enum { FILE_ = 0, VM = 1, FILES = 2, FS = 3, SIGHAND = 4, IO = 5, EPOLL_TFD = 7 };

    /* One process shares everything with itself; dup shares the file. */
    int a = open("/dev/null", O_RDONLY | O_CLOEXEC);
    int b = open("/dev/null", O_RDONLY | O_CLOEXEC);
    int c = dup(a);
    if (t_kcmp(me, me, VM, 0, 0) != 0 || t_kcmp(me, me, FILES, 0, 0) != 0 ||
        t_kcmp(me, me, FS, 0, 0) != 0 || t_kcmp(me, me, SIGHAND, 0, 0) != 0 ||
        t_kcmp(me, me, IO, 0, 0) != 0 || t_kcmp(me, me, FILE_, a, c) != 0)
        f |= 1;
    long ab = t_kcmp(me, me, FILE_, a, b), ba = t_kcmp(me, me, FILE_, b, a);
    if (!((ab == 1 && ba == 2) || (ab == 2 && ba == 1)))
        f |= 2;
    if (!(t_kcmp(me, me, 99, 0, 0) == -1 && errno == EINVAL) ||
        !(t_kcmp(me, 0x3ffffff0, VM, 0, 0) == -1 && errno == ESRCH) ||
        !(t_kcmp(me, me, FILE_, a, 0x7ffe) == -1 && errno == EBADF))
        f |= 4;
    /* An epoll instance's registration of a pipe is the file its
     * descriptor names. */
    int ep = epoll_create1(EPOLL_CLOEXEC), pp[2] = {-1, -1};
    struct epoll_event ev = {EPOLLIN, {0}};
    if (pipe(pp) != 0 || epoll_ctl(ep, EPOLL_CTL_ADD, pp[0], &ev) != 0) f |= 8;
    uint32_t slot[3] = {(uint32_t)ep, (uint32_t)pp[0], 0};
    if (t_kcmp(me, me, EPOLL_TFD, pp[0], (unsigned long)slot) != 0 ||
        t_kcmp(me, me, EPOLL_TFD, pp[1], (unsigned long)slot) == 0)
        f |= 8;
    slot[2] = 1;
    if (!(t_kcmp(me, me, EPOLL_TFD, pp[0], (unsigned long)slot) == -1 && errno == ENOENT)) f |= 8;
    slot[0] = (uint32_t)b;
    slot[2] = 0;
    if (!(t_kcmp(me, me, EPOLL_TFD, pp[0], (unsigned long)slot) == -1 && errno == EINVAL)) f |= 8;
    close(ep);
    close(pp[0]);
    close(pp[1]);

    /* A fork child: its own memory, files and handlers, the same open file
     * behind an inherited descriptor; its memory read and written here. */
    memset(t_pvm_buf, 'p', sizeof(t_pvm_buf));
    memcpy(t_pvm_buf + 4090, "boundary", 8);
    int go[2], back[2];
    if (pipe(go) != 0 || pipe(back) != 0) f |= 16;
    pid_t pid = fork();
    if (pid == 0) {
        char x;
        memset(t_pvm_buf, 'c', 16);
        if (write(back[1], "r", 1) != 1) _exit(2);
        if (read(go[0], &x, 1) != 1) _exit(3);
        _exit(memcmp(t_pvm_buf + 100, "written", 7) == 0 ? 0 : 1);
    }
    char x;
    if (read(back[0], &x, 1) != 1) f |= 16;
    if (t_kcmp(me, pid, VM, 0, 0) == 0 || t_kcmp(me, pid, FILES, 0, 0) == 0 ||
        t_kcmp(me, pid, SIGHAND, 0, 0) == 0 || t_kcmp(me, pid, FILE_, a, a) != 0)
        f |= 16;
    char got[32];
    struct iovec l = {got, 24}, r = {t_pvm_buf + 4086, 24};
    memset(got, 0, sizeof(got));
    /* Across a page boundary of the child's memory. */
    if (process_vm_readv(pid, &l, 1, &r, 1, 0) != 24 || memcmp(got + 4, "boundary", 8) != 0)
        f |= 32;
    r.iov_base = t_pvm_buf;
    r.iov_len = 16;
    if (process_vm_readv(pid, &l, 1, &r, 1, 0) != 16 || got[0] != 'c') f |= 32;
    /* What was copied before a fault, or EFAULT if nothing was. */
    struct iovec rr[2] = {{t_pvm_buf, 8}, {(void *)16, 8}};
    if (process_vm_readv(pid, &l, 1, rr, 2, 0) != 8) f |= 64;
    if (!(process_vm_readv(pid, &l, 1, rr + 1, 1, 0) == -1 && errno == EFAULT)) f |= 64;
    if (!(process_vm_readv(pid, &l, 1, &r, 1, 1) == -1 && errno == EINVAL) ||
        !(process_vm_readv(0x3ffffff0, &l, 1, &r, 1, 0) == -1 && errno == ESRCH) ||
        process_vm_readv(pid, &l, 0, &r, 1, 0) != 0)
        f |= 64;
    struct iovec wl = {(void *)"written", 7}, wr = {t_pvm_buf + 100, 7};
    if (process_vm_writev(pid, &wl, 1, &wr, 1, 0) != 7) f |= 128;
    if (write(go[1], "g", 1) != 1) f |= 128;
    int st;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 128;
    /* The parent's copy was not touched. */
    if (t_pvm_buf[0] != 'p' || memcmp(t_pvm_buf + 100, "pppp", 4) != 0) f |= 128;
    close(go[0]);
    close(go[1]);
    close(back[0]);
    close(back[1]);
    close(a);
    close(b);
    close(c);

    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("kcmp_process_vm", f == 0, why);
}

/* personality, as Linux 7.3: any value is stored and the old one returned
 * (0xffffffff only asks), STICKY_TIMEOUTS leaves select's timeout alone,
 * UNAME26 makes the release 2.6.x, and the value is inherited by a fork
 * child. Also kill of a zombie succeeds (only a reaped process is ESRCH)
 * and waitpid(INT_MIN) is ESRCH. */
static void test_personality(void)
{
    char why[160];
    int f = 0;
    int old = personality(0xffffffff);

    if (old != 0) f |= 1;
    if (personality(PER_LINUX | STICKY_TIMEOUTS) != 0 ||
        personality(0xffffffff) != (PER_LINUX | STICKY_TIMEOUTS))
        f |= 1;
    /* pselect6 itself: musl's select and pselect pass the kernel a copy of
     * the timeout, so only the system call shows what it writes back. */
    struct timespec ts = {0, 10000};
    fd_set rfds;
    FD_ZERO(&rfds);
    if (syscall(SYS_pselect6, 0, &rfds, NULL, NULL, &ts, NULL) != 0 || ts.tv_nsec != 10000)
        f |= 2;
    personality(PER_LINUX);
    ts.tv_nsec = 10000;
    if (syscall(SYS_pselect6, 0, &rfds, NULL, NULL, &ts, NULL) != 0 || ts.tv_nsec != 0) f |= 2;

    struct utsname u;
    personality(PER_LINUX | UNAME26);
    if (uname(&u) != 0 || strncmp(u.release, "2.6.", 4) != 0) f |= 4;
    pid_t pid = fork();
    if (pid == 0)
        _exit(personality(0xffffffff) == (PER_LINUX | UNAME26) ? 0 : 1);
    int st;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st) || WEXITSTATUS(st) != 0) f |= 8;
    personality(PER_LINUX);
    if (uname(&u) != 0 || strncmp(u.release, "2.6.", 4) == 0) f |= 4;

    /* A zombie takes a signal; a reaped process does not exist. */
    pid = fork();
    if (pid == 0)
        _exit(0);
    siginfo_t si;
    if (waitid(P_PID, pid, &si, WEXITED | WNOWAIT) != 0 || kill(pid, SIGTERM) != 0 ||
        kill(pid, 0) != 0)
        f |= 16;
    if (waitpid(pid, &st, 0) != pid || !WIFEXITED(st)) f |= 16;
    if (!(kill(pid, 0) == -1 && errno == ESRCH)) f |= 16;
    if (!(waitpid(INT_MIN, NULL, 0) == -1 && errno == ESRCH) ||
        !(kill(INT_MIN, 0) == -1 && errno == ESRCH))
        f |= 32;

    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("personality", f == 0, why);
}

/* Namespaces, as Linux 7.3 (ADR 0022 H1): /proc/<pid>/ns links read
 * "type:[inode]" and open an nsfs file with that inode, its ioctls
 * describe it, unshare(CLONE_NEWUTS) gives a host name of its own, and
 * setns joins another process's UTS namespace, from an nsfs file or a
 * pidfd. */
#ifndef SYS_pidfd_open
#define SYS_pidfd_open 434
#endif
#define T_NS_GET_PARENT 0xb702
#define T_NS_GET_NSTYPE 0xb703
#define T_NS_GET_OWNER_UID 0xb704
#define T_NS_GET_ID 0x8008b70d
#define T_NSFS_MAGIC 0x6e736673

static unsigned long t_ns_ino(pid_t pid, const char *name)
{
    char path[64];
    struct stat st;
    snprintf(path, sizeof(path), "/proc/%d/ns/%s", (int)pid, name);
    return stat(path, &st) == 0 ? (unsigned long)st.st_ino : 0;
}

static void test_namespaces_uts(void)
{
    char why[160], buf[64], path[64];
    int f = 0;
    static const char *names[] = {"cgroup", "ipc", "mnt", "net", "pid", "pid_for_children",
                                  "time", "time_for_children", "user", "uts"};

    /* Every link reads type:[inode], the inode stat finds behind it. */
    for (unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) {
        char want[64];
        snprintf(path, sizeof(path), "/proc/self/ns/%s", names[i]);
        ssize_t n = readlink(path, buf, sizeof(buf) - 1);
        if (n <= 0) {
            f |= 1;
            continue;
        }
        buf[n] = 0;
        const char *type = names[i];
        if (strncmp(type, "pid_for", 7) == 0) type = "pid";
        if (strncmp(type, "time_for", 8) == 0) type = "time";
        snprintf(want, sizeof(want), "%s:[%lu]", type, t_ns_ino(getpid(), names[i]));
        if (strcmp(buf, want) != 0) f |= 1;
    }

    /* The nsfs file: its type, a 64-bit id, EINVAL for other types' ioctls
     * and for reads, ENOTTY for an unknown ioctl, NSFS_MAGIC. */
    int uts = open("/proc/self/ns/uts", O_RDONLY | O_CLOEXEC);
    struct stat st;
    struct statfs sfs;
    uint64_t id = 0;
    char c;
    if (uts < 0 || fstat(uts, &st) != 0 || st.st_ino != t_ns_ino(getpid(), "uts") ||
        ioctl(uts, T_NS_GET_NSTYPE) != CLONE_NEWUTS || ioctl(uts, T_NS_GET_ID, &id) != 0 ||
        id == 0)
        f |= 2;
    unsigned uid;
    if (!(ioctl(uts, T_NS_GET_PARENT) == -1 && errno == EINVAL) ||
        !(ioctl(uts, T_NS_GET_OWNER_UID, &uid) == -1 && errno == EINVAL) ||
        !(ioctl(uts, 0xb7ff) == -1 && errno == ENOTTY) ||
        !(read(uts, &c, 1) == -1 && errno == EINVAL) ||
        fstatfs(uts, &sfs) != 0 || sfs.f_type != T_NSFS_MAGIC)
        f |= 4;
    /* setns checks the type it is given against the file's. */
    if (!(setns(uts, CLONE_NEWIPC) == -1 && errno == EINVAL)) f |= 8;
    if (!(unshare(0x8000) == -1 && errno == EINVAL)) f |= 8;
    static char longname[80];
    memset(longname, 'a', sizeof(longname));
    if (!(sethostname(longname, 65) == -1 && errno == EINVAL)) f |= 8;

    /* A child with a UTS namespace and host name of its own. */
    struct utsname before, u;
    uname(&before);
    int go[2], ready[2];
    if (pipe(go) != 0 || pipe(ready) != 0) f |= 16;
    pid_t child = fork();
    if (child == 0) {
        if (unshare(CLONE_NEWUTS) != 0 || sethostname("ns-h1-test", 10) != 0) _exit(2);
        if (write(ready[1], "r", 1) != 1) _exit(3);
        if (read(go[0], &c, 1) != 1) _exit(4);
        struct utsname cu;
        uname(&cu);
        _exit(strcmp(cu.nodename, "ns-h1-test") == 0 ? 0 : 1);
    }
    close(ready[1]);
    close(go[0]);
    if (read(ready[0], &c, 1) != 1) f |= 16;
    uname(&u);
    if (strcmp(u.nodename, before.nodename) != 0 ||
        t_ns_ino(child, "uts") == t_ns_ino(getpid(), "uts") ||
        t_ns_ino(child, "ipc") != t_ns_ino(getpid(), "ipc"))
        f |= 16;

    /* Two more children join it: one through its nsfs file, one through a
     * pidfd for it. */
    snprintf(path, sizeof(path), "/proc/%d/ns/uts", (int)child);
    int theirs = open(path, O_RDONLY | O_CLOEXEC);
    int pidfd = (int)syscall(SYS_pidfd_open, child, 0);
    for (int way = 0; way < 2; way++) {
        pid_t j = fork();
        if (j == 0) {
            int r = way == 0 ? setns(theirs, 0) : setns(pidfd, CLONE_NEWUTS | CLONE_NEWIPC);
            struct utsname ju;
            uname(&ju);
            _exit(r == 0 && strcmp(ju.nodename, "ns-h1-test") == 0 ? 0 : 1);
        }
        int st2;
        if (waitpid(j, &st2, 0) != j || !WIFEXITED(st2) || WEXITSTATUS(st2) != 0) f |= 32;
    }
    if (write(go[1], "g", 1) != 1) f |= 64;
    int st3;
    if (waitpid(child, &st3, 0) != child || !WIFEXITED(st3) || WEXITSTATUS(st3) != 0) f |= 64;
    uname(&u);
    if (strcmp(u.nodename, before.nodename) != 0) f |= 64;

    /* Without CAP_SYS_ADMIN: EPERM. */
    pid_t un = fork();
    if (un == 0) {
        if (setgid(65534) != 0 || setuid(65534) != 0) _exit(0); /* not root: nothing to test */
        _exit(unshare(CLONE_NEWUTS) == -1 && errno == EPERM &&
                      sethostname("x", 1) == -1 && errno == EPERM
                  ? 0
                  : 1);
    }
    int st4;
    if (waitpid(un, &st4, 0) != un || !WIFEXITED(st4) || WEXITSTATUS(st4) != 0) f |= 128;

    close(uts);
    close(theirs);
    close(pidfd);
    close(go[1]);
    close(ready[0]);
    snprintf(why, sizeof(why), "f=%#x errno=%d", f, errno);
    report("namespaces_uts", f == 0, why);
}

/* Time namespaces, as Linux 7.3 (ADR 0022 H2): unshare(CLONE_NEWTIME)
 * makes a namespace for children whose offsets /proc/self/timens_offsets
 * sets until a process enters it; a child (fork or exec) then reads
 * CLOCK_MONOTONIC and CLOCK_BOOTTIME that far ahead, in clock_gettime,
 * /proc/uptime and sysinfo, and its absolute deadlines (clock_nanosleep,
 * timerfd, POSIX timers, futex) are taken back to the host's clock. All
 * of it runs in a child, so the rest of the suite keeps its namespace. */
#ifndef CLONE_NEWTIME
#define CLONE_NEWTIME 0x80
#endif

static int64_t t_ns_now(clockid_t c)
{
    struct timespec ts;
    clock_gettime(c, &ts);
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

static unsigned long t_time_ino(const char *link)
{
    char path[64];
    struct stat st;
    snprintf(path, sizeof(path), "/proc/self/ns/%s", link);
    return stat(path, &st) == 0 ? (unsigned long)st.st_ino : 0;
}

static int t_write_offsets(const char *text)
{
    /* O_TRUNC, as fopen("w") and so LTP's SAFE_FILE_PRINTF. */
    int fd = open("/proc/self/timens_offsets", O_WRONLY | O_TRUNC | O_CLOEXEC);
    if (fd < 0) return -1;
    ssize_t n = write(fd, text, strlen(text));
    int saved = errno;
    close(fd);
    errno = saved;
    return n == (ssize_t)strlen(text) ? 0 : -1;
}

static struct timespec t_ts(int64_t ns)
{
    struct timespec ts = {ns / 1000000000, ns % 1000000000};
    return ts;
}

/* In the child that entered the namespace: what it sees (bits 1..7 of
 * the exit status). */
static int t_time_child(int64_t parent_mono, int64_t parent_boot)
{
    int f = 0;
    int64_t sec = 1000000000;
    int64_t mono = t_ns_now(CLOCK_MONOTONIC), boot = t_ns_now(CLOCK_BOOTTIME);
    if (mono - parent_mono < 1000 * sec || mono - parent_mono > 1010 * sec) f |= 1;
    if (boot - parent_boot < 2000 * sec || boot - parent_boot > 2010 * sec) f |= 1;
    if (t_ns_now(CLOCK_MONOTONIC_COARSE) - parent_mono < 999 * sec) f |= 1;

    char buf[64] = {0};
    int fd = open("/proc/uptime", O_RDONLY);
    if (fd < 0 || read(fd, buf, sizeof(buf) - 1) <= 0 || atol(buf) < 2000) f |= 2;
    if (fd >= 0) close(fd);
    struct sysinfo si;
    if (sysinfo(&si) != 0 || si.uptime < 2000) f |= 2;

    /* Absolute deadlines 20 ms ahead return about then, not 1000 s on. */
    int64_t wall = t_ns_now(CLOCK_REALTIME);
    struct timespec at = t_ts(t_ns_now(CLOCK_MONOTONIC) + 20000000);
    if (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &at, NULL) != 0) f |= 4;
    at = t_ts(t_ns_now(CLOCK_BOOTTIME) + 20000000);
    if (clock_nanosleep(CLOCK_BOOTTIME, TIMER_ABSTIME, &at, NULL) != 0) f |= 4;
    uint32_t word = 0;
    at = t_ts(t_ns_now(CLOCK_MONOTONIC) + 20000000);
    if (!(syscall(SYS_futex, &word, FUTEX_WAIT_BITSET, 0, &at, NULL, ~0u) == -1 &&
          errno == ETIMEDOUT))
        f |= 4;
    if (t_ns_now(CLOCK_REALTIME) - wall > 5 * sec) f |= 4;

    /* A timer set to an absolute time 1 s ahead has about 1 s left. */
    struct itimerspec its = {{0, 0}, t_ts(t_ns_now(CLOCK_MONOTONIC) + sec)}, cur;
    int tfd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC);
    if (tfd < 0 || timerfd_settime(tfd, TFD_TIMER_ABSTIME, &its, NULL) != 0 ||
        timerfd_gettime(tfd, &cur) != 0 || cur.it_value.tv_sec != 0 ||
        cur.it_value.tv_nsec < 500000000)
        f |= 8;
    if (tfd >= 0) close(tfd);
    timer_t timer;
    struct sigevent sev = {0};
    sev.sigev_notify = SIGEV_NONE;
    its.it_value = t_ts(t_ns_now(CLOCK_BOOTTIME) + sec);
    if (timer_create(CLOCK_BOOTTIME, &sev, &timer) != 0 ||
        timer_settime(timer, TIMER_ABSTIME, &its, NULL) != 0 ||
        timer_gettime(timer, &cur) != 0 || cur.it_value.tv_sec != 0 ||
        cur.it_value.tv_nsec < 500000000)
        f |= 8;

    /* It runs in its parent's namespace for children. */
    if (t_time_ino("time") != t_time_ino("time_for_children")) f |= 16;
    return f;
}

static void test_namespaces_time(void)
{
    char why[160];
    int f = 0;
    pid_t top = fork();
    if (top == 0) {
        int g = 0;
        unsigned long before = t_time_ino("time");
        if (before == 0 || before != t_time_ino("time_for_children")) g |= 1;
        int64_t mono0 = t_ns_now(CLOCK_MONOTONIC);
        if (unshare(CLONE_NEWTIME) != 0) _exit(0x7f);
        /* The caller stays where it was; its children will not. */
        if (t_time_ino("time") != before || t_time_ino("time_for_children") == before) g |= 1;
        if (t_ns_now(CLOCK_MONOTONIC) - mono0 > 5000000000LL) g |= 1;

        char buf[128] = {0};
        long s1 = -1, n1 = -1, s2 = -1, n2 = -1;
        char c1[16], c2[16];
        int fd = open("/proc/self/timens_offsets", O_RDONLY);
        if (fd < 0 || read(fd, buf, sizeof(buf) - 1) <= 0 ||
            sscanf(buf, "%15s %ld %ld %15s %ld %ld", c1, &s1, &n1, c2, &s2, &n2) != 6 ||
            strcmp(c1, "monotonic") || strcmp(c2, "boottime") || s1 || n1 || s2 || n2)
            g |= 2;
        if (fd >= 0) close(fd);
        if (!(t_write_offsets("realtime 1 0\n") == -1 && errno == EINVAL) ||
            !(t_write_offsets("monotonic 1 1000000000\n") == -1 && errno == EINVAL) ||
            !(t_write_offsets("monotonic 9223372037 0\n") == -1 && errno == ERANGE) ||
            t_write_offsets("monotonic 1000 0\nboottime 2000 0\n") != 0)
            g |= 2;

        int64_t mono = t_ns_now(CLOCK_MONOTONIC), boot = t_ns_now(CLOCK_BOOTTIME);
        if (mono - mono0 > 5000000000LL) g |= 4; /* not the caller's */
        pid_t child = fork();
        if (child == 0) _exit(t_time_child(mono, boot));
        int st;
        if (waitpid(child, &st, 0) != child || !WIFEXITED(st)) g |= 8;
        else g |= WEXITSTATUS(st) << 8;
        /* Entered: the offsets are fixed. */
        if (!(t_write_offsets("monotonic 5 0\n") == -1 && errno == EACCES)) g |= 4;

        /* setns into it from a nsfs file. */
        child = fork();
        if (child == 0) {
            int ns = open("/proc/self/ns/time_for_children", O_RDONLY | O_CLOEXEC);
            _exit(ns >= 0 && setns(ns, CLONE_NEWTIME) == 0 &&
                          t_ns_now(CLOCK_MONOTONIC) - mono > 999000000000LL
                      ? 0
                      : 1);
        }
        if (waitpid(child, &st, 0) != child || !WIFEXITED(st) || WEXITSTATUS(st) != 0)
            g |= 16;

        /* Exec enters it too: a new program's /proc/uptime is ahead. */
        int pfd[2];
        if (pipe(pfd) != 0) g |= 32;
        child = fork();
        if (child == 0) {
            if (unshare(CLONE_NEWTIME) != 0 || t_write_offsets("boottime 3000 0\n") != 0)
                _exit(1);
            dup2(pfd[1], 1);
            execl("/bin/cat", "cat", "/proc/uptime", (char *)NULL);
            _exit(1);
        }
        close(pfd[1]);
        memset(buf, 0, sizeof(buf));
        if (read(pfd[0], buf, sizeof(buf) - 1) <= 0 || atol(buf) < 3000) g |= 32;
        close(pfd[0]);
        waitpid(child, &st, 0);

        /* Without CAP_SYS_ADMIN: EPERM. */
        child = fork();
        if (child == 0) {
            if (setgid(65534) != 0 || setuid(65534) != 0) _exit(0);
            _exit(unshare(CLONE_NEWTIME) == -1 && errno == EPERM ? 0 : 1);
        }
        if (waitpid(child, &st, 0) != child || !WIFEXITED(st) || WEXITSTATUS(st) != 0)
            g |= 64;
        printf("time-ns g=%#x\n", g);
        fflush(stdout);
        _exit(g == 0 ? 0 : 1);
    }
    int st;
    if (waitpid(top, &st, 0) != top || !WIFEXITED(st)) f |= 1;
    else if (WEXITSTATUS(st) == 0x7f) f |= 2; /* unshare refused */
    else if (WEXITSTATUS(st) != 0) f |= 4;
    snprintf(why, sizeof(why), "f=%#x", f);
    report("namespaces_time", f == 0, why);
}

/* Mount namespaces, as Linux 7.3 (ADR 0022 H3): /proc/<pid>/mounts and
 * mountinfo list the process's namespace (/proc/mounts is a link to
 * self/mounts), the namespace root being its own parent; unshare(CLONE_NEWNS)
 * gives a copy of the tree with new mount ids and the same devices, and
 * UNSHARE_EMPTY_MNTNS one with the root mount only, moving the root and
 * working directory to it; setns returns to the first. All of it runs in
 * a child, so the rest of the suite keeps its namespace. */
#define T_UNSHARE_EMPTY_MNTNS 0x00100000

/* Up to `len - 1` bytes of `path`, NUL-terminated; the length or -1. */
static ssize_t t_slurp(const char *path, char *buf, size_t len)
{
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return -1;
    size_t n = 0;
    ssize_t r;
    while (n + 1 < len && (r = read(fd, buf + n, len - 1 - n)) > 0)
        n += (size_t)r;
    close(fd);
    buf[n] = 0;
    return (ssize_t)n;
}

/* The mount ids in a mountinfo text, in order; how many. */
static int t_mount_ids(const char *info, int *ids, int max)
{
    int n = 0;
    for (const char *line = info; *line && n < max;) {
        ids[n++] = atoi(line);
        const char *nl = strchr(line, '\n');
        if (!nl) break;
        line = nl + 1;
    }
    return n;
}

static void test_namespaces_mnt(void)
{
    char why[160];
    int f = 0;
    pid_t top = fork();
    if (top == 0) {
        static char before[4096], after[4096], info[4096], pinfo[4096], buf[64];
        int g = 0;
        struct stat root_before, root_after;
        unsigned long first = t_ns_ino(getpid(), "mnt");

        /* The initial namespace: /proc/mounts is self/mounts, the root
         * comes first and is its own parent, /proc is there. */
        ssize_t n = readlink("/proc/mounts", buf, sizeof(buf) - 1);
        if (n != 11 || memcmp(buf, "self/mounts", 11) != 0) g |= 1;
        if (t_slurp("/proc/self/mounts", before, sizeof(before)) <= 0 ||
            t_slurp("/proc/mounts", after, sizeof(after)) <= 0 || strcmp(before, after) != 0 ||
            !strstr(before, " /proc proc rw,relatime 0 0\n"))
            g |= 1;
        int id, parent;
        if (t_slurp("/proc/self/mountinfo", pinfo, sizeof(pinfo)) <= 0 ||
            sscanf(pinfo, "%d %d", &id, &parent) != 2 || id != parent ||
            !strstr(pinfo, " / / "))
            g |= 2;

        /* A copy: another namespace, the same mount points, new ids, the
         * same device for the root. */
        if (stat("/", &root_before) != 0 || unshare(CLONE_NEWNS) != 0) {
            printf("mnt-ns unshare errno=%d\n", errno);
            fflush(stdout);
            _exit(0x7f);
        }
        if (t_ns_ino(getpid(), "mnt") == first) g |= 4;
        if (t_slurp("/proc/self/mounts", after, sizeof(after)) <= 0 || strcmp(before, after) != 0)
            g |= 8;
        int old_ids[64], new_ids[64];
        int old_n = t_mount_ids(pinfo, old_ids, 64);
        if (t_slurp("/proc/self/mountinfo", info, sizeof(info)) <= 0 ||
            t_mount_ids(info, new_ids, 64) != old_n)
            g |= 16;
        for (int i = 0; i < old_n; i++)
            for (int j = 0; j < old_n; j++)
                if (old_ids[i] == new_ids[j]) g |= 16;
        if (stat("/", &root_after) != 0 || root_after.st_dev != root_before.st_dev) g |= 32;

        /* An empty one, from /proc: the root mount only, root and working
         * directory at it. */
        pid_t e = fork();
        if (e == 0) {
            char cwd[64];
            if (chdir("/proc") != 0 || unshare(T_UNSHARE_EMPTY_MNTNS) != 0) _exit(1);
            /* /proc is no longer a mount: its directory on the root is empty. */
            int fd = open("/proc/self/mounts", O_RDONLY);
            if (fd >= 0) _exit(2);
            _exit(getcwd(cwd, sizeof(cwd)) && strcmp(cwd, "/") == 0 ? 0 : 3);
        }
        int st;
        if (waitpid(e, &st, 0) != e || !WIFEXITED(st) || WEXITSTATUS(st) != 0) g |= 64;
        /* The same from clone3's CLONE_EMPTY_MNTNS, which implies
         * CLONE_NEWNS: the child starts at the root, without /proc. */
        struct t_clone_args ca;
        memset(&ca, 0, sizeof(ca));
        ca.flags = 1ULL << 37;
        ca.exit_signal = SIGCHLD;
        if (chdir("/proc") != 0) g |= 64;
        long c = t_clone3(&ca, sizeof(ca));
        if (c == 0) {
            char cwd[64];
            int fd = open("/proc/self/mounts", O_RDONLY);
            if (fd >= 0) _exit(2);
            _exit(getcwd(cwd, sizeof(cwd)) && strcmp(cwd, "/") == 0 ? 0 : 3);
        }
        if (c < 0 || t_exit_status((pid_t)c) != 0) g |= 64;
        if (chdir("/") != 0) g |= 64;

        /* setns back to the parent's namespace. */
        char path[64];
        snprintf(path, sizeof(path), "/proc/%d/ns/mnt", (int)getppid());
        int theirs = open(path, O_RDONLY | O_CLOEXEC);
        if (theirs < 0 || setns(theirs, CLONE_NEWNS) != 0 || t_ns_ino(getpid(), "mnt") != first)
            g |= 128;
        if (theirs >= 0) close(theirs);

        /* Without CAP_SYS_ADMIN: EPERM. */
        pid_t un = fork();
        if (un == 0) {
            if (setgid(65534) != 0 || setuid(65534) != 0) _exit(0);
            _exit(unshare(CLONE_NEWNS) == -1 && errno == EPERM ? 0 : 1);
        }
        if (waitpid(un, &st, 0) != un || !WIFEXITED(st) || WEXITSTATUS(st) != 0) g |= 256;
        printf("mnt-ns g=%#x\n", g);
        fflush(stdout);
        _exit(g == 0 ? 0 : 1);
    }
    int st;
    if (waitpid(top, &st, 0) != top || !WIFEXITED(st)) f |= 1;
    else if (WEXITSTATUS(st) == 0x7f) f |= 2; /* unshare refused */
    else if (WEXITSTATUS(st) != 0) f |= 4;
    snprintf(why, sizeof(why), "f=%#x", f);
    report("namespaces_mnt", f == 0, why);
}

/* mount(2) and umount2, as Linux 7.3 (ADR 0022 H3b), in a mount namespace
 * of a child's own: a new tmpfs with options and per-mount flags, which
 * the parent's namespace does not show; a bind mount of a directory in it
 * and a read-only remount of that; MS_MOVE; propagation changes (private
 * and slave accepted, shared refused); umount2 EBUSY while the working
 * directory is in the mount, MNT_DETACH, bad flags, not a mount point;
 * ENODEV for an unknown type, EPERM without CAP_SYS_ADMIN; and
 * /proc/filesystems. */
static int t_info_has(const char *needle)
{
    static char info[8192];
    return t_slurp("/proc/self/mountinfo", info, sizeof(info)) > 0 && strstr(info, needle);
}

static void test_mount_calls(void)
{
    char why[160];
    int f = 0, go[2], done[2];
    mkdir("/tmp", 01777);
    rmdir("/tmp/mc-a");
    rmdir("/tmp/mc-b");
    rmdir("/tmp/mc-c");
    if (mkdir("/tmp/mc-a", 0755) != 0 || mkdir("/tmp/mc-b", 0755) != 0 ||
        mkdir("/tmp/mc-c", 0755) != 0 || pipe(go) != 0 || pipe(done) != 0) {
        report("mount_calls", 0, "setup");
        return;
    }
    pid_t child = fork();
    if (child == 0) {
        int g = 0;
        char c = 0;
        struct stat st;
        if (unshare(CLONE_NEWNS) != 0) _exit(0x7f);
        if (mount("none", "/tmp/mc-a", "tmpfs", MS_NOSUID | MS_NODEV, "size=1m,mode=700") != 0)
            g |= 1;
        if (stat("/tmp/mc-a", &st) != 0 || (st.st_mode & 07777) != 0700) g |= 1;
        if (!t_info_has(" /tmp/mc-a rw,nosuid,nodev,relatime - tmpfs none rw")) g |= 2;
        /* A bind mount of a directory in it, then read-only. */
        int fd;
        if (mkdir("/tmp/mc-a/d", 0755) != 0 ||
            (fd = open("/tmp/mc-a/d/x", O_CREAT | O_WRONLY, 0644)) < 0)
            g |= 4;
        else
            close(fd);
        if (mount("/tmp/mc-a/d", "/tmp/mc-b", NULL, MS_BIND, NULL) != 0 ||
            access("/tmp/mc-b/x", F_OK) != 0 || !t_info_has(" /d /tmp/mc-b rw,nosuid,nodev"))
            g |= 4;
        if (mount(NULL, "/tmp/mc-b", NULL, MS_REMOUNT | MS_BIND | MS_RDONLY, NULL) != 0 ||
            !t_info_has(" /d /tmp/mc-b ro,relatime - tmpfs none rw"))
            g |= 8;
        /* Propagation: private and slave are fine, shared is refused,
         * two at once is EINVAL. */
        if (mount(NULL, "/", NULL, MS_REC | MS_PRIVATE, NULL) != 0 ||
            mount(NULL, "/tmp/mc-a", NULL, MS_SLAVE, NULL) != 0 ||
            !(mount(NULL, "/tmp/mc-a", NULL, MS_SHARED, NULL) == -1 && errno == EINVAL) ||
            !(mount(NULL, "/tmp/mc-a", NULL, MS_PRIVATE | MS_SLAVE, NULL) == -1 &&
              errno == EINVAL))
            g |= 16;
        /* umount2: busy while the working directory is in it. */
        if (chdir("/tmp/mc-b") != 0 || !(umount2("/tmp/mc-b", 0) == -1 && errno == EBUSY) ||
            chdir("/") != 0 || umount2("/tmp/mc-b", 0) != 0 || access("/tmp/mc-b/x", F_OK) == 0)
            g |= 32;
        /* MS_MOVE: the tmpfs moves to mc-c. */
        if (mount("/tmp/mc-a", "/tmp/mc-c", NULL, MS_MOVE, NULL) != 0 ||
            access("/tmp/mc-c/d/x", F_OK) != 0 || access("/tmp/mc-a/d", F_OK) == 0)
            g |= 64;
        /* Errors. */
        if (!(mount("none", "/tmp/mc-a", "nosuchfs", 0, NULL) == -1 && errno == ENODEV) ||
            !(mount("none", "/tmp/mc-a", "tmpfs", 0, "bogus") == -1 && errno == EINVAL) ||
            !(mount("none", "/tmp/nonexistent", "tmpfs", 0, NULL) == -1 && errno == ENOENT) ||
            !(umount2("/tmp/mc-c", 0x100) == -1 && errno == EINVAL) ||
            !(umount2("/tmp/mc-a", 0) == -1 && errno == EINVAL))
            g |= 128;
        /* The parent's namespace does not have these mounts. */
        if (write(go[1], "m", 1) != 1 || read(done[0], &c, 1) != 1 || c != 'y') g |= 256;
        if (umount2("/tmp/mc-c", MNT_DETACH) != 0 || t_info_has("/tmp/mc-c")) g |= 512;
        /* Without CAP_SYS_ADMIN: EPERM. */
        pid_t un = fork();
        if (un == 0) {
            if (setgid(65534) != 0 || setuid(65534) != 0) _exit(0);
            _exit(mount("none", "/tmp/mc-a", "tmpfs", 0, NULL) == -1 && errno == EPERM &&
                          umount2("/", 0) == -1 && errno == EPERM
                      ? 0
                      : 1);
        }
        if (t_exit_status(un) != 0) g |= 1024;
        printf("mount-calls g=%#x\n", g);
        fflush(stdout);
        _exit(g == 0 ? 0 : 1);
    }
    char c = 0;
    if (child < 0 || read(go[0], &c, 1) != 1) f |= 1;
    /* Here the directories are empty and nothing is mounted on them. */
    if (t_info_has("/tmp/mc-") || access("/tmp/mc-c/d", F_OK) == 0) f |= 2;
    static char fsl[512];
    if (t_slurp("/proc/filesystems", fsl, sizeof(fsl)) <= 0 || !strstr(fsl, "nodev\ttmpfs\n"))
        f |= 4;
    if (write(done[1], "y", 1) != 1) f |= 1;
    int code = t_exit_status(child);
    if (code == 0x7f) f |= 8;
    else if (code != 0) f |= 16;
    close(go[0]);
    close(go[1]);
    close(done[0]);
    close(done[1]);
    rmdir("/tmp/mc-a");
    rmdir("/tmp/mc-b");
    rmdir("/tmp/mc-c");
    snprintf(why, sizeof(why), "f=%#x", f);
    report("mount_calls", f == 0, why);
}

/* Copy `from` to a new file `to` with mode `mode` (chmod'ed, so the
 * umask does not apply); 0 on success. */
static int t_copy(const char *from, const char *to, mode_t mode)
{
    int in = open(from, O_RDONLY), out = open(to, O_WRONLY | O_CREAT | O_TRUNC, 0700);
    int ok = in >= 0 && out >= 0;
    char buf[4096];
    ssize_t n;
    while (ok && (n = read(in, buf, sizeof(buf))) > 0)
        ok = write(out, buf, (size_t)n) == n;
    if (in >= 0)
        close(in);
    if (out >= 0)
        close(out);
    return ok && chmod(to, mode) == 0 ? 0 : -1;
}

/* What a mount's flags refuse (ADR 0022 H3c), on a tmpfs mounted in a
 * mount namespace of its own: ro (EROFS at every change, EEXIST first,
 * devices still written; EBUSY to make it ro while a file is open for
 * writing), noexec (exec and access X_OK EACCES, mmap PROT_EXEC EPERM,
 * mprotect to executable EACCES), nodev (opening a device EACCES),
 * nosymfollow (ELOOP) and nosuid (a set-user-ID program gains nothing),
 * each reported in statfs's f_flags. */
static void test_mount_flags(void)
{
#ifndef MS_NOSYMFOLLOW
#define MS_NOSYMFOLLOW 256
#endif
    enum { RDONLY = 1, NOSUID = 2, NODEV = 4, NOEXEC = 8, NOSYMFOLLOW = 0x2000 };
    char why[160];
    int f = 0;
    mkdir("/tmp", 01777);
    rmdir("/tmp/mf");
    if (mkdir("/tmp/mf", 0755) != 0) {
        report("mount_flags", 0, "setup");
        return;
    }
    pid_t child = fork();
    if (child == 0) {
        int g = 0, fd;
        struct statfs sfs;
        if (unshare(CLONE_NEWNS) != 0)
            _exit(0x7f);
        if (mount("none", "/tmp/mf", "tmpfs", 0, "size=4m,mode=755") != 0)
            _exit(0x7e);
        fd = open("/tmp/mf/f", O_CREAT | O_WRONLY, 0644);
        if (fd < 0 || write(fd, "x", 1) != 1)
            g |= 1;
        if (fd >= 0)
            close(fd);
        if (mkdir("/tmp/mf/d", 0755) != 0 || symlink("f", "/tmp/mf/l") != 0 ||
            mknod("/tmp/mf/null", S_IFCHR | 0666, makedev(1, 3)) != 0 ||
            t_copy("/bin/musl_runtime_test", "/tmp/mf/prog", 04755) != 0)
            g |= 1;
        /* Read-only: not while a file is open for writing. */
        fd = open("/tmp/mf/f", O_WRONLY);
        if (fd < 0 || !(mount(NULL, "/tmp/mf", NULL, MS_REMOUNT | MS_RDONLY, NULL) == -1 &&
                        errno == EBUSY))
            g |= 2;
        if (fd >= 0)
            close(fd);
        if (mount(NULL, "/tmp/mf", NULL, MS_REMOUNT | MS_RDONLY, NULL) != 0)
            g |= 2;
#define T_EROFS(call) (!((call) == -1 && errno == EROFS))
        if (T_EROFS(open("/tmp/mf/f", O_WRONLY)) ||
            T_EROFS(open("/tmp/mf/f", O_RDONLY | O_TRUNC)) ||
            T_EROFS(open("/tmp/mf/new", O_CREAT | O_WRONLY, 0644)) ||
            T_EROFS(mkdir("/tmp/mf/e", 0755)) || T_EROFS(unlink("/tmp/mf/f")) ||
            T_EROFS(unlink("/tmp/mf/none")) || T_EROFS(rmdir("/tmp/mf/d")) ||
            T_EROFS(rename("/tmp/mf/f", "/tmp/mf/g")))
            g |= 4;
        if (T_EROFS(chmod("/tmp/mf/f", 0600)) || T_EROFS(chown("/tmp/mf/f", 1, 1)) ||
            T_EROFS(truncate("/tmp/mf/f", 0)) || T_EROFS(utimes("/tmp/mf/f", NULL)) ||
            T_EROFS(symlink("f", "/tmp/mf/m")) || T_EROFS(link("/tmp/mf/f", "/tmp/mf/h")) ||
            T_EROFS(access("/tmp/mf/f", W_OK)) ||
            T_EROFS(setxattr("/tmp/mf/f", "user.a", "1", 1, 0)) ||
            T_EROFS(mknod("/tmp/mf/p", S_IFIFO | 0644, 0)))
            g |= 8;
#undef T_EROFS
        /* EEXIST before EROFS; reading, and writing a device, still work. */
        if (!(mkdir("/tmp/mf/d", 0755) == -1 && errno == EEXIST))
            g |= 16;
        if ((fd = open("/tmp/mf/f", O_RDONLY)) < 0)
            g |= 16;
        else
            close(fd);
        if ((fd = open("/tmp/mf/null", O_WRONLY)) < 0 || write(fd, "x", 1) != 1)
            g |= 16;
        if (fd >= 0)
            close(fd);
        if (statfs("/tmp/mf", &sfs) != 0 || !(sfs.f_flags & RDONLY))
            g |= 32;
        /* Writable again. */
        if (mount(NULL, "/tmp/mf", NULL, MS_REMOUNT, NULL) != 0 ||
            (fd = open("/tmp/mf/f", O_WRONLY)) < 0)
            g |= 64;
        else
            close(fd);
        /* noexec, nodev, nosymfollow. */
        if (mount(NULL, "/tmp/mf", NULL, MS_REMOUNT | MS_NOEXEC | MS_NODEV | MS_NOSYMFOLLOW,
                  NULL) != 0) {
            g |= 128;
        } else {
            pid_t e = fork();
            if (e == 0) {
                execl("/tmp/mf/prog", "prog", "--ids", (char *)0);
                _exit(errno == EACCES ? 0 : 1);
            }
            if (t_exit_status(e) != 0 || !(access("/tmp/mf/prog", X_OK) == -1 && errno == EACCES))
                g |= 128;
            fd = open("/tmp/mf/prog", O_RDONLY);
            void *m = fd >= 0 ? mmap(NULL, 4096, PROT_READ | PROT_EXEC, MAP_PRIVATE, fd, 0)
                              : MAP_FAILED;
            if (!(m == MAP_FAILED && errno == EPERM))
                g |= 256;
            m = fd >= 0 ? mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0) : MAP_FAILED;
            if (m == MAP_FAILED ||
                !(mprotect(m, 4096, PROT_READ | PROT_EXEC) == -1 && errno == EACCES))
                g |= 256;
            if (m != MAP_FAILED)
                munmap(m, 4096);
            if (fd >= 0)
                close(fd);
            if (!(open("/tmp/mf/null", O_WRONLY) == -1 && errno == EACCES))
                g |= 512;
            if (!(open("/tmp/mf/l", O_RDONLY) == -1 && errno == ELOOP))
                g |= 1024;
            if (statfs("/tmp/mf", &sfs) != 0 ||
                (sfs.f_flags & (RDONLY | NOEXEC | NODEV | NOSYMFOLLOW)) !=
                    (NOEXEC | NODEV | NOSYMFOLLOW))
                g |= 2048;
        }
        /* nosuid: the set-user-ID program gains root without it, not
         * with it. */
        static char ids[64], nosuid_ids[64];
        int st = -1, nosuid_st = -1;
        if (mount(NULL, "/tmp/mf", NULL, MS_REMOUNT, NULL) != 0 ||
            (st = run_ids_as_user("/tmp/mf/prog", IDS_PLAIN, ids, sizeof(ids))) != 0 ||
            strcmp(ids, "1000 0 0 1 0 prog") != 0)
            g |= 4096;
        if (mount(NULL, "/tmp/mf", NULL, MS_REMOUNT | MS_NOSUID, NULL) != 0 ||
            (nosuid_st = run_ids_as_user("/tmp/mf/prog", IDS_PLAIN, nosuid_ids,
                                         sizeof(nosuid_ids))) != 0 ||
            strcmp(nosuid_ids, "1000 1000 1000 0 1 prog") != 0 ||
            statfs("/tmp/mf", &sfs) != 0 || !(sfs.f_flags & NOSUID))
            g |= 8192;
        printf("mount-flags g=%#x ids='%s'/%d nosuid='%s'/%d\n", g, ids, st, nosuid_ids,
               nosuid_st);
        fflush(stdout);
        _exit(g == 0 ? 0 : 1);
    }
    int code = t_exit_status(child);
    if (code == 0x7f || code == 0x7e)
        f |= 1;
    else if (code != 0)
        f |= 2;
    rmdir("/tmp/mf");
    snprintf(why, sizeof(why), "f=%#x code=%d", f, code);
    report("mount_flags", f == 0, why);
}

/* How many mounts /proc/self/mountinfo shows at mount point `point`. */
/* A file does not leave a read-only bind mount by another name: a hard
 * link to another mount of the same filesystem is EXDEV, and a directory
 * with a mount under it cannot be renamed (mounts stay where they are, so
 * the moved tree would lose the mount's flags: EBUSY). PR #35 review. */
static void test_mount_no_escape(void)
{
    char why[96];
    rmdir("/tmp/me");
    if (mkdir("/tmp/me", 0755) != 0) {
        report("mount_no_escape", 0, "setup");
        return;
    }
    pid_t child = fork();
    if (child == 0) {
        int g = 0;
        if (unshare(CLONE_NEWNS) != 0 || mount("none", "/tmp/me", "tmpfs", 0, "size=1m") != 0)
            _exit(0x7f);
        int fd = -1;
        if (mkdir("/tmp/me/top", 0755) != 0 || mkdir("/tmp/me/top/a", 0755) != 0 ||
            mkdir("/tmp/me/top/ro", 0755) != 0 || mkdir("/tmp/me/top/b", 0755) != 0 ||
            (fd = open("/tmp/me/top/a/f", O_CREAT | O_WRONLY, 0644)) < 0 ||
            mount("/tmp/me/top/a", "/tmp/me/top/ro", NULL, MS_BIND, NULL) != 0 ||
            mount(NULL, "/tmp/me/top/ro", NULL, MS_REMOUNT | MS_BIND | MS_RDONLY, NULL) != 0)
            _exit(0x7e);
        close(fd);
        errno = 0;
        if (!(link("/tmp/me/top/ro/f", "/tmp/me/top/b/g") == -1 && errno == EXDEV)) g |= 1;
        if (link("/tmp/me/top/a/f", "/tmp/me/top/b/g") != 0) g |= 2;
        errno = 0;
        if (!(rename("/tmp/me/top", "/tmp/me/moved") == -1 && errno == EBUSY)) g |= 4;
        errno = 0;
        if (!(open("/tmp/me/top/ro/f", O_WRONLY) == -1 && errno == EROFS)) g |= 8;
        if (umount("/tmp/me/top/ro") != 0 || rename("/tmp/me/top", "/tmp/me/moved") != 0)
            g |= 16;
        _exit(g);
    }
    int code = t_exit_status(child);
    rmdir("/tmp/me");
    snprintf(why, sizeof(why), "code=%#x", code);
    report("mount_no_escape", code == 0, why);
}

static int t_mounts_at(const char *point)
{
    static char info[8192];
    char needle[96];
    int n = 0;
    snprintf(needle, sizeof(needle), " %s ", point);
    if (t_slurp("/proc/self/mountinfo", info, sizeof(info)) <= 0) return -1;
    for (char *line = info; *line;) {
        char *end = strchr(line, '\n');
        if (end) *end = 0;
        /* id parent dev root point ...: the fifth field. */
        char *f = line;
        for (int i = 0; i < 4 && f; i++) {
            f = strchr(f, ' ');
            if (f) f++;
        }
        if (f && strncmp(f - 1, needle, strlen(needle)) == 0) n++;
        if (!end) break;
        line = end + 1;
    }
    return n;
}

/* pivot_root (ADR 0022 H3d): LTP pivot_root01's steps, in a mount
 * namespace of its own chrooted into a tmpfs. The errors (EBUSY when
 * new_root is on the root's mount, EINVAL when put_old is not under it,
 * ENOTDIR, ENOENT, EPERM), the pivot (the new root at /, the old one at
 * put_old with its mounts, the working directory moved), then
 * pivot_root(".", ".") and umount2(".", MNT_DETACH) as container
 * runtimes do it. */
static void test_pivot_root(void)
{
    char why[160];
    mkdir("/tmp", 01777);
    rmdir("/tmp/pr");
    if (mkdir("/tmp/pr", 0755) != 0) {
        report("pivot_root", 0, "setup");
        return;
    }
    pid_t child = fork();
    if (child == 0) {
        int g = 0, fd;
        char cwd[64];
        if (unshare(CLONE_NEWNS | CLONE_FS) != 0) _exit(0x7f);
        if (mount(NULL, "/", NULL, MS_REC | MS_PRIVATE, NULL) != 0 ||
            mount("none", "/tmp/pr", "tmpfs", 0, NULL) != 0 || chroot("/tmp/pr") != 0 ||
            chdir("/") != 0)
            _exit(0x7e);
        /* new_root not a mount point is on the root's mount: EBUSY. */
        if (mkdir("/new", 0755) != 0 ||
            !(syscall(SYS_pivot_root, "/new", "/new") == -1 && errno == EBUSY))
            g |= 1;
        if (mount("none", "/new", "tmpfs", 0, NULL) != 0 || mkdir("/new/old", 0755) != 0 ||
            mkdir("/new/proc", 0755) != 0 || mount("proc", "/new/proc", "proc", 0, NULL) != 0 ||
            (fd = open("/new/marker", O_CREAT | O_WRONLY, 0644)) < 0)
            g |= 2;
        else
            close(fd);
        /* put_old not under new_root: EINVAL; a file: ENOTDIR; missing:
         * ENOENT; put_old on the root's mount: EBUSY. */
        if (mkdir("/other", 0755) != 0 || mount("none", "/other", "tmpfs", 0, NULL) != 0 ||
            mkdir("/other/old", 0755) != 0 ||
            !(syscall(SYS_pivot_root, "/new", "/other/old") == -1 && errno == EINVAL) ||
            !(syscall(SYS_pivot_root, "/new", "/new/marker") == -1 && errno == ENOTDIR) ||
            !(syscall(SYS_pivot_root, "/new", "/new/none") == -1 && errno == ENOENT) ||
            !(syscall(SYS_pivot_root, "/new", "/") == -1 && errno == EBUSY))
            g |= 4;
        /* Without CAP_SYS_ADMIN: EPERM (after the lookups). */
        pid_t un = fork();
        if (un == 0) {
            if (setgid(65534) != 0 || setuid(65534) != 0) _exit(0);
            _exit(syscall(SYS_pivot_root, "/new", "/new/old") == -1 && errno == EPERM &&
                          syscall(SYS_pivot_root, "/new", "/new/marker") == -1 &&
                          errno == ENOTDIR
                      ? 0
                      : 1);
        }
        if (t_exit_status(un) != 0) g |= 8;
        /* The pivot: the new root and its /proc at /, the old root at
         * /old with /other, the working directory (the old root) at the
         * new root. */
        if (syscall(SYS_pivot_root, "/new", "/new/old") != 0 || access("/marker", F_OK) != 0 ||
            access("/old/new", F_OK) != 0 || access("/old/other/old", F_OK) != 0 ||
            access("/proc/self/mountinfo", R_OK) != 0)
            g |= 16;
        if (!getcwd(cwd, sizeof(cwd)) || strcmp(cwd, "/") != 0 || t_mounts_at("/") != 1 ||
            t_mounts_at("/old") != 1 || t_mounts_at("/old/other") != 1)
            g |= 32;
        /* The old root goes. */
        if (umount2("/old", MNT_DETACH) != 0 || access("/old/other", F_OK) == 0 ||
            t_mounts_at("/old") != 0)
            g |= 64;
        /* pivot_root(".", "."): the old root over the new one, reached by
         * no path, until umount2(".") takes it off. */
        if (mkdir("/n2", 0755) != 0 || mount("none", "/n2", "tmpfs", 0, NULL) != 0 ||
            mkdir("/n2/proc", 0755) != 0 || mount("proc", "/n2/proc", "proc", 0, NULL) != 0 ||
            (fd = open("/n2/m2", O_CREAT | O_WRONLY, 0644)) < 0)
            g |= 128;
        else
            close(fd);
        if (chdir("/n2") != 0 || syscall(SYS_pivot_root, ".", ".") != 0 ||
            access("/m2", F_OK) != 0 || access("/marker", F_OK) == 0 || t_mounts_at("/") != 2 ||
            !getcwd(cwd, sizeof(cwd)) || strcmp(cwd, "/") != 0)
            g |= 256;
        if (umount2(".", MNT_DETACH) != 0 || t_mounts_at("/") != 1 || access("/m2", F_OK) != 0)
            g |= 512;
        if (g != 0) {
            printf("pivot-root g=%#x\n", g);
            fflush(stdout);
        }
        _exit(g == 0 ? 0 : 1);
    }
    int f = 0, code = t_exit_status(child);
    if (code == 0x7f || code == 0x7e)
        f |= 1;
    else if (code != 0)
        f |= 2;
    /* The parent's namespace is untouched. */
    if (t_info_has("/tmp/pr")) f |= 4;
    rmdir("/tmp/pr");
    snprintf(why, sizeof(why), "f=%#x code=%d", f, code);
    report("pivot_root", f == 0, why);
}

/* Write `text` to `path`; 0 or the errno. */
static int t_write_text(const char *path, const char *text)
{
    int fd = open(path, O_WRONLY);
    if (fd < 0) return errno;
    ssize_t n = write(fd, text, strlen(text));
    int e = n == (ssize_t)strlen(text) ? 0 : (n < 0 ? errno : EIO);
    close(fd);
    return e;
}

/* Read up to `len - 1` bytes of `path` into `buf`; the count, or -1. */
static ssize_t t_read_text(const char *path, char *buf, size_t len)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1;
    ssize_t n = read(fd, buf, len - 1);
    close(fd);
    buf[n < 0 ? 0 : n] = 0;
    return n;
}

#ifndef NS_GET_USERNS
#define NS_GET_USERNS 0xb701
#define NS_GET_PARENT 0xb702
#define NS_GET_OWNER_UID 0xb704
#endif

/* User namespaces (ADR 0022 H4a): the map files and their write rules, the
 * unprivileged_userns_clone switch (off by default), the nsfs ioctls, and
 * that every capability in a new namespace counts for nothing outside it. */
static void test_user_ns(void)
{
    char why[200];
    const char *sw = "/proc/sys/kernel/unprivileged_userns_clone";
    pid_t child = fork();
    if (child == 0) {
        int g = 0;
        char buf[128];
        struct stat st;
        /* The initial namespace maps every ID to itself. */
        if (t_read_text("/proc/self/uid_map", buf, sizeof(buf)) < 0 ||
            strcmp(buf, "         0          0 4294967295\n") != 0)
            g |= 1;
        pid_t b = fork();
        if (b == 0) {
            int h = 0, fd;
            unsigned owner = 99;
            if (unshare(CLONE_NEWUSER) != 0) _exit(0x7f);
            if (t_read_text("/proc/self/uid_map", buf, sizeof(buf)) != 0) h |= 1;
            /* One write; the second EPERM. */
            if (t_write_text("/proc/self/uid_map", "0 0 1") != 0 ||
                t_write_text("/proc/self/uid_map", "0 0 1") != EPERM)
                h |= 2;
            if (t_write_text("/proc/self/setgroups", "deny") != 0 ||
                t_write_text("/proc/self/gid_map", "0 0 1\n") != 0 ||
                t_write_text("/proc/self/setgroups", "allow") != EPERM)
                h |= 4;
            if (t_read_text("/proc/self/uid_map", buf, sizeof(buf)) < 0 ||
                strcmp(buf, "         0          0          1\n") != 0)
                h |= 8;
            /* Every capability here, none over the initial namespace's
             * UTS namespace. */
            if (!(sethostname("x", 1) == -1 && errno == EPERM)) h |= 16;
            /* Its UTS namespace's owner is above this one: EPERM; so is
             * this namespace's parent. Its own owner reads as uid 0. */
            fd = open("/proc/self/ns/uts", O_RDONLY);
            if (fd < 0 || !(ioctl(fd, NS_GET_USERNS) == -1 && errno == EPERM)) h |= 32;
            if (fd >= 0) close(fd);
            fd = open("/proc/self/ns/user", O_RDONLY);
            if (fd < 0 || !(ioctl(fd, NS_GET_PARENT) == -1 && errno == EPERM) ||
                ioctl(fd, NS_GET_OWNER_UID, &owner) != 0 || owner != 0)
                h |= 64;
            if (fd >= 0) close(fd);
            _exit(h);
        }
        int code = t_exit_status(b);
        if (code != 0) g |= code == 0x7f ? 2 : code << 8;
        /* From the initial namespace: NS_GET_USERNS opens it (inode
         * 4026531837); it has no parent. */
        int fd = open("/proc/self/ns/uts", O_RDONLY), u = -1;
        if (fd < 0 || (u = ioctl(fd, NS_GET_USERNS)) < 0 || fstat(u, &st) != 0 ||
            st.st_ino != 4026531837u || !(ioctl(u, NS_GET_PARENT) == -1 && errno == EPERM))
            g |= 4;
        if (fd >= 0) close(fd);
        if (u >= 0) close(u);
        /* Unprivileged, while the switch is 0: EPERM. */
        if (t_read_text(sw, buf, sizeof(buf)) < 0 || strcmp(buf, "0\n") != 0) g |= 8;
        pid_t c = fork();
        if (c == 0) {
            if (setgid(65534) != 0 || setuid(65534) != 0) _exit(1);
            _exit(unshare(CLONE_NEWUSER) == -1 && errno == EPERM ? 0 : 1);
        }
        if (t_exit_status(c) != 0) g |= 16;
        /* With it 1: an unprivileged user maps itself, the gid only after
         * denying setgroups, and cannot go back to the parent. */
        if (t_write_text(sw, "1") != 0) g |= 32;
        /* A directory the namespace's root may write: it holds no
         * capability over root's /tmp, whose owner it does not map. */
        unlink("/tmp/userns-d/f");
        mkdir("/tmp/userns-d", 0777);
        chmod("/tmp/userns-d", 0777);
        c = fork();
        if (c == 0) {
            int k = 0, init_ns = open("/proc/self/ns/user", O_RDONLY);
            if (setgid(65534) != 0 || setuid(65534) != 0 || init_ns < 0) _exit(0x7f);
            /* The ID change made the process non-dumpable, which makes
             * root own its /proc files; an exec would undo that. */
            if (prctl(PR_SET_DUMPABLE, 1, 0, 0, 0) != 0) _exit(0x7d);
            if (unshare(CLONE_NEWUSER) != 0) _exit(0x7e);
            if (t_write_text("/proc/self/uid_map", "0 65533 1") != EPERM ||
                t_write_text("/proc/self/uid_map", "0 65534 1") != 0)
                k |= 1;
            if (t_write_text("/proc/self/gid_map", "0 65534 1") != EPERM ||
                t_write_text("/proc/self/setgroups", "deny") != 0 ||
                t_write_text("/proc/self/gid_map", "0 65534 1") != 0)
                k |= 2;
            /* IDs as this namespace sees them (ADR 0022 H4b): its own
             * are 0; root's files, unmapped, show the overflow IDs; an
             * unmapped ID is EINVAL; setgroups stays denied. */
            struct stat rs;
            char text[1024];
            if (getuid() != 0 || geteuid() != 0 || getgid() != 0 || getegid() != 0) k |= 8;
            if (stat("/", &rs) != 0 || rs.st_uid != 65534 || rs.st_gid != 65534) k |= 16;
            if (!(setuid(5) == -1 && errno == EINVAL) ||
                !(setgroups(0, NULL) == -1 && errno == EPERM))
                k |= 32;
            if (t_read_text("/proc/self/status", text, sizeof(text)) <= 0 ||
                !strstr(text, "Uid:\t0\t0\t0\t0\n"))
                k |= 64;
            /* Its root keeps its capabilities across exec. */
            int pfd[2];
            if (pipe(pfd) != 0) _exit(0x7c);
            pid_t e = fork();
            if (e == 0) {
                dup2(pfd[1], 1);
                execl("/bin/cat", "cat", "/proc/self/status", (char *)NULL);
                _exit(1);
            }
            close(pfd[1]);
            ssize_t got = 0, n;
            while (got < (ssize_t)sizeof(text) - 1 &&
                   (n = read(pfd[0], text + got, sizeof(text) - 1 - got)) > 0)
                got += n;
            text[got] = 0;
            close(pfd[0]);
            waitpid(e, NULL, 0);
            const char *eff = strstr(text, "CapEff:\t");
            if (!eff || strncmp(eff + 8, "0000000000000000", 16) == 0) k |= 128;
            /* What its root may do over what the namespace owns (ADR 0022
             * H4c): make a UTS namespace and name it, and give its own file
             * a capability, stored as v3 naming it (checked outside). */
            pid_t h = fork();
            if (h == 0) {
                int m = 0;
                struct utsname un;
                static const unsigned char v2[20] = {1, 0, 0, 2, 0, 4};
                unsigned char back[32];
                if (unshare(CLONE_NEWUTS) != 0 || sethostname("inner", 5) != 0 ||
                    uname(&un) != 0 || strcmp(un.nodename, "inner") != 0)
                    m |= 1;
                int fd = open("/tmp/userns-d/f", O_CREAT | O_WRONLY | O_EXCL, 0755);
                if (fd < 0)
                    m |= 8;
                else if (setxattr("/tmp/userns-d/f", "security.capability", v2, sizeof(v2),
                                  0) != 0)
                    m |= 2;
                if (m & 10) {
                    printf("user-ns fcap errno=%d\n", errno);
                    fflush(stdout);
                }
                if (fd >= 0) close(fd);
                if (getxattr("/tmp/userns-d/f", "security.capability", back,
                             sizeof(back)) != 20 || memcmp(back, v2, 20) != 0)
                    m |= 4;
                _exit(m);
            }
            int hcode = t_exit_status(h);
            if (hcode != 0) {
                printf("user-ns h4c=%#x\n", hcode);
                fflush(stdout);
                k |= 4;
            }
            if (!(setns(init_ns, CLONE_NEWUSER) == -1 && errno == EPERM)) k |= 4;
            /* Its capabilities here set no ids outside: an unmapped id
             * fails (Linux EINVAL; EPERM until ids are translated). */
            if (setuid(1) != -1 || setgid(1) != -1) k |= 8;
            _exit(k);
        }
        code = t_exit_status(c);
        if (code != 0) g |= code << 16;
        /* Outside, the capability the namespace's root set is v3, naming
         * that root (kernel uid 65534): it is no capability of ours. */
        unsigned char fcap[32];
        if (getxattr("/tmp/userns-d/f", "security.capability", fcap, sizeof(fcap)) != 24 ||
            memcmp(fcap, "\x01\0\0\x03", 4) != 0 || memcmp(fcap + 20, "\xfe\xff\0\0", 4) != 0)
            g |= 512;
        unlink("/tmp/userns-d/f");
        rmdir("/tmp/userns-d");
        if (t_write_text(sw, "0") != 0) g |= 64;
        if (t_read_text("/proc/sys/user/max_user_namespaces", buf, sizeof(buf)) <= 0 ||
            atoi(buf) <= 0)
            g |= 128;
        /* waitid reports the child's real uid. */
        c = fork();
        if (c == 0) _exit(0);
        siginfo_t si;
        memset(&si, 0, sizeof(si));
        if (waitid(P_PID, c, &si, WEXITED) != 0 || si.si_pid != c || si.si_uid != getuid())
            g |= 256;
        if (g != 0) {
            printf("user-ns g=%#x\n", g);
            fflush(stdout);
        }
        _exit(g == 0 ? 0 : 1);
    }
    int code = t_exit_status(child);
    /* Never leave the switch on. */
    t_write_text(sw, "0");
    snprintf(why, sizeof(why), "code=%d", code);
    report("user_ns", code == 0, why);
}

/* PID namespaces, as Linux 7.3 (ADR 0022 H5). A child unshares
 * CLONE_NEWPID; its next child is the new namespace's init, 1 there with
 * no parent it can see, whose own child is 2. Inside, the init takes only
 * the signals it handles, a procfs mounted there lists 1 and 2, and status
 * shows NSpid from that mount's view; outside, the init is seen by its
 * kernel pid, NSpid shows both levels and the nsfs ioctls translate. When
 * the init exits the rest of the namespace goes with it and no process can
 * be made there any more (ENOMEM). */
#ifndef CLONE_NEWPID
#define CLONE_NEWPID 0x20000000
#endif
#define T_NS_GET_PID_FROM_PIDNS 0x8004b706
#define T_NS_GET_PID_IN_PIDNS 0x8004b708

/* How many process directories a procfs at `dir` lists, and whether they
 * are exactly "1" and "2". */
static int t_pidns_listing(const char *dir)
{
    DIR *d = opendir(dir);
    if (!d) return -1;
    int n = 0, one = 0, two = 0;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        if (e->d_name[0] < '0' || e->d_name[0] > '9') continue;
        n++;
        one |= strcmp(e->d_name, "1") == 0;
        two |= strcmp(e->d_name, "2") == 0;
    }
    closedir(d);
    return n == 2 && one && two ? 0 : n + 100;
}

/* Whether the status file at `path` has the line `want`. */
static int t_status_has(const char *path, const char *want)
{
    char text[2048];
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return 0;
    ssize_t n = read(fd, text, sizeof(text) - 1);
    close(fd);
    if (n <= 0) return 0;
    text[n] = 0;
    return strstr(text, want) != NULL;
}

#define T_KCMP_VM 1

/* Process `pid`'s CPU-time clock (Linux's MAKE_PROCESS_CPUCLOCK with
 * CPUCLOCK_SCHED). */
static clockid_t t_cpuclock(pid_t pid)
{
    return (clockid_t)((~(unsigned)pid << 3) | 2u);
}

/* The pid crossings from inside (ADR 0022 H5b), in the init while its
 * child 2 lives: every pid it passes is a number of its own namespace, so
 * `outside` (the maker's pid) names nothing, and what it is told about the
 * maker (a socket peer, a lock holder) is 0. Bits printed as pid-ns h5b. */
static int t_pidns_crossings(pid_t outside, int peer, int lockfd)
{
    int f = 0;
    struct rlimit rl;
    cpu_set_t set;
    /* Scheduling, priority, prlimit, ptrace, robust lists (musl's
     * sched_getscheduler is ENOSYS by design: the raw call). */
    if (syscall(SYS_sched_getscheduler, 1) < 0 || syscall(SYS_sched_getscheduler, 2) < 0)
        f |= 1;
    if (!(syscall(SYS_sched_getscheduler, outside) == -1 && errno == ESRCH)) f |= 1;
    if (!(sched_getaffinity(outside, sizeof(set), &set) == -1 && errno == ESRCH)) f |= 1;
    errno = 0;
    if (getpriority(PRIO_PROCESS, 2) == -1 && errno != 0) f |= 2;
    if (!(getpriority(PRIO_PROCESS, outside) == -1 && errno == ESRCH)) f |= 2;
    if (prlimit(1, RLIMIT_NOFILE, NULL, &rl) != 0) f |= 4;
    if (!(prlimit(outside, RLIMIT_NOFILE, NULL, &rl) == -1 && errno == ESRCH)) f |= 4;
    if (!(syscall(SYS_ptrace, 16 /* PTRACE_ATTACH */, outside, 0, 0) == -1 && errno == ESRCH))
        f |= 4;
    void *head;
    size_t hlen;
    if (syscall(SYS_get_robust_list, 1, &head, &hlen) != 0) f |= 4;
    if (!(syscall(SYS_get_robust_list, outside, &head, &hlen) == -1 && errno == ESRCH))
        f |= 4;
    /* PRIO_USER reaches only processes this namespace numbers: 1 and 2
     * here, never the maker (which checks its own nice). */
    if (setpriority(PRIO_USER, 0, 7) != 0 || getpriority(PRIO_PROCESS, 2) != 7) f |= 8;
    /* pidfds, kcmp, process_vm, capget. */
    int pfd = (int)syscall(SYS_pidfd_open, 2, 0);
    if (pfd < 0) f |= 16;
    else close(pfd);
    if (!(syscall(SYS_pidfd_open, outside, 0) == -1 && errno == ESRCH)) f |= 16;
    if (t_kcmp(1, 1, T_KCMP_VM, 0, 0) != 0 || t_kcmp(1, 2, T_KCMP_VM, 0, 0) <= 0 ||
        !(t_kcmp(1, outside, T_KCMP_VM, 0, 0) == -1 && errno == ESRCH))
        f |= 32;
    char byte;
    struct iovec l = {&byte, 1}, r = {&byte, 1};
    if (!(process_vm_readv(outside, &l, 1, &r, 1, 0) == -1 && errno == ESRCH)) f |= 32;
    struct {
        uint32_t version;
        int pid;
    } hdr = {0x20080522, 1};
    uint32_t data[6];
    if (syscall(SYS_capget, &hdr, data) != 0) f |= 64;
    hdr.pid = outside;
    if (!(syscall(SYS_capget, &hdr, data) == -1 && errno == ESRCH)) f |= 64;
    /* CPU clocks and SIGEV_THREAD_ID name numbers here too. */
    struct timespec ts;
    if (syscall(SYS_clock_gettime, t_cpuclock(2), &ts) != 0) f |= 128;
    if (!(syscall(SYS_clock_gettime, t_cpuclock(outside), &ts) == -1 && errno == EINVAL))
        f |= 128;
    struct {
        uint64_t value;
        int signo, notify, tid, pad[11];
    } ev = {0, SIGUSR1, 4 /* SIGEV_THREAD_ID */, 1, {0}};
    int timer;
    if (syscall(SYS_timer_create, CLOCK_MONOTONIC, &ev, &timer) != 0) f |= 256;
    else syscall(SYS_timer_delete, timer);
    ev.tid = outside;
    if (!(syscall(SYS_timer_create, CLOCK_MONOTONIC, &ev, &timer) == -1 && errno == EINVAL))
        f |= 256;
    /* A PI futex word holds this namespace's numbers: 1 for the init. */
    static uint32_t word;
    word = 0;
    if (syscall(SYS_futex, &word, FUTEX_LOCK_PI_PRIVATE, 0, NULL, NULL, 0) != 0 ||
        (word & FUTEX_TID_MASK) != 1 ||
        syscall(SYS_futex, &word, FUTEX_UNLOCK_PI_PRIVATE, 0, NULL, NULL, 0) != 0 || word != 0)
        f |= 512;
    /* The maker, as a socket peer and as a lock holder, is process 0. */
    struct ucred c;
    socklen_t len = sizeof(c);
    if (getsockopt(peer, SOL_SOCKET, SO_PEERCRED, &c, &len) != 0 || c.pid != 0) f |= 1024;
    struct flock fl = {.l_type = F_WRLCK, .l_whence = SEEK_SET, .l_start = 0, .l_len = 0};
    if (fcntl(lockfd, F_GETLK, &fl) != 0 || fl.l_type != F_WRLCK || fl.l_pid != 0) f |= 2048;
    /* So is the terminal's group, led from outside. */
    pid_t pg = -1;
    if (ioctl(0, TIOCGPGRP, &pg) == 0 && pg != 0) f |= 4096;
    if (f != 0) {
        printf("pid-ns h5b=%#x\n", f);
        fflush(stdout);
    }
    return f;
}

/* The namespace's init: its own child, a procfs of its own, then it
 * waits for the go-ahead and exits, taking the child with it. */
static int t_pidns_init(int ready, int go, pid_t outside, int peer, int lockfd)
{
    int f = 0;
    char c;
    if (getpid() != 1 || getppid() != 0) f |= 1;
    /* Its group and session are led from outside the namespace. */
    if (getpgid(0) != 0 || getsid(0) != 0) f |= 2;
    int sync[2];
    if (pipe(sync) != 0) return 64;
    pid_t g = fork();
    if (g == 0) {
        int bad = getpid() != 2 || getppid() != 1;
        /* From inside, the init's default actions are not taken. */
        bad |= kill(1, SIGTERM) != 0 || kill(1, SIGKILL) != 0;
        if (write(sync[1], bad ? "b" : "o", 1) != 1) _exit(1);
        for (;;) pause();
    }
    if (g != 2) f |= 4;
    if (read(sync[0], &c, 1) != 1 || c != 'o') f |= 8;
    /* A reaped child's number reads as the namespace numbers it: its
     * number is freed by the reap, which must not make waitpid and waitid
     * report 0 (PR #35 review). */
    pid_t h = fork();
    if (h == 0) _exit(7);
    int st = 0;
    if (h <= 0 || waitpid(h, &st, 0) != h || !WIFEXITED(st) || WEXITSTATUS(st) != 7) f |= 4;
    pid_t k = fork();
    if (k == 0) _exit(8);
    siginfo_t si;
    memset(&si, 0, sizeof(si));
    if (k <= 0 || waitid(P_PID, k, &si, WEXITED) != 0 || si.si_pid != k || si.si_status != 8)
        f |= 4;
    if (t_pidns_crossings(outside, peer, lockfd) != 0) f |= 128;
    /* A procfs mounted here shows this namespace (in a mount namespace of
     * its own, so nothing is left behind). */
    if (unshare(CLONE_NEWNS) != 0 ||
        mount("proc", "/tmp/pidns-proc", "proc", 0, NULL) != 0 ||
        t_pidns_listing("/tmp/pidns-proc") != 0 ||
        !t_status_has("/tmp/pidns-proc/self/status", "NSpid:\t1\n") ||
        !t_status_has("/tmp/pidns-proc/2/status", "PPid:\t1\n"))
        f |= 16;
    if (write(ready, "r", 1) != 1) f |= 32;
    if (read(go, &c, 1) != 1) f |= 32;
    return f;
}

/* The process that made the namespace: it sees the init from outside. */
static int t_pidns_maker(void)
{
    int f = 0;
    char c, buf[128];
    if (unshare(CLONE_NEWPID) != 0) return 1;
    /* A second one before the first is entered (copy_pid_ns), and the
     * namespace for children has no init yet. */
    if (!(unshare(CLONE_NEWPID) == -1 && errno == EINVAL)) f |= 2;
    if (!(readlink("/proc/self/ns/pid_for_children", buf, sizeof(buf)) == -1 &&
          errno == ENOENT))
        f |= 1 << 16;
    int ready[2], go[2], sv[2];
    if (pipe(ready) != 0 || pipe(go) != 0) return 4;
    /* For the init to look at: a socket pair this process made (its peer)
     * and a lock it holds; its own nice, which the init's PRIO_USER must
     * not reach. */
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) != 0) return 4;
    int lockfd = open("/tmp/pidns-lock", O_RDWR | O_CREAT | O_CLOEXEC, 0644);
    struct flock fl = {.l_type = F_WRLCK, .l_whence = SEEK_SET, .l_start = 0, .l_len = 0};
    if (lockfd < 0 || fcntl(lockfd, F_SETLK, &fl) != 0) return 4;
    errno = 0;
    int nice0 = getpriority(PRIO_PROCESS, 0);
    pid_t me = getpid();
    pid_t init = fork();
    if (init == 0) {
        int lfd = open("/tmp/pidns-lock", O_RDWR | O_CLOEXEC);
        _exit(t_pidns_init(ready[1], go[0], me, sv[1], lfd));
    }
    if (init < 0 || read(ready[0], &c, 1) != 1) return 8;
    if (getpriority(PRIO_PROCESS, 0) != nice0) f |= 1 << 17;
    struct ucred cred;
    socklen_t clen = sizeof(cred);
    if (getsockopt(sv[0], SOL_SOCKET, SO_PEERCRED, &cred, &clen) != 0 || cred.pid != me)
        f |= 1 << 17;
    close(sv[0]);
    close(sv[1]);
    close(lockfd);
    unlink("/tmp/pidns-lock");
    /* Our own namespace is unchanged; its init is ours to see. */
    if (getpid() == 1 || init == 1) f |= 16;
    char path[64], want[64];
    snprintf(path, sizeof(path), "/proc/%d/status", (int)init);
    snprintf(want, sizeof(want), "NSpid:\t%d\t1\n", (int)init);
    if (!t_status_has(path, want)) f |= 32;
    if (readlink("/proc/self/ns/pid_for_children", buf, sizeof(buf)) <= 0) f |= 32;
    snprintf(path, sizeof(path), "/proc/%d/ns/pid", (int)init);
    int nsfd = open(path, O_RDONLY | O_CLOEXEC);
    pid_t grandchild = nsfd < 0 ? -1 : ioctl(nsfd, T_NS_GET_PID_FROM_PIDNS, 2);
    if (nsfd < 0 || ioctl(nsfd, T_NS_GET_PID_FROM_PIDNS, 1) != init ||
        ioctl(nsfd, T_NS_GET_PID_IN_PIDNS, init) != 1 || grandchild <= 0 ||
        !(ioctl(nsfd, T_NS_GET_PID_IN_PIDNS, getpid()) == -1 && errno == ESRCH))
        f |= 64;
    int parent = nsfd < 0 ? -1 : ioctl(nsfd, T_NS_GET_PARENT);
    if (parent < 0) f |= 64;
    else close(parent);
    if (nsfd >= 0) close(nsfd);
    /* The init exits, and its child with it. */
    if (write(go[1], "g", 1) != 1) f |= 128;
    int st;
    if (waitpid(init, &st, 0) != init || !WIFEXITED(st)) f |= 128;
    else f |= WEXITSTATUS(st) << 8;
    if (grandchild > 0 && !(kill(grandchild, 0) == -1 && errno == ESRCH)) f |= 128;
    /* No process can be made there any more. */
    pid_t late = fork();
    if (late == 0) _exit(0);
    if (late >= 0) {
        waitpid(late, NULL, 0);
        f |= 1 << 15;
    } else if (errno != ENOMEM) {
        f |= 1 << 15;
    }
    return f;
}

static void test_pid_ns(void)
{
    char why[96];
    mkdir("/tmp/pidns-proc", 0755);
    int code = -1;
    pid_t maker = fork();
    if (maker == 0) {
        int f = t_pidns_maker();
        printf("pid-ns maker f=%#x\n", f);
        fflush(stdout);
        _exit(f == 0 ? 0 : 1);
    }
    int st;
    if (maker > 0 && waitpid(maker, &st, 0) == maker && WIFEXITED(st)) code = WEXITSTATUS(st);
    rmdir("/tmp/pidns-proc");
    snprintf(why, sizeof(why), "code=%d", code);
    report("pid_ns", code == 0, why);
}

/* ADR 0022 H6: network namespaces. */
#define T_NETNS_PORT 47123

/* A TCP socket bound to the wildcard address on `port` (a new socket, or
 * `s` when not negative) and listening: its fd, or -errno. */
static int t_netns_listen(int s, int port)
{
    if (s < 0) s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) return -errno;
    struct sockaddr_in a;
    memset(&a, 0, sizeof(a));
    a.sin_family = AF_INET;
    a.sin_port = htons(port);
    a.sin_addr.s_addr = htonl(INADDR_ANY);
    if (bind(s, (struct sockaddr *)&a, sizeof(a)) != 0 || listen(s, 4) != 0) {
        int e = errno;
        close(s);
        return -e;
    }
    return s;
}

/* The interface lines of /proc/net/dev (after its two header lines), up to
 * `max` names into `names`: how many, or -1. */
static int t_netns_devices(char names[][16], int max)
{
    FILE *fp = fopen("/proc/net/dev", "r");
    if (!fp) return -1;
    char line[256];
    int n = 0, row = 0;
    while (fgets(line, sizeof(line), fp)) {
        if (row++ < 2) continue;
        char *colon = strchr(line, ':');
        if (!colon) continue;
        *colon = 0;
        char *name = line;
        while (*name == ' ') name++;
        if (n < max) snprintf(names[n], 16, "%.15s", name);
        n++;
    }
    fclose(fp);
    return n;
}

/* In a child: a new network namespace has its own identity and ports, only
 * a loopback interface, and reaches nothing (its loopback is down); a
 * socket made before keeps the old one's ports; setns goes back. */
static int t_netns_child(int initial_fd, unsigned long initial_ino)
{
    int f = 0;
    char names[4][16];
    int before = socket(AF_INET, SOCK_STREAM, 0);
    if (unshare(CLONE_NEWNET) != 0) return 1;
    if (t_ns_ino(getpid(), "net") == initial_ino) f |= 2;
    if (!(t_netns_devices(names, 4) == 1 && strcmp(names[0], "lo") == 0)) f |= 4;
    /* The port the parent listens on is free here, once. */
    int l = t_netns_listen(-1, T_NETNS_PORT);
    if (l < 0) f |= 8;
    if (t_netns_listen(-1, T_NETNS_PORT) != -EADDRINUSE) f |= 16;
    /* The socket made before the unshare is in the parent's namespace. */
    if (before < 0 || t_netns_listen(before, T_NETNS_PORT) != -EADDRINUSE) f |= 32;
    /* Nothing is reachable: not the host, not even 127.0.0.1. */
    struct sockaddr_in to;
    memset(&to, 0, sizeof(to));
    to.sin_family = AF_INET;
    to.sin_port = htons(9);
    to.sin_addr.s_addr = htonl(0x0A000202); /* 10.0.2.2 */
    int u = socket(AF_INET, SOCK_DGRAM, 0);
    if (u < 0 || !(sendto(u, "x", 1, 0, (struct sockaddr *)&to, sizeof(to)) == -1 &&
                   errno == ENETUNREACH))
        f |= 64;
    int c = socket(AF_INET, SOCK_STREAM, 0);
    to.sin_port = htons(T_NETNS_PORT);
    to.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (c < 0 ||
        !(connect(c, (struct sockaddr *)&to, sizeof(to)) == -1 && errno == ENETUNREACH))
        f |= 128;
    /* A sockaddr shorter than sockaddr_in is EINVAL, another family
     * EAFNOSUPPORT (N-64). */
    to.sin_family = AF_INET6;
    if (!(bind(c, (struct sockaddr *)&to, sizeof(to)) == -1 && errno == EAFNOSUPPORT) ||
        !(bind(c, (struct sockaddr *)&to, 8) == -1 && errno == EINVAL))
        f |= 256;
    /* setns back to the initial one, with its devices. */
    if (setns(initial_fd, CLONE_NEWNET) != 0 || t_ns_ino(getpid(), "net") != initial_ino ||
        t_netns_devices(names, 4) < 1 || strcmp(names[0], "lo") == 0)
        f |= 512;
    if (u >= 0) close(u);
    if (c >= 0) close(c);
    if (l >= 0) close(l);
    return f;
}

static void test_net_ns(void)
{
    char why[96], names[4][16], buf[32];
    int f = 0;
    unsigned long ino = t_ns_ino(getpid(), "net");
    int nsfd = open("/proc/self/ns/net", O_RDONLY | O_CLOEXEC);
    if (nsfd < 0 || ioctl(nsfd, T_NS_GET_NSTYPE) != CLONE_NEWNET) f |= 1;
    /* The initial namespace has the devices, and the limit its default. */
    if (t_netns_devices(names, 4) < 1) f |= 2;
    int lim = open("/proc/sys/user/max_net_namespaces", O_RDONLY);
    ssize_t n = lim < 0 ? -1 : read(lim, buf, sizeof(buf) - 1);
    if (n <= 0 || (buf[n] = 0, strcmp(buf, "15000\n") != 0)) f |= 4;
    if (lim >= 0) close(lim);
    int l = t_netns_listen(-1, T_NETNS_PORT);
    if (l < 0) f |= 8;
    int code = -1;
    pid_t child = fork();
    if (child == 0) {
        int r = t_netns_child(nsfd, ino);
        printf("net-ns child f=%#x\n", r);
        fflush(stdout);
        _exit(r == 0 ? 0 : 1);
    }
    int st;
    if (child > 0 && waitpid(child, &st, 0) == child && WIFEXITED(st)) code = WEXITSTATUS(st);
    if (code != 0) f |= 16;
    /* The parent's namespace and listener are unchanged. */
    if (t_ns_ino(getpid(), "net") != ino || t_netns_listen(-1, T_NETNS_PORT) != -EADDRINUSE)
        f |= 32;
    if (l >= 0) close(l);
    if (nsfd >= 0) close(nsfd);
    snprintf(why, sizeof(why), "f=%#x child=%d", f, code);
    report("net_ns", f == 0, why);
}

int main(int argc, char **argv)
{
    if (argc >= 2 && strcmp(argv[1], "--ids") == 0)
        return print_ids();
    if (argc >= 4 && strcmp(argv[1], "--exe") == 0)
        return print_exe_check(argv[2], argv[3]);
    if (argc >= 3 && strcmp(argv[1], "--only") == 0) {
        /* `--only NAME [N]`: run it N times (default once), to catch a
         * test that fails only sometimes. */
        int repeat = argc >= 4 ? atoi(argv[3]) : 1;
        for (size_t i = 0; i < sizeof(only_tests) / sizeof(only_tests[0]); i++) {
            if (strcmp(argv[2], only_tests[i].name) == 0) {
                for (int r = 0; r < (repeat > 0 ? repeat : 1); r++)
                    only_tests[i].fn();
                printf("MUSL-ONLY %s: %d/%d\n", argv[2], passed, total);
                return passed != total;
            }
        }
        printf("MUSL-ONLY %s: no such test\n", argv[2]);
        return 2;
    }
    test_auxv(argc > 0 ? argv[0] : NULL);
    test_pie();
    test_fsync();
    test_flock();
    test_prctl();
    test_raise_abort();
    test_waitid();
    test_mprotect();
    test_wx_default();
    test_wx_opt_in();
    test_wx_note_tool();
    test_mdwe();
    test_mkfifo();
    test_mknod();
    test_open_creat_dir();
    test_unix_socket_file();
    test_threads();
    test_thread_guards();
    test_cow_race();
    test_signals();
    test_thread_fork_exec();
    test_sleeps();
    test_futex_queues();
    test_blocking_io();
    test_pipe_write_semantics();
    test_unix_socket_wakeups();
    test_creds_and_paths();
    test_spawn();
    test_sigaltstack();
    test_page_cache();
    test_program_pages_shared();
    test_dynamic_program();
    test_event_fds();
    test_signalfd();
    test_epoll();
    test_select();
    test_arch_prctl();
    test_tids();
    test_flock_blocking();
    test_robust_futex();
    test_sched();
    test_smp_placement();
    test_dl_balance();
    test_gs_base();
    test_tlb_remote_flush();
    test_signal_kick();
    test_dir_check_race();
    test_relative_entry_paths();
    test_setuid_exec("/bin/musl_runtime_test");
    test_uname_identity();
    test_uname_linux_mode();
    test_clocks_and_usage();
    test_process_clock_monotonic();
    test_many_threads();
    test_rlimits();
    test_sigpipe();
    test_memory_calls();
    test_file_calls();
    test_review_fixes();
    test_accounts();
    test_unix_accept_blocks();
    test_peercred();
    test_fs_ids();
    test_siginfo();
    test_pty();
    test_console();
    test_thread_join();
    test_ipc();
    test_priority_inheritance();
    test_dev_shm();
    test_statfs();
    test_fallocate();
    test_utimes();
    test_itimers();
    test_posix_timers();
    test_restart_syscall();
    test_adjtimex();
    test_preadv_pwritev();
    test_creat_lchown_fchmodat2();
    test_close_range_and_advice();
    test_splice_family();
    test_statx();
    test_openat2();
    test_execveat();
    test_fhandle();
    test_ioctl();
    test_record_locks();
    test_fcntl_flags();
    test_vectored_io();
    test_link_errors();
    test_stat_numbers();
    test_open_flags();
    test_text_busy();
    test_inotify_events();
    test_pidfd();
    test_clone3();
    test_kcmp_process_vm();
    test_clone_share();
    test_personality();
    test_namespaces_uts();
    test_namespaces_time();
    test_namespaces_mnt();
    test_mount_calls();
    test_mount_flags();
    test_mount_no_escape();
    test_pivot_root();
    test_user_ns();
    test_pid_ns();
    test_net_ns();
#if defined(__x86_64__)
    test_ustat_sysfs();
#endif
    printf("MUSL-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
