/*
 * audit_runtime_test -- in-guest checks for the v0.26 audit fixes that the
 * 33 boot tests never exercise: process and thread exit/reaping, the libc
 * allocator under threads, scanf field widths, fd numbering, rename and
 * permission enforcement for a non-root user, sticky directories, sockets
 * as per-process fds and SCM_RIGHTS, directory search permission, and the
 * direction flag across a system call.
 *
 * Run as root from a BusyBox shell: /bin/audit_runtime_test [threads]
 * Prints one "PASS <name>" or "FAIL <name>: <why>" line per check and a
 * final "AUDIT-RUNTIME: <passed>/<total>" summary.
 */

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <stdio.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
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
}

/* --- Process exit and reaping (N-03): many short-lived children. ------- */
static void test_fork_exit(void)
{
    int ok = 1;
    for (int i = 0; i < 40 && ok; i++) {
        pid_t pid = fork();
        if (pid < 0) {
            ok = 0;
            break;
        }
        if (pid == 0)
            _exit(i & 0x7f);
        int status = 0;
        if (waitpid(pid, &status, 0) != pid || !WIFEXITED(status) ||
            WEXITSTATUS(status) != (i & 0x7f))
            ok = 0;
    }
    report("fork_exit_reap_x40", ok, "fork/waitpid/exit status mismatch");
}

/* --- Copy-on-write fork (MEM-PERF-03, PROC-ARCH-01). ------------------
 * Parent and child share pages after fork; each side's writes must stay its
 * own. The child's read() lands in a page it still shares with its parent,
 * so the kernel's user-copy write has to take a private copy (a protection
 * fault on a present page, resolved as COW instead of EFAULT), and the
 * parent must not see the data. The pipe is filled before the fork so no
 * syscall blocks (a blocked syscall cannot yet yield to another process). */
static int cow_global = 7;

/* CowShared from /proc/meminfo, in kB (-1 if unreadable). */
static int cow_shared_kb(void)
{
    char info[1024];
    int mfd = open("/proc/meminfo", O_RDONLY);
    if (mfd < 0)
        return -1;
    ssize_t m = read(mfd, info, sizeof(info) - 1);
    close(mfd);
    if (m <= 0)
        return -1;
    info[m] = 0;
    const char *c = strstr(info, "CowShared:");
    return c ? atoi(c + 10) : -1;
}

static void test_cow(void)
{
    char stack_buf[64];
    memset(stack_buf, 's', sizeof(stack_buf));
    char *heap = malloc(8192);
    int fds[2];
    if (!heap || pipe(fds) != 0 || write(fds[1], "hello", 5) != 5) {
        report("fork_copy_on_write_isolation", 0, "setup failed");
        return;
    }
    memset(heap, 'p', 8192);
    pid_t pid = fork();
    if (pid < 0) {
        report("fork_copy_on_write_isolation", 0, "fork failed");
        return;
    }
    if (pid == 0) {
        /* While the parent is alive the pages are shared, which
         * /proc/meminfo reports as CowShared (a deep-copying fork shows 0). */
        if (cow_shared_kb() <= 0)
            _exit(5);
        cow_global = 99;
        stack_buf[0] = 'S';
        heap[4096] = 'C';
        ssize_t n = read(fds[0], heap, 5);
        int ok = n == 5 && memcmp(heap, "hello", 5) == 0 && cow_global == 99 &&
                 stack_buf[0] == 'S' && heap[4096] == 'C';
        _exit(ok ? 0 : (n < 0 ? 10 + (errno & 0x3f) : 1));
    }
    int status = 0;
    int child_ok = waitpid(pid, &status, 0) == pid && WIFEXITED(status) &&
                   WEXITSTATUS(status) == 0;
    int parent_ok = cow_global == 7 && stack_buf[0] == 's' && heap[0] == 'p' &&
                    heap[4096] == 'p';
    /* With the child gone nothing is shared any more: owner counts drop. */
    int shared_after = cow_shared_kb();
    parent_ok = parent_ok && shared_after == 0;
    cow_global = 8; /* the last owner just regains write access */
    parent_ok = parent_ok && cow_global == 8;
    static char why[96];
    snprintf(why, sizeof(why), "child %d (status 0x%x) parent %d, CowShared %d kB after", child_ok,
             (unsigned)status, parent_ok, shared_after);
    report("fork_copy_on_write_isolation", child_ok && parent_ok, why);
    close(fds[0]);
    close(fds[1]);
    free(heap);
}

/* --- 2 MiB pages through mmap(MAP_HUGETLB) (MEM-ARCH-01). -------------
 * Map 4 MiB, use every 2 MiB page, check fork gives the child its own
 * copy, and that munmap returns all 1024 frames (MemFree is back). */
static long mem_free_kb(void)
{
    char info[1024];
    int fd = open("/proc/meminfo", O_RDONLY);
    if (fd < 0)
        return -1;
    ssize_t n = read(fd, info, sizeof(info) - 1);
    close(fd);
    if (n <= 0)
        return -1;
    info[n] = 0;
    const char *m = strstr(info, "MemFree:");
    return m ? atol(m + 8) : -1;
}

static void test_huge_pages(void)
{
    const size_t len = 4u << 20;
    long before = mem_free_kb();
    unsigned char *p = mmap(NULL, len, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB, -1, 0);
    if (p == MAP_FAILED) {
        report("mmap_huge_pages", 0, "mmap(MAP_HUGETLB) failed");
        return;
    }
    int ok = ((unsigned long)p & ((2u << 20) - 1)) == 0;
    for (size_t off = 0; off < len; off += 4096)
        p[off] = (unsigned char)(off >> 12);
    for (size_t off = 0; off < len; off += 4096)
        ok = ok && p[off] == (unsigned char)(off >> 12);
    int zero_ok = p[1] == 0 && p[len - 1] == 0;
    pid_t pid = fork();
    if (pid == 0) {
        /* The child has its own copy of every 2 MiB chunk, holding the
         * parent's data (a multi-chunk mapping once copied only the first). */
        int c_ok = 1;
        for (size_t off = 0; off < len; off += 4096)
            c_ok = c_ok && p[off] == (unsigned char)(off >> 12);
        p[0] = 0xAA;
        p[len - 4096] = 0xBB;
        _exit(c_ok && p[0] == 0xAA ? 0 : 1);
    }
    int status = 0;
    int child_ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status) &&
                   WEXITSTATUS(status) == 0;
    int parent_ok = p[0] == 0 && p[len - 4096] == (unsigned char)((len - 4096) >> 12);
    int unmap_ok = munmap(p, len) == 0;
    long after = mem_free_kb();
    int freed_ok = before > 0 && after >= before - 64; /* small slack for tables */
    static char why[128];
    snprintf(why, sizeof(why), "aligned+rw %d zero %d child %d parent %d unmap %d free %ld -> %ld kB",
             ok, zero_ok, child_ok, parent_ok, unmap_ok, before, after);
    report("mmap_huge_pages", ok && zero_ok && child_ok && parent_ok && unmap_ok && freed_ok, why);
}

