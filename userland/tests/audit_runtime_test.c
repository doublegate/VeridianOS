/*
 * audit_runtime_test -- in-guest checks for the v0.26 audit fixes that the
 * 33 boot tests never exercise: process and thread exit/reaping, the libc
 * allocator under threads, scanf field widths, fd numbering, rename and
 * permission enforcement for a non-root user, sticky directories.
 *
 * Run as root from a BusyBox shell: /bin/audit_runtime_test [threads]
 * Prints one "PASS <name>" or "FAIL <name>: <why>" line per check and a
 * final "AUDIT-RUNTIME: <passed>/<total>" summary.
 */

#include <fcntl.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
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
    for (int i = 0; i < 4; i++)
        if (pthread_create(&t[i], NULL, alloc_worker, (void *)(long)(i + 1)) != 0)
            ok = 0;
    for (int i = 0; i < 4; i++) {
        void *r = NULL;
        pthread_join(t[i], &r);
        if (r)
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
    struct stat st;
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

    struct stat st;
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
    struct stat st;
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
    struct stat st;
    int ok = write_file("/tmp/audit_umask", "u", 0666) == 0 &&
             stat("/tmp/audit_umask", &st) == 0 && (st.st_mode & 0777) == 0644;
    umask(old);
    static char why[48];
    snprintf(why, sizeof(why), "mode %o, expected 644", (unsigned)(st.st_mode & 0777));
    report("umask_applied", ok, why);
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
    printf("AUDIT-RUNTIME: %d/%d\n", passed, total);
    return passed == total ? 0 : 1;
}
