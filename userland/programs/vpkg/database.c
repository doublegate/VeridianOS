/*
 * VeridianOS Package Manager -- database.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Package database read/write operations.
 * The database is stored as a flat binary file at /var/db/vpkg/packages.db.
 *
 * File format:
 *   [4 bytes]  Magic: "VPDB"
 *   [4 bytes]  Version: 1
 *   [4 bytes]  Package count
 *   [N * sizeof(vpkg_pkg_t)] Package records
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <errno.h>

#include "vpkg.h"

/* Database file magic and version */
#define VPDB_MAGIC      0x42445056  /* "VPDB" in little-endian */
#define VPDB_VERSION    1

/* Database file header */
typedef struct {
    uint32_t magic;
    uint32_t version;
    uint32_t count;
} vpdb_header_t;

/* ========================================================================= */
/* Helper: ensure the database directory exists                              */
/* ========================================================================= */

static int ensure_db_dir(void)
{
    /* Try to create /var, /var/db, /var/db/vpkg -- ignore EEXIST */
    mkdir("/var", 0755);
    mkdir("/var/db", 0755);
    mkdir("/var/db/vpkg", 0755);
    return 0;
}

/* ========================================================================= */
/* Load                                                                      */
/* ========================================================================= */

int vpkg_db_load(vpkg_db_t *db)
{
    int fd;
    vpdb_header_t hdr;
    ssize_t n;

    if (!db)
        return VPKG_ERR_ARGS;

    /* Initialize empty database */
    memset(db, 0, sizeof(*db));

    /* Ensure directory exists */
    ensure_db_dir();

    /* Try to open existing database */
    fd = open(VPKG_DB_FILE, O_RDONLY);
    if (fd < 0) {
        /* No database file yet -- start with empty database */
        return VPKG_OK;
    }

    /* Read header */
    n = read(fd, &hdr, sizeof(hdr));
    if (n != (ssize_t)sizeof(hdr)) {
        close(fd);
        /* Corrupted or empty file -- start fresh */
        return VPKG_OK;
    }

    /* Validate magic and version */
    if (hdr.magic != VPDB_MAGIC || hdr.version != VPDB_VERSION) {
        close(fd);
        fprintf(stderr, "vpkg: warning: database has invalid magic/version, starting fresh\n");
        return VPKG_OK;
    }

    /* Validate count */
    if (hdr.count > MAX_PACKAGES) {
        close(fd);
        fprintf(stderr, "vpkg: warning: database has too many packages (%u), truncating\n",
                hdr.count);
        hdr.count = MAX_PACKAGES;
    }

    db->count = hdr.count;

    /* Read package records */
    if (hdr.count > 0) {
        size_t data_size = (size_t)hdr.count * sizeof(vpkg_pkg_t);
        n = read(fd, db->packages, data_size);
        if (n != (ssize_t)data_size) {
            close(fd);
            fprintf(stderr, "vpkg: warning: database truncated, loaded %u of %u packages\n",
                    (unsigned)(n / sizeof(vpkg_pkg_t)), hdr.count);
            db->count = (uint32_t)(n / sizeof(vpkg_pkg_t));
            return VPKG_OK;
        }
    }

    close(fd);
    return VPKG_OK;
}

/* ========================================================================= */
/* Save                                                                      */
/* ========================================================================= */

int vpkg_db_save(const vpkg_db_t *db)
{
    int fd;
    vpdb_header_t hdr;
    ssize_t n;

    if (!db)
        return VPKG_ERR_ARGS;

    /* Ensure directory exists */
    ensure_db_dir();

    /* Open/create database file */
    fd = open(VPKG_DB_FILE, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        fprintf(stderr, "vpkg: error: cannot create database file: %s\n", VPKG_DB_FILE);
        return VPKG_ERR_IO;
    }

    /* Write header */
    hdr.magic = VPDB_MAGIC;
    hdr.version = VPDB_VERSION;
    hdr.count = db->count;

    n = write(fd, &hdr, sizeof(hdr));
    if (n != (ssize_t)sizeof(hdr)) {
        close(fd);
        return VPKG_ERR_IO;
    }

    /* Write package records */
    if (db->count > 0) {
        size_t data_size = (size_t)db->count * sizeof(vpkg_pkg_t);
        n = write(fd, db->packages, data_size);
        if (n != (ssize_t)data_size) {
            close(fd);
            return VPKG_ERR_IO;
        }
    }

    close(fd);
    return VPKG_OK;
}

/* ========================================================================= */
/* Find                                                                      */
/* ========================================================================= */

vpkg_pkg_t *vpkg_db_find(vpkg_db_t *db, const char *name)
{
    uint32_t i;

    if (!db || !name)
        return NULL;

    for (i = 0; i < db->count; i++) {
        if (strcmp(db->packages[i].name, name) == 0) {
            return &db->packages[i];
        }
    }

    return NULL;
}

/* ========================================================================= */
/* Add                                                                       */
/* ========================================================================= */

int vpkg_db_add(vpkg_db_t *db, const vpkg_pkg_t *pkg)
{
    if (!db || !pkg)
        return VPKG_ERR_ARGS;

    /* Check for duplicates */
    if (vpkg_db_find(db, pkg->name) != NULL) {
        return VPKG_ERR_EXISTS;
    }

    /* Check capacity */
    if (db->count >= MAX_PACKAGES) {
        fprintf(stderr, "vpkg: error: package database is full (%u packages)\n", MAX_PACKAGES);
        return VPKG_ERR_DB;
    }

    /* Append to the end */
    memcpy(&db->packages[db->count], pkg, sizeof(vpkg_pkg_t));
    db->count++;

    return VPKG_OK;
}

/* ========================================================================= */
/* Remove                                                                    */
/* ========================================================================= */

int vpkg_db_remove(vpkg_db_t *db, const char *name)
{
    uint32_t i;

    if (!db || !name)
        return VPKG_ERR_ARGS;

    for (i = 0; i < db->count; i++) {
        if (strcmp(db->packages[i].name, name) == 0) {
            /* Shift remaining entries down */
            if (i < db->count - 1) {
                memmove(&db->packages[i],
                        &db->packages[i + 1],
                        (size_t)(db->count - i - 1) * sizeof(vpkg_pkg_t));
            }
            db->count--;
            return VPKG_OK;
        }
    }

    return VPKG_ERR_NOT_FOUND;
}
