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
#include <grp.h>
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

    /* PROT_NONE -> read-write in fresh page tables (musl's thread stacks:
     * mmap PROT_NONE, mprotect all but the guard). The tables created for
     * the kernel-only pages lacked the user bit, which mprotect did not
     * add, so the writes faulted. In a child, since that is the failure. */
    pid_t gp = fork();
    if (gp == 0) {
        const size_t len = 4UL << 20;
        volatile char *m = mmap(0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (m == MAP_FAILED || mprotect((char *)m + 8192, len - 8192, PROT_READ | PROT_WRITE) != 0)
            _exit(2);
        for (size_t off = 8192; off < len; off += 1UL << 20)
            m[off] = 1;
        m[len - 1] = 1;
        _exit(0);
    }
    int gst = 0;
    int gok = gp > 0 && waitpid(gp, &gst, 0) == gp && WIFEXITED(gst) && WEXITSTATUS(gst) == 0;
    static char gwhy[48];
    snprintf(gwhy, sizeof(gwhy), "status=0x%x", gst);
    report("mprotect_none_to_rw_fresh_tables", gok, gwhy);

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
#include <veridian/sysno.h> /* system call numbers (ADR 0009) */

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
    /* wait4 stores the status through the kernel's validated user copy
     * (`rep movsb`). With DF set that copy ran downwards from the status
     * address. */
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
    long r = raw_syscall4_df(SYS_wait4, pid, (long)(area + DF_GUARD), 0, 0);
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

/* --- Exceptions in user code (N-168, N-175). ------------------------------
 * Each trap kills only the child that caused it, with the signal Linux
 * sends; the parent and the kernel carry on. */
static int child_traps(int which)
{
    pid_t pid = fork();
    if (pid == 0) {
        switch (which) {
        case 0: /* #UD -> SIGILL */
            __asm__ volatile("ud2");
            break;
        case 1: /* #DE -> SIGFPE (asm: GCC rewrites 1 / x as a branch) */
            __asm__ volatile("xor %%edx, %%edx\n\tmov $1, %%eax\n\txor %%ecx, %%ecx\n\tdiv %%ecx"
                             ::: "eax", "ecx", "edx");
            break;
        case 2: /* #BP from int3 (a DPL-3 gate) -> SIGTRAP */
            __asm__ volatile("int3");
            break;
        case 3: /* privileged instruction, #GP -> SIGSEGV */
            __asm__ volatile("hlt");
            break;
        case 4: /* int to a kernel-only vector, #GP -> SIGSEGV */
            __asm__ volatile("int $0x30");
            break;
        }
        _exit(0);
    }
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid)
        return -1;
    last_status = st;
    return WIFSIGNALED(st) ? WTERMSIG(st) : 0;
}

static void test_traps(void)
{
    static const struct {
        const char *name;
        int sig;
    } cases[] = {
        {"trap_ud2_sigill", SIGILL},       {"trap_divide_sigfpe", SIGFPE},
        {"trap_int3_sigtrap", SIGTRAP},    {"trap_hlt_sigsegv", SIGSEGV},
        {"trap_kernel_vector_sigsegv", SIGSEGV},
    };
    for (int i = 0; i < 5; i++) {
        int sig = child_traps(i);
        static char why[64];
        snprintf(why, sizeof(why), "got signal %d (status 0x%x)", sig, last_status);
        report(cases[i].name, sig == cases[i].sig, why);
    }
}

/* --- Timer preemption of user code (ADR 0006 stage D3). -------------------
 * A child spins in user mode without system calls. The parent must still
 * run (its sleep ends), and SIGKILL must stop the spinning child. Without
 * preemption the parent never runs again and the suite hangs. */
static void test_preemption(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        for (;;)
            __asm__ volatile("" ::: "memory");
    }
    struct timespec ts = {0, 50 * 1000 * 1000};
    int slept = pid > 0 && nanosleep(&ts, 0) == 0;
    int killed = pid > 0 && kill(pid, SIGKILL) == 0;
    int st = 0;
    int reaped = pid > 0 && waitpid(pid, &st, 0) == pid;
    static char why[80];
    snprintf(why, sizeof(why), "slept=%d killed=%d reaped=%d status=0x%x", slept, killed, reaped, st);
    report("preempt_spinning_child_then_kill", slept && killed && reaped && WIFSIGNALED(st) &&
                                                    WTERMSIG(st) == SIGKILL,
           why);
}

/* --- A fatal signal ends a process blocked in a system call. ------------
 * The child sleeps for ten minutes; SIGKILL must end it at once (it used to
 * be acted on only when the sleep returned; security review of stage D2). */
static void test_kill_sleeping(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        struct timespec ts = {600, 0};
        nanosleep(&ts, 0);
        _exit(0);
    }
    struct timespec ts = {0, 20 * 1000 * 1000};
    nanosleep(&ts, 0);
    int killed = pid > 0 && kill(pid, SIGKILL) == 0;
    int st = 0;
    int reaped = pid > 0 && waitpid(pid, &st, 0) == pid;
    static char why[64];
    snprintf(why, sizeof(why), "killed=%d reaped=%d status=0x%x", killed, reaped, st);
    report("kill_ends_blocked_sleep", killed && reaped && WIFSIGNALED(st) && WTERMSIG(st) == SIGKILL,
           why);
}

