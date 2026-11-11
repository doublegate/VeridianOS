/*
 * VeridianOS: giving up authority capabilities (ADR 0024, ADR 0026).
 *
 * Every program the kernel starts holds a capability on each namespace it
 * is in, both clocks and the system, and keeps them across fork and exec.
 * A Linux privilege check on one of those objects needs the capability as
 * well as the POSIX capability or uid, so a launcher confines what it runs
 * by dropping, before exec, what that program should not have:
 *
 *     // root that cannot mount: MODIFY off its mount namespace and off
 *     // every user namespace above it, which stand for it (NS_GET_ID ids),
 *     // and CREATE off its user namespace, or it could make a user
 *     // namespace of its own (granted to its creator) and mount in a mount
 *     // namespace made inside that
 *     veridian_capability_drop(VERIDIAN_CAP_NAMESPACE, mnt_id, VERIDIAN_RIGHT_MODIFY);
 *     veridian_capability_drop(VERIDIAN_CAP_NAMESPACE, user_id,
 *                              VERIDIAN_RIGHT_MODIFY | VERIDIAN_RIGHT_CREATE);
 *
 * `rights` 0 (VERIDIAN_DROP_WHOLE), or every right held, removes the
 * capability; otherwise it is
 * narrowed to the rights it has minus `rights`, and removed once it has
 * none. Rights only go away: nothing gives one back but a new grant from
 * the kernel. Only the caller's own space changes, never its parent's.
 *
 * Returns the number of capabilities narrowed or removed (0 when there was
 * nothing to change), or -1 with errno EINVAL for a kind and id naming
 * nothing that can be dropped, or rights bits above CREATE.
 *
 * Header only, for the native libc and musl alike. Linux has no such call
 * (ENOSYS there).
 */
#ifndef VERIDIAN_CAPABILITY_H
#define VERIDIAN_CAPABILITY_H

/* The call's number, the same on every architecture. */
#define VERIDIAN_SYS_CAPABILITY_DROP 1056

/* What `kind` names, and the ids within it. */
#define VERIDIAN_CAP_NAMESPACE   9  /* id: the namespace's NS_GET_ID id */
#define VERIDIAN_CAP_CLOCK       10 /* id: one of the two below */
#define VERIDIAN_CAP_CLOCK_SYSTEM 0 /* set or steer the system clock */
#define VERIDIAN_CAP_CLOCK_ALARM  1 /* arm a timer on an alarm clock */
#define VERIDIAN_CAP_SYSTEM      11 /* id: 0 */

/* Rights, as the capability token encodes them. */
#define VERIDIAN_RIGHT_READ    0x01u
#define VERIDIAN_RIGHT_WRITE   0x02u
#define VERIDIAN_RIGHT_EXECUTE 0x04u
#define VERIDIAN_RIGHT_GRANT   0x08u
#define VERIDIAN_RIGHT_REVOKE  0x10u
#define VERIDIAN_RIGHT_DELETE  0x20u
#define VERIDIAN_RIGHT_MODIFY  0x40u
#define VERIDIAN_RIGHT_CREATE  0x80u
/* As a drop mask: remove the capability whole. */
#define VERIDIAN_DROP_WHOLE    0x00u

#if defined(__VERIDIAN_LIBC__)
#include <veridian/syscall.h>

static inline long veridian_capability_drop(unsigned kind, unsigned long id, unsigned rights)
{
    return __syscall_ret(veridian_syscall3(VERIDIAN_SYS_CAPABILITY_DROP, kind, id, rights));
}
#else
#include <unistd.h>

static inline long veridian_capability_drop(unsigned kind, unsigned long id, unsigned rights)
{
    return syscall(VERIDIAN_SYS_CAPABILITY_DROP, kind, id, rights);
}
#endif

#endif /* VERIDIAN_CAPABILITY_H */
