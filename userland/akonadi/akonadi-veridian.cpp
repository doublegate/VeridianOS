/*
 * akonadi-veridian.cpp -- Akonadi PIM data store for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Flat-file implementation of the Akonadi data store.  Data is stored
 * under a configurable root directory:
 *
 *   <data_dir>/
 *     counter.dat                 -- next auto-increment ID
 *     collections/<id>/metadata   -- collection name + content types
 *     items/<id>.dat              -- item payload + metadata
 *
 * Default collections created on first init:
 *   1 - Contacts  (text/vcard)
 *   2 - Calendar  (text/calendar)
 *   3 - Notes     (text/x-vnd.akonadi.note)
 *
 * Known limitation: local storage only -- no sync agents.
 */

#include "akonadi-veridian.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <dirent.h>

/* ======================================================================
 * Internal state
 * ====================================================================== */

/* Root data directory (set by akonadi_init) */
static char s_data_dir[512];

/* Next auto-increment ID */
static int64_t s_next_id = 100;

/* Whether the store has been initialized */
static bool s_initialized = false;

/* ======================================================================
 * Helper: ensure directory exists (recursive)
 * ====================================================================== */

static int ensure_dir(const char *path)
{
    struct stat st;
    if (stat(path, &st) == 0 && S_ISDIR(st.st_mode)) {
        return 0;
    }
    return mkdir(path, 0755);
}

static int ensure_dir_recursive(const char *path)
{
    char tmp[512];
    strncpy(tmp, path, sizeof(tmp) - 1);
    tmp[sizeof(tmp) - 1] = '\0';

    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') {
            *p = '\0';
            ensure_dir(tmp);
            *p = '/';
        }
    }
    return ensure_dir(tmp);
}

/* ======================================================================
 * Helper: path construction
 * ====================================================================== */

static int build_path(char *out, size_t out_len,
                      const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(out, out_len, fmt, ap);
    va_end(ap);
    if (n < 0 || (size_t)n >= out_len) {
        return -1;
    }
    return 0;
}

/* ======================================================================
 * ID counter persistence
 * ====================================================================== */

static void load_counter(void)
{
    char path[512];
    snprintf(path, sizeof(path), "%s/counter.dat", s_data_dir);

    FILE *fp = fopen(path, "r");
    if (fp) {
        char buf[32];
        if (fgets(buf, sizeof(buf), fp)) {
            int64_t val = atoll(buf);
            if (val > s_next_id) {
                s_next_id = val;
            }
        }
        fclose(fp);
    }
}

static void save_counter(void)
{
    char path[512];
    snprintf(path, sizeof(path), "%s/counter.dat", s_data_dir);

    FILE *fp = fopen(path, "w");
    if (fp) {
        fprintf(fp, "%lld\n", (long long)s_next_id);
        fclose(fp);
    }
}

static int64_t alloc_id(void)
{
    int64_t id = s_next_id++;
    save_counter();
    return id;
}

/* ======================================================================
 * Collection storage
 * ====================================================================== */

static int write_collection_metadata(const akonadi_collection_t *col)
{
    char dir_path[512];
    snprintf(dir_path, sizeof(dir_path), "%s/collections/%lld",
             s_data_dir, (long long)col->id);
    ensure_dir_recursive(dir_path);

    char meta_path[512];
    snprintf(meta_path, sizeof(meta_path), "%s/metadata", dir_path);

    FILE *fp = fopen(meta_path, "w");
    if (!fp) {
        return -1;
    }

    fprintf(fp, "id=%lld\n", (long long)col->id);
    fprintf(fp, "parent_id=%lld\n", (long long)col->parent_id);
    fprintf(fp, "name=%s\n", col->name);
    fprintf(fp, "content_types=%s\n", col->content_types);
    fprintf(fp, "item_count=%d\n", col->item_count);
    fclose(fp);

    return 0;
}

