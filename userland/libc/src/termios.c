/*
 * VeridianOS libc -- termios.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Terminal I/O functions.  These wrap the ioctl syscall for
 * TCGETS/TCSETS operations using the real kernel terminal state.
 */

#include <termios.h>
#include <errno.h>

/* ioctl request codes for terminal get/set (matching Linux values) */
#define TCGETS  0x5401
#define TCSETS  0x5402
#define TCSETSW 0x5403
#define TCSETSF 0x5404

/* Forward declaration -- ioctl is in syscall.c */
extern int ioctl(int fd, unsigned long request, ...);

int tcgetattr(int fd, struct termios *termios_p)
{
    if (!termios_p) {
        errno = EINVAL;
        return -1;
    }

    return ioctl(fd, TCGETS, termios_p);
}

int tcsetattr(int fd, int optional_actions, const struct termios *termios_p)
{
    if (!termios_p) {
        errno = EINVAL;
        return -1;
    }

    unsigned long request;
    switch (optional_actions) {
    case TCSANOW:
        request = TCSETS;
        break;
    case TCSADRAIN:
        request = TCSETSW;
        break;
    case TCSAFLUSH:
        request = TCSETSF;
        break;
    default:
        errno = EINVAL;
        return -1;
    }

    return ioctl(fd, request, (void *)termios_p);
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

void cfmakeraw(struct termios *termios_p)
{
    if (!termios_p)
        return;

    /* Clear input flags: no break processing, no CR-to-NL, no parity,
     * no strip, no flow control */
    termios_p->c_iflag &= ~(unsigned int)(IGNBRK | BRKINT | PARMRK | ISTRIP |
                                           INLCR | IGNCR | ICRNL | IXON);

    /* Clear output flags: disable post-processing */
    termios_p->c_oflag &= ~(unsigned int)OPOST;

    /* Clear local flags: disable echo, canonical mode, extended input,
     * signal generation */
    termios_p->c_lflag &= ~(unsigned int)(ECHO | ECHONL | ICANON | ISIG | IEXTEN);

    /* Clear control flags: character size mask and parity */
    termios_p->c_cflag &= ~(unsigned int)(CSIZE | PARENB);

    /* Set 8-bit characters */
    termios_p->c_cflag |= CS8;

    /* Set raw-mode control characters: read returns after 1 byte,
     * no timeout */
    termios_p->c_cc[VMIN]  = 1;
    termios_p->c_cc[VTIME] = 0;
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
