/*
 * VeridianOS libc -- posix_stubs3.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Stub implementations for POSIX functions needed by BusyBox.
 * These provide minimal functionality -- just enough to compile
 * and run BusyBox applets that don't heavily depend on the
 * underlying feature.
 *
 * Functions already defined elsewhere in libc are NOT duplicated here:
 *   - nanosleep, usleep, sleep (time.c / unistd.c)
 *   - gethostname (unistd.c)
 *   - alarm (posix_stubs2.c)
 *   - getline, getdelim (stdio.c)
 *   - fcntl, rename, readlink, mkdir, rmdir (syscall.c)
 *   - setuid, setgid, geteuid, getegid, setpgid, etc. (syscall.c)
 *   - link, symlink, fchmod, truncate, ftruncate (syscall.c)
 *   - chown, fchown, lchown, mknod (syscall.c)
 *   - tcsetpgrp, tcgetpgrp (termios.c)
 *   - sigprocmask, sigaction (signal stubs in posix_stubs2.c / syscall.c)
 */

#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>
#include <sys/types.h>
#include <unistd.h>
#include <limits.h>
#include <grp.h>
#include <veridian/syscall.h>

/* ========================================================================= */
/* libgen.h -- basename() and dirname()                                      */
/* ========================================================================= */

static char dot[] = ".";

char *basename(char *path)
{
    char *p;

    if (path == NULL || *path == '\0')
        return dot;

    /* Strip trailing slashes. */
    p = path + strlen(path) - 1;
    while (p > path && *p == '/')
        *p-- = '\0';

    /* Find last component. */
    p = strrchr(path, '/');
    return p ? p + 1 : path;
}

char *dirname(char *path)
{
    char *p;

    if (path == NULL || *path == '\0')
        return dot;

    /* Strip trailing slashes. */
    p = path + strlen(path) - 1;
    while (p > path && *p == '/')
        *p-- = '\0';

    /* Find last slash. */
    p = strrchr(path, '/');
    if (p == NULL)
        return dot;

    /* Strip trailing slashes from directory component. */
    while (p > path && *p == '/')
        p--;
    *(p + 1) = '\0';

    return path;
}

/* ========================================================================= */
/* sys/statvfs.h -- filesystem stats                                         */
/* ========================================================================= */

#include <sys/statvfs.h>

int statvfs(const char *path, struct statvfs *buf)
{
    (void)path;
    if (buf == NULL) {
        errno = EFAULT;
        return -1;
    }
    memset(buf, 0, sizeof(*buf));
    buf->f_bsize  = 4096;
    buf->f_frsize = 4096;
    buf->f_blocks = 1048576;    /* ~4GB */
    buf->f_bfree  = 524288;    /* ~2GB free */
    buf->f_bavail = 524288;
    buf->f_files  = 65536;
    buf->f_ffree  = 32768;
    buf->f_favail = 32768;
    buf->f_namemax = 255;
    return 0;
}

int fstatvfs(int fd, struct statvfs *buf)
{
    (void)fd;
    return statvfs("/", buf);
}

/* ========================================================================= */
/* syslog.h -- system logging                                                */
/* ========================================================================= */

static int syslog_mask = 0xFF;

void openlog(const char *ident, int option, int facility)
{
    (void)ident;
    (void)option;
    (void)facility;
}

void syslog(int priority, const char *format, ...)
{
    (void)priority;
    (void)format;
}

void closelog(void)
{
}

int setlogmask(int mask)
{
    int old = syslog_mask;
    if (mask != 0)
        syslog_mask = mask;
    return old;
}

/* ========================================================================= */
/* mntent.h -- mount table parsing                                           */
/* ========================================================================= */

#include <mntent.h>

FILE *setmntent(const char *filename, const char *type)
{
    (void)filename;
    (void)type;
    return NULL;
}

struct mntent *getmntent(FILE *stream)
{
    (void)stream;
    return NULL;
}

int endmntent(FILE *stream)
{
    (void)stream;
    return 1;
}

char *hasmntopt(const struct mntent *mnt, const char *opt)
{
    (void)mnt;
    (void)opt;
    return NULL;
}

/* ========================================================================= */
/* utmp.h -- user accounting                                                 */
/* ========================================================================= */

#include <utmp.h>

void setutent(void) {}
struct utmp *getutent(void) { return NULL; }
void endutent(void) {}

struct utmp *getutid(const struct utmp *ut)
{
    (void)ut;
    return NULL;
}

struct utmp *getutline(const struct utmp *ut)
{
    (void)ut;
    return NULL;
}

struct utmp *pututline(const struct utmp *ut)
{
    (void)ut;
    return NULL;
}

void utmpname(const char *file)
{
    (void)file;
}

/* ========================================================================= */
/* sys/mount.h -- mount/umount                                               */
/* ========================================================================= */

int mount(const char *source, const char *target,
          const char *filesystemtype, unsigned long mountflags,
          const void *data)
{
    (void)source;
    (void)target;
    (void)filesystemtype;
    (void)mountflags;
    (void)data;
    errno = ENOSYS;
    return -1;
}

int umount(const char *target)
{
    (void)target;
    errno = ENOSYS;
    return -1;
}

int umount2(const char *target, int flags)
{
    (void)target;
    (void)flags;
    errno = ENOSYS;
    return -1;
}

/* ========================================================================= */
/* Additional BusyBox-required stubs                                         */
/* ========================================================================= */

int clearenv(void)
{
    extern char **environ;
    if (environ)
        environ[0] = NULL;
    return 0;
}