static int read_collection_metadata(const char *meta_path,
                                     akonadi_collection_t *col)
{
    FILE *fp = fopen(meta_path, "r");
    if (!fp) {
        return -1;
    }

    memset(col, 0, sizeof(*col));
    char line[512];

    while (fgets(line, sizeof(line), fp)) {
        line[strcspn(line, "\n\r")] = '\0';

        if (strncmp(line, "id=", 3) == 0) {
            col->id = atoll(line + 3);
        } else if (strncmp(line, "parent_id=", 10) == 0) {
            col->parent_id = atoll(line + 10);
        } else if (strncmp(line, "name=", 5) == 0) {
            strncpy(col->name, line + 5, AKONADI_MAX_NAME - 1);
        } else if (strncmp(line, "content_types=", 14) == 0) {
            strncpy(col->content_types, line + 14,
                    AKONADI_MAX_CONTENT_TYPES - 1);
        } else if (strncmp(line, "item_count=", 11) == 0) {
            col->item_count = atoi(line + 11);
        }
    }

    fclose(fp);
    return 0;
}

/* ======================================================================
 * Item storage
 * ====================================================================== */

static int write_item(const akonadi_item_t *item)
{
    char items_dir[512];
    snprintf(items_dir, sizeof(items_dir), "%s/items", s_data_dir);
    ensure_dir(items_dir);

    char item_path[512];
    snprintf(item_path, sizeof(item_path), "%s/%lld.dat",
             items_dir, (long long)item->id);

    FILE *fp = fopen(item_path, "w");
    if (!fp) {
        return -1;
    }

    /* Write header fields */
    fprintf(fp, "id=%lld\n", (long long)item->id);
    fprintf(fp, "collection_id=%lld\n", (long long)item->collection_id);
    fprintf(fp, "mime_type=%s\n", item->mime_type);
    fprintf(fp, "payload_len=%d\n", item->payload_len);
    if (item->remote_id[0] != '\0') {
        fprintf(fp, "remote_id=%s\n", item->remote_id);
    }
    if (item->flags[0] != '\0') {
        fprintf(fp, "flags=%s\n", item->flags);
    }
    fprintf(fp, "---\n");

    /* Write payload */
    if (item->payload_len > 0) {
        fwrite(item->payload, 1, (size_t)item->payload_len, fp);
    }

    fclose(fp);
    return 0;
}

static int read_item(const char *item_path, akonadi_item_t *item)
{
    FILE *fp = fopen(item_path, "r");
    if (!fp) {
        return -1;
    }

    memset(item, 0, sizeof(*item));
    char line[512];
    bool in_payload = false;
    int payload_offset = 0;

    while (fgets(line, sizeof(line), fp)) {
        if (in_payload) {
            /* Append to payload */
            size_t len = strlen(line);
            if (payload_offset + (int)len < AKONADI_MAX_PAYLOAD) {
                memcpy(item->payload + payload_offset, line, len);
                payload_offset += (int)len;
            }
            continue;
        }

        line[strcspn(line, "\n\r")] = '\0';

        if (strcmp(line, "---") == 0) {
            in_payload = true;
            continue;
        }

        if (strncmp(line, "id=", 3) == 0) {
            item->id = atoll(line + 3);
        } else if (strncmp(line, "collection_id=", 14) == 0) {
            item->collection_id = atoll(line + 14);
        } else if (strncmp(line, "mime_type=", 10) == 0) {
            strncpy(item->mime_type, line + 10, AKONADI_MAX_MIME_TYPE - 1);
        } else if (strncmp(line, "payload_len=", 12) == 0) {
            item->payload_len = atoi(line + 12);
        } else if (strncmp(line, "remote_id=", 10) == 0) {
            strncpy(item->remote_id, line + 10, AKONADI_MAX_REMOTE_ID - 1);
        } else if (strncmp(line, "flags=", 6) == 0) {
            strncpy(item->flags, line + 6, AKONADI_MAX_FLAGS - 1);
        }
    }

    /* Update payload_len from actual data read */
    if (in_payload) {
        item->payload_len = payload_offset;
    }

    fclose(fp);
    return 0;
}

/* ======================================================================
 * Initialization
 * ====================================================================== */

