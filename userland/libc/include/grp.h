/*
 * VeridianOS C Library -- <grp.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _GRP_H
#define _GRP_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

struct group {
    char  *gr_name;
    char  *gr_passwd;
    gid_t  gr_gid;
    char **gr_mem;
};

struct group *getgrnam(const char *name);
struct group *getgrgid(gid_t gid);
int getgrouplist(const char *user, gid_t group, gid_t *groups, int *ngroups);

/** Rewind the group database to the beginning. */
void setgrent(void);

/** Close the group database. */
void endgrent(void);

/** Read the next entry from the group database. */
struct group *getgrent(void);

#ifdef __cplusplus
}
#endif

#endif /* _GRP_H */
