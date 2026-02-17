/*
 * VeridianOS libc -- termios.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Terminal I/O functions.  These wrap the ioctl syscall for
 * TCGETS/TCSETS operations.
 */

#include <termios.h>
#include <errno.h>
#include <string.h>

/* ioctl request codes for terminal get/set */
#define TCGETS  0x5401
#define TCSETS  0x5402
#define TCSETSW 0x5403
#define TCSETSF 0x5404

/* Forward declaration — ioctl is in syscall.c */
extern int ioctl(int fd, unsigned long request, ...);

int tcgetattr(int fd, struct termios *termios_p)
{
    if (!termios_p) {
        errno = EINVAL;
        return -1;
    }

    /*
     * Stub: return a reasonable default for a serial console.
     * A real implementation would ioctl(fd, TCGETS, termios_p).
     */
    memset(termios_p, 0, sizeof(*termios_p));
    termios_p->c_iflag = ICRNL | IXON;
    termios_p->c_oflag = OPOST | ONLCR;
    termios_p->c_cflag = CS8 | CREAD | CLOCAL;
    termios_p->c_lflag = ISIG | ICANON | ECHO | ECHOE | ECHOK | IEXTEN;

    termios_p->c_cc[VINTR]  = 3;    /* ^C */
    termios_p->c_cc[VQUIT]  = 28;   /* ^\ */
    termios_p->c_cc[VERASE] = 127;  /* DEL */
    termios_p->c_cc[VKILL]  = 21;   /* ^U */
    termios_p->c_cc[VEOF]   = 4;    /* ^D */
    termios_p->c_cc[VMIN]   = 1;
    termios_p->c_cc[VTIME]  = 0;
    termios_p->c_cc[VSTART] = 17;   /* ^Q */
    termios_p->c_cc[VSTOP]  = 19;   /* ^S */
    termios_p->c_cc[VSUSP]  = 26;   /* ^Z */

    termios_p->c_ispeed = B9600;
    termios_p->c_ospeed = B9600;

    return 0;
}

int tcsetattr(int fd, int optional_actions, const struct termios *termios_p)
{
    (void)fd;
    (void)optional_actions;
    (void)termios_p;

    /* Stub: accept but ignore. */
    return 0;
}

speed_t cfgetispeed(const struct termios *termios_p)
{
    return termios_p ? termios_p->c_ispeed : B0;
}

speed_t cfgetospeed(const struct termios *termios_p)
{
    return termios_p ? termios_p->c_ospeed : B0;
}

int cfsetispeed(struct termios *termios_p, speed_t speed)
{
    if (!termios_p) {
        errno = EINVAL;
        return -1;
    }
    termios_p->c_ispeed = speed;
    return 0;
}

int cfsetospeed(struct termios *termios_p, speed_t speed)
{
    if (!termios_p) {
        errno = EINVAL;
        return -1;
    }
    termios_p->c_ospeed = speed;
    return 0;
}

int tcsendbreak(int fd, int duration)
{
    (void)fd;
    (void)duration;
    return 0;
}

int tcdrain(int fd)
{
    (void)fd;
    return 0;
}

int tcflush(int fd, int queue_selector)
{
    (void)fd;
    (void)queue_selector;
    return 0;
}

int tcflow(int fd, int action)
{
    (void)fd;
    (void)action;
    return 0;
}

int tcsetpgrp(int fd, pid_t pgrp)
{
    (void)fd;
    (void)pgrp;
    return 0;
}

pid_t tcgetpgrp(int fd)
{
    (void)fd;
    return 0;
}
