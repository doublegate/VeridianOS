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
        int stat_ok = stat("/tmp/audit_private/open", &st) == 0;
        _exit(fd < 0 && errno == EACCES && !stat_ok ? 0 : 1);
    }
    int status = 0;
    int ok = pid > 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status) &&
             WEXITSTATUS(status) == 0;
    report("dir_search_permission_enforced", ok,
           "a file under a 0700 directory was reachable as uid 1000");
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
    test_rename();
    test_nonroot_permissions();
    test_sticky_dir();
    test_umask();
    test_sockets();
    test_timed_waits();
    test_map_fixed_limits();
    test_rename_directory();
    test_closed_stdio();
    test_unix_bind_connect();
    test_dir_search_permission();
    test_direction_flag();
    printf("AUDIT-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