char *strsignal(int sig)
{
    static char buf[32];
    switch (sig) {
    case 1:  return "Hangup";
    case 2:  return "Interrupt";
    case 3:  return "Quit";
    case 6:  return "Aborted";
    case 9:  return "Killed";
    case 11: return "Segmentation fault";
    case 13: return "Broken pipe";
    case 14: return "Alarm clock";
    case 15: return "Terminated";
    default:
        snprintf(buf, sizeof(buf), "Signal %d", sig);
        return buf;
    }
}

/*
 * The terminal open on fd. The native ABI has no fd-to-path query; the
 * terminal a native program sees is the console, so a tty fd is named
 * /dev/console and anything else is ENOTTY.
 */
int ttyname_r(int fd, char *buf, size_t buflen)
{
    static const char console[] = "/dev/console";

    if (!isatty(fd))
        return errno == EBADF ? EBADF : ENOTTY;
    if (buf == NULL || buflen < sizeof(console))
        return ERANGE;
    memcpy(buf, console, sizeof(console));
    return 0;
}

char *ttyname(int fd)
{
    static char name[32];
    int err = ttyname_r(fd, name, sizeof(name));

    if (err) {
        errno = err;
        return NULL;
    }
    return name;
}

int getpagesize(void)
{
    return 4096;
}

int sethostname(const char *name, size_t len)
{
    (void)name;
    (void)len;
    errno = EPERM;
    return -1;
}

int mkfifo(const char *pathname, mode_t mode)
{
    (void)pathname;
    (void)mode;
    errno = ENOSYS;
    return -1;
}

/* seteuid / setegid / setreuid / setregid / getgroups / setgroups */
/* Credentials (N-248): the kernel keeps real, effective and saved IDs and
 * the supplementary groups and enforces the Linux rules. These reported
 * success without changing anything. (uid_t)-1 leaves an ID unchanged. */
int seteuid(uid_t uid)
{
    return (int)__syscall_ret(veridian_syscall3(SYS_setresuid, -1L, (long)uid, -1L));
}

int setegid(gid_t gid)
{
    return (int)__syscall_ret(veridian_syscall3(SYS_setresgid, -1L, (long)gid, -1L));
}

int setreuid(uid_t ruid, uid_t euid)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_setreuid, (long)ruid, (long)euid));
}

int setregid(gid_t rgid, gid_t egid)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_setregid, (long)rgid, (long)egid));
}

int setresuid(uid_t ruid, uid_t euid, uid_t suid)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_setresuid, (long)ruid, (long)euid, (long)suid));
}

int setresgid(gid_t rgid, gid_t egid, gid_t sgid)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_setresgid, (long)rgid, (long)egid, (long)sgid));
}

int getresuid(uid_t *ruid, uid_t *euid, uid_t *suid)
{
    return (int)__syscall_ret(veridian_syscall3(SYS_getresuid, ruid, euid, suid));
}

int getresgid(gid_t *rgid, gid_t *egid, gid_t *sgid)
{
    return (int)__syscall_ret(veridian_syscall3(SYS_getresgid, rgid, egid, sgid));
}

int getgroups(int size, gid_t list[])
{
    return (int)__syscall_ret(veridian_syscall2(SYS_getgroups, (long)size, list));
}

int setgroups(size_t size, const gid_t *list)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_setgroups, (long)size, list));
}

/* dup3() -- dup2 with flags */
int dup3(int oldfd, int newfd, int flags)
{
    (void)flags;
    extern int dup2(int, int);
    return dup2(oldfd, newfd);
}

/* pipe2() -- pipe with flags */
int pipe2(int pipefd[2], int flags)
{
    (void)flags;
    extern int pipe(int[2]);
    return pipe(pipefd);
}

/* clock_gettime() -- already defined in time.c, not duplicated here */
#include <time.h>

/* ioctl() -- already defined in BusyBox built-in.o / syscall.c, not duplicated here */
#include <sys/ioctl.h>
#include <stdarg.h>

/* vfork() is in vfork_x86_64.S. */

/* execvp() -- already defined in unistd.c, not duplicated here */
#include <unistd.h>

int execvpe(const char *file, char *const argv[], char *const envp[])
{
    if (file == NULL) {
        errno = ENOENT;
        return -1;
    }

    if (strchr(file, '/') != NULL)
        return execve(file, argv, envp);

    const char *path = getenv("PATH");
    if (path == NULL)
        path = "/bin:/usr/bin";

    char buf[256];
    const char *p = path;
    while (*p) {
        const char *end = strchr(p, ':');
        size_t len = end ? (size_t)(end - p) : strlen(p);

        if (len + 1 + strlen(file) + 1 <= sizeof(buf)) {
            memcpy(buf, p, len);
            buf[len] = '/';
            strcpy(buf + len + 1, file);

            extern int access(const char *, int);
            if (access(buf, 1) == 0)
                return execve(buf, argv, envp);
        }

        if (end == NULL)
            break;
        p = end + 1;
    }

    errno = ENOENT;
    return -1;
}

/* waitid() -- the kernel's waitid: P_ALL, P_PID, P_PGID; WEXITED,
 * WSTOPPED, WCONTINUED, WNOHANG, WNOWAIT. */
#include <sys/wait.h>

int waitid(idtype_t idtype, id_t id, siginfo_t *infop, int options)
{
    return (int)__syscall_ret(
        veridian_syscall5(SYS_waitid, idtype, id, infop, options, 0));
}

/*
 * wait3() / wait4() -- BSD-style wait, with the child's resource usage
 * (the kernel fills it, as on Linux).
 */
#include <sys/resource.h>

pid_t wait4(pid_t pid, int *wstatus, int options, struct rusage *rusage)
{
    return (pid_t)__syscall_ret(
        veridian_syscall4(SYS_wait4, pid, wstatus, options, rusage));
}

