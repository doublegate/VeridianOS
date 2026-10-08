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
#include <errno.h>
#include <grp.h>
#include <sys/stat.h>
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
#include <sys/mman.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/sysinfo.h>
#include <sys/time.h>
#include <sys/times.h>
#include <sys/select.h>
#include <spawn.h>
#include <sys/wait.h>
#include <sys/socket.h>
#include <sys/auxv.h>
#include <sys/signalfd.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <sys/timerfd.h>
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
static void burn_cpu(long ms)
{
    struct timespec start, now;
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &start);
    do {
        for (volatile int i = 0; i < 10000; i++)
            ;
        clock_gettime(CLOCK_THREAD_CPUTIME_ID, &now);
    } while ((now.tv_sec - start.tv_sec) * 1000 + (now.tv_nsec - start.tv_nsec) / 1000000 < ms);
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

    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &a);
    burn_cpu(30);
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &b);
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
    clock_gettime(CLOCK_REALTIME, &rt);
    rt.tv_nsec += 50000000;
    if (rt.tv_nsec >= 1000000000) {
        rt.tv_sec++;
        rt.tv_nsec -= 1000000000;
    }
    pthread_mutex_lock(&cond_lock);
    clock_gettime(CLOCK_MONOTONIC, &a);
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
    if (getrusage(RUSAGE_SELF, &ru) != 0 || ru.ru_utime.tv_sec * 1000 + ru.ru_utime.tv_usec / 1000
                                                  + ru.ru_stime.tv_sec * 1000
                                                  + ru.ru_stime.tv_usec / 1000
                                              < 30
        || ru.ru_maxrss <= 0)
        f |= 512;
    errno = 0;
    if (getrusage(7, &ru) != -1 || errno != EINVAL) f |= 512;
    pid = fork();
    if (pid == 0) {
        burn_cpu(40);
        _exit(0);
    }
    struct rusage child_ru;
    if (wait4(pid, &st, 0, &child_ru) != pid
        || child_ru.ru_utime.tv_sec * 1000 + child_ru.ru_utime.tv_usec / 1000
                   + child_ru.ru_stime.tv_sec * 1000 + child_ru.ru_stime.tv_usec / 1000
               < 40)
        f |= 1024;
    if (getrusage(RUSAGE_CHILDREN, &ru) != 0
        || ru.ru_utime.tv_sec * 1000 + ru.ru_utime.tv_usec / 1000 + ru.ru_stime.tv_sec * 1000
                   + ru.ru_stime.tv_usec / 1000
               < 40)
        f |= 1024;
    struct tms tm;
    clock_t since_boot = times(&tm);
    if (since_boot <= 0 || tm.tms_utime + tm.tms_stime < 3 || tm.tms_cutime + tm.tms_cstime < 4)
        f |= 2048;
    struct sysinfo si;
    if (sysinfo(&si) != 0 || si.totalram == 0 || si.freeram == 0 || si.freeram > si.totalram
        || si.procs == 0 || si.mem_unit != 1 || si.uptime <= 0)
        f |= 4096;

    static char why[64];
    snprintf(why, sizeof(why), "fail=%#x", f);
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
    void *brk0 = sbrk(0);
    if (sbrk(1 << 20) != (void *)-1 || sbrk(0) != brk0) f |= 64;
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
 * AT_SECURE (so the loader ignores LD_PRELOAD); under no_new_privs, or when
 * another process shares its directories and umask (LSM_UNSAFE_SHARE), it
 * gains nothing; the command name is the file's. A write by a non-root owner and
 * any chown clear the bit (file_remove_privs, chown). */
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
               && st_shared == 0 && strcmp(shared, "1000 1000 1000 0 1 suid_ids") == 0,
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
    unlink(copy);
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

/* Guards found by the security review of stage D2: a clone with a
 * non-canonical TLS base is refused (it used to reach IA32_FS_BASE and #GP
 * the kernel at every switch), and exec is refused while another thread of
 * the process runs (it freed page tables that thread still used). */
static void *sleeper(void *arg)
{
    (void)arg;
    struct timespec ts = {0, 200 * 1000 * 1000};
    nanosleep(&ts, 0);
    return 0;
}

static void test_thread_guards(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        static char stack[16384] __attribute__((aligned(16)));
        long flags = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD |
                     CLONE_SYSVSEM | CLONE_SETTLS;
        errno = 0;
        long r = syscall(SYS_clone, flags, stack + sizeof(stack), 0, 0, 0x8000000000000000UL);
        int tls_refused = r == -1 && errno == EINVAL;

        pthread_t t;
        int exec_refused = 0;
        if (pthread_create(&t, 0, sleeper, 0) == 0) {
            char *argv[] = {"true", 0};
            errno = 0;
            exec_refused = execve("/bin/true", argv, 0) == -1 && errno == EAGAIN;
            pthread_join(t, 0);
        }
        _exit(tls_refused && exec_refused ? 0 : 1 + tls_refused + 2 * exec_refused);
    }
    int st = 0;
    int ok = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;
    static char why[48];
    snprintf(why, sizeof(why), "status=0x%x", st);
    report("musl_clone_tls_and_exec_guards", ok, why);
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

