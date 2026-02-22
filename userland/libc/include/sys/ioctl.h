/*
 * VeridianOS libc -- <sys/ioctl.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * I/O control operations.
 */

#ifndef _SYS_IOCTL_H
#define _SYS_IOCTL_H

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Terminal ioctl requests                                                   */
/* ========================================================================= */

/** Get terminal attributes (struct termios). */
#define TCGETS      0x5401
/** Set terminal attributes immediately. */
#define TCSETS      0x5402
/** Set terminal attributes after draining output. */
#define TCSETSW     0x5403
/** Set terminal attributes after draining output + flushing input. */
#define TCSETSF     0x5404
/** Get terminal window size. */
#define TIOCGWINSZ  0x5413
/** Set terminal window size. */
#define TIOCSWINSZ  0x5414
/** Get process group ID of foreground process. */
#define TIOCGPGRP   0x540F
/** Set process group ID of foreground process. */
#define TIOCSPGRP   0x5410
/** Non-blocking I/O. */
#define FIONBIO     0x5421
/** Get number of bytes available for reading. */
#define FIONREAD    0x541B

/** Terminal window size structure. */
struct winsize {
    unsigned short ws_row;      /* Rows, in characters */
    unsigned short ws_col;      /* Columns, in characters */
    unsigned short ws_xpixel;   /* Horizontal size, pixels (unused) */
    unsigned short ws_ypixel;   /* Vertical size, pixels (unused) */
};

/** Generic ioctl. */
int ioctl(int fd, unsigned long request, ...);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_IOCTL_H */