pid_t wait3(int *wstatus, int options, struct rusage *rusage)
{
    return wait4(-1, wstatus, options, rusage);
}

/* ========================================================================= */
/* BusyBox link-time stubs (added for BusyBox cross-compilation)             */
/* ========================================================================= */

/* --- String functions ---------------------------------------------------- */

char *strchrnul(const char *s, int c)
{
    while (*s && *s != (char)c)
        s++;
    return (char *)s;
}

char *strsep(char **stringp, const char *delim)
{
    char *s, *tok;
    if ((s = *stringp) == NULL)
        return NULL;
    tok = s;
    for (;;) {
        int ch = *s++;
        const char *d = delim;
        do {
            if (*d == ch) {
                if (ch == '\0')
                    s = NULL;
                else
                    s[-1] = '\0';
                *stringp = s;
                return tok;
            }
        } while (*d++);
    }
}

char *stpcpy(char *dest, const char *src)
{
    while ((*dest = *src) != '\0') {
        dest++;
        src++;
    }
    return dest;
}

char *stpncpy(char *dest, const char *src, size_t n)
{
    size_t i;
    for (i = 0; i < n && src[i] != '\0'; i++)
        dest[i] = src[i];
    for (; i < n; i++)
        dest[i] = '\0';
    return dest + i;
}

char *strcasestr(const char *haystack, const char *needle)
{
    if (!*needle)
        return (char *)haystack;
    size_t nlen = strlen(needle);
    for (; *haystack; haystack++) {
        if (strncasecmp(haystack, needle, nlen) == 0)
            return (char *)haystack;
    }
    return NULL;
}

char *strtok_r(char *str, const char *delim, char **saveptr)
{
    char *tok;
    if (str == NULL)
        str = *saveptr;
    str += strspn(str, delim);
    if (*str == '\0') {
        *saveptr = str;
        return NULL;
    }
    tok = str;
    str = strpbrk(tok, delim);
    if (str) {
        *str = '\0';
        *saveptr = str + 1;
    } else {
        *saveptr = tok + strlen(tok);
    }
    return tok;
}

int strverscmp(const char *s1, const char *s2)
{
    /* Simplified version compare: treats runs of digits numerically */
    while (*s1 && *s2) {
        if ((*s1 >= '0' && *s1 <= '9') && (*s2 >= '0' && *s2 <= '9')) {
            unsigned long n1 = strtoul(s1, (char **)&s1, 10);
            unsigned long n2 = strtoul(s2, (char **)&s2, 10);
            if (n1 != n2)
                return (n1 < n2) ? -1 : 1;
        } else {
            if (*s1 != *s2)
                return (unsigned char)*s1 - (unsigned char)*s2;
            s1++;
            s2++;
        }
    }
    return (unsigned char)*s1 - (unsigned char)*s2;
}

/* --- Unlocked stdio (just call the locked versions) ---------------------- */

int getc_unlocked(FILE *stream)
{
    return fgetc(stream);
}

int getchar_unlocked(void)
{
    extern FILE *stdin;
    return fgetc(stdin);
}

int putc_unlocked(int c, FILE *stream)
{
    return fputc(c, stream);
}

int putchar_unlocked(int c)
{
    extern FILE *stdout;
    return fputc(c, stdout);
}

char *fgets_unlocked(char *s, int size, FILE *stream)
{
    return fgets(s, size, stream);
}

int fputs_unlocked(const char *s, FILE *stream)
{
    return fputs(s, stream);
}

int feof_unlocked(FILE *stream)
{
    return feof(stream);
}

int ferror_unlocked(FILE *stream)
{
    return ferror(stream);
}

int fileno_unlocked(FILE *stream)
{
    return fileno(stream);
}

/* --- Process I/O (popen / pclose) ---------------------------------------- */

/*
 * popen/pclose implementation using fork + exec + pipe.
 *
 * A static table maps FILE pointers to child PIDs so that pclose() can
 * waitpid() for the correct child.  Up to 16 simultaneous popen'd
 * processes are supported.
 */

#define POPEN_MAX 16

static struct {
    FILE *fp;
    pid_t pid;
} __popen_table[POPEN_MAX];

FILE *popen(const char *command, const char *type)
{
    int pipefd[2];
    int reading;

    if (command == NULL || type == NULL) {
        errno = EINVAL;
        return NULL;
    }

    if (type[0] == 'r')
        reading = 1;
    else if (type[0] == 'w')
        reading = 0;
    else {
        errno = EINVAL;
        return NULL;
    }

    if (pipe(pipefd) < 0)
        return NULL;

    pid_t pid = fork();
    if (pid < 0) {
        close(pipefd[0]);
        close(pipefd[1]);
        return NULL;
    }

    if (pid == 0) {
        /* Child process. */
        if (reading) {
            /* Parent reads from pipe: child writes stdout to pipe. */
            close(pipefd[0]);           /* Close read end in child. */
            if (pipefd[1] != STDOUT_FILENO) {
                dup2(pipefd[1], STDOUT_FILENO);
                close(pipefd[1]);
            }
        } else {
            /* Parent writes to pipe: child reads stdin from pipe. */
            close(pipefd[1]);           /* Close write end in child. */
            if (pipefd[0] != STDIN_FILENO) {
                dup2(pipefd[0], STDIN_FILENO);
                close(pipefd[0]);
            }
        }

        /* Execute the command via /bin/sh -c. */
        char *argv[4];
        argv[0] = "sh";
        argv[1] = "-c";
        argv[2] = (char *)command;
        argv[3] = NULL;
        extern char **environ;
        execve("/bin/sh", argv, environ);
        _exit(127);  /* execve failed */
    }

    /* Parent process. */
    FILE *fp;
    if (reading) {
        close(pipefd[1]);              /* Close write end in parent. */
        fp = fdopen(pipefd[0], "r");
    } else {
        close(pipefd[0]);              /* Close read end in parent. */
        fp = fdopen(pipefd[1], "w");
    }

    if (fp == NULL) {
        /* fdopen failed -- clean up. */
        close(reading ? pipefd[0] : pipefd[1]);
        waitpid(pid, NULL, 0);
        return NULL;
    }

    /* Record the child PID for pclose(). */
    for (int i = 0; i < POPEN_MAX; i++) {
        if (__popen_table[i].fp == NULL) {
            __popen_table[i].fp = fp;
            __popen_table[i].pid = pid;
            return fp;
        }
    }

    /* No slots available -- this should be rare (>16 simultaneous popens). */
    fclose(fp);
    waitpid(pid, NULL, 0);
    errno = EMFILE;
    return NULL;
}