static int create_default_collections(void)
{
    /* Contacts */
    akonadi_collection_t contacts = {0};
    contacts.id = AKONADI_COLLECTION_CONTACTS;
    contacts.parent_id = 0;
    strncpy(contacts.name, "Contacts", AKONADI_MAX_NAME - 1);
    strncpy(contacts.content_types, AKONADI_MIME_VCARD,
            AKONADI_MAX_CONTENT_TYPES - 1);
    contacts.item_count = 0;
    write_collection_metadata(&contacts);

    /* Calendar */
    akonadi_collection_t calendar = {0};
    calendar.id = AKONADI_COLLECTION_CALENDAR;
    calendar.parent_id = 0;
    strncpy(calendar.name, "Calendar", AKONADI_MAX_NAME - 1);
    strncpy(calendar.content_types, AKONADI_MIME_CALENDAR,
            AKONADI_MAX_CONTENT_TYPES - 1);
    calendar.item_count = 0;
    write_collection_metadata(&calendar);

    /* Notes */
    akonadi_collection_t notes = {0};
    notes.id = AKONADI_COLLECTION_NOTES;
    notes.parent_id = 0;
    strncpy(notes.name, "Notes", AKONADI_MAX_NAME - 1);
    strncpy(notes.content_types, AKONADI_MIME_NOTE,
            AKONADI_MAX_CONTENT_TYPES - 1);
    notes.item_count = 0;
    write_collection_metadata(&notes);

    fprintf(stderr, "[akonadi] Created default collections: "
            "Contacts, Calendar, Notes\n");
    return 0;
}

int akonadi_init(const char *data_dir)
{
    if (!data_dir) {
        return -1;
    }

    strncpy(s_data_dir, data_dir, sizeof(s_data_dir) - 1);
    s_data_dir[sizeof(s_data_dir) - 1] = '\0';

    /* Create directory structure */
    ensure_dir_recursive(s_data_dir);

    char collections_dir[512];
    snprintf(collections_dir, sizeof(collections_dir),
             "%s/collections", s_data_dir);
    ensure_dir(collections_dir);

    char items_dir[512];
    snprintf(items_dir, sizeof(items_dir), "%s/items", s_data_dir);
    ensure_dir(items_dir);

    /* Load or initialize ID counter */
    load_counter();

    /* Create default collections if they don't exist */
    char contacts_meta[512];
    snprintf(contacts_meta, sizeof(contacts_meta),
             "%s/collections/%d/metadata",
             s_data_dir, AKONADI_COLLECTION_CONTACTS);

    struct stat st;
    if (stat(contacts_meta, &st) != 0) {
        create_default_collections();
        /* Ensure next_id is above the reserved collection IDs */
        if (s_next_id <= 3) {
            s_next_id = 100;
            save_counter();
        }
    }

    s_initialized = true;
    fprintf(stderr, "[akonadi] Data store initialized at %s\n", s_data_dir);
    return 0;
}

void akonadi_shutdown(void)
{
    if (!s_initialized) {
        return;
    }

    save_counter();
    s_initialized = false;
    fprintf(stderr, "[akonadi] Data store shut down\n");
}

/* ======================================================================
 * Collection API implementation
 * ====================================================================== */

int64_t akonadi_create_collection(int64_t parent_id, const char *name,
                                   const char *content_types)
{
    if (!s_initialized || !name) {
        return -1;
    }

    akonadi_collection_t col = {0};
    col.id = alloc_id();
    col.parent_id = parent_id;
    strncpy(col.name, name, AKONADI_MAX_NAME - 1);
    if (content_types) {
        strncpy(col.content_types, content_types,
                AKONADI_MAX_CONTENT_TYPES - 1);
    }
    col.item_count = 0;

    if (write_collection_metadata(&col) < 0) {
        return -1;
    }

    fprintf(stderr, "[akonadi] Created collection '%s' (id=%lld)\n",
            name, (long long)col.id);
    return col.id;
}

int akonadi_delete_collection(int64_t id)
{
    if (!s_initialized) {
        return -1;
    }

    /* Delete all items in this collection */
    char items_dir[512];
    snprintf(items_dir, sizeof(items_dir), "%s/items", s_data_dir);

    DIR *dp = opendir(items_dir);
    if (dp) {
        struct dirent *ent;
        while ((ent = readdir(dp)) != NULL) {
            if (ent->d_name[0] == '.') {
                continue;
            }

            char item_path[512];
            snprintf(item_path, sizeof(item_path),
                     "%s/%s", items_dir, ent->d_name);

            akonadi_item_t item;
            if (read_item(item_path, &item) == 0 &&
                item.collection_id == id) {
                unlink(item_path);
            }
        }
        closedir(dp);
    }

    /* Delete collection metadata */
    char meta_path[512];
    snprintf(meta_path, sizeof(meta_path),
             "%s/collections/%lld/metadata",
             s_data_dir, (long long)id);
    unlink(meta_path);

    char col_dir[512];
    snprintf(col_dir, sizeof(col_dir),
             "%s/collections/%lld",
             s_data_dir, (long long)id);
    rmdir(col_dir);

    fprintf(stderr, "[akonadi] Deleted collection id=%lld\n",
            (long long)id);
    return 0;
}

