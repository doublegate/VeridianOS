/*
 * VeridianOS Package Manager -- vpkg
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * User-space package management tool for VeridianOS.
 * Provides install, remove, search, list, info, and update commands.
 *
 * Usage:
 *   vpkg install <package> [version]
 *   vpkg remove <package>
 *   vpkg search <pattern>
 *   vpkg list
 *   vpkg info <package>
 *   vpkg update
 *   vpkg --version
 *   vpkg --help
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "vpkg.h"

/* ========================================================================= */
/* Utility functions                                                         */
/* ========================================================================= */

void vpkg_print_version(const vpkg_version_t *v)
{
    printf("%u.%u.%u", v->major, v->minor, v->patch);
}

int vpkg_parse_version(const char *str, vpkg_version_t *out)
{
    unsigned int maj = 0, min = 0, pat = 0;
    const char *p = str;
    int field = 0;

    if (!str || !out)
        return -1;

    /* Parse "X.Y.Z" manually (no sscanf in minimal libc) */
    while (*p) {
        if (*p >= '0' && *p <= '9') {
            unsigned int digit = (unsigned int)(*p - '0');
            switch (field) {
            case 0: maj = maj * 10 + digit; break;
            case 1: min = min * 10 + digit; break;
            case 2: pat = pat * 10 + digit; break;
            default: return -1;
            }
        } else if (*p == '.') {
            field++;
            if (field > 2)
                return -1;
        } else {
            return -1;
        }
        p++;
    }

    out->major = maj;
    out->minor = min;
    out->patch = pat;
    return 0;
}

/* ========================================================================= */
/* Usage and help                                                            */
/* ========================================================================= */

static void print_usage(void)
{
    printf("vpkg %s -- VeridianOS Package Manager\n\n", VPKG_VERSION);
    printf("Usage:\n");
    printf("  vpkg install <package> [version]   Install a package\n");
    printf("  vpkg remove <package>              Remove a package\n");
    printf("  vpkg search <pattern>              Search for packages\n");
    printf("  vpkg list                          List installed packages\n");
    printf("  vpkg info <package>                Show package details\n");
    printf("  vpkg update                        Update package lists\n");
    printf("  vpkg --version                     Show vpkg version\n");
    printf("  vpkg --help                        Show this help\n");
}

/* ========================================================================= */
/* Command dispatch                                                          */
/* ========================================================================= */

int main(int argc, char *argv[])
{
    vpkg_db_t db;
    int ret;

    if (argc < 2) {
        print_usage();
        return VPKG_ERR_ARGS;
    }

    /* Handle flags */
    if (strcmp(argv[1], "--version") == 0 || strcmp(argv[1], "-V") == 0) {
        printf("vpkg %s\n", VPKG_VERSION);
        return VPKG_OK;
    }

    if (strcmp(argv[1], "--help") == 0 || strcmp(argv[1], "-h") == 0) {
        print_usage();
        return VPKG_OK;
    }

    /* Load the package database */
    ret = vpkg_db_load(&db);
    if (ret != VPKG_OK) {
        fprintf(stderr, "vpkg: error: failed to load package database\n");
        return ret;
    }

    /* Dispatch command */
    if (strcmp(argv[1], "install") == 0) {
        if (argc < 3) {
            fprintf(stderr, "vpkg: error: install requires a package name\n");
            return VPKG_ERR_ARGS;
        }
        const char *version = (argc >= 4) ? argv[3] : "*";
        ret = vpkg_install(&db, argv[2], version);

    } else if (strcmp(argv[1], "remove") == 0) {
        if (argc < 3) {
            fprintf(stderr, "vpkg: error: remove requires a package name\n");
            return VPKG_ERR_ARGS;
        }
        ret = vpkg_remove(&db, argv[2]);

    } else if (strcmp(argv[1], "search") == 0) {
        if (argc < 3) {
            fprintf(stderr, "vpkg: error: search requires a pattern\n");
            return VPKG_ERR_ARGS;
        }
        ret = vpkg_search(&db, argv[2]);

    } else if (strcmp(argv[1], "list") == 0) {
        ret = vpkg_list(&db);

    } else if (strcmp(argv[1], "info") == 0) {
        if (argc < 3) {
            fprintf(stderr, "vpkg: error: info requires a package name\n");
            return VPKG_ERR_ARGS;
        }
        ret = vpkg_info(&db, argv[2]);

    } else if (strcmp(argv[1], "update") == 0) {
        ret = vpkg_update(&db);

    } else {
        fprintf(stderr, "vpkg: error: unknown command '%s'\n", argv[1]);
        print_usage();
        ret = VPKG_ERR_ARGS;
    }

    return ret;
}
