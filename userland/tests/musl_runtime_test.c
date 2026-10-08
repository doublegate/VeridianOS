/*
 * musl_runtime_test -- in-guest checks for programs built against the
 * patched musl (tools/cross/build-musl.sh), as the KDE binaries are.
 *
 * audit_runtime_test is built with VeridianOS's own C library, so it never
 * exercises musl's syscall remapping. This program does: fsync and flock
 * (which share native syscall 73), prctl, raise/abort (tkill), waitid, the
 * memory protections, and threads (which run as tasks of their own since
 * stage D2), each as musl issues them.
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
#include <spawn.h>
#include <sys/wait.h>
#include <sys/socket.h>
#include <sys/syscall.h>
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

/* prctl reaches prctl (it used to land on native unlink; N-103), the
 * name options work and security options fail closed (N-151). */
static void test_prctl(void)
{
    char name[16] = {0};
    int set = prctl(PR_SET_NAME, "musltest", 0, 0, 0);
    int get = prctl(PR_GET_NAME, name, 0, 0, 0);
    errno = 0;
    int nnp = prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    int nnp_errno = errno;
    static char why[80];
    snprintf(why, sizeof(why), "set=%d get=%d nnp=%d/%d", set, get, nnp, nnp_errno);
    report("musl_prctl", set == 0 && get == 0 && nnp == -1 && nnp_errno == EINVAL, why);
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

int main(void)
{
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
    printf("MUSL-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
