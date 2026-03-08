/*
 * VeridianOS libc -- <sys/signalfd.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Signal notification file descriptor.
 * Used by D-Bus daemon and service managers for unified event loops.
 */

#ifndef _SYS_SIGNALFD_H
#define _SYS_SIGNALFD_H

#include <stdint.h>
#include <signal.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Flags for signalfd4() */
#define SFD_CLOEXEC     02000000
#define SFD_NONBLOCK    00004000

/* Signal information structure returned by read(2) on a signalfd.
 * Each read returns one or more of these (128 bytes each). */
struct signalfd_siginfo {
    uint32_t ssi_signo;     /* Signal number */
    int32_t  ssi_errno;     /* Error number (unused) */
    int32_t  ssi_code;      /* Signal code */
    uint32_t ssi_pid;       /* Sending PID */
    uint32_t ssi_uid;       /* Sending UID */
    int32_t  ssi_fd;        /* File descriptor (SIGIO) */
    uint32_t ssi_tid;       /* Kernel timer ID */
    uint32_t ssi_band;      /* Band event (SIGIO) */
    uint32_t ssi_overrun;   /* POSIX timer overrun */
    uint32_t ssi_trapno;    /* Trap number */
    int32_t  ssi_status;    /* Exit status (SIGCHLD) */
    int32_t  ssi_int;       /* Integer from sigqueue */
    uint64_t ssi_ptr;       /* Pointer from sigqueue */
    uint64_t ssi_utime;     /* User CPU time (SIGCHLD) */
    uint64_t ssi_stime;     /* System CPU time (SIGCHLD) */
    uint64_t ssi_addr;      /* Signal-generating address */
    uint16_t ssi_addr_lsb;  /* Address LSB (SIGBUS) */
    uint8_t  __pad[46];     /* Pad to 128 bytes */
};

/**
 * Create or update a signal notification fd.
 *
 * @param fd    -1 to create new, or existing signalfd to update mask
 * @param mask  Signal mask (set of signals to monitor)
 * @param flags SFD_NONBLOCK | SFD_CLOEXEC
 * @return signalfd on success, -1 on error (sets errno)
 */
int signalfd(int fd, const sigset_t *mask, int flags);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_SIGNALFD_H */