/* --- Joinable and detached thread exit (PROC-SEC-02). ------------------ */
static void *thread_ret(void *arg)
{
    return arg;
}

static void test_threads(void)
{
    int ok = 1;
    for (int i = 0; i < 20 && ok; i++) {
        pthread_t t;
        void *ret = NULL;
        if (pthread_create(&t, NULL, thread_ret, (void *)(long)(i + 1)) != 0 ||
            pthread_join(t, &ret) != 0 || ret != (void *)(long)(i + 1))
            ok = 0;
    }
    report("thread_join_x20", ok, "create/join/return value");

    ok = 1;
    for (int i = 0; i < 20 && ok; i++) {
        pthread_t t;
        if (pthread_create(&t, NULL, thread_ret, NULL) != 0 ||
            pthread_detach(t) != 0)
            ok = 0;
    }
    /* Give the detached threads time to exit and be reaped. */
    usleep(500000);
    report("thread_detach_x20", ok, "create/detach");
}

/* --- Allocator under threads (LIBC-SEC-01) and aligned_alloc. ---------- */
static void *alloc_worker(void *arg)
{
    unsigned seed = (unsigned)(long)arg;
    void *slots[32] = {0};
    for (int i = 0; i < 20000; i++) {
        seed = seed * 1103515245u + 12345u;
        int k = (int)((seed >> 16) % 32);
        if (slots[k]) {
            free(slots[k]);
            slots[k] = NULL;
        } else {
            size_t sz = 1 + (seed >> 8) % 1500;
            slots[k] = (seed & 7) == 0 ? aligned_alloc(64, sz) : malloc(sz);
            if (!slots[k])
                return (void *)1;
            memset(slots[k], k, sz);
        }
    }
    for (int k = 0; k < 32; k++)
        free(slots[k]);
    return NULL;
}

static void test_allocator(void)
{
    pthread_t t[4];
    int ok = 1;
    int created = 0;
    /* Join only the threads that were created: t[i] is unset after a
     * failed pthread_create (review of the v0.26.0 stack, PR #9). */
    for (int i = 0; i < 4; i++) {
        if (pthread_create(&t[created], NULL, alloc_worker, (void *)(long)(i + 1)) != 0)
            ok = 0;
        else
            created++;
    }
    for (int i = 0; i < created; i++) {
        void *r = NULL;
        if (pthread_join(t[i], &r) != 0 || r)
            ok = 0;
    }
    report("malloc_threads_4x20000", ok, "allocation failed or thread error");

    void *p = aligned_alloc(4096, 100);
    ok = p && ((unsigned long)p & 4095) == 0;
    free(p); /* used to corrupt the heap */
    void *q = malloc(64);
    ok = ok && q;
    free(q);
    report("aligned_alloc_free", ok, "misaligned or heap corrupted");
}

/* --- scanf widths (LIBC-SEC-03). --------------------------------------- */
static void test_scanf(void)
{
    char buf[8];
    int a = 0, b = 0;
    memset(buf, 'X', sizeof(buf));
    int n = sscanf("abcdefghij", "%7s", buf);
    int ok = n == 1 && strcmp(buf, "abcdefg") == 0;
    n = sscanf("12345 6", "%2d%d", &a, &b);
    ok = ok && n == 2 && a == 12 && b == 345;
    report("sscanf_width", ok, "width not honoured");
}

/* --- First open() returns fd 3 (N-06). --------------------------------- */
static void test_fd_numbering(void)
{
    int fd = open("/tmp/audit_fd", O_CREAT | O_WRONLY | O_TRUNC, 0644);
    static char why[48];
    snprintf(why, sizeof(why), "first open returned %d", fd);
    report("first_open_is_fd3", fd == 3, why);
    if (fd >= 0)
        close(fd);
}

static int write_file(const char *path, const char *text, mode_t mode)
{
    int fd = open(path, O_CREAT | O_WRONLY | O_TRUNC, mode);
    if (fd < 0)
        return -1;
    ssize_t w = write(fd, text, strlen(text));
    close(fd);
    return w == (ssize_t)strlen(text) ? 0 : -1;
}

static int read_file(const char *path, char *buf, size_t len)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0)
        return -1;
    ssize_t r = read(fd, buf, len - 1);
    close(fd);
    if (r < 0)
        return -1;
    buf[r] = '\0';
    return 0;
}

/* --- rename moves the node and replaces, not follows, a symlink. ------- */
static void test_rename(void)
{
    char buf[64];
    unlink("/tmp/audit_src");
    unlink("/tmp/audit_dst");
    unlink("/tmp/audit_link");
    unlink("/tmp/audit_victim");
    int ok = write_file("/tmp/audit_src", "moved", 0640) == 0 &&
             rename("/tmp/audit_src", "/tmp/audit_dst") == 0 &&
             read_file("/tmp/audit_dst", buf, sizeof(buf)) == 0 &&
             strcmp(buf, "moved") == 0 && access("/tmp/audit_src", F_OK) != 0;
    struct stat st = {0};
    int st_ok = stat("/tmp/audit_dst", &st) == 0;
    static char why_mv[96];
    snprintf(why_mv, sizeof(why_mv), "content/old-name ok=%d stat=%d mode=%o", ok, st_ok,
             st_ok ? (unsigned)(st.st_mode & 07777) : 0u);
    ok = ok && st_ok && (st.st_mode & 0777) == 0640;
    report("rename_moves_node", ok, why_mv);

    ok = write_file("/tmp/audit_victim", "original", 0644) == 0 &&
         symlink("/tmp/audit_victim", "/tmp/audit_link") == 0 &&
         write_file("/tmp/audit_src", "attacker", 0644) == 0 &&
         rename("/tmp/audit_src", "/tmp/audit_link") == 0 &&
         read_file("/tmp/audit_victim", buf, sizeof(buf)) == 0 &&
         strcmp(buf, "original") == 0;
    report("rename_replaces_symlink", ok, "symlink target was overwritten");
}

