/*
 * akonadi-veridian.h -- Akonadi PIM data store for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Provides a local PIM (Personal Information Management) data store
 * compatible with the KDE Akonadi framework.  Stores contacts (vCard),
 * calendar events (iCalendar), and notes in a flat-file backend.
 *
 * This is the VeridianOS-native Akonadi storage backend.  It provides
 * the same collection/item model as the full Akonadi server but uses
 * flat files instead of a database, making it suitable for the
 * resource-constrained VeridianOS environment.
 *
 * Known limitation: no sync agents -- local storage only.
 */

#ifndef AKONADI_VERIDIAN_H
#define AKONADI_VERIDIAN_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ======================================================================
 * Constants
 * ====================================================================== */

#define AKONADI_MAX_NAME        128
#define AKONADI_MAX_MIME_TYPE   64
#define AKONADI_MAX_PAYLOAD     8192
#define AKONADI_MAX_REMOTE_ID   128
#define AKONADI_MAX_FLAGS       256
#define AKONADI_MAX_QUERY       256
#define AKONADI_MAX_CONTENT_TYPES 256

/* Standard MIME types */
#define AKONADI_MIME_VCARD      "text/vcard"
#define AKONADI_MIME_CALENDAR   "text/calendar"
#define AKONADI_MIME_NOTE       "text/x-vnd.akonadi.note"

/* Default collection IDs */
#define AKONADI_COLLECTION_CONTACTS   1
#define AKONADI_COLLECTION_CALENDAR   2
#define AKONADI_COLLECTION_NOTES      3

/* ======================================================================
 * Data types
 * ====================================================================== */

/* A single PIM item (contact, event, or note) */
typedef struct {
    int64_t     id;
    int64_t     collection_id;
    char        mime_type[AKONADI_MAX_MIME_TYPE];
    char        payload[AKONADI_MAX_PAYLOAD];
    int         payload_len;
    char        remote_id[AKONADI_MAX_REMOTE_ID];
    char        flags[AKONADI_MAX_FLAGS];
} akonadi_item_t;

/* A collection (folder) containing items */
typedef struct {
    int64_t     id;
    int64_t     parent_id;
    char        name[AKONADI_MAX_NAME];
    char        content_types[AKONADI_MAX_CONTENT_TYPES];
    int         item_count;
} akonadi_collection_t;

/* ======================================================================
 * Lifecycle
 * ====================================================================== */

/*
 * Initialize the Akonadi data store.
 * Creates the data directory structure and default collections if they
 * do not already exist.  data_dir is typically /var/lib/akonadi.
 * Returns 0 on success, -1 on error.
 */
int akonadi_init(const char *data_dir);

/*
 * Shut down the Akonadi data store.
 * Flushes any pending writes and releases resources.
 */
void akonadi_shutdown(void);

/* ======================================================================
 * Collection API
 * ====================================================================== */

/*
 * Create a new collection (folder).
 * Returns the new collection ID (>0) on success, -1 on error.
 */
int64_t akonadi_create_collection(int64_t parent_id, const char *name,
                                   const char *content_types);

/*
 * Delete a collection and all its items.
 * Returns 0 on success, -1 on error.
 */
int akonadi_delete_collection(int64_t id);

/*
 * List child collections of a parent.
 * Fills the out array (up to max entries).
 * Returns the number of collections found.
 */
int akonadi_list_collections(int64_t parent_id,
                              akonadi_collection_t *out, int max);

/* ======================================================================
 * Item API
 * ====================================================================== */

/*
 * Add a new item to a collection.
 * Returns the new item ID (>0) on success, -1 on error.
 */
int64_t akonadi_add_item(int64_t collection_id, const char *mime_type,
                          const char *payload, int payload_len);

/*
 * Update an existing item's payload.
 * Returns 0 on success, -1 on error.
 */
int akonadi_update_item(int64_t id, const char *payload, int payload_len);

/*
 * Delete an item.
 * Returns 0 on success, -1 on error.
 */
int akonadi_delete_item(int64_t id);

/*
 * Get an item by ID.
 * Returns true if found, false otherwise.
 */
bool akonadi_get_item(int64_t id, akonadi_item_t *item_out);

/*
 * Search items within a collection by query string and optional MIME type.
 * The query matches against the payload content (substring search).
 * If mime_type is NULL, all types are searched.
 * Returns the number of matching items.
 */
int akonadi_search_items(int64_t collection_id, const char *query,
                          const char *mime_type,
                          akonadi_item_t *out, int max);

/*
 * Get all items in a collection.
 * Returns the number of items found.
 */
int akonadi_get_collection_items(int64_t collection_id,
                                  akonadi_item_t *out, int max);

#ifdef __cplusplus
}
#endif

#endif /* AKONADI_VERIDIAN_H */