/* --- A blocking pipe read waits for the writer (N-119). ----------------
 * The child writes after a delay while the parent is already reading; the
 * read must block and return the byte, not fail with EAGAIN. Then EOF
 * once the writer has gone. */
static void test_blocking_pipe(void)
{
    int fds[2];
    if (pipe(fds) != 0) {
        report("pipe_read_blocks_for_writer", 0, "pipe failed");
        return;
    }
    pid_t pid = fork();
    if (pid == 0) {
        close(fds[0]);
        struct timespec ts = {0, 30 * 1000 * 1000};
        nanosleep(&ts, 0);
        _exit(write(fds[1], "x", 1) == 1 ? 0 : 1);
    }
    close(fds[1]);
    char c = 0;
    errno = 0;
    ssize_t n = read(fds[0], &c, 1);
    int e1 = errno;
    ssize_t eof = read(fds[0], &c, 1);
    close(fds[0]);
    int st = 0;
    waitpid(pid, &st, 0);
    static char why[64];
    snprintf(why, sizeof(why), "n=%d errno=%d c=%d eof=%d", (int)n, e1, c, (int)eof);
    report("pipe_read_blocks_for_writer", n == 1 && c == 'x' && eof == 0, why);
}

/* --- Signal handlers (sprint D3; N-96, N-98, N-109, N-113). ------------- */
static volatile int d3_usr1_count, d3_usr2_count;
static void d3_on_usr1(int sig) { (void)sig; d3_usr1_count++; }
static void d3_on_usr2(int sig)
{
    /* Clobber the FP registers: the interrupted code must not notice. */
    volatile double x = 1.0;
    for (int i = 0; i < 64; i++)
        x = x * 1.000001 + 0.5;
    (void)sig;
    d3_usr2_count++;
}
static void d3_on_segv(int sig) { (void)sig; _exit(42); }

static int d3_install(int sig, void (*fn)(int), int flags)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = fn;
    sa.sa_flags = flags;
    sigemptyset(&sa.sa_mask);
    return sigaction(sig, &sa, 0);
}

static void test_signal_handlers(void)
{
    /* A handler runs on the way back from the kill that raised it. */
    d3_usr1_count = 0;
    d3_install(SIGUSR1, d3_on_usr1, 0);
    kill(getpid(), SIGUSR1);
    report("signal_handler_runs", d3_usr1_count == 1, "handler did not run");

    /* A blocked signal waits; unblocking delivers it (Linux set layout). */
    sigset_t set, old;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    sigprocmask(SIG_BLOCK, &set, &old);
    kill(getpid(), SIGUSR1);
    int held = d3_usr1_count == 1;
    sigprocmask(SIG_SETMASK, &old, 0);
    report("signal_mask_blocks_then_delivers", held && d3_usr1_count == 2, "mask not honoured");

    /* Asynchronous handlers interrupt FP work without corrupting it. */
    d3_usr2_count = 0;
    d3_install(SIGUSR2, d3_on_usr2, 0);
    pid_t parent = getpid();
    pid_t pid = fork();
    if (pid == 0) {
        for (int i = 0; i < 20; i++) {
            struct timespec ts = {0, 2 * 1000 * 1000};
            nanosleep(&ts, 0);
            kill(parent, SIGUSR2);
        }
        _exit(0);
    }
    double a = 0.0, expect = 0.0;
    for (int i = 1; i <= 2000000; i++)
        a += 1.0 / (double)i;
    for (int i = 1; i <= 2000000; i++)
        expect += 1.0 / (double)i;
    int st = 0;
    /* The sender's signals interrupt the wait (no SA_RESTART): retry, so
     * none is still in flight when the next test starts. */
    while (waitpid(pid, &st, 0) < 0 && errno == EINTR)
        ;
    static char why[80];
    snprintf(why, sizeof(why), "handlers=%d fp_equal=%d", d3_usr2_count, a == expect);
    report("signal_handlers_preserve_fp_state", a == expect && d3_usr2_count > 0, why);

    /* SA_RESTART restarts an interrupted wait; without it, EINTR. */
    for (int restart = 1; restart >= 0; restart--) {
        d3_install(SIGUSR1, d3_on_usr1, restart ? SA_RESTART : 0);
        pid_t slow = fork();
        if (slow == 0) {
            struct timespec ts = {0, 120 * 1000 * 1000};
            nanosleep(&ts, 0);
            _exit(5);
        }
        pid_t poker = fork();
        if (poker == 0) {
            struct timespec ts = {0, 30 * 1000 * 1000};
            nanosleep(&ts, 0);
            kill(parent, SIGUSR1);
            _exit(0);
        }
        int s2 = 0;
        errno = 0;
        pid_t r = waitpid(slow, &s2, 0);
        int e = errno;
        if (r != slow)
            while (waitpid(slow, &s2, 0) < 0 && errno == EINTR)
                ;
        while (waitpid(poker, 0, 0) < 0 && errno == EINTR)
            ;
        static char why2[64];
        snprintf(why2, sizeof(why2), "r=%d errno=%d", (int)r, e);
        if (restart)
            report("signal_sa_restart_restarts_wait", r == slow && WEXITSTATUS(s2) == 5, why2);
        else
            report("signal_without_restart_eintr", r == -1 && e == EINTR, why2);
    }

    /* SIGCHLD's default action is to ignore it: it must not end a sleep
     * (N-98). */
    signal(SIGCHLD, SIG_DFL);
    pid = fork();
    if (pid == 0)
        _exit(0);
    struct timespec ts = {0, 40 * 1000 * 1000};
    int slept = nanosleep(&ts, 0);
    waitpid(pid, 0, 0);
    report("sigchld_default_does_not_interrupt", slept == 0, "nanosleep was interrupted");

    /* A fault reaches an installed SIGSEGV handler. */
    pid = fork();
    if (pid == 0) {
        d3_install(SIGSEGV, d3_on_segv, 0);
        /* An unmapped address, chosen at run time so the store is a real
         * fault rather than something the compiler reasons about. */
        volatile uintptr_t unmapped = 8;
        *(volatile int *)unmapped = 1;
        _exit(1);
    }
    st = 0;
    waitpid(pid, &st, 0);
    static char why3[48];
    snprintf(why3, sizeof(why3), "status=0x%x", st);
    report("sigsegv_handler_runs", WIFEXITED(st) && WEXITSTATUS(st) == 42, why3);

    signal(SIGUSR1, SIG_DFL);
    signal(SIGUSR2, SIG_DFL);
}