int pclose(FILE *stream)
{
    if (stream == NULL) {
        errno = EINVAL;
        return -1;
    }

    /* Find the child PID for this stream. */
    pid_t pid = -1;
    int slot = -1;
    for (int i = 0; i < POPEN_MAX; i++) {
        if (__popen_table[i].fp == stream) {
            pid = __popen_table[i].pid;
            slot = i;
            break;
        }
    }

    if (pid == -1) {
        /* Not a popen'd stream. */
        errno = EINVAL;
        return -1;
    }

    /* Close the stream (flushes buffers, closes fd). */
    fclose(stream);

    /* Clear the table entry. */
    __popen_table[slot].fp = NULL;
    __popen_table[slot].pid = 0;

    /* Wait for the child to exit and return its status. */
    int status;
    pid_t ret;
    do {
        ret = waitpid(pid, &status, 0);
    } while (ret == -1 && errno == EINTR);

    if (ret == -1)
        return -1;

    return status;
}

/* --- stdio extensions ---------------------------------------------------- */

int dprintf(int fd, const char *format, ...)
{
    char buf[1024];
    va_list ap;
    va_start(ap, format);
    int n = vsnprintf(buf, sizeof(buf), format, ap);
    va_end(ap);
    if (n > 0) {
        extern ssize_t write(int, const void *, size_t);
        int w = (int)write(fd, buf, (size_t)(n < (int)sizeof(buf) ? n : (int)sizeof(buf) - 1));
        return w;
    }
    return n;
}

/* vasprintf is defined in stdio.c -- not duplicated here. */

int fseeko(FILE *stream, off_t offset, int whence)
{
    return fseek(stream, (long)offset, whence);
}

/* --- Group database ------------------------------------------------------ */

#include <grp.h>
#include <unistd.h>
#include <fcntl.h>

/*
 * Static fallback group entry returned when /etc/group is absent or the
 * requested group is not found in the file.
 */
static struct group stub_group = {
    .gr_name   = "root",
    .gr_passwd = "x",
    .gr_gid    = 0,
    .gr_mem    = NULL
};

/*
 * Scratch buffers for /etc/group parsing.  getgrnam/getgrgid return a
 * pointer to static storage (POSIX allows this for the non-_r variants).
 * The gr_* fields point into __gr_line_buf which holds the last parsed
 * line from /etc/group.
 *
 * Format: name:passwd:gid:member,member,...
 *
 * Member list: pointers are stored in __gr_members[], pointing into the
 * members region of __gr_line_buf.  Up to GR_MAX_MEMBERS are supported.
 */
#define GR_MAX_MEMBERS 32

static struct group _parsed_gr;
static char __gr_line_buf[512];
static char *__gr_members[GR_MAX_MEMBERS + 1]; /* +1 for NULL terminator */

/*
 * __parse_group_line: parse a colon-delimited /etc/group line into *gr.
 *
 * Operates on __gr_line_buf in-place (NUL-terminates each field).
 * Returns 1 on success, 0 if the line is malformed or a comment.
 */
static int __parse_group_line(struct group *gr)
{
    char *p = __gr_line_buf;

    /* Skip comment lines and blank lines. */
    if (*p == '#' || *p == '\n' || *p == '\0')
        return 0;

    /* Strip trailing newline. */
    size_t len = 0;
    while (p[len] && p[len] != '\n') len++;
    p[len] = '\0';

    /* gr_name */
    gr->gr_name = p;
    while (*p && *p != ':') p++;
    if (!*p) return 0;
    *p++ = '\0';

    /* gr_passwd */
    gr->gr_passwd = p;
    while (*p && *p != ':') p++;
    if (!*p) return 0;
    *p++ = '\0';

    /* gr_gid */
    gr->gr_gid = (gid_t)0;
    while (*p >= '0' && *p <= '9') {
        gr->gr_gid = (gid_t)(gr->gr_gid * 10 + (unsigned)(*p - '0'));
        p++;
    }
    if (*p != ':') return 0;
    p++;

    /* gr_mem: comma-separated member list (may be empty). */
    int nmem = 0;
    gr->gr_mem = __gr_members;

    while (*p && nmem < GR_MAX_MEMBERS) {
        char *start = p;
        while (*p && *p != ',' && *p != '\n') p++;
        if (p > start) {
            char save = *p;
            *p = '\0';
            __gr_members[nmem++] = start;
            if (save == ',')
                p++;    /* skip comma separator */
        } else {
            break;      /* empty members field */
        }
    }
    __gr_members[nmem] = NULL;

    return 1;
}