/* --- Permission checks as uid 1000 (FS-SEC-02, review fixes). ---------- */
static void test_nonroot_permissions(void)
{
    write_file("/tmp/audit_secret", "secret", 0600); /* root-owned */
    pid_t pid = fork();
    if (pid == 0) {
        int fails = 0;
        if (setuid(1000) != 0)
            _exit(100);
        if (open("/tmp/audit_secret", O_RDONLY) >= 0)
            fails |= 1;                         /* open: EACCES */
        if (openat(AT_FDCWD, "/tmp/audit_secret", O_RDONLY) >= 0)
            fails |= 2;                         /* openat: EACCES */
        if (chmod("/tmp/audit_secret", 0666) == 0)
            fails |= 4;                         /* chmod: not owner */
        if (chown("/tmp/audit_secret", 1000, 1000) == 0)
            fails |= 8;                         /* chown: root only */
        _exit(fails);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    int code = WIFEXITED(status) ? WEXITSTATUS(status) : 255;
    static char why[64];
    snprintf(why, sizeof(why), "child exit code %d (bitmask of failures)", code);
    report("nonroot_open_chmod_chown_denied", code == 0, why);

    struct stat st = {0};
    int ok = stat("/tmp/audit_secret", &st) == 0 && (st.st_mode & 0777) == 0600;
    static char why_sec[64];
    snprintf(why_sec, sizeof(why_sec), "mode is %o, expected 600", (unsigned)(st.st_mode & 07777));
    report("secret_mode_unchanged", ok, why_sec);
}

/* --- Sticky directories (W-17): /tmp-style 1777 dirs. ----------------- */
static void test_sticky_dir(void)
{
    mkdir("/tmp/audit_sticky", 0777);
    mkdir("/tmp/audit_open", 0777);
    int setup = chmod("/tmp/audit_sticky", 01777) == 0 &&
                chmod("/tmp/audit_open", 0777) == 0 &&
                write_file("/tmp/audit_sticky/rootfile", "r", 0644) == 0 &&
                write_file("/tmp/audit_open/rootfile", "r", 0644) == 0;
    struct stat st = {0};
    int sticky_set = stat("/tmp/audit_sticky", &st) == 0 && (st.st_mode & 01000);

    pid_t pid = fork();
    if (pid == 0) {
        int fails = 0;
        if (setuid(1000) != 0)
            _exit(100);
        struct stat own;
        if (write_file("/tmp/audit_sticky/mine", "m", 0644) != 0)
            fails |= 1;
        if (stat("/tmp/audit_sticky/mine", &own) != 0 || own.st_uid != 1000)
            fails |= 16;                        /* creator owns new file */
        if (unlink("/tmp/audit_sticky/mine") != 0)
            fails |= 1;                         /* own entry: allowed */
        if (unlink("/tmp/audit_sticky/rootfile") == 0)
            fails |= 2;                         /* other's entry: EPERM */
        if (rename("/tmp/audit_sticky/rootfile", "/tmp/audit_sticky/moved") == 0)
            fails |= 4;                         /* rename away: EPERM */
        if (unlink("/tmp/audit_open/rootfile") != 0)
            fails |= 8;                         /* control: no sticky bit */
        _exit(fails);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    int code = WIFEXITED(status) ? WEXITSTATUS(status) : 255;
    static char why[80];
    snprintf(why, sizeof(why), "setup=%d sticky_bit=%d child exit %d (bitmask)",
             setup, sticky_set, code);
    report("sticky_dir_protects_entries", setup && sticky_set && code == 0, why);
}

/* --- umask is applied to new files. ---------------------------------- */
static void test_umask(void)
{
    mode_t old = umask(022);
    unlink("/tmp/audit_umask");
    struct stat st = {0};
    int ok = write_file("/tmp/audit_umask", "u", 0666) == 0 &&
             stat("/tmp/audit_umask", &st) == 0 && (st.st_mode & 0777) == 0644;
    umask(old);
    static char why[48];
    snprintf(why, sizeof(why), "mode %o, expected 644", (unsigned)(st.st_mode & 0777));
    report("umask_applied", ok, why);
}

/* --- Sockets are per-process fds; SCM_RIGHTS passes open files (W-14). */
static void test_sockets(void)
{
    char buf[32] = {0};
    int sv[2] = {-1, -1};
    int filefd = open("/tmp/audit_sockfile", O_CREAT | O_RDWR | O_TRUNC, 0644);
    int ok = socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0 &&
             sv[0] >= 3 && sv[1] >= 3 && sv[0] != sv[1] &&
             filefd >= 0 && filefd != sv[0] && filefd != sv[1];
    ok = ok && write(sv[0], "ping", 4) == 4 && read(sv[1], buf, sizeof(buf)) == 4 &&
         memcmp(buf, "ping", 4) == 0;
    ok = ok && send(sv[1], "pong", 4, 0) == 4 && recv(sv[0], buf, sizeof(buf), 0) == 4 &&
         memcmp(buf, "pong", 4) == 0;
    report("socketpair_fds_read_write", ok, "socketpair fds or data path wrong");

    /* A process that does not hold the fd cannot reach the socket. */
    pid_t pid = fork();
    if (pid == 0) {
        int fails = 0;
        close(sv[0]);
        close(sv[1]);
        if (send(sv[0], "x", 1, 0) >= 0)
            fails |= 1;
        else if (errno != EBADF)
            fails |= 2;
        struct pollfd p = { .fd = sv[1], .events = POLLIN };
        if (poll(&p, 1, 0) != 1 || !(p.revents & POLLNVAL))
            fails |= 4;
        _exit(fails);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    int code = WIFEXITED(status) ? WEXITSTATUS(status) : 255;
    static char why[64];
    snprintf(why, sizeof(why), "child exit %d (bitmask)", code);
    report("socket_unreachable_without_fd", code == 0, why);
    ok = write(sv[0], "a", 1) == 1 && read(sv[1], buf, 1) == 1 && buf[0] == 'a';
    report("socket_survives_child_close", ok, "parent's socket broken by child close");

    /* SCM_RIGHTS: the receiver gets its own new fd for the sender's file. */
    ok = write(filefd, "scm-payload", 11) == 11 && lseek(filefd, 0, SEEK_SET) == 0;
    char one = 'f';
    struct iovec iov = { .iov_base = &one, .iov_len = 1 };
    union { char b[CMSG_SPACE(sizeof(int))]; struct cmsghdr align; } cs, cr;
    memset(&cs, 0, sizeof(cs));
    memset(&cr, 0, sizeof(cr));
    struct msghdr m = { .msg_iov = &iov, .msg_iovlen = 1,
                        .msg_control = cs.b, .msg_controllen = sizeof(cs.b) };
    struct cmsghdr *c = CMSG_FIRSTHDR(&m);
    c->cmsg_level = SOL_SOCKET;
    c->cmsg_type = SCM_RIGHTS;
    c->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(c), &filefd, sizeof(int));
    ok = ok && sendmsg(sv[0], &m, 0) == 1;

    char got = 0;
    struct iovec riov = { .iov_base = &got, .iov_len = 1 };
    struct msghdr r = { .msg_iov = &riov, .msg_iovlen = 1,
                        .msg_control = cr.b, .msg_controllen = sizeof(cr.b) };
    ok = ok && recvmsg(sv[1], &r, 0) == 1 && got == 'f';
    int newfd = -1;
    struct cmsghdr *rc = CMSG_FIRSTHDR(&r);
    if (rc && rc->cmsg_level == SOL_SOCKET && rc->cmsg_type == SCM_RIGHTS)
        memcpy(&newfd, CMSG_DATA(rc), sizeof(int));
    memset(buf, 0, sizeof(buf));
    ok = ok && newfd >= 0 && newfd != filefd && read(newfd, buf, 11) == 11 &&
         memcmp(buf, "scm-payload", 11) == 0;
    static char why_scm[64];
    snprintf(why_scm, sizeof(why_scm), "newfd=%d filefd=%d read '%.11s'", newfd, filefd, buf);
    report("scm_rights_passes_open_file", ok, why_scm);

    if (newfd >= 0)
        close(newfd);
    close(filefd);
    close(sv[0]);
    close(sv[1]);
}

/* --- Syscalls that wait for the clock (W-13). ------------------------- */
static long elapsed_ms(const struct timespec *a, const struct timespec *b)
{
    return (b->tv_sec - a->tv_sec) * 1000 + (b->tv_nsec - a->tv_nsec) / 1000000;
}

static void test_timed_waits(void)
{
    struct timespec t0, t1, req = { 0, 100 * 1000000 };
    clock_gettime(CLOCK_MONOTONIC, &t0);
    int ok = nanosleep(&req, NULL) == 0;
    clock_gettime(CLOCK_MONOTONIC, &t1);
    long slept = elapsed_ms(&t0, &t1);
    static char why[48];
    snprintf(why, sizeof(why), "slept %ld ms for 100", slept);
    report("nanosleep_waits", ok && slept >= 90 && slept < 2000, why);

    int p[2];
    ok = pipe(p) == 0;
    struct pollfd pf = { .fd = p[0], .events = POLLIN };
    clock_gettime(CLOCK_MONOTONIC, &t0);
    int n = ok ? poll(&pf, 1, 150) : -1;
    clock_gettime(CLOCK_MONOTONIC, &t1);
    long waited = elapsed_ms(&t0, &t1);
    static char why_poll[64];
    snprintf(why_poll, sizeof(why_poll), "poll returned %d after %ld ms (150)", n, waited);
    report("poll_times_out", n == 0 && waited >= 140 && waited < 2000, why_poll);
    if (ok) {
        close(p[0]);
        close(p[1]);
    }
}

/* --- User address-space limit (mm::user_layout). ----------------------- */
static void test_map_fixed_limits(void)
{
    int flags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED;
    /* Highest legitimate page: allowed. */
    void *top = mmap((void *)0x7FFFFFFFE000UL, 4096, PROT_READ | PROT_WRITE, flags, -1, 0);
    int ok_top = top == (void *)0x7FFFFFFFE000UL;
    if (ok_top) {
        *(volatile int *)top = 42;
        ok_top = *(volatile int *)top == 42;
        munmap(top, 4096);
    }
    /* Reserved top canonical page (SYSRET / Ryzen) and the kernel half:
     * rejected. */
    void *guard = mmap((void *)0x7FFFFFFFF000UL, 4096, PROT_READ, flags, -1, 0);
    void *kern = mmap((void *)0xFFFF800000000000UL, 4096, PROT_READ, flags, -1, 0);
    /* A range that starts below the limit but runs past it. */
    void *span = mmap((void *)0x7FFFFFFFE000UL, 8192, PROT_READ, flags, -1, 0);
    static char why[96];
    snprintf(why, sizeof(why), "top=%p guard=%p kernel=%p span=%p", top, guard, kern, span);
    report("map_fixed_user_limits",
           ok_top && guard == MAP_FAILED && kern == MAP_FAILED && span == MAP_FAILED, why);
}

/* --- Memory protection (N-132 to N-137). -------------------------------
 * Each probe runs in a child that must die on its access; reaching _exit(0)
 * means the access was allowed. */
static int last_status;

static int child_faults(int which)
{
    pid_t pid = fork();
    if (pid == 0) {
        volatile unsigned char *p;
        switch (which) {
        case 0: /* write to a PROT_READ mapping */
            p = mmap(0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (p == MAP_FAILED)
                _exit(2);
            p[0] = 1;
            break;
        case 1: /* read a PROT_NONE mapping */
            p = mmap(0, 4096, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (p == MAP_FAILED)
                _exit(2);
            (void)p[0];
            break;
        case 2: { /* execute from a PROT_READ|PROT_WRITE mapping */
            p = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (p == MAP_FAILED)
                _exit(2);
            p[0] = 0xC3; /* ret */
            ((void (*)(void))(unsigned long)p)();
            break;
        }
        case 3: { /* execute from the brk heap */
            unsigned char *h = sbrk(4096);
            if (h == (void *)-1)
                _exit(2);
            h[0] = 0xC3;
            ((void (*)(void))(unsigned long)h)();
            break;
        }
        case 4: /* read after mprotect(PROT_NONE) */
            p = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (p == MAP_FAILED || mprotect((void *)p, 4096, PROT_NONE) != 0)
                _exit(2);
            (void)p[0];
            break;
        }
        _exit(0);
    }
    int status = 0;
    if (pid < 0 || waitpid(pid, &status, 0) != pid)
        return 0;
    last_status = status;
    /* Killed by SIGSEGV, reported as such (N-99). */
    return WIFSIGNALED(status) && WTERMSIG(status) == SIGSEGV;
}

static void test_memory_protection(void)
{
    static const char *names[] = {
        "mmap_prot_read_blocks_write", "mmap_prot_none_blocks_read",
        "mmap_rw_not_executable", "brk_heap_not_executable",
        "mprotect_none_revokes_access",
    };
    for (int i = 0; i < 5; i++) {
        int ok = child_faults(i);
        static char why[64];
        snprintf(why, sizeof(why), "access was allowed (wait status 0x%x)", last_status);
        report(names[i], ok, why);
    }

    /* mprotect round trip keeps the contents; partly unmapped -> ENOMEM. */
    int *q = mmap(0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    int ok = q != MAP_FAILED;
    if (ok) {
        q[0] = 1234;
        ok = mprotect(q, 4096, PROT_NONE) == 0 && mprotect(q, 4096, PROT_READ | PROT_WRITE) == 0 &&
             q[0] == 1234;
        munmap((char *)q + 4096, 4096);
        errno = 0;
        ok = ok && mprotect(q, 8192, PROT_READ) != 0 && errno == ENOMEM;
        munmap(q, 4096);
    }
    report("mprotect_restore_and_enomem", ok, "contents lost or no ENOMEM for a hole");

    /* Absurd sizes fail cleanly instead of halting the kernel. */
    void *huge = mmap(0, 1UL << 46, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    void *huge2 = mmap(0, (size_t)-4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    void *brk_huge = sbrk((intptr_t)1 << 45);
    void *after = mmap(0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    static char why[96];
    snprintf(why, sizeof(why), "huge=%p huge2=%p brk=%p after=%p", huge, huge2, brk_huge, after);
    report("mmap_brk_huge_lengths_fail_cleanly",
           huge == MAP_FAILED && huge2 == MAP_FAILED && brk_huge == (void *)-1 &&
               after != MAP_FAILED,
           why);
}

/* --- fork inherits credentials and cwd; sigprocmask in place (N-93, N-97). */
static void test_fork_inheritance(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        if (chdir("/tmp") != 0 || setgid(1000) != 0 || setuid(1000) != 0)
            _exit(2);
        pid_t g = fork();
        if (g == 0) {
            char cwd[64];
            int ok = getuid() == 1000 && getgid() == 1000 && getcwd(cwd, sizeof(cwd)) &&
                     strcmp(cwd, "/tmp") == 0;
            _exit(ok ? 0 : 1);
        }
        int st = 0;
        if (g < 0 || waitpid(g, &st, 0) != g)
            _exit(3);
        _exit(WIFEXITED(st) ? WEXITSTATUS(st) : 4);
    }
    int status = 0;
    int ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status) &&
             WEXITSTATUS(status) == 0;
    static char why[48];
    snprintf(why, sizeof(why), "child status 0x%x", status);
    report("fork_inherits_uid_gid_cwd", ok, why);

    sigset_t s, cur, saved;
    sigprocmask(SIG_SETMASK, NULL, &saved);
    sigemptyset(&s);
    sigaddset(&s, SIGUSR1);
    /* Same buffer for set and oldset: the new mask must still be applied. */
    int r = sigprocmask(SIG_BLOCK, &s, &s);
    sigprocmask(SIG_SETMASK, NULL, &cur);
    int applied = r == 0 && sigismember(&cur, SIGUSR1) == 1 && sigismember(&s, SIGUSR1) == 0;
    sigprocmask(SIG_SETMASK, &saved, NULL);
    report("sigprocmask_same_buffer", applied, "mask not applied or oldset wrong");
}

/* --- open(2) flags (N-116) and kill(2) (N-92, N-99). -------------------- */
static void test_open_flags(void)
{
    unlink("/tmp/of_x");
    int fd = open("/tmp/of_x", O_CREAT | O_EXCL | O_RDWR, 0644);
    int created = fd >= 0 && write(fd, "0123456789", 10) == 10;
    if (fd >= 0)
        close(fd);
    errno = 0;
    int ex = open("/tmp/of_x", O_CREAT | O_EXCL | O_RDWR, 0644);
    int ex_errno = errno;
    /* openat with O_TRUNC truncates (it used to skip it). */
    int tfd = openat(AT_FDCWD, "/tmp/of_x", O_WRONLY | O_TRUNC);
    struct stat stt;
    int truncated = tfd >= 0 && fstat(tfd, &stt) == 0 && stt.st_size == 0;
    if (tfd >= 0)
        close(tfd);
    errno = 0;
    int nd = open("/tmp/of_x", O_RDONLY | O_DIRECTORY);
    int nd_errno = errno;
    symlink("/tmp/of_x", "/tmp/of_link");
    errno = 0;
    int nf = open("/tmp/of_link", O_RDONLY | O_NOFOLLOW);
    int nf_errno = errno;
    errno = 0;
    int dw = open("/tmp", O_WRONLY);
    int dw_errno = errno;
    static char why[128];
    snprintf(why, sizeof(why), "created=%d excl=%d/%d trunc=%d dir=%d/%d nofollow=%d/%d dirw=%d/%d",
             created, ex, ex_errno, truncated, nd, nd_errno, nf, nf_errno, dw, dw_errno);
    report("open_excl_trunc_directory_nofollow",
           created && ex == -1 && ex_errno == EEXIST && truncated && nd == -1 &&
               nd_errno == ENOTDIR && nf == -1 && nf_errno == ELOOP && dw == -1 &&
               dw_errno == EISDIR,
           why);
}

static void test_kill(void)
{
    /* A non-root process may not signal a root one (EPERM). Signal 0
     * checks permission without delivering anything. */
    pid_t pid = fork();
    if (pid == 0) {
        pid_t parent = getppid();
        if (setuid(1000) != 0)
            _exit(2);
        errno = 0;
        int r = kill(parent, 0);
        _exit(r == -1 && errno == EPERM ? 0 : 1);
    }
    int st = 0;
    int eperm = pid > 0 && waitpid(pid, &st, 0) == pid && WIFEXITED(st) && WEXITSTATUS(st) == 0;

    /* kill reaches the real process, and the parent sees the signal. */
    pid = fork();
    if (pid == 0) {
        kill(getpid(), SIGTERM);
        _exit(0);
    }
    int st2 = 0;
    int self = pid > 0 && waitpid(pid, &st2, 0) == pid && WIFSIGNALED(st2) &&
               WTERMSIG(st2) == SIGTERM;

    /* No such process: ESRCH; nothing left to wait for: ECHILD. */
    errno = 0;
    int esrch = kill(99999, 0) == -1 && errno == ESRCH;
    /* Checked in a fresh child, which has no children of its own. */
    pid = fork();
    if (pid == 0) {
        errno = 0;
        int r = waitpid(-1, NULL, 0);
        _exit(r == -1 && errno == ECHILD ? 0 : 1);
    }
    int st3 = 0;
    int echild = pid > 0 && waitpid(pid, &st3, 0) == pid && WIFEXITED(st3) &&
                 WEXITSTATUS(st3) == 0;
    static char why[96];
    snprintf(why, sizeof(why), "eperm=%d(0x%x) self=%d(0x%x) esrch=%d echild=%d", eperm, st, self,
             st2, esrch, echild);
    report("kill_permissions_esrch_echild", eperm && self && esrch && echild, why);
}

/* --- sigaction keeps the whole action and fits its struct (N-95, N-105). - */
static void on_usr1(int sig) { (void)sig; }

static void test_sigaction(void)
{
    /* A guard after `old`: the kernel used to write 32 bytes into the
     * 24-byte struct and overwrite whatever followed it. */
    struct {
        struct sigaction old;
        unsigned long guard;
    } box;
    box.guard = 0xA5A5A5A5A5A5A5A5UL;
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = on_usr1;
    sa.sa_flags = SA_RESTART;
    sigemptyset(&sa.sa_mask);
    sigaddset(&sa.sa_mask, SIGUSR2);
    int r1 = sigaction(SIGUSR1, &sa, NULL);
    int r2 = sigaction(SIGUSR1, NULL, &box.old);
    int kept = r1 == 0 && r2 == 0 && box.old.sa_handler == on_usr1 &&
               (box.old.sa_flags & SA_RESTART) && sigismember(&box.old.sa_mask, SIGUSR2) == 1;
    int guard_ok = box.guard == 0xA5A5A5A5A5A5A5A5UL;
    signal(SIGUSR1, SIG_DFL);
    /* SIGKILL may be queried; changing it is EINVAL. */
    struct sigaction q;
    int query = sigaction(SIGKILL, NULL, &q);
    errno = 0;
    int set = sigaction(SIGKILL, &sa, NULL);
    int set_errno = errno;
    static char why[96];
    snprintf(why, sizeof(why), "kept=%d guard=%d query=%d set=%d/%d", kept, guard_ok, query, set,
             set_errno);
    report("sigaction_roundtrip", kept && guard_ok && query == 0 && set == -1 && set_errno == EINVAL,
           why);
}

/* --- Directory rename (FS-PERF-03): the node moves, ".." follows. ----- */
static void test_rename_directory(void)
{
    char buf[16] = {0};
    struct stat parent, dotdot;
    mkdir("/tmp/rd", 0755);
    mkdir("/tmp/rd/a", 0755);
    mkdir("/tmp/rd/b", 0755);
    int ok = write_file("/tmp/rd/a/inner", "inside", 0644) == 0 &&
             rename("/tmp/rd/a", "/tmp/rd/b/moved") == 0 &&
             access("/tmp/rd/a", F_OK) != 0 &&
             read_file("/tmp/rd/b/moved/inner", buf, sizeof(buf)) == 0 &&
             strcmp(buf, "inside") == 0 &&
             stat("/tmp/rd/b", &parent) == 0 &&
             stat("/tmp/rd/b/moved/..", &dotdot) == 0 &&
             parent.st_ino == dotdot.st_ino;
    report("rename_directory_moves_subtree", ok, "contents or .. wrong after move");

    errno = 0;
    int r = rename("/tmp/rd/b", "/tmp/rd/b/moved/sub");
    int e1 = errno;
    static char why[80];
    /* The same, spelled so a string-prefix check misses it. */
    symlink("/tmp/rd/b", "/tmp/rd/blink");
    errno = 0;
    int r2 = rename("/tmp/rd/b", "/tmp/rd/./b/moved/sub");
    int e2 = errno;
    errno = 0;
    int r3 = rename("/tmp/rd/b", "/tmp/rd/blink/moved/sub");
    int e3 = errno;
    errno = 0;
    int r4 = rename("/tmp/rd/b", "/tmp/rd/b/moved/../moved/sub");
    int e4 = errno;
    snprintf(why, sizeof(why), "%d/%d %d/%d %d/%d %d/%d", r, e1, r2, e2, r3, e3, r4, e4);
    report("rename_into_own_subtree_einval",
           r != 0 && e1 == EINVAL && r2 != 0 && e2 == EINVAL && r3 != 0 && e3 == EINVAL && r4 != 0 && e4 == EINVAL,
           why);
}

/* --- A closed standard descriptor is closed (review of the v0.26.0 stack):
 * write(2) after close(2) must fail with EBADF, not reach the serial
 * console. Run in a child so the test's own stderr is untouched. ---------- */
static void test_closed_stdio(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        close(2);
        errno = 0;
        ssize_t w = write(2, "x", 1);
        _exit(w == -1 && errno == EBADF ? 0 : 1);
    }
    int status = 0;
    int ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status) &&
             WEXITSTATUS(status) == 0;
    report("closed_stderr_is_ebadf", ok, "write(2) after close(2) did not fail with EBADF");
}

/* --- AF_UNIX bind/connect by path (review of the v0.26.0 stack): bind
 * read the name from the start of the sockaddr, family bytes included, so
 * nothing could connect to the path that was bound. ----------------------- */
static void test_unix_bind_connect(void)
{
    struct sockaddr_un sa;
    memset(&sa, 0, sizeof(sa));
    sa.sun_family = AF_UNIX;
    strcpy(sa.sun_path, "/tmp/audit_bind.sock");
    unlink(sa.sun_path);
    char buf[8] = {0};
    int srv = socket(AF_UNIX, SOCK_STREAM, 0);
    int cli = socket(AF_UNIX, SOCK_STREAM, 0);
    int ok = srv >= 0 && cli >= 0 &&
             bind(srv, (struct sockaddr *)&sa, sizeof(sa)) == 0 && listen(srv, 1) == 0 &&
             connect(cli, (struct sockaddr *)&sa, sizeof(sa)) == 0;
    int conn = ok ? accept(srv, NULL, NULL) : -1;
    ok = ok && conn >= 0 && write(cli, "hi", 2) == 2 && read(conn, buf, sizeof(buf)) == 2 &&
         memcmp(buf, "hi", 2) == 0;
    /* A path nobody bound is not reachable. */
    struct sockaddr_un other = sa;
    strcpy(other.sun_path, "/tmp/audit_nobody.sock");
    int cli2 = socket(AF_UNIX, SOCK_STREAM, 0);
    ok = ok && cli2 >= 0 && connect(cli2, (struct sockaddr *)&other, sizeof(other)) != 0;
    if (conn >= 0)
        close(conn);
    if (cli2 >= 0)
        close(cli2);
    if (srv >= 0)
        close(srv);
    if (cli >= 0)
        close(cli);
    report("unix_bind_connect_by_path", ok, "bind/listen/connect/accept by path failed");
}

/* --- Socket API details fixed in review of the v0.26.0 stack (PR #10). -- */
static void test_socket_api_details(void)
{
    int sv[2] = {-1, -1};
    int type = 0;
    socklen_t len = sizeof(type);
    char buf[8];

    /* socketpair honours the type: datagrams keep their boundaries. */
    int ok = socketpair(AF_UNIX, SOCK_DGRAM, 0, sv) == 0 && write(sv[0], "ab", 2) == 2 &&
             write(sv[0], "cd", 2) == 2 && read(sv[1], buf, sizeof(buf)) == 2;
    ok = ok && getsockopt(sv[0], SOL_SOCKET, SO_TYPE, &type, &len) == 0 && type == SOCK_DGRAM &&
         len == sizeof(type);
    if (sv[0] >= 0) { close(sv[0]); close(sv[1]); }
    report("socketpair_dgram_type_and_so_type", ok, "datagram boundaries or SO_TYPE wrong");

    /* An unknown option is ENOPROTOOPT, not a fake success. */
    ok = socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == 0;
    len = sizeof(type);
    errno = 0;
    ok = ok && getsockopt(sv[0], SOL_SOCKET, 0x7fff, &type, &len) == -1 && errno == ENOPROTOOPT;
    report("getsockopt_unknown_is_enoprotoopt", ok, "unknown option did not fail with ENOPROTOOPT");

    /* An empty stream send queues nothing, so the peer does not see EOF. */
    ok = sv[0] >= 0 && send(sv[0], "", 0, 0) == 0 && write(sv[0], "z", 1) == 1 &&
         read(sv[1], buf, sizeof(buf)) == 1 && buf[0] == 'z';
    report("empty_stream_send_is_not_eof", ok, "an empty send reached the peer as EOF");

    /* SCM_RIGHTS with fd -1 is EBADF. */
    {
        char data = 'x';
        struct iovec iov = {&data, 1};
        union { struct cmsghdr h; char b[CMSG_SPACE(sizeof(int))]; } ctl;
        memset(&ctl, 0, sizeof(ctl));
        struct msghdr m;
        memset(&m, 0, sizeof(m));
        m.msg_iov = &iov;
        m.msg_iovlen = 1;
        m.msg_control = ctl.b;
        m.msg_controllen = sizeof(ctl.b);
        struct cmsghdr *c = CMSG_FIRSTHDR(&m);
        c->cmsg_level = SOL_SOCKET;
        c->cmsg_type = SCM_RIGHTS;
        c->cmsg_len = CMSG_LEN(sizeof(int));
        int bad = -1;
        memcpy(CMSG_DATA(c), &bad, sizeof(int));
        errno = 0;
        ok = sv[0] >= 0 && sendmsg(sv[0], &m, 0) == -1 && errno == EBADF;
        report("sendmsg_bad_fd_is_ebadf", ok, "SCM_RIGHTS with fd -1 did not fail with EBADF");
    }
    if (sv[0] >= 0) { close(sv[0]); close(sv[1]); }

    /* accept reports an AF_UNIX address and honours addrlen. */
    struct sockaddr_un sa, peer;
    memset(&sa, 0, sizeof(sa));
    sa.sun_family = AF_UNIX;
    strcpy(sa.sun_path, "/tmp/audit_accept.sock");
    unlink(sa.sun_path);
    int srv = socket(AF_UNIX, SOCK_STREAM, 0), cli = socket(AF_UNIX, SOCK_STREAM, 0);
    socklen_t plen = sizeof(peer);
    memset(&peer, 0x55, sizeof(peer));
    ok = srv >= 0 && cli >= 0 && bind(srv, (struct sockaddr *)&sa, sizeof(sa)) == 0 &&
         listen(srv, 1) == 0 && connect(cli, (struct sockaddr *)&sa, sizeof(sa)) == 0;
    int setup_ok = ok;
    errno = 0;
    int conn = ok ? accept(srv, (struct sockaddr *)&peer, &plen) : -1;
    int accept_errno = errno;
    ok = ok && conn >= 0 && plen >= sizeof(sa_family_t) && peer.sun_family == AF_UNIX;
    if (conn >= 0) close(conn);
    if (srv >= 0) close(srv);
    if (cli >= 0) close(cli);
    static char why_acc[96];
    snprintf(why_acc, sizeof(why_acc), "setup %d, accept %d (errno %d), addrlen %u, family %u",
             setup_ok, conn, accept_errno, (unsigned)plen, (unsigned)peer.sun_family);
    report("accept_reports_unix_address", ok, why_acc);
}

/* --- Search permission on directories (FS-SEC-02, review of the v0.26.0
 * stack): a non-root user cannot reach a file through a 0700 directory it
 * does not own, even when the file itself is world-readable. ---------- */
static void test_dir_search_permission(void)
{
    mkdir("/tmp/audit_private", 0700); /* root-owned */
    write_file("/tmp/audit_private/open", "visible", 0644);
    chmod("/tmp/audit_private", 0700);
    pid_t pid = fork();
    if (pid == 0) {
        struct stat st = {0};
        if (setuid(1000) != 0)
            _exit(100);
        errno = 0;
        int fd = open("/tmp/audit_private/open", O_RDONLY);
        int open_errno = errno; /* before stat() can overwrite it */
        int stat_ok = stat("/tmp/audit_private/open", &st) == 0;
        if (fd < 0 && open_errno == EACCES && !stat_ok)
            _exit(0);
        /* Encode what happened: bit 7 = open succeeded, bit 6 = stat
         * succeeded, low bits = open's errno. */
        _exit((fd >= 0 ? 0x80 : 0) | (stat_ok ? 0x40 : 0) | (open_errno & 0x3f));
    }
    int status = 0;
    int ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status) &&
             WEXITSTATUS(status) == 0;
    static char why_dir[96];
    snprintf(why_dir, sizeof(why_dir),
             "uid 1000 under a 0700 dir: open %s (errno %d), stat %s",
             (WEXITSTATUS(status) & 0x80) ? "succeeded" : "failed", WEXITSTATUS(status) & 0x3f,
             (WEXITSTATUS(status) & 0x40) ? "succeeded" : "failed");
    report("dir_search_permission_enforced", ok, why_dir);
}

/* --- The kernel ignores the caller's direction flag (review of the v0.26.0
 * stack): a system call made with DF set must not copy below the user
 * buffer it validated. Before the fix the kernel ran with the user's DF; an
 * RFLAGS probe in the syscall handler read 0x446 for this call. This check
 * is a smoke test only: it also passed on the unfixed kernel, where DF was
 * clear again by the time the wait path copied the status out (what clears
 * it was not determined). ------------------------------------------------ */
#define DF_GUARD 64
#define VERIDIAN_SYS_WAIT 14 /* Syscall::ProcessWait */

static long raw_syscall4_df(long nr, long a, long b, long c, long d)
{
#if defined(__x86_64__)
    register long rax __asm__("rax") = nr;
    register long rdi __asm__("rdi") = a;
    register long rsi __asm__("rsi") = b;
    register long rdx __asm__("rdx") = c;
    register long r10 __asm__("r10") = d;
    __asm__ volatile("std\n\tsyscall\n\tcld"
                     : "+r"(rax)
                     : "r"(rdi), "r"(rsi), "r"(rdx), "r"(r10)
                     : "rcx", "r11", "memory", "cc");
    return rax;
#else
    (void)nr; (void)a; (void)b; (void)c; (void)d;
    return -1;
#endif
}

static void test_direction_flag(void)
{
#if defined(__x86_64__)
    /* The native wait call stores the status through the kernel's
     * validated user copy (`rep movsb`). With DF set that copy ran
     * downwards from the status address. Raw numbers are the native ABI;
     * only 0-7 are translated as Linux numbers (N-33). */
    unsigned char area[DF_GUARD + sizeof(int) + DF_GUARD];
    memset(area, 0x55, sizeof(area));
    /* Let the child become a zombie first, so the wait does not block: a
     * blocked wait resumes through a context switch that restores RFLAGS
     * and would hide the user's DF. */
    pid_t pid = fork();
    if (pid == 0)
        _exit(42);
    struct timespec nap = {0, 200 * 1000 * 1000};
    nanosleep(&nap, NULL);
    long r = raw_syscall4_df(VERIDIAN_SYS_WAIT, pid, (long)(area + DF_GUARD), 0, 0);
    int status;
    memcpy(&status, area + DF_GUARD, sizeof(status));
    int guards = 1;
    for (int i = 0; i < DF_GUARD; i++)
        guards &= area[i] == 0x55 && area[DF_GUARD + sizeof(int) + i] == 0x55;
    int ok = r == pid && guards && WIFEXITED(status) && WEXITSTATUS(status) == 42;
    static char why[96];
    snprintf(why, sizeof(why), "wait with DF set: ret %ld, guards %s, status 0x%x", r,
             guards ? "intact" : "OVERWRITTEN", (unsigned)status);
    report("syscall_ignores_user_direction_flag", ok, why);
#endif
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    test_fd_numbering();
    test_scanf();
    /* User threads need a ring-3 entry path in the scheduler that does not
     * exist yet (see KNOWN-LIMITATIONS); run those checks only on request. */
    if (argc > 1 && strcmp(argv[1], "threads") == 0) {
        test_allocator();
        test_threads();
    }
    test_fork_exit();
    test_cow();
    test_huge_pages();
    test_rename();
    test_nonroot_permissions();
    test_sticky_dir();
    test_umask();
    test_sockets();
    test_timed_waits();
    test_map_fixed_limits();
    test_memory_protection();
    test_fork_inheritance();
    test_open_flags();
    test_kill();
    test_sigaction();
    test_rename_directory();
    test_closed_stdio();
    test_unix_bind_connect();
    test_socket_api_details();
    test_dir_search_permission();
    test_direction_flag();
    printf("AUDIT-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