/* --- Relative paths follow the caller's own working directory (N-115). ---
 * They resolved against the kernel shell's directory, whatever the program
 * had chdir'd to. A child's chdir must not move the parent either. */
static void test_relative_paths(void)
{
    char saved[256];
    if (!getcwd(saved, sizeof(saved)))
        strcpy(saved, "/");
    unlink("/tmp/relprobe");
    unlink("/relprobe_child");
    int ok = chdir("/tmp") == 0;
    int fd = open("relprobe", O_CREAT | O_WRONLY, 0644);
    if (fd >= 0)
        close(fd);
    struct stat st;
    int in_tmp = stat("/tmp/relprobe", &st) == 0;

    pid_t pid = fork();
    if (pid == 0) {
        if (chdir("/") != 0)
            _exit(2);
        int cfd = open("relprobe_child", O_CREAT | O_WRONLY, 0644);
        _exit(cfd >= 0 ? 0 : 1);
    }
    int cst = 0;
    waitpid(pid, &cst, 0);
    int child_in_root = stat("/relprobe_child", &st) == 0;
    int parent_unmoved = stat("relprobe", &st) == 0; /* still /tmp */
    unlink("/tmp/relprobe");
    unlink("/relprobe_child");
    if (chdir(saved) != 0)
        ok = 0;
    static char why[80];
    snprintf(why, sizeof(why), "in_tmp=%d child_in_root=%d parent_unmoved=%d", in_tmp,
             child_in_root, parent_unmoved);
    report("relative_paths_follow_own_cwd", ok && in_tmp && child_in_root && parent_unmoved, why);
}

/* N-101: exec needs an executable regular file; a refused exec leaves the
 * caller running. Root needs at least one execute bit, as on Linux. */
