/*
 * VeridianOS Package Manager -- query.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Package query, search, list, info, and update operations.
 * Uses SYS_PKG_QUERY (92), SYS_PKG_LIST (93), and SYS_PKG_UPDATE (94)
 * syscalls for kernel-side operations.
 */

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <veridian/syscall.h>

#include "vpkg.h"

/* ========================================================================= */
/* Substring match helper                                                    */
/* ========================================================================= */

/*
 * Simple case-insensitive-ish substring search.
 * Returns 1 if needle is found in haystack, 0 otherwise.
 * (Full case-insensitive search requires tolower, which we keep simple.)
 */
static int contains(const char *haystack, const char *needle)
{
    size_t hlen, nlen, i;

    if (!haystack || !needle)
        return 0;

    hlen = strlen(haystack);
    nlen = strlen(needle);

    if (nlen == 0)
        return 1;
    if (nlen > hlen)
        return 0;

    for (i = 0; i <= hlen - nlen; i++) {
        if (strncmp(&haystack[i], needle, nlen) == 0)
            return 1;
    }

    return 0;
}

/* ========================================================================= */
/* Search                                                                    */
/* ========================================================================= */

int vpkg_search(vpkg_db_t *db, const char *pattern)
{
    uint32_t i;
    int found = 0;

    if (!db || !pattern)
        return VPKG_ERR_ARGS;

    /*
     * First, query the kernel for available (not just installed) packages.
     * SYS_PKG_QUERY (92) expects:
     *   arg1: pointer to pattern string
     *   arg2: length of pattern
     *
     * The kernel may print results to the console or return them
     * in a buffer. For now, we also search the local database.
     */
    veridian_syscall2(SYS_PKG_QUERY, pattern, strlen(pattern));

    /* Search local installed database */
    printf("Installed packages matching '%s':\n", pattern);
    for (i = 0; i < db->count; i++) {
        if (contains(db->packages[i].name, pattern) ||
            contains(db->packages[i].description, pattern)) {
            printf("  %s ", db->packages[i].name);
            vpkg_print_version(&db->packages[i].version);
            if (db->packages[i].description[0])
                printf(" - %s", db->packages[i].description);
            printf("\n");
            found++;
        }
    }

    if (found == 0)
        printf("  (no installed packages match)\n");

    return VPKG_OK;
}

/* ========================================================================= */
/* List                                                                      */
/* ========================================================================= */

int vpkg_list(vpkg_db_t *db)
{
    uint32_t i;

    if (!db)
        return VPKG_ERR_ARGS;

    /*
     * Also invoke kernel-side list via SYS_PKG_LIST (93).
     * arg1: pointer to output buffer (or 0 for console)
     * arg2: buffer size
     */
    veridian_syscall2(SYS_PKG_LIST, 0, 0);

    if (db->count == 0) {
        printf("No packages installed.\n");
        return VPKG_OK;
    }

    printf("Installed packages (%u):\n", db->count);
    for (i = 0; i < db->count; i++) {
        printf("  %-30s ", db->packages[i].name);
        vpkg_print_version(&db->packages[i].version);
        printf("\n");
    }

    return VPKG_OK;
}

/* ========================================================================= */
/* Info                                                                      */
/* ========================================================================= */

int vpkg_info(vpkg_db_t *db, const char *name)
{
    vpkg_pkg_t *pkg;
    uint32_t i;

    if (!db || !name)
        return VPKG_ERR_ARGS;

    pkg = vpkg_db_find(db, name);
    if (!pkg) {
        fprintf(stderr, "vpkg: package '%s' is not installed\n", name);
        return VPKG_ERR_NOT_FOUND;
    }

    printf("Package:      %s\n", pkg->name);
    printf("Version:      ");
    vpkg_print_version(&pkg->version);
    printf("\n");
    printf("Author:       %s\n", pkg->author[0] ? pkg->author : "(unknown)");
    printf("Description:  %s\n", pkg->description[0] ? pkg->description : "(none)");
    printf("License:      %s\n", pkg->license[0] ? pkg->license : "(unknown)");

    if (pkg->installed_size > 0) {
        if (pkg->installed_size >= 1048576)
            printf("Size:         %lu MB\n",
                   (unsigned long)(pkg->installed_size / 1048576));
        else if (pkg->installed_size >= 1024)
            printf("Size:         %lu KB\n",
                   (unsigned long)(pkg->installed_size / 1024));
        else
            printf("Size:         %lu bytes\n",
                   (unsigned long)pkg->installed_size);
    }

    if (pkg->dep_count > 0) {
        printf("Dependencies: ");
        for (i = 0; i < pkg->dep_count; i++) {
            if (i > 0) printf(", ");
            printf("%s", pkg->deps[i].name);
            if (pkg->deps[i].version_req[0] &&
                strcmp(pkg->deps[i].version_req, "*") != 0) {
                printf("@%s", pkg->deps[i].version_req);
            }
        }
        printf("\n");
    } else {
        printf("Dependencies: (none)\n");
    }

    return VPKG_OK;
}

/* ========================================================================= */
/* Update                                                                    */
/* ========================================================================= */

int vpkg_update(vpkg_db_t *db)
{
    long ret;

    if (!db)
        return VPKG_ERR_ARGS;

    printf("Updating package lists...\n");

    /*
     * Invoke kernel-side repository update via SYS_PKG_UPDATE (94).
     *
     * The kernel's sys_pkg_update() expects:
     *   arg1: flags (0 = default)
     *
     * The kernel handles:
     *   - Fetching updated package lists from configured repositories
     *   - Updating the resolver's package index
     */
    ret = veridian_syscall1(SYS_PKG_UPDATE, 0);
    if (ret < 0) {
        fprintf(stderr, "vpkg: kernel update failed (error %ld)\n", ret);
        return VPKG_ERR_SYSCALL;
    }

    printf("Package lists updated successfully.\n");
    return VPKG_OK;
}
