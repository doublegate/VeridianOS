/*
 * VeridianOS -- baloo-veridian-index.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Inverted index storage for the Baloo file indexer backend.
 *
 * Provides a word-to-file-paths mapping with trigram support for
 * substring search.  The index can be persisted to disk in a compact
 * binary format ("BLIX" -- Baloo Linux IndeX).
 */

#ifndef BALOO_VERIDIAN_INDEX_H
#define BALOO_VERIDIAN_INDEX_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Opaque index handle                                                       */
/* ========================================================================= */

typedef struct BalooIndex BalooIndex;

/* ========================================================================= */
/* Search result (matches baloo-veridian-backend.h BalooResult layout)       */
/* ========================================================================= */

typedef struct {
    char path[1024];
    int  relevance;
} BalooIndexResult;

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

/**
 * Create a new empty index.
 *
 * @param storage_path  Directory where index files are stored.
 * @return              Opaque index handle, or NULL on error.
 */
BalooIndex *baloo_index_create(const char *storage_path);

/**
 * Load an existing index from disk.
 *
 * @param storage_path  Directory containing saved index files.
 * @return              Opaque index handle, or NULL if no saved index.
 */
BalooIndex *baloo_index_load(const char *storage_path);

/**
 * Save the current index to disk in BLIX binary format.
 *
 * @param index         Index handle.
 * @param storage_path  Directory to write index files.
 * @return              0 on success, -1 on error.
 */
int baloo_index_save(BalooIndex *index, const char *storage_path);

/**
 * Destroy an index and free all associated memory.
 */
void baloo_index_destroy(BalooIndex *index);

/* ========================================================================= */
/* Index manipulation                                                        */
/* ========================================================================= */

/**
 * Add a word->file_path mapping with the given relevance.
 *
 * The word is tokenized into trigrams for substring search support.
 * Multiple words can map to the same file path with different relevances.
 */
void baloo_index_add(BalooIndex *index, const char *word,
                     const char *file_path, int relevance);

/**
 * Remove all index entries referencing the given file path.
 */
void baloo_index_remove_file(BalooIndex *index, const char *file_path);

/* ========================================================================= */
/* Search                                                                    */
/* ========================================================================= */

/**
 * Search the index for files matching the query.
 *
 * Uses word-prefix matching and trigram-based substring search.
 * Results are sorted by combined relevance.
 *
 * @param index    Index handle.
 * @param query    Search query (may contain multiple words).
 * @param results  Caller-allocated result array.
 * @param max      Maximum number of results to return.
 * @return         Number of results written (0..max).
 */
int baloo_index_search(BalooIndex *index, const char *query,
                       BalooIndexResult *results, int max);

/* ========================================================================= */
/* Statistics                                                                */
/* ========================================================================= */

/**
 * Return the number of unique file paths in the index.
 */
uint64_t baloo_index_get_count(BalooIndex *index);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* BALOO_VERIDIAN_INDEX_H */