static void test_exec_permission(void)
{
    const char *path = "/tmp/noexec_probe";
    unlink(path);
    int fd = open(path, O_CREAT | O_WRONLY | O_TRUNC, 0644);
    if (fd >= 0) {
        const char body[] = "#!/bin/sh\nexit 0\n";
        if (write(fd, body, sizeof(body) - 1) < 0) {
            /* checked through the exec results below */
        }
        close(fd);
    }
    chmod(path, 0644);
    char *args[] = {(char *)path, NULL};
    char *env[] = {NULL};

    /* Root, no execute bit anywhere: EACCES, and we are still here. */
    errno = 0;
    int r = execve(path, args, env);
    int root_eacces = r == -1 && errno == EACCES;

    /* A directory is not executable either. */
    char *dargs[] = {"/tmp", NULL};
    errno = 0;
    r = execve("/tmp", dargs, env);
    int dir_eacces = r == -1 && errno == EACCES;

    /* Executable but not a program: ENOEXEC (N-127), caller unharmed. */
    const char *junk = "/tmp/noexec_junk";
    int jfd = open(junk, O_CREAT | O_WRONLY | O_TRUNC, 0755);
    if (jfd >= 0) {
        if (write(jfd, "junk\n", 5) < 0) {
            /* checked through the exec result */
        }
        close(jfd);
    }
    chmod(junk, 0755);
    char *jargs[] = {(char *)junk, NULL};
    errno = 0;
    r = execve(junk, jargs, env);
    int noexec = r == -1 && errno == ENOEXEC;
    unlink(junk);

    /* Non-root, mode 0700 owned by root: EACCES in the child. */
    chmod(path, 0700);
    pid_t pid = fork();
    if (pid == 0) {
        if (setuid(1000) != 0)
            _exit(3);
        execve(path, args, env);
        _exit(errno == EACCES ? 0 : 1);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    int user_eacces = WIFEXITED(st) && WEXITSTATUS(st) == 0;

    /* Control: an executable program still runs (busybox false -> 1). */
    pid = fork();
    if (pid == 0) {
        char *fargs[] = {"false", NULL};
        execve("/bin/false", fargs, env);
        _exit(42);
    }
    st = 0;
    waitpid(pid, &st, 0);
    int control = WIFEXITED(st) && WEXITSTATUS(st) == 1;
    unlink(path);

    static char why[112];
    snprintf(why, sizeof(why), "root=%d dir=%d noexec=%d user=%d control=%d(st=%#x)",
             root_eacces, dir_eacces, noexec, user_eacces, control, st);
    report("exec_requires_execute_permission",
           root_eacces && dir_eacces && noexec && user_eacces && control, why);
}

/* N-140: MAP_SHARED anonymous memory stays shared with a forked child,
 * also after an mprotect round trip; MAP_PRIVATE stays private. */
static void test_shared_anon_fork(void)
{
    volatile int *sh = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    volatile int *pr =
        mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (sh == MAP_FAILED || pr == MAP_FAILED) {
        report("map_shared_survives_fork", 0, "mmap failed");
        return;
    }
    sh[0] = 1;
    sh[1] = 1;
    pr[0] = 1;
    pid_t pid = fork();
    if (pid == 0) {
        sh[0] = 42;
        pr[0] = 42;
        _exit(0);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    int seen_child = sh[0] == 42;
    int private_kept = pr[0] == 1;

    /* Read-only and back: still the shared page, not a private copy. */
    int prot_ok = mprotect((void *)sh, 4096, PROT_READ) == 0 &&
                  mprotect((void *)sh, 4096, PROT_READ | PROT_WRITE) == 0;
    pid = fork();
    if (pid == 0) {
        sh[1] = 7;
        _exit(0);
    }
    waitpid(pid, &st, 0);
    sh[0] = 5; /* parent write after the child is gone */
    int after_prot = sh[1] == 7 && sh[0] == 5;
    munmap((void *)sh, 4096);
    munmap((void *)pr, 4096);

    static char why[96];
    snprintf(why, sizeof(why), "child_write=%d private=%d prot=%d after_prot=%d", seen_child,
             private_kept, prot_ok, after_prot);
    report("map_shared_survives_fork", seen_child && private_kept && prot_ok && after_prot, why);
}

/* N-141: MAP_FIXED replaces what is mapped; munmap spans mappings and
 * holes; munmap of an unmapped range succeeds. */
static void test_map_fixed_replace(void)
{
    char *base = mmap(NULL, 4 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (base == MAP_FAILED) {
        report("map_fixed_replaces_and_munmap_spans", 0, "mmap failed");
        return;
    }
    memset(base, 'a', 4 * 4096);
    /* Replace the middle two pages: fresh zero pages, neighbours kept. */
    char *mid = mmap(base + 4096, 2 * 4096, PROT_READ | PROT_WRITE,
                     MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    int replaced = mid == base + 4096 && mid[0] == 0 && mid[4096] == 0 && base[0] == 'a' &&
                   base[3 * 4096] == 'a';
    /* Punch a hole, then unmap across both sides of it at once. */
    int hole = munmap(base + 4096, 4096) == 0;
    int span = munmap(base, 4 * 4096) == 0;
    /* The range is free again: MAP_FIXED there works without replacing. */
    char *again = mmap(base, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
                       -1, 0);
    int reuse = again == base && again[0] == 0;
    int empty = munmap(base, 4096) == 0 && munmap(base, 4 * 4096) == 0;

    /* A fixed mapping just ahead of the kernel's mmap cursor must not make
     * the next kernel-chosen mapping fail. */
    char *p = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *ahead = p == MAP_FAILED ? MAP_FAILED
                                  : mmap(p + 2 * 4096, 4096, PROT_READ,
                                         MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    char *next = mmap(NULL, 4 * 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    int cursor = p != MAP_FAILED && ahead != MAP_FAILED && next != MAP_FAILED;
    if (p != MAP_FAILED)
        munmap(p, 4096);
    if (ahead != MAP_FAILED)
        munmap(ahead, 4096);
    if (next != MAP_FAILED)
        munmap(next, 4 * 4096);

    static char why[112];
    snprintf(why, sizeof(why), "replaced=%d hole=%d span=%d reuse=%d empty=%d cursor=%d",
             replaced, hole, span, reuse, empty, cursor);
    report("map_fixed_replaces_and_munmap_spans",
           replaced && hole && span && reuse && empty && cursor, why);
}

/* D3 job control and N-99: SIGSTOP stops every thread of the child and
 * waitpid(WUNTRACED) reports it once; SIGCONT resumes it and
 * waitpid(WCONTINUED) reports that; SIGKILL ends a stopped child; a
 * process-group wait finds a child by its group. */
static void test_stop_continue(void)
{
    volatile unsigned long *ctr =
        mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (ctr == MAP_FAILED) {
        report("stop_continue_and_group_wait", 0, "mmap failed");
        return;
    }
    ctr[0] = 0;
    pid_t pid = fork();
    if (pid == 0) {
        if (setpgid(0, 0) != 0)
            _exit(3);
        for (;;)
            ctr[0]++;
    }
    struct timespec ms20 = {0, 20 * 1000 * 1000};
    /* Let it run, so a stop is observable. */
    for (int i = 0; i < 100 && ctr[0] == 0; i++)
        nanosleep(&ms20, NULL);
    int st = 0;

    kill(pid, SIGSTOP);
    pid_t r = waitpid(pid, &st, WUNTRACED);
    int stopped = r == pid && WIFSTOPPED(st) && WSTOPSIG(st) == SIGSTOP;
    unsigned long a = ctr[0];
    nanosleep(&ms20, NULL);
    nanosleep(&ms20, NULL);
    int frozen = ctr[0] == a;
    /* Reported once: a second WNOHANG wait sees nothing new. */
    int once = waitpid(pid, &st, WUNTRACED | WNOHANG) == 0;

    kill(pid, SIGCONT);
    r = waitpid(pid, &st, WCONTINUED);
    int continued = r == pid && WIFCONTINUED(st);
    unsigned long b = ctr[0];
    for (int i = 0; i < 100 && ctr[0] == b; i++)
        nanosleep(&ms20, NULL);
    int resumed = ctr[0] != b;

    /* Stopped again, then killed: SIGKILL ends it; the wait goes by the
     * child's own process group. */
    kill(pid, SIGTSTP);
    r = waitpid(-pid, &st, WUNTRACED);
    int tstp = r == pid && WIFSTOPPED(st) && WSTOPSIG(st) == SIGTSTP;
    kill(pid, SIGKILL);
    r = waitpid(-pid, &st, 0);
    int killed = r == pid && WIFSIGNALED(st) && WTERMSIG(st) == SIGKILL;
    munmap((void *)ctr, 4096);

    static char why[112];
    snprintf(why, sizeof(why), "stop=%d frozen=%d once=%d cont=%d resumed=%d tstp=%d kill=%d",
             stopped, frozen, once, continued, resumed, tstp, killed);
    report("stop_continue_and_group_wait",
           stopped && frozen && once && continued && resumed && tstp && killed, why);
}

/* --- Credentials, chroot, fchdir, utimensat (N-248 to N-250). ---------
 * Each check runs in a child, so changed IDs, root or cwd never leak into
 * the rest of the suite. A child reports failures as a bitmask exit code. */
static int child_result(pid_t pid)
{
    int status = 0;
    if (waitpid(pid, &status, 0) != pid)
        return 254;
    return WIFEXITED(status) ? WEXITSTATUS(status) : 255;
}

static void report_child(const char *name, pid_t pid)
{
    int code = child_result(pid);
    static char why[64];
    snprintf(why, sizeof(why), "child exit code %d (bitmask of failures)", code);
    report(name, code == 0, why);
}

static void test_credentials_and_paths(void)
{
    /* A setuid-root style process: real 1000, effective and saved 0. It
     * can drop the effective ID and take it back through the saved one;
     * once all three are 1000 it cannot. seteuid used to report success
     * without changing anything. */
    pid_t pid = fork();
    if (pid == 0) {
        int f = 0;
        uid_t r, e, s;
        if (setresuid(1000, 0, 0) != 0) f |= 1;
        if (seteuid(1000) != 0 || geteuid() != 1000) f |= 2;
        if (seteuid(0) != 0 || geteuid() != 0) f |= 4;
        if (getresuid(&r, &e, &s) != 0 || r != 1000 || e != 0 || s != 0) f |= 8;
        if (setresuid(1000, 1000, 1000) != 0) f |= 16;
        if (seteuid(0) == 0 || errno != EPERM) f |= 32;
        _exit(f);
    }
    report_child("saved_uid_round_trip_and_final_drop", pid);

    /* Supplementary groups grant group access; only root may set them. */
    write_file("/tmp/audit_grp", "g", 0640);
    chown("/tmp/audit_grp", 0, 20);
    pid = fork();
    if (pid == 0) {
        int f = 0;
        gid_t list[4] = {0};
        gid_t want[2] = {20, 30};
        if (setgroups(2, want) != 0) f |= 1;
        if (getgroups(0, NULL) != 2) f |= 2;
        if (getgroups(4, list) != 2 || list[0] != 20 || list[1] != 30) f |= 4;
        if (setgid(100) != 0 || setuid(1000) != 0) f |= 8;
        int fd = open("/tmp/audit_grp", O_RDONLY);
        if (fd < 0) f |= 16;
        else close(fd);
        if (setgroups(0, NULL) == 0 || errno != EPERM) f |= 32;
        _exit(f);
    }
    report_child("supplementary_groups_grant_access", pid);
    pid = fork();
    if (pid == 0) {
        setgroups(0, NULL);
        if (setgid(100) != 0 || setuid(1000) != 0)
            _exit(100);
        _exit(open("/tmp/audit_grp", O_RDONLY) >= 0 ? 1 : 0);
    }
    report_child("no_group_no_group_access", pid);

    /* access(2): owner bits for the owner (it checked only "other"). */
    write_file("/tmp/audit_owned", "o", 0700);
    chown("/tmp/audit_owned", 1000, 1000);
    write_file("/tmp/audit_noexec", "n", 0644);
    pid = fork();
    if (pid == 0) {
        int f = 0;
        if (access("/tmp/audit_noexec", X_OK) == 0) f |= 1; /* root needs an x bit */
        if (setuid(1000) != 0) _exit(100);
        if (access("/tmp/audit_owned", R_OK | W_OK | X_OK) != 0) f |= 2;
        _exit(f);
    }
    report_child("access_uses_owner_bits", pid);

    /* chroot: lookups stay inside, including "..", absolute paths and
     * absolute symlink targets; only root may call it. */
    mkdir("/tmp/audit_jail", 0755);
    mkdir("/tmp/audit_jail/etc", 0755);
    write_file("/tmp/audit_jail/etc/hello", "jail", 0644);
    write_file("/tmp/audit_outside", "out", 0644);
    unlink("/tmp/audit_jail/link");
    symlink("/etc", "/tmp/audit_jail/link");
    pid = fork();
    if (pid == 0) {
        int f = 0;
        char buf[64];
        if (chroot("/tmp/audit_jail") != 0) _exit(100);
        if (chdir("/") != 0) f |= 1;
        int fd = open("/etc/hello", O_RDONLY);
        if (fd < 0) f |= 2;
        else close(fd);
        if (open("/../../tmp/audit_outside", O_RDONLY) >= 0) f |= 4;
        if (open("/link/hello", O_RDONLY) < 0) f |= 8;
        if (!getcwd(buf, sizeof(buf)) || strcmp(buf, "/") != 0) f |= 16;
        if (chdir("..") != 0 || !getcwd(buf, sizeof(buf)) || strcmp(buf, "/") != 0) f |= 32;
        if (setuid(1000) != 0 || chroot("/") == 0 || errno != EPERM) f |= 64;
        _exit(f);
    }
    report_child("chroot_confines_lookups", pid);

    /* fchdir: to an open directory; ENOTDIR for a file, EBADF for a bad fd. */
    pid = fork();
    if (pid == 0) {
        int f = 0;
        char buf[64];
        int dfd = open("/tmp", O_RDONLY | O_DIRECTORY);
        int ffd = open("/tmp/audit_outside", O_RDONLY);
        if (dfd < 0 || ffd < 0) _exit(100);
        if (chdir("/") != 0 || fchdir(dfd) != 0) f |= 1;
        if (!getcwd(buf, sizeof(buf)) || strcmp(buf, "/tmp") != 0) f |= 2;
        if (fchdir(ffd) == 0 || errno != ENOTDIR) f |= 4;
        if (fchdir(999) == 0 || errno != EBADF) f |= 8;
        _exit(f);
    }
    report_child("fchdir_changes_directory", pid);

    /* A directory renamed after it was opened, with another created at its
     * old name: fchdir must never land in the newcomer (the fd's recorded
     * path no longer names its directory; this fails closed with ENOENT). */
    pid = fork();
    if (pid == 0) {
        char buf[64];
        rmdir("/tmp/audit_swap");
        rmdir("/tmp/audit_swap_moved");
        if (mkdir("/tmp/audit_swap", 0755) != 0) _exit(100);
        int dfd = open("/tmp/audit_swap", O_RDONLY | O_DIRECTORY);
        if (dfd < 0 || rename("/tmp/audit_swap", "/tmp/audit_swap_moved") != 0 ||
            mkdir("/tmp/audit_swap", 0755) != 0)
            _exit(101);
        if (fchdir(dfd) == 0 && getcwd(buf, sizeof(buf)) && strcmp(buf, "/tmp/audit_swap") == 0)
            _exit(1);
        _exit(0);
    }
    report_child("fchdir_never_enters_a_replacement", pid);

    /* utimensat / futimens: explicit times, UTIME_OMIT, and EPERM for a
     * non-owner setting explicit times (they did nothing). */
    write_file("/tmp/audit_times", "t", 0666);
    chmod("/tmp/audit_times", 0666); /* past the umask: writable by others */
    struct timespec ts[2] = {{1000, 0}, {2000, 0}};
    struct stat st = {0};
    int ok = utimensat(AT_FDCWD, "/tmp/audit_times", ts, 0) == 0 &&
             stat("/tmp/audit_times", &st) == 0 && st.st_atime == 1000 && st.st_mtime == 2000;
    int fd = open("/tmp/audit_times", O_RDONLY);
    struct timespec omit[2] = {{0, UTIME_OMIT}, {3000, 0}};
    ok = ok && fd >= 0 && futimens(fd, omit) == 0 && stat("/tmp/audit_times", &st) == 0 &&
         st.st_atime == 1000 && st.st_mtime == 3000;
    if (fd >= 0)
        close(fd);
    static char why_t[80];
    snprintf(why_t, sizeof(why_t), "atime %ld mtime %ld", (long)st.st_atime, (long)st.st_mtime);
    report("utimensat_sets_times", ok, why_t);
    pid = fork();
    if (pid == 0) {
        int f = 0;
        if (setuid(1001) != 0) _exit(100);
        if (utimensat(AT_FDCWD, "/tmp/audit_times", ts, 0) == 0 || errno != EPERM) f |= 1;
        if (utimensat(AT_FDCWD, "/tmp/audit_times", NULL, 0) != 0) f |= 2; /* writable */
        _exit(f);
    }
    report_child("utimensat_owner_rules", pid);
}

/* --- pread/pwrite, truncate/ftruncate and mkdir checks (N-190..N-192):
 * the access mode was ignored (pwrite worked through O_RDONLY), streams
 * accepted an offset, lengths were unsigned, truncate and mkdir checked no
 * permission. Error order as Linux's ksys_pread64/do_sys_ftruncate. ---- */
static void test_positioned_io_and_truncate(void)
{
    int fails = 0;
    char c = 'x';
    write_file("/tmp/audit_pio", "abcdef", 0644);
    int rd = open("/tmp/audit_pio", O_RDONLY);
    errno = 0;
    if (pwrite(rd, &c, 1, 0) != -1 || errno != EBADF)
        fails |= 1;                             /* not open for writing */
    errno = 0;
    if (ftruncate(rd, 0) != -1 || errno != EINVAL)
        fails |= 2;                             /* not open for writing */
    errno = 0;
    if (pread(rd, &c, 1, -1) != -1 || errno != EINVAL)
        fails |= 4;                             /* negative offset */
    close(rd);
    int wr = open("/tmp/audit_pio", O_WRONLY);
    errno = 0;
    if (pread(wr, &c, 1, 0) != -1 || errno != EBADF)
        fails |= 8;                             /* not open for reading */
    errno = 0;
    if (ftruncate(wr, -1) != -1 || errno != EINVAL)
        fails |= 16;                            /* negative length */
    if (ftruncate(wr, 3) != 0)
        fails |= 32;
    close(wr);
    int p[2];
    if (pipe(p) == 0) {
        errno = 0;
        if (pread(p[0], &c, 1, 0) != -1 || errno != ESPIPE)
            fails |= 64;                        /* a stream has no offset */
        errno = 0;
        if (pwrite(p[1], &c, 1, 0) != -1 || errno != ESPIPE)
            fails |= 128;
        errno = 0;
        if (lseek(p[0], 0, SEEK_SET) != -1 || errno != ESPIPE)
            fails |= 2048;
        struct stat pst = {0};
        if (fstat(p[0], &pst) != 0 || !S_ISFIFO(pst.st_mode))
            fails |= 4096;                      /* a pipe is a FIFO */
        close(p[0]);
        close(p[1]);
    } else {
        fails |= 64;
    }
    errno = 0;
    if (truncate("/tmp", 0) != -1 || errno != EISDIR)
        fails |= 256;
    errno = 0;
    if (truncate("/tmp/audit_pio", -1) != -1 || errno != EINVAL)
        fails |= 512;
    struct stat st = {0};
    if (stat("/tmp/audit_pio", &st) != 0 || st.st_size != 3)
        fails |= 1024;
    /* read/write: the wrong access mode and an unopened fd are EBADF, a
     * directory read EISDIR (N-197). */
    rd = open("/tmp/audit_pio", O_RDONLY);
    wr = open("/tmp/audit_pio", O_WRONLY);
    errno = 0;
    if (write(rd, &c, 1) != -1 || errno != EBADF)
        fails |= 8192;
    errno = 0;
    if (read(wr, &c, 1) != -1 || errno != EBADF)
        fails |= 16384;
    close(rd);
    close(wr);
    errno = 0;
    if (write(999, &c, 1) != -1 || errno != EBADF)
        fails |= 32768;
    int dfd = open("/tmp", O_RDONLY | O_DIRECTORY);
    errno = 0;
    if (dfd < 0 || read(dfd, &c, 1) != -1 || errno != EISDIR)
        fails |= 65536;
    if (dfd >= 0)
        close(dfd);
    static char why[64];
    snprintf(why, sizeof(why), "bitmask of failures %d", fails);
    report("pread_pwrite_ftruncate_checks", fails == 0, why);

    /* As a user: no write permission on the file or the directory. */
    write_file("/tmp/audit_ro", "data", 0644);  /* root-owned */
    mkdir("/tmp/audit_rodir", 0755);
    rmdir("/tmp/audit_rodir/sub");
    pid_t pid = fork();
    if (pid == 0) {
        int child_fails = 0;
        if (setuid(1000) != 0)
            _exit(100);
        errno = 0;
        if (truncate("/tmp/audit_ro", 0) != -1 || errno != EACCES)
            child_fails |= 1;
        errno = 0;
        if (mkdir("/tmp/audit_rodir/sub", 0755) != -1 || errno != EACCES)
            child_fails |= 2;
        _exit(child_fails);
    }
    report_child("nonroot_truncate_mkdir_denied", pid);
    int kept = stat("/tmp/audit_ro", &st) == 0 && st.st_size == 4 &&
               stat("/tmp/audit_rodir/sub", &st) != 0;
    report("nonroot_truncate_mkdir_left_no_trace", kept, "file truncated or directory created");
}

/* --- execve with an empty or NULL envp passes no environment (N-211):
 * the kernel substituted the caller's, so `env -i` leaked it. A child
 * re-executes this program with AUDIT_ENV_MARK in its environment; that
 * image (exec_env_helper) then runs sh with an empty or NULL envp, and sh
 * exits 0 only if the mark did not come through. ---------------------- */
static void exec_env_helper(const char *mode)
{
    char *args[] = {"sh", "-c", "test -z \"$AUDIT_ENV_MARK\"", NULL};
    char *empty[] = {NULL};
    execve("/bin/sh", args, strcmp(mode, "exec-env-null") == 0 ? NULL : empty);
    _exit(100);
}

static void exec_without_env(const char *mode, const char *name)
{
    pid_t pid = fork();
    if (pid == 0) {
        char *args[] = {"audit_runtime_test", (char *)mode, NULL};
        char *env[] = {"AUDIT_ENV_MARK=leaked", NULL};
        execve("/bin/audit_runtime_test", args, env);
        _exit(101);
    }
    report_child(name, pid);
}

static void test_exec_environment(void)
{
    exec_without_env("exec-env-empty", "execve_empty_envp_is_empty");
    exec_without_env("exec-env-null", "execve_null_envp_is_empty");
}

/* --- memfd_create (N-229): it returned an eventfd id that was never an
 * fd. Now a sizeable, readable, mappable, sealable anonymous file. ---- */
static void test_memfd(void)
{
    int fails = 0;
    int fd = memfd_create("audit", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    struct stat st = {0};
    char buf[8] = {0};
    if (fd < 0) {
        report("memfd_create_file_and_seals", 0, "memfd_create failed");
        return;
    }
    if (ftruncate(fd, 4096) != 0)
        fails |= 1;
    if (pwrite(fd, "seal", 4, 100) != 4 || pread(fd, buf, 4, 100) != 4 || memcmp(buf, "seal", 4) != 0)
        fails |= 2;
    if (fstat(fd, &st) != 0 || !S_ISREG(st.st_mode) || st.st_size != 4096)
        fails |= 4;
    if (!(fcntl(fd, F_GETFD) & FD_CLOEXEC))
        fails |= 8;
    char *map = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0);
    if (map == MAP_FAILED || memcmp(map + 100, "seal", 4) != 0)
        fails |= 16;
    if (map != MAP_FAILED)
        munmap(map, 4096);
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_SHRINK | F_SEAL_SEAL) != 0)
        fails |= 32;
    errno = 0;
    if (ftruncate(fd, 0) != -1 || errno != EPERM)
        fails |= 64;                            /* shrinking is sealed */
    if (ftruncate(fd, 8192) != 0)
        fails |= 128;                           /* growing is not */
    if (fcntl(fd, F_GET_SEALS) != (F_SEAL_SHRINK | F_SEAL_SEAL))
        fails |= 256;
    errno = 0;
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_GROW) != -1 || errno != EPERM)
        fails |= 512;                           /* F_SEAL_SEAL */
    close(fd);

    /* Without MFD_ALLOW_SEALING no seal can be added. */
    fd = memfd_create("plain", 0);
    errno = 0;
    if (fd < 0 || fcntl(fd, F_GET_SEALS) != F_SEAL_SEAL ||
        fcntl(fd, F_ADD_SEALS, F_SEAL_WRITE) != -1 || errno != EPERM)
        fails |= 1024;
    if (fd >= 0 && (fcntl(fd, F_GETFD) & FD_CLOEXEC))
        fails |= 2048;
    if (fd >= 0)
        close(fd);
    /* A regular file cannot be sealed; unknown flags are refused. */
    write_file("/tmp/audit_seal", "x", 0644);
    fd = open("/tmp/audit_seal", O_RDWR);
    errno = 0;
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_WRITE) != -1 || errno != EINVAL)
        fails |= 4096;
    close(fd);
    errno = 0;
    if (memfd_create("bad", 0x100) != -1 || errno != EINVAL)
        fails |= 8192;
    errno = 0;
    if (memfd_create("huge", MFD_HUGETLB) != -1 || errno != EINVAL)
        fails |= 16384;
    static char why[64];
    snprintf(why, sizeof(why), "bitmask of failures %d", fails);
    report("memfd_create_file_and_seals", fails == 0, why);
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc > 1 && strncmp(argv[1], "exec-env-", 9) == 0)
        exec_env_helper(argv[1]);
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
    test_traps();
    test_preemption();
    test_kill_sleeping();
    test_blocking_pipe();
    test_signal_handlers();
    test_relative_paths();
    test_exec_permission();
    test_shared_anon_fork();
    test_map_fixed_replace();
    test_stop_continue();
    test_credentials_and_paths();
    test_positioned_io_and_truncate();
    test_exec_environment();
    test_memfd();
    printf("AUDIT-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
