/*
 * VeridianOS Package Manager -- install.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Package installation logic.
 * Uses SYS_PKG_INSTALL (90) syscall for kernel-side package processing,
 * then records the installed package in the local database.
 */

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <veridian/syscall.h>

#include "vpkg.h"

/* ========================================================================= */
/* Install                                                                   */
/* ========================================================================= */

int vpkg_install(vpkg_db_t *db, const char *name, const char *version)
{
    vpkg_pkg_t pkg;
    vpkg_version_t ver;
    long ret;
    int rc;

    if (!db || !name)
        return VPKG_ERR_ARGS;

    /* Check if already installed */
    if (vpkg_db_find(db, name) != NULL) {
        fprintf(stderr, "vpkg: package '%s' is already installed\n", name);
        return VPKG_ERR_EXISTS;
    }

    /* Parse version (default to 0.0.0 if wildcard or missing) */
    memset(&ver, 0, sizeof(ver));
    if (version && version[0] != '*' && version[0] != '\0') {
        if (vpkg_parse_version(version, &ver) != 0) {
            fprintf(stderr, "vpkg: invalid version string '%s'\n", version);
            return VPKG_ERR_ARGS;
        }
    }

    printf("Installing %s", name);
    if (ver.major || ver.minor || ver.patch)
        printf(" %u.%u.%u", ver.major, ver.minor, ver.patch);
    printf("...\n");

    /*
     * Invoke kernel-side package installation via SYS_PKG_INSTALL (90).
     *
     * The kernel's sys_pkg_install() expects:
     *   arg1 (rdi): pointer to package name string
     *   arg2 (rsi): length of the name string
     *
     * The kernel handles:
     *   - Repository download
     *   - Signature verification (Ed25519 + optional Dilithium)
     *   - Hash integrity check
     *   - Dependency resolution
     *   - File extraction to /usr/local/packages/<name>/
     */
    ret = veridian_syscall2(SYS_PKG_INSTALL, name, strlen(name));
    if (ret < 0) {
        fprintf(stderr, "vpkg: kernel install failed for '%s' (error %ld)\n", name, ret);
        return VPKG_ERR_SYSCALL;
    }

    /* Build package record for local database */
    memset(&pkg, 0, sizeof(pkg));
    strncpy(pkg.name, name, MAX_PKG_NAME - 1);
    pkg.version = ver;
    strncpy(pkg.description, "(installed via vpkg)", MAX_PKG_DESC - 1);
    strncpy(pkg.license, "unknown", MAX_PKG_LICENSE - 1);
    pkg.install_time = 0;  /* TODO(future): get current time via clock_gettime */
    pkg.installed_size = 0; /* TODO(future): compute from extracted files */

    /* Add to local database */
    rc = vpkg_db_add(db, &pkg);
    if (rc != VPKG_OK) {
        fprintf(stderr, "vpkg: warning: installed but failed to record in database\n");
        return rc;
    }

    /* Persist database */
    rc = vpkg_db_save(db);
    if (rc != VPKG_OK) {
        fprintf(stderr, "vpkg: warning: installed but failed to save database\n");
    }

    printf("Successfully installed %s", name);
    if (ver.major || ver.minor || ver.patch)
        printf(" %u.%u.%u", ver.major, ver.minor, ver.patch);
    printf("\n");

    return VPKG_OK;
}