/*
 * __read_group_line: read one line from fd into __gr_line_buf.
 *
 * Returns the number of bytes read (>= 1) on success, 0 at EOF.
 */
static int __read_group_line(int fd)
{
    size_t pos = 0;
    char ch;

    while (pos < sizeof(__gr_line_buf) - 1) {
        ssize_t r = read(fd, &ch, 1);
        if (r <= 0)
            break;
        __gr_line_buf[pos++] = ch;
        if (ch == '\n')
            break;
    }
    __gr_line_buf[pos] = '\0';
    return (int)pos;
}

struct group *getgrnam(const char *name)
{
    if (!name)
        return &stub_group;

    int fd = open("/etc/group", O_RDONLY);
    if (fd < 0)
        return &stub_group;

    while (__read_group_line(fd) > 0) {
        if (__parse_group_line(&_parsed_gr)) {
            if (_parsed_gr.gr_name &&
                strcmp(_parsed_gr.gr_name, name) == 0) {
                close(fd);
                return &_parsed_gr;
            }
        }
    }

    close(fd);
    return NULL;    /* Not found -- POSIX allows NULL. */
}

struct group *getgrgid(gid_t gid)
{
    int fd = open("/etc/group", O_RDONLY);
    if (fd < 0) {
        /* Fall back to static root group when /etc/group is absent. */
        if (gid == 0)
            return &stub_group;
        return NULL;
    }

    while (__read_group_line(fd) > 0) {
        if (__parse_group_line(&_parsed_gr)) {
            if (_parsed_gr.gr_gid == gid) {
                close(fd);
                return &_parsed_gr;
            }
        }
    }

    close(fd);
    return NULL;    /* Not found. */
}

/* ========================================================================= */
/* Group database iteration (setgrent / getgrent / endgrent)                 */
/* ========================================================================= */

static int __gr_fd = -1;

void setgrent(void)
{
    if (__gr_fd >= 0)
        close(__gr_fd);
    __gr_fd = open("/etc/group", O_RDONLY);
}

void endgrent(void)
{
    if (__gr_fd >= 0) {
        close(__gr_fd);
        __gr_fd = -1;
    }
}

struct group *getgrent(void)
{
    if (__gr_fd < 0) {
        __gr_fd = open("/etc/group", O_RDONLY);
        if (__gr_fd < 0)
            return (struct group *)0;
    }

    while (__read_group_line(__gr_fd) > 0) {
        if (__parse_group_line(&_parsed_gr))
            return &_parsed_gr;
    }
    return (struct group *)0;
}

/* The groups `user` belongs to: `group` first, then every /etc/group entry
 * that lists the user. Returns the count, or -1 when it does not fit in
 * *ngroups (which is then set to the count needed), as glibc does. It
 * reported only `group`. */
int getgrouplist(const char *user, gid_t group, gid_t *groups, int *ngroups)
{
    int n = 0, cap = *ngroups;
    struct group *gr;

    if (n < cap)
        groups[n] = group;
    n++;
    setgrent();
    while ((gr = getgrent()) != NULL) {
        if (gr->gr_gid == group || !gr->gr_mem)
            continue;
        for (char **m = gr->gr_mem; *m; m++) {
            if (strcmp(*m, user) == 0) {
                if (n < cap)
                    groups[n] = gr->gr_gid;
                n++;
                break;
            }
        }
    }
    endgrent();
    *ngroups = n;
    return n <= cap ? n : -1;
}

/* --- Time functions ------------------------------------------------------ */

int clock_settime(clockid_t clk_id, const struct timespec *tp)
{
    (void)clk_id;
    (void)tp;
    errno = EPERM;
    return -1;
}

struct tm *localtime_r(const time_t *timep, struct tm *result)
{
    struct tm *t = localtime(timep);
    if (t && result) {
        *result = *t;
        return result;
    }
    return NULL;
}

char *strptime(const char *s, const char *format, struct tm *tm)
{
    /* Minimal stub -- BusyBox date uses this for -s parsing */
    (void)format;
    (void)tm;
    if (s == NULL) return NULL;
    return (char *)s; /* pretend we consumed nothing */
}

/* --- System functions ---------------------------------------------------- */

#include <sys/sysinfo.h>
#include <sys/times.h>
#include <sched.h>

/* sysinfo() and times(): the kernel's figures (they were made up). */
int sysinfo(struct sysinfo *info)
{
    return (int)__syscall_ret(veridian_syscall1(SYS_sysinfo, info));
}

clock_t times(struct tms *buf)
{
    return (clock_t)__syscall_ret(veridian_syscall1(SYS_times, buf));
}

int sched_getaffinity(pid_t pid, size_t cpusetsize, cpu_set_t *mask)
{
    (void)pid;
    if (mask && cpusetsize >= sizeof(cpu_set_t)) {
        CPU_ZERO(mask);
        CPU_SET(0, mask);
        return 0;
    }
    errno = EINVAL;
    return -1;
}

int prctl(int option, unsigned long arg2, unsigned long arg3,
          unsigned long arg4, unsigned long arg5)
{
    (void)option;
    (void)arg2;
    (void)arg3;
    (void)arg4;
    (void)arg5;
    errno = ENOSYS;
    return -1;
}

#include <signal.h>

#include <veridian/syscall.h>

/* Wait with `mask` until a signal is caught (rt_sigsuspend); always ends
 * with EINTR. It returned at once without waiting. */
int sigsuspend(const sigset_t *mask)
{
    long ret = veridian_syscall1(SYS_rt_sigsuspend, (long)mask);
    errno = ret < 0 ? (int)(-ret) : EINTR;
    return -1;
}