int akonadi_list_collections(int64_t parent_id,
                              akonadi_collection_t *out, int max)
{
    if (!s_initialized || !out || max <= 0) {
        return 0;
    }

    char collections_dir[512];
    snprintf(collections_dir, sizeof(collections_dir),
             "%s/collections", s_data_dir);

    DIR *dp = opendir(collections_dir);
    if (!dp) {
        return 0;
    }

    int count = 0;
    struct dirent *ent;
    while ((ent = readdir(dp)) != NULL && count < max) {
        if (ent->d_name[0] == '.') {
            continue;
        }

        char meta_path[512];
        snprintf(meta_path, sizeof(meta_path),
                 "%s/%s/metadata", collections_dir, ent->d_name);

        akonadi_collection_t col;
        if (read_collection_metadata(meta_path, &col) == 0) {
            if (col.parent_id == parent_id) {
                out[count++] = col;
            }
        }
    }

    closedir(dp);
    return count;
}

/* ======================================================================
 * Item API implementation
 * ====================================================================== */

static void update_collection_item_count(int64_t collection_id, int delta)
{
    char meta_path[512];
    snprintf(meta_path, sizeof(meta_path),
             "%s/collections/%lld/metadata",
             s_data_dir, (long long)collection_id);

    akonadi_collection_t col;
    if (read_collection_metadata(meta_path, &col) == 0) {
        col.item_count += delta;
        if (col.item_count < 0) {
            col.item_count = 0;
        }
        write_collection_metadata(&col);
    }
}

int64_t akonadi_add_item(int64_t collection_id, const char *mime_type,
                          const char *payload, int payload_len)
{
    if (!s_initialized || !mime_type || !payload || payload_len <= 0) {
        return -1;
    }

    if (payload_len >= AKONADI_MAX_PAYLOAD) {
        fprintf(stderr, "[akonadi] Payload too large (%d bytes)\n",
                payload_len);
        return -1;
    }

    akonadi_item_t item = {0};
    item.id = alloc_id();
    item.collection_id = collection_id;
    strncpy(item.mime_type, mime_type, AKONADI_MAX_MIME_TYPE - 1);
    memcpy(item.payload, payload, (size_t)payload_len);
    item.payload_len = payload_len;

    if (write_item(&item) < 0) {
        return -1;
    }

    update_collection_item_count(collection_id, 1);

    fprintf(stderr, "[akonadi] Added item id=%lld to collection %lld "
            "(%s, %d bytes)\n",
            (long long)item.id, (long long)collection_id,
            mime_type, payload_len);
    return item.id;
}

int akonadi_update_item(int64_t id, const char *payload, int payload_len)
{
    if (!s_initialized || !payload || payload_len <= 0) {
        return -1;
    }

    if (payload_len >= AKONADI_MAX_PAYLOAD) {
        return -1;
    }

    char item_path[512];
    snprintf(item_path, sizeof(item_path),
             "%s/items/%lld.dat", s_data_dir, (long long)id);

    akonadi_item_t item;
    if (read_item(item_path, &item) < 0) {
        return -1;
    }

    memset(item.payload, 0, sizeof(item.payload));
    memcpy(item.payload, payload, (size_t)payload_len);
    item.payload_len = payload_len;

    return write_item(&item);
}

int akonadi_delete_item(int64_t id)
{
    if (!s_initialized) {
        return -1;
    }

    char item_path[512];
    snprintf(item_path, sizeof(item_path),
             "%s/items/%lld.dat", s_data_dir, (long long)id);

    /* Read item to get collection_id for count update */
    akonadi_item_t item;
    if (read_item(item_path, &item) == 0) {
        update_collection_item_count(item.collection_id, -1);
    }

    if (unlink(item_path) < 0 && errno != ENOENT) {
        return -1;
    }

    fprintf(stderr, "[akonadi] Deleted item id=%lld\n", (long long)id);
    return 0;
}

