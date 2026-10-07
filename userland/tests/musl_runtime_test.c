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
#include <sys/wait.h>
#include <sys/syscall.h>
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
    printf("MUSL-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