/* Both did nothing and reported success (N-249). */
int utimensat(int dirfd, const char *pathname,
              const struct timespec times[2], int flags)
{
    return (int)__syscall_ret(
        veridian_syscall4(SYS_utimensat, (long)dirfd, pathname, times, (long)flags));
}

int futimens(int fd, const struct timespec times[2])
{
    /* A NULL path makes utimensat act on the fd itself. */
    return (int)__syscall_ret(
        veridian_syscall4(SYS_utimensat, (long)fd, 0L, times, 0L));
}

/* --- fnmatch ------------------------------------------------------------- */

#include <fnmatch.h>

/*
 * fnmatch: POSIX filename pattern matching.
 *
 * Handles *, ?, and [...] (including [!...] / [^...] negation and
 * a-z range expressions).  FNM_PATHNAME, FNM_PERIOD, FNM_NOESCAPE,
 * and FNM_CASEFOLD flags are supported.
 */
static int __fnmatch_lower(int c)
{
    return (c >= 'A' && c <= 'Z') ? c + ('a' - 'A') : c;
}

int fnmatch(const char *pattern, const char *string, int flags)
{
    const char *p = pattern, *s = string;

    while (*p) {
        if (*s == '\0' && *p != '*')
            return FNM_NOMATCH;

        /* FNM_PERIOD: leading dot must be matched literally. */
        if ((flags & FNM_PERIOD) && *s == '.' &&
            (s == string || ((flags & FNM_PATHNAME) && *(s - 1) == '/'))) {
            if (*p != '.')
                return FNM_NOMATCH;
            p++;
            s++;
            continue;
        }

        switch (*p) {
        case '?':
            if ((flags & FNM_PATHNAME) && *s == '/')
                return FNM_NOMATCH;
            s++;
            p++;
            break;

        case '*':
            p++;
            while (*p == '*')
                p++;
            if (*p == '\0')
                return ((flags & FNM_PATHNAME) && strchr(s, '/'))
                    ? FNM_NOMATCH : 0;
            while (*s) {
                if (fnmatch(p, s, flags & ~FNM_PERIOD) == 0)
                    return 0;
                if ((flags & FNM_PATHNAME) && *s == '/')
                    break;
                s++;
            }
            return FNM_NOMATCH;

        case '[': {
            /* Character class: [abc], [a-z], [!abc], [^abc] */
            if ((flags & FNM_PATHNAME) && *s == '/')
                return FNM_NOMATCH;

            p++; /* skip '[' */
            int negate = 0;
            if (*p == '!' || *p == '^') {
                negate = 1;
                p++;
            }
            int matched = 0;
            int ch = (flags & FNM_CASEFOLD) ? __fnmatch_lower(*s) : *s;

            /* ']' at start of class is literal. */
            if (*p == ']') {
                if (ch == ']')
                    matched = 1;
                p++;
            }

            while (*p && *p != ']') {
                int lo = (flags & FNM_CASEFOLD) ? __fnmatch_lower(*p) : *p;
                p++;
                if (*p == '-' && *(p + 1) != '\0' && *(p + 1) != ']') {
                    p++; /* skip '-' */
                    int hi = (flags & FNM_CASEFOLD) ? __fnmatch_lower(*p) : *p;
                    p++;
                    if (ch >= lo && ch <= hi)
                        matched = 1;
                } else {
                    if (ch == lo)
                        matched = 1;
                }
            }
            if (*p == ']')
                p++;

            if (negate)
                matched = !matched;
            if (!matched)
                return FNM_NOMATCH;
            s++;
            break;
        }

        case '\\':
            if (!(flags & FNM_NOESCAPE)) {
                p++;
                if (*p == '\0')
                    return FNM_NOMATCH;
            }
            /* Fall through to literal match. */
            /* FALLTHROUGH */
        default: {
            int pc = *p, sc = *s;
            if (flags & FNM_CASEFOLD) {
                pc = __fnmatch_lower(pc);
                sc = __fnmatch_lower(sc);
            }
            if (pc != sc)
                return FNM_NOMATCH;
            p++;
            s++;
            break;
        }
        }
    }

    return (*s == '\0') ? 0 : FNM_NOMATCH;
}

/* --- glob ---------------------------------------------------------------- */

#include <glob.h>
#include <dirent.h>

/*
 * glob: POSIX filename generation (wildcard expansion).
 *
 * Handles single-level patterns (no path separator in the pattern).
 * Multi-level path patterns (e.g. dir/ * /file) are handled by
 * splitting at '/' and recursing on each path component.  This
 * implementation is sufficient for ash shell filename expansion.
 */

/* Helper: check if a string contains any glob metacharacters. */
static int __glob_has_magic(const char *p)
{
    for (; *p; p++) {
        if (*p == '*' || *p == '?' || *p == '[')
            return 1;
    }
    return 0;
}

/*
 * Slots allocated for an array holding `used` slots: the next power of two,
 * minimum 16. glob_t has no capacity field, so the capacity is always this
 * function of the slot count, and __glob_add grows the array (by doubling)
 * only when the next entry crosses it.
 */
static size_t __glob_capacity(size_t used)
{
    size_t cap = 16;
    while (cap < used)
        cap *= 2;
    return cap;
}