bool akonadi_get_item(int64_t id, akonadi_item_t *item_out)
{
    if (!s_initialized || !item_out) {
        return false;
    }

    char item_path[512];
    snprintf(item_path, sizeof(item_path),
             "%s/items/%lld.dat", s_data_dir, (long long)id);

    return (read_item(item_path, item_out) == 0);
}

int akonadi_search_items(int64_t collection_id, const char *query,
                          const char *mime_type,
                          akonadi_item_t *out, int max)
{
    if (!s_initialized || !out || max <= 0) {
        return 0;
    }

    char items_dir[512];
    snprintf(items_dir, sizeof(items_dir), "%s/items", s_data_dir);

    DIR *dp = opendir(items_dir);
    if (!dp) {
        return 0;
    }

    int count = 0;
    struct dirent *ent;
    while ((ent = readdir(dp)) != NULL && count < max) {
        if (ent->d_name[0] == '.') {
            continue;
        }

        /* Only process .dat files */
        const char *ext = strrchr(ent->d_name, '.');
        if (!ext || strcmp(ext, ".dat") != 0) {
            continue;
        }

        char item_path[512];
        snprintf(item_path, sizeof(item_path),
                 "%s/%s", items_dir, ent->d_name);

        akonadi_item_t item;
        if (read_item(item_path, &item) < 0) {
            continue;
        }

        /* Filter by collection */
        if (collection_id > 0 && item.collection_id != collection_id) {
            continue;
        }

        /* Filter by MIME type */
        if (mime_type && mime_type[0] != '\0' &&
            strcmp(item.mime_type, mime_type) != 0) {
            continue;
        }

        /* Filter by query (substring search in payload) */
        if (query && query[0] != '\0') {
            if (strstr(item.payload, query) == NULL) {
                continue;
            }
        }

        out[count++] = item;
    }

    closedir(dp);
    return count;
}

int akonadi_get_collection_items(int64_t collection_id,
                                  akonadi_item_t *out, int max)
{
    return akonadi_search_items(collection_id, NULL, NULL, out, max);
}

/* ======================================================================
 * D-Bus service registration (stub)
 * ====================================================================== */

/*
 * In a full KDE environment, the Akonadi server would register on
 * D-Bus at org.freedesktop.Akonadi and expose the collection/item
 * API as D-Bus methods.
 *
 * For VeridianOS, the D-Bus registration is a stub that logs the
 * intended registration.  Clients use the C API directly via
 * shared library linking.
 */

static void register_dbus_service(void)
{
    fprintf(stderr, "[akonadi] D-Bus service: org.freedesktop.Akonadi\n");
    fprintf(stderr, "[akonadi]   Methods: CreateCollection, "
            "DeleteCollection, ListCollections\n");
    fprintf(stderr, "[akonadi]   Methods: AddItem, UpdateItem, "
            "DeleteItem, GetItem, SearchItems\n");
    fprintf(stderr, "[akonadi]   Note: D-Bus bridge is a stub; "
            "use C API via libakonadi-veridian.so\n");
}

/* ======================================================================
 * Entry point (standalone daemon mode)
 * ====================================================================== */

/*
 * When built as a standalone binary, the Akonadi data store runs as
 * a daemon that registers on D-Bus and serves requests.  When linked
 * as a library, applications call akonadi_init() directly.
 */

#ifdef AKONADI_STANDALONE

#include <signal.h>

static volatile sig_atomic_t s_running = 1;

static void handle_signal(int sig)
{
    (void)sig;
    s_running = 0;
}

int main(void)
{
    fprintf(stderr, "[akonadi] VeridianOS Akonadi PIM data store starting\n");

    const char *data_dir = getenv("AKONADI_DATA_DIR");
    if (!data_dir) {
        data_dir = "/var/lib/akonadi";
    }

    if (akonadi_init(data_dir) < 0) {
        fprintf(stderr, "[akonadi] Initialization failed\n");
        return 1;
    }

    register_dbus_service();

    /* Install signal handlers */
    signal(SIGTERM, handle_signal);
    signal(SIGINT, handle_signal);

    fprintf(stderr, "[akonadi] Ready -- serving requests\n");

    /* Main loop: wait for D-Bus requests or signals */
    while (s_running) {
        sleep(1);
    }

    akonadi_shutdown();
    fprintf(stderr, "[akonadi] Exiting\n");
    return 0;
}

#endif /* AKONADI_STANDALONE */
