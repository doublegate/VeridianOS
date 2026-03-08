/*
 * VeridianOS -- baloo-veridian-backend.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Baloo file indexer backend for VeridianOS.
 *
 * Provides filesystem crawling, content indexing, inotify-based watch,
 * and query interface for KDE's file search infrastructure.  The index
 * is persisted to /var/lib/baloo/ in a compact binary format.
 *
 * D-Bus service: org.kde.baloo
 */

#ifndef BALOO_VERIDIAN_BACKEND_H
#define BALOO_VERIDIAN_BACKEND_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Index state                                                               */
/* ========================================================================= */

/**
 * Current state of the Baloo indexer.
 */
typedef enum {
    BALOO_IDLE      = 0,  /* Not actively indexing */
    BALOO_CRAWLING  = 1,  /* Performing initial directory walk */
    BALOO_INDEXING  = 2,  /* Indexing file contents */
    BALOO_SUSPENDED = 3   /* Indexing paused (e.g., on battery) */
} BalooIndexState;

/* ========================================================================= */
/* Query types                                                               */
/* ========================================================================= */

/**
 * Type of search query to execute.
 */
typedef enum {
    BALOO_QUERY_FILENAME = 0,  /* Search filename index only */
    BALOO_QUERY_CONTENT  = 1,  /* Search file content index */
    BALOO_QUERY_TAG      = 2,  /* Search by extended-attribute tags */
    BALOO_QUERY_TYPE     = 3   /* Search by MIME type */
} BalooQueryType;

/* ========================================================================= */
/* Search result                                                             */
/* ========================================================================= */

/**
 * A single file search result returned by baloo_query().
 */
typedef struct {
    char     path[1024];           /* Absolute file path */
    char     filename[256];        /* Basename for display */
    uint64_t mtime;                /* Last modification (Unix timestamp) */
    uint64_t size;                 /* File size in bytes */
    char     content_snippet[256]; /* Matched content excerpt (if content query) */
    int      relevance;            /* 0-100 relevance score */
} BalooResult;

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

/**
 * Initialize the Baloo backend.
 *
 * @param index_path  Directory to store/load the persistent index.
 *                    Typically "/var/lib/baloo".
 * @return 0 on success, -1 on error.
 */
int baloo_init(const char *index_path);

/**
 * Shut down the Baloo backend and flush the index to disk.
 */
void baloo_destroy(void);

/* ========================================================================= */
/* Crawl control                                                             */
/* ========================================================================= */

/**
 * Begin crawling from root_path, indexing all discovered files.
 * Non-blocking: crawling proceeds in batches with yields.
 */
void baloo_start_crawl(const char *root_path);

/**
 * Stop an in-progress crawl.
 */
void baloo_stop_crawl(void);

/* ========================================================================= */
/* Query interface                                                           */
/* ========================================================================= */

/**
 * Search the index.
 *
 * @param query_string  Search query text.
 * @param type          Type of search (filename, content, tag, type).
 * @param results_out   Caller-allocated array to receive results.
 * @param max_results   Size of results_out array.
 * @return              Number of results written, or -1 on error.
 */
int baloo_query(const char *query_string,
                BalooQueryType type,
                BalooResult *results_out,
                int max_results);

/* ========================================================================= */
/* State queries                                                             */
/* ========================================================================= */

/**
 * Return the current indexer state.
 */
BalooIndexState baloo_get_state(void);

/**
 * Return the number of files currently in the index.
 */
uint64_t baloo_get_indexed_count(void);

/* ========================================================================= */
/* Indexer control                                                           */
/* ========================================================================= */

/**
 * Temporarily suspend indexing (e.g., on battery or high load).
 */
void baloo_suspend(void);

/**
 * Resume indexing after a suspend.
 */
void baloo_resume(void);

/**
 * Set directories to exclude from indexing.
 *
 * @param paths  Array of absolute directory paths.
 * @param count  Number of entries in paths.
 */
void baloo_set_excluded_paths(const char **paths, int count);

/**
 * Manually add a single file to the index.
 */
void baloo_index_file(const char *path);

/**
 * Remove a file from the index.
 */
void baloo_remove_file(const char *path);

/* ========================================================================= */
/* Inotify integration                                                       */
/* ========================================================================= */

/**
 * Begin watching a directory for file changes via inotify.
 * CREATE/MODIFY events trigger re-indexing; DELETE events remove entries.
 */
void baloo_watch_directory(const char *path);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* BALOO_VERIDIAN_BACKEND_H */