/* Helper: add a path to the glob result, growing the array as needed. */
static int __glob_add(glob_t *pglob, const char *path)
{
    size_t idx = pglob->gl_pathc + pglob->gl_offs;
    size_t needed = idx + 2; /* +1 for entry, +1 for trailing NULL */
    size_t capacity = pglob->gl_pathv == NULL ? 0 : __glob_capacity(idx + 1);

    if (needed > capacity) {
        char **nv = (char **)realloc(pglob->gl_pathv,
                                     __glob_capacity(needed) * sizeof(char *));
        if (nv == NULL)
            return GLOB_NOSPACE;
        pglob->gl_pathv = nv;
    }

    /* Initialize gl_offs entries to NULL on first allocation. */
    if (pglob->gl_pathc == 0) {
        for (size_t i = 0; i < pglob->gl_offs; i++)
            pglob->gl_pathv[i] = NULL;
    }

    char *dup = strdup(path);
    if (dup == NULL)
        return GLOB_NOSPACE;

    pglob->gl_pathv[idx] = dup;
    pglob->gl_pathc++;
    pglob->gl_pathv[idx + 1] = NULL;
    return 0;
}

/* Simple string comparison for qsort. */
static int __glob_strcmp(const void *a, const void *b)
{
    return strcmp(*(const char **)a, *(const char **)b);
}

int glob(const char *pattern, int flags,
         int (*errfunc)(const char *, int), glob_t *pglob)
{
    (void)errfunc;

    if (!(flags & GLOB_APPEND)) {
        pglob->gl_pathc = 0;
        pglob->gl_pathv = NULL;
        if (!(flags & GLOB_DOOFFS))
            pglob->gl_offs = 0;
    }

    /* Split pattern into directory and base components. */
    const char *slash = strrchr(pattern, '/');
    char dir[4096];
    const char *base;

    if (slash) {
        size_t dlen = (size_t)(slash - pattern);
        if (dlen == 0) {
            dir[0] = '/';
            dir[1] = '\0';
        } else if (dlen < sizeof(dir)) {
            memcpy(dir, pattern, dlen);
            dir[dlen] = '\0';
        } else {
            return GLOB_NOSPACE;
        }
        base = slash + 1;
    } else {
        strcpy(dir, ".");
        base = pattern;
    }

    /* If the base component has no wildcards, check literal existence. */
    if (!__glob_has_magic(base)) {
        char full[4096];
        if (slash) {
            size_t dl = strlen(dir);
            size_t bl = strlen(base);
            if (dl + 1 + bl + 1 > sizeof(full))
                return GLOB_NOSPACE;
            memcpy(full, dir, dl);
            full[dl] = '/';
            memcpy(full + dl + 1, base, bl + 1);
        } else {
            size_t bl = strlen(base);
            if (bl + 1 > sizeof(full))
                return GLOB_NOSPACE;
            memcpy(full, base, bl + 1);
        }

        extern int access(const char *, int);
        if (access(full, 0 /* F_OK */) == 0) {
            int rc = __glob_add(pglob, full);
            if (rc) return rc;
        } else if (flags & GLOB_NOCHECK) {
            int rc = __glob_add(pglob, pattern);
            if (rc) return rc;
        } else {
            return GLOB_NOMATCH;
        }
        return 0;
    }

    /* Open the directory and match entries against the base pattern. */
    DIR *dp = opendir(dir);
    if (dp == NULL) {
        if (flags & GLOB_NOCHECK) {
            int rc = __glob_add(pglob, pattern);
            if (rc) return rc;
            return 0;
        }
        return GLOB_ABORTED;
    }

    int found = 0;
    struct dirent *de;
    while ((de = readdir(dp)) != NULL) {
        /* Skip . and .. */
        if (de->d_name[0] == '.' && (de->d_name[1] == '\0' ||
            (de->d_name[1] == '.' && de->d_name[2] == '\0')))
            continue;

        /* Skip hidden files unless pattern starts with '.' */
        if (de->d_name[0] == '.' && base[0] != '.')
            continue;

        if (fnmatch(base, de->d_name, 0) == 0) {
            char full[4096];
            if (slash) {
                size_t dl = strlen(dir);
                size_t nl = strlen(de->d_name);
                if (dl + 1 + nl + 1 > sizeof(full))
                    continue;
                memcpy(full, dir, dl);
                full[dl] = '/';
                memcpy(full + dl + 1, de->d_name, nl + 1);
            } else {
                size_t nl = strlen(de->d_name);
                if (nl + 1 > sizeof(full))
                    continue;
                memcpy(full, de->d_name, nl + 1);
            }

            if (flags & GLOB_MARK) {
                /* Append '/' if entry is a directory.  We skip this check
                 * since stat() support may be limited; ash doesn't depend on it. */
            }

            int rc = __glob_add(pglob, full);
            if (rc) {
                closedir(dp);
                return rc;
            }
            found++;
        }
    }
    closedir(dp);

    if (found == 0) {
        if (flags & GLOB_NOCHECK) {
            int rc = __glob_add(pglob, pattern);
            if (rc) return rc;
        } else {
            return GLOB_NOMATCH;
        }
    }

    /* Sort results unless GLOB_NOSORT is set. */
    if (!(flags & GLOB_NOSORT) && pglob->gl_pathc > 1) {
        qsort(pglob->gl_pathv + pglob->gl_offs,
              pglob->gl_pathc, sizeof(char *), __glob_strcmp);
    }

    return 0;
}

void globfree(glob_t *pglob)
{
    if (pglob == NULL || pglob->gl_pathv == NULL)
        return;

    for (size_t i = pglob->gl_offs; i < pglob->gl_offs + pglob->gl_pathc; i++) {
        free(pglob->gl_pathv[i]);
    }
    free(pglob->gl_pathv);
    pglob->gl_pathv = NULL;
    pglob->gl_pathc = 0;
}

/* setjmp / longjmp are assembly: src/setjmp_<arch>.S. */

/* h_errno global */
int h_errno = 0;

