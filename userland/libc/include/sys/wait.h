/*
 * VeridianOS libc -- <sys/wait.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Wait-status macros and waitpid() declaration.
 */

#ifndef _SYS_WAIT_H
#define _SYS_WAIT_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Wait-status interpretation macros                                         */
/* ========================================================================= */

/*
 * Status layout (matches kernel convention):
 *   bits  7:0  - if signalled: signal number (1-127)
 *   bit   7    - core dump flag
 *   bits 15:8  - if exited: exit code
 *
 * A process that exited normally has bits 7:0 == 0.
 */

/** True if the child terminated normally (via _exit / exit). */
#define WIFEXITED(s)    (((s) & 0x7F) == 0)

/** Exit status (meaningful only when WIFEXITED is true). */
#define WEXITSTATUS(s)  (((s) >> 8) & 0xFF)

/** True if the child was killed by a signal. */
#define WIFSIGNALED(s)  (((s) & 0x7F) != 0 && ((s) & 0x7F) != 0x7F)

/** Signal number that killed the child (meaningful only when WIFSIGNALED). */
#define WTERMSIG(s)     ((s) & 0x7F)

/** True if the child is currently stopped. */
#define WIFSTOPPED(s)   (((s) & 0xFF) == 0x7F)

/** Signal that stopped the child (meaningful only when WIFSTOPPED). */
#define WSTOPSIG(s)     (((s) >> 8) & 0xFF)

/** True if the child produced a core dump (meaningful only when WIFSIGNALED). */
#define WCOREDUMP(s)    ((s) & 0x80)

/** True if the child has continued (Linux extension). */
#define WIFCONTINUED(s) ((s) == 0xFFFF)

/* ========================================================================= */
/* waitpid() option flags                                                    */
/* ========================================================================= */

/** Return immediately if no child has exited. */
#define WNOHANG     1

/** Also report stopped children. */
#define WUNTRACED   2

/* ========================================================================= */
/* Function declarations                                                     */
/* ========================================================================= */

/**
 * Wait for a child process to change state.
 *
 * @param pid       Process to wait for (-1 = any child).
 * @param wstatus   Pointer to receive the status (may be NULL).
 * @param options   WNOHANG, WUNTRACED, or 0.
 * @return PID of the child on success, 0 (WNOHANG, no change), -1 on error.
 */
pid_t waitpid(pid_t pid, int *wstatus, int options);

/**
 * Wait for any child process (equivalent to waitpid(-1, wstatus, 0)).
 */
pid_t wait(int *wstatus);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_WAIT_H */