/* The auxiliary vector (ADR 0010), as getauxval reads it. */
static void test_auxv(const char *argv0)
{
    const unsigned char *random = (const unsigned char *)getauxval(AT_RANDOM);
    const char *execfn = (const char *)getauxval(AT_EXECFN);
    const char *platform = (const char *)getauxval(AT_PLATFORM);
    int random_set = 0;
    for (int i = 0; random && i < 16; i++)
        random_set |= random[i];
    int ok = getauxval(AT_PAGESZ) == 4096 && random_set && execfn && argv0
             && strcmp(execfn, argv0) == 0 && platform && strcmp(platform, "x86_64") == 0
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
 * (0x5555_5555_4000), and musl relocated it before main, so code, data and
 * the pointers in data all refer to addresses there. */
static const char *const pie_string = "relocated";
static void test_pie(void)
{
    uintptr_t code = (uintptr_t)&test_pie;
    uintptr_t data = (uintptr_t)&pie_string;
    uintptr_t entry = getauxval(AT_ENTRY);
    int ok = code >= 0x555555554000UL && code < 0x555565554000UL && data > code
             && entry >= 0x555555554000UL && entry < code + 0x10000000UL
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
    /* The original eventfd(initval) call: eventfd2 without flags. */
    int lfd = syscall(SYS_eventfd, 3);
    v = 0;
    int legacy = lfd >= 0 && (fcntl(lfd, F_GETFL) & O_NONBLOCK) == 0 && read(lfd, &v, 8) == 8
                 && v == 3;

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
     * flags, so a blocking descriptor. */
    int lsfd = syscall(SYS_signalfd, -1, &usr2, 8);
    kill(getpid(), SIGUSR2);
    int legacy = lsfd >= 0 && (fcntl(lsfd, F_GETFL) & O_NONBLOCK) == 0
                 && read(lsfd, &info, sizeof(info)) == sizeof(info) && info.ssi_signo == SIGUSR2;
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
    int e_size = syscall(SYS_epoll_create, 0) == -1 && errno == EINVAL;
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
    int idle = syscall(SYS_select, p[0] + 1, &r, NULL, NULL, tv_raw) == 0 && !FD_ISSET(p[0], &r);
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
    int einval = syscall(SYS_select, 1, &r, NULL, NULL, bad_tv) == -1 && errno == EINVAL;

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
        long long t0 = mono_ns();
        int ok = c >= 0 && flock(c, LOCK_EX) == 0;
        long waited = (long)((mono_ns() - t0) / 1000000);
        (void)!write(fds[1], &waited, sizeof(waited));
        _exit(ok ? 0 : 1);
    }
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
struct test_sched_attr {
    uint32_t size, policy;
    uint64_t flags;
    int32_t nice;
    uint32_t priority;
    uint64_t runtime, deadline, period;
};

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
        /* 90% more does not fit next to the parent's 10%. */
        struct test_sched_attr big = {sizeof(big), 6, 0, 0, 0, 9000000, 10000000, 10000000};
        errno = 0;
        int busy = syscall(SYS_sched_setattr, 0, &big, 0) == -1 && errno == EBUSY;
        _exit(busy ? 0 : 1);
    }
    int admission = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status)
                    && WEXITSTATUS(status) == 0;

    /* Back to SCHED_OTHER with a nice value, read through getpriority. */
    struct test_sched_attr n = {sizeof(n), SCHED_OTHER, 0, 5, 0, 0, 0, 0};
    errno = 0;
    int nice5 = syscall(SYS_sched_setattr, 0, &n, 0) == 0 && getpriority(PRIO_PROCESS, 0) == 5
                && errno == 0 && setpriority(PRIO_PROCESS, 0, 3) == 0
                && getpriority(PRIO_PROCESS, 0) == 3;

    /* One CPU runs tasks: the mask and the processor count say so. */
    cpu_set_t set;
    CPU_ZERO(&set);
    int aff = sched_getaffinity(0, sizeof(set), &set) == 0 && CPU_COUNT(&set) == 1
              && CPU_ISSET(0, &set) && sysconf(_SC_NPROCESSORS_ONLN) == 1;
    CPU_ZERO(&set);
    CPU_SET(1, &set);
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

int main(int argc, char **argv)
{
    if (argc >= 2 && strcmp(argv[1], "--ids") == 0)
        return print_ids();
    test_auxv(argc > 0 ? argv[0] : NULL);
    test_pie();
    test_fsync();
    test_flock();
    test_prctl();
    test_raise_abort();
    test_waitid();
    test_mprotect();
    test_threads();
    test_thread_guards();
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
    test_setuid_exec("/bin/musl_runtime_test");
    test_clocks_and_usage();
    test_rlimits();
    test_sigpipe();
    printf("MUSL-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