const char *hstrerror(int err)
{
    switch (err) {
    case 1: return "Host not found";
    case 2: return "Try again";
    case 3: return "Non-recoverable error";
    case 4: return "No data";
    default: return "Unknown error";
    }
}

/* getservbyname / getservbyport stubs */
#include <netdb.h>

struct servent *getservbyname(const char *name, const char *proto)
{
    (void)name;
    (void)proto;
    return NULL;
}

struct servent *getservbyport(int port, const char *proto)
{
    (void)port;
    (void)proto;
    return NULL;
}

struct hostent *gethostbyaddr(const void *addr, socklen_t len, int type)
{
    (void)addr;
    (void)len;
    (void)type;
    h_errno = 1; /* HOST_NOT_FOUND */
    return NULL;
}

/* ===================== Additional stubs for BusyBox link =================== */

#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <sys/time.h>
#include <grp.h>
#include <veridian/syscall.h>

struct hostent *gethostbyname(const char *name)
{
    (void)name;
    h_errno = 1; /* HOST_NOT_FOUND */
    return NULL;
}

/*
 * getpeername() -- get name of connected peer socket.
 *
 * Kernel syscall SYS_getpeername (52): (fd, addr_ptr, len_ptr)
 * The kernel writes the remote address into *addr and sets *addrlen to 16.
 */
int getpeername(int sockfd, struct sockaddr *addr, socklen_t *addrlen)
{
    long ret = veridian_syscall3(SYS_getpeername, sockfd, addr, addrlen);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/*
 * getsockname() -- get socket name (local bound address).
 *
 * Kernel syscall SYS_getsockname (51): (fd, addr_ptr, len_ptr)
 * The kernel writes the local address into *addr and sets *addrlen to 16.
 */
int getsockname(int sockfd, struct sockaddr *addr, socklen_t *addrlen)
{
    long ret = veridian_syscall3(SYS_getsockname, sockfd, addr, addrlen);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* inet_aton, inet_ntoa and the other address conversions: inet.c */

/*
 * sendto() -- send a message to a specific destination address.
 *
 * Kernel syscall SYS_sendto (44): (fd, buf_ptr, buf_len, addr_ptr, addr_len)
 * The flags argument is not forwarded (not yet implemented in the kernel).
 * When dest_addr is NULL (connected socket), falls back to SYS_SOCKET_SEND.
 */
ssize_t sendto(int sockfd, const void *buf, size_t len, int flags,
               const struct sockaddr *dest_addr, socklen_t addrlen)
{
    (void)flags;
    long ret;
    if (dest_addr != NULL) {
        ret = veridian_syscall5(SYS_sendto,
                                 sockfd, buf, len, dest_addr, addrlen);
    } else {
        /* No destination address: send on connected socket. */
        ret = veridian_syscall3(SYS_SOCKET_SEND, sockfd, buf, len);
    }
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (ssize_t)ret;
}

/*
 * recvfrom() -- receive a message and optionally capture sender's address.
 *
 * Kernel syscall SYS_recvfrom (45): (fd, buf_ptr, buf_len, addr_ptr)
 * addr_ptr may be NULL (kernel handles gracefully).  addrlen is updated to
 * the fixed sockaddr size (16) when an address was written.
 * The flags argument is not forwarded.
 */
ssize_t recvfrom(int sockfd, void *buf, size_t len, int flags,
                 struct sockaddr *src_addr, socklen_t *addrlen)
{
    (void)flags;
    long ret = veridian_syscall4(SYS_recvfrom,
                                  sockfd, buf, len, src_addr);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    /* If the caller requested the source address and we wrote one, update
     * addrlen to reflect the actual sockaddr size the kernel wrote. */
    if (src_addr != NULL && addrlen != NULL)
        *addrlen = (socklen_t)sizeof(struct sockaddr);
    return (ssize_t)ret;
}

/*
 * socketpair() -- create a pair of connected sockets (AF_UNIX only).
 *
 * Kernel syscall SYS_socketpair (53) takes the Linux argument order
 * (domain, type, protocol, int sv[2]) and writes the two new fds into sv.
 */
int socketpair(int domain, int type, int protocol, int sv[2])
{
    long ret = veridian_syscall4(SYS_socketpair, domain, type, protocol, sv);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* sendmsg()/recvmsg() -- data plus ancillary data (SCM_RIGHTS). */
ssize_t sendmsg(int sockfd, const struct msghdr *msg, int flags)
{
    long ret = veridian_syscall3(SYS_sendmsg, sockfd, msg, flags);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (ssize_t)ret;
}

ssize_t recvmsg(int sockfd, struct msghdr *msg, int flags)
{
    long ret = veridian_syscall3(SYS_recvmsg, sockfd, msg, flags);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (ssize_t)ret;
}

/* Set the supplementary groups to those `user` belongs to (plus `group`).
 * It did nothing and reported success. */
int initgroups(const char *user, gid_t group)
{
    gid_t groups[NGROUPS_MAX];
    int n = NGROUPS_MAX;

    if (getgrouplist(user, group, groups, &n) < 0) {
        errno = EINVAL;  /* more groups than the kernel holds */
        return -1;
    }
    return setgroups((size_t)n, groups);
}

/* endgrent() is implemented above with setgrent()/getgrent(). */

int chroot(const char *path)
{
    return (int)__syscall_ret(veridian_syscall1(SYS_chroot, path));
}

int fchdir(int fd)
{
    return (int)__syscall_ret(veridian_syscall1(SYS_fchdir, (long)fd));
}

int settimeofday(const struct timeval *tv, const struct timezone *tz)
{
    (void)tv;
    (void)tz;
    errno = ENOSYS;
    return -1;
}
