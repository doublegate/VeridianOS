/*
 * VeridianOS Package Manager -- vpkg
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Shared types and function declarations for the vpkg user-space tool.
 */

#ifndef VPKG_H
#define VPKG_H

#include <stdint.h>
#include <stddef.h>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

#define VPKG_VERSION        "0.1.0"
#define VPKG_DB_DIR         "/var/db/vpkg"
#define VPKG_DB_FILE        "/var/db/vpkg/packages.db"
#define VPKG_LOCK_FILE      "/var/db/vpkg/lock"
#define VPKG_INSTALL_BASE   "/usr/local/packages"

#define MAX_PKG_NAME        128
#define MAX_PKG_VERSION     32
#define MAX_PKG_DESC        256
#define MAX_PKG_AUTHOR      128
#define MAX_PKG_LICENSE     64
#define MAX_DEPS            32
#define MAX_PACKAGES        256
#define MAX_PATH            1024

/* ========================================================================= */
/* Syscall numbers for package management (kernel/src/syscall/mod.rs)        */
/* ========================================================================= */

#define SYS_PKG_INSTALL     90
#define SYS_PKG_REMOVE      91
#define SYS_PKG_QUERY       92
#define SYS_PKG_LIST        93
#define SYS_PKG_UPDATE      94

/* ========================================================================= */
/* Data structures                                                           */
/* ========================================================================= */

/* Semantic version */
typedef struct {
    uint32_t major;
    uint32_t minor;
    uint32_t patch;
} vpkg_version_t;

/* Package dependency */
typedef struct {
    char name[MAX_PKG_NAME];
    char version_req[MAX_PKG_VERSION];
} vpkg_dep_t;

/* Package record in the database */
typedef struct {
    char name[MAX_PKG_NAME];
    vpkg_version_t version;
    char author[MAX_PKG_AUTHOR];
    char description[MAX_PKG_DESC];
    char license[MAX_PKG_LICENSE];
    uint32_t dep_count;
    vpkg_dep_t deps[MAX_DEPS];
    uint64_t install_time;      /* Seconds since epoch */
    uint64_t installed_size;    /* Bytes */
} vpkg_pkg_t;

/* In-memory package database */
typedef struct {
    uint32_t count;
    vpkg_pkg_t packages[MAX_PACKAGES];
} vpkg_db_t;

/* Return codes */
#define VPKG_OK             0
#define VPKG_ERR_NOT_FOUND  1
#define VPKG_ERR_EXISTS     2
#define VPKG_ERR_IO         3
#define VPKG_ERR_ARGS       4
#define VPKG_ERR_DEPS       5
#define VPKG_ERR_SYSCALL    6
#define VPKG_ERR_DB         7

/* ========================================================================= */
/* database.c                                                                */
/* ========================================================================= */

/*
 * Load the package database from VPKG_DB_FILE into memory.
 * Returns VPKG_OK on success, VPKG_ERR_IO on failure.
 * If the file does not exist, initializes an empty database.
 */
int vpkg_db_load(vpkg_db_t *db);

/*
 * Save the in-memory database to VPKG_DB_FILE.
 * Returns VPKG_OK on success, VPKG_ERR_IO on failure.
 */
int vpkg_db_save(const vpkg_db_t *db);

/*
 * Find a package by name in the database.
 * Returns a pointer to the package record, or NULL if not found.
 */
vpkg_pkg_t *vpkg_db_find(vpkg_db_t *db, const char *name);

/*
 * Add a package record to the database.
 * Returns VPKG_OK on success, VPKG_ERR_EXISTS if already present,
 * VPKG_ERR_DB if the database is full.
 */
int vpkg_db_add(vpkg_db_t *db, const vpkg_pkg_t *pkg);

/*
 * Remove a package record from the database.
 * Returns VPKG_OK on success, VPKG_ERR_NOT_FOUND if not present.
 */
int vpkg_db_remove(vpkg_db_t *db, const char *name);

/* ========================================================================= */
/* install.c                                                                 */
/* ========================================================================= */

/*
 * Install a package by name and optional version string.
 * Uses SYS_PKG_INSTALL syscall to invoke kernel-side installation,
 * then records the package in the local database.
 *
 * Returns VPKG_OK on success.
 */
int vpkg_install(vpkg_db_t *db, const char *name, const char *version);

/* ========================================================================= */
/* remove.c                                                                  */
/* ========================================================================= */

/*
 * Remove an installed package by name.
 * Checks for reverse dependencies before removal.
 * Uses SYS_PKG_REMOVE syscall and removes from local database.
 *
 * Returns VPKG_OK on success.
 */
int vpkg_remove(vpkg_db_t *db, const char *name);

/* ========================================================================= */
/* query.c                                                                   */
/* ========================================================================= */

/*
 * Search for packages matching a pattern (substring match on name).
 * Uses SYS_PKG_QUERY syscall for kernel-side search plus local database.
 *
 * Returns VPKG_OK on success.
 */
int vpkg_search(vpkg_db_t *db, const char *pattern);

/*
 * List all installed packages.
 * Returns VPKG_OK on success.
 */
int vpkg_list(vpkg_db_t *db);

/*
 * Display detailed information about a package.
 * Returns VPKG_OK on success, VPKG_ERR_NOT_FOUND if not installed.
 */
int vpkg_info(vpkg_db_t *db, const char *name);

/*
 * Update package lists from repositories.
 * Uses SYS_PKG_UPDATE syscall.
 *
 * Returns VPKG_OK on success.
 */
int vpkg_update(vpkg_db_t *db);

/* ========================================================================= */
/* Utility functions (in main.c)                                             */
/* ========================================================================= */

/*
 * Print a formatted version string (e.g., "1.2.3").
 */
void vpkg_print_version(const vpkg_version_t *v);

/*
 * Parse a version string "X.Y.Z" into a vpkg_version_t.
 * Returns 0 on success, -1 on parse failure.
 */
int vpkg_parse_version(const char *str, vpkg_version_t *out);

#endif /* VPKG_H */
