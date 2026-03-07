/*
 * VeridianOS C Library -- <pwd.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _PWD_H
#define _PWD_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

struct passwd {
    char  *pw_name;
    char  *pw_passwd;
    uid_t  pw_uid;
    gid_t  pw_gid;
    char  *pw_gecos;
    char  *pw_dir;
    char  *pw_shell;
};

struct passwd *getpwnam(const char *name);
struct passwd *getpwuid(uid_t uid);

/** Rewind the password database to the beginning. */
void setpwent(void);

/** Close the password database. */
void endpwent(void);

/** Read the next entry from the password database. */
struct passwd *getpwent(void);

#ifdef __cplusplus
}
#endif

#endif /* _PWD_H */
