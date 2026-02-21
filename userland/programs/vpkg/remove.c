/*
 * VeridianOS Package Manager -- remove.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Package removal logic.
 * Checks for reverse dependencies before removing.
 * Uses SYS_PKG_REMOVE (91) syscall for kernel-side cleanup.
 */

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <veridian/syscall.h>

#include "vpkg.h"

/* ========================================================================= */
/* Check reverse dependencies                                                */
/* ========================================================================= */

/*
 * Check if any installed package depends on the given package name.
 * Returns the name of the first dependent found, or NULL if none.
 */
static const char *find_dependent(const vpkg_db_t *db, const char *name)
{
    uint32_t i, j;

    for (i = 0; i < db->count; i++) {
        /* Skip the package itself */
        if (strcmp(db->packages[i].name, name) == 0)
            continue;

        for (j = 0; j < db->packages[i].dep_count; j++) {
            if (strcmp(db->packages[i].deps[j].name, name) == 0) {
                return db->packages[i].name;
            }
        }
    }

    return NULL;
}

/* ========================================================================= */
/* Remove                                                                    */
/* ========================================================================= */

int vpkg_remove(vpkg_db_t *db, const char *name)
{
    const char *dependent;
    long ret;
    int rc;

    if (!db || !name)
        return VPKG_ERR_ARGS;

    /* Check that the package is installed */
    if (vpkg_db_find(db, name) == NULL) {
        fprintf(stderr, "vpkg: package '%s' is not installed\n", name);
        return VPKG_ERR_NOT_FOUND;
    }

    /* Check for reverse dependencies */
    dependent = find_dependent(db, name);
    if (dependent != NULL) {
        fprintf(stderr, "vpkg: cannot remove '%s': required by '%s'\n",
                name, dependent);
        return VPKG_ERR_DEPS;
    }

    printf("Removing %s...\n", name);

    /*
     * Invoke kernel-side package removal via SYS_PKG_REMOVE (91).
     *
     * The kernel's sys_pkg_remove() expects:
     *   arg1 (rdi): pointer to package name string
     *   arg2 (rsi): length of the name string
     *
     * The kernel handles:
     *   - Removing extracted files from /usr/local/packages/<name>/
     *   - Updating the kernel-side package registry
     */
    ret = veridian_syscall2(SYS_PKG_REMOVE, name, strlen(name));
    if (ret < 0) {
        fprintf(stderr, "vpkg: kernel remove failed for '%s' (error %ld)\n", name, ret);
        return VPKG_ERR_SYSCALL;
    }

    /* Remove from local database */
    rc = vpkg_db_remove(db, name);
    if (rc != VPKG_OK) {
        fprintf(stderr, "vpkg: warning: removed from kernel but failed to update database\n");
        return rc;
    }

    /* Persist database */
    rc = vpkg_db_save(db);
    if (rc != VPKG_OK) {
        fprintf(stderr, "vpkg: warning: removed but failed to save database\n");
    }

    printf("Successfully removed %s\n", name);
    return VPKG_OK;
}
