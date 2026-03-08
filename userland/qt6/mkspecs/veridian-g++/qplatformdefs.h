/*
 * VeridianOS -- qplatformdefs.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Qt platform definitions for VeridianOS.  Provides the standard POSIX
 * includes and type mappings that Qt's platform abstraction layer expects.
 * Based on the Linux qplatformdefs.h with VeridianOS-specific adjustments.
 */

#ifndef QPLATFORMDEFS_H
#define QPLATFORMDEFS_H

/* Standard C / POSIX headers */
#include <fcntl.h>
#include <unistd.h>
#include <dirent.h>
#include <signal.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>
#include <errno.h>
#include <limits.h>

/* System headers */
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/select.h>

/* Socket headers */
#include <sys/socket.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>

/* Threading */
#include <pthread.h>

/* Qt file-related type mappings */
#define QT_STATBUF              struct stat
#define QT_STATBUF4TSTAT        struct stat
#define QT_STAT                 stat
#define QT_FSTAT                fstat
#define QT_LSTAT                lstat
#define QT_OPEN                 open
#define QT_CLOSE                close
#define QT_LSEEK                lseek
#define QT_READ                 read
#define QT_WRITE                write
#define QT_ACCESS               access
#define QT_GETCWD               getcwd
#define QT_CHDIR                chdir
#define QT_MKDIR                mkdir
#define QT_RMDIR                rmdir
#define QT_OPEN_LARGEFILE       O_LARGEFILE
#define QT_OPEN_RDONLY          O_RDONLY
#define QT_OPEN_WRONLY          O_WRONLY
#define QT_OPEN_RDWR            O_RDWR
#define QT_OPEN_CREAT           O_CREAT
#define QT_OPEN_TRUNC           O_TRUNC
#define QT_OPEN_APPEND          O_APPEND

/* Large file support -- VeridianOS uses 64-bit off_t natively */
#ifndef O_LARGEFILE
#define O_LARGEFILE 0
#endif

#define QT_OFF_T                off_t
#define QT_FPOS_T               fpos_t

/* Signal handler type */
#define QT_SIGNAL_RETTYPE       void
#define QT_SIGNAL_ARGS          int
#define QT_SIGNAL_IGNORE        SIG_IGN

/* Socket type mappings */
#define QT_SOCKLEN_T            socklen_t
#define QT_SOCKET_CONNECT       ::connect
#define QT_SOCKET_BIND          ::bind
#define QT_SOCKET_LISTEN        ::listen
#define QT_SOCKET_ACCEPT        ::accept

/* VeridianOS has large-file support natively (64-bit off_t) */
#define QT_LARGEFILE_SUPPORT    64

#endif /* QPLATFORMDEFS_H */
