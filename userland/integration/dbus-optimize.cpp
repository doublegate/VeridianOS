/*
 * dbus-optimize.cpp -- D-Bus Performance Optimization for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Implementation of D-Bus optimization for KDE Plasma 6.  Reduces
 * context switches via message batching, credential caching, and
 * binary protocol shortcuts for same-process communication.
 */

#include "dbus-optimize.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* ======================================================================
 * Constants
 * ====================================================================== */

#define MAX_BATCH_QUEUE        256
#define MAX_BATCH_DEST_LEN     256
#define DEFAULT_MAX_BATCH_SIZE 32
#define MAX_CRED_CACHE         64
#define CRED_CACHE_TTL_S       60
#define MAX_LOCAL_SERVICES     32
#define MAX_BUS_NAME           128
#define MAX_STATS_SERVICES     64
#define STATS_LOG_INTERVAL_S   60

/* ======================================================================
 * Batch queue
 * ====================================================================== */

struct batch_entry {
    char dest[MAX_BATCH_DEST_LEN];
    struct dbus_opt_message msg;
    /* Inline copy of body data (up to 4 KB) */
    char body_data[4096];
    int valid;
};

static struct batch_entry g_batch_queue[MAX_BATCH_QUEUE];
static int g_batch_count = 0;
static uint32_t g_batch_interval_ms = 0;
static uint32_t g_max_batch_size = DEFAULT_MAX_BATCH_SIZE;
static int g_batching_enabled = 0;
static time_t g_last_batch_flush = 0;

/* ======================================================================
 * Credential cache
 * ====================================================================== */

struct cred_entry {
    char sender[MAX_BUS_NAME];
    uint32_t uid;
    uint32_t gid;
    uint32_t pid;
    time_t cached_at;
    int valid;
};

static struct cred_entry g_cred_cache[MAX_CRED_CACHE];
static int g_cred_count = 0;
static int g_cred_lru_idx = 0;   /* next eviction candidate */

/* ======================================================================
 * Binary shortcut (local service registry)
 * ====================================================================== */

typedef int (*local_handler_fn)(const char *interface, const char *method,
                                 const char *body, uint32_t body_len);

struct local_service {
    char bus_name[MAX_BUS_NAME];
    local_handler_fn handler;
    int active;
};

static struct local_service g_local_services[MAX_LOCAL_SERVICES];
static int g_local_count = 0;
static int g_binary_shortcut_enabled = 0;

/* ======================================================================
 * Statistics
 * ====================================================================== */

struct dbus_stats {
    uint64_t sent;
    uint64_t received;
    uint64_t batched;
    uint64_t batch_flushes;
    uint64_t cache_hits;
    uint64_t cache_misses;
    uint64_t shortcut_calls;
};

static struct dbus_stats g_stats;
static time_t g_last_stats_log = 0;
static int g_monitor_enabled = 0;

/* Global init flag */
static int g_initialized = 0;

/* ======================================================================
 * Internal helpers
 * ====================================================================== */

static void dbus_log(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    fprintf(stderr, "[dbus-opt] ");
    vfprintf(stderr, fmt, ap);
    fprintf(stderr, "\n");
    va_end(ap);
}

/* Flush all queued batch entries for a given destination. */
static int flush_batch_for_dest(const char *dest) {
    int flushed = 0;

    /* Count messages for this destination */
    int msg_count = 0;
    for (int i = 0; i < g_batch_count; i++) {
        if (g_batch_queue[i].valid &&
            strcmp(g_batch_queue[i].dest, dest) == 0) {
            msg_count++;
        }
    }

    if (msg_count == 0) {
        return 0;
    }

    /* In a real implementation, we would construct a single D-Bus
     * method call to org.freedesktop.DBus.BatchedCall containing
     * all msg_count messages serialized into the body.  The receiver
     * would demux them and process sequentially.
     *
     * For now we simulate sending them individually (fallback path)
     * while still counting the batch savings. */

    for (int i = 0; i < g_batch_count; i++) {
        if (!g_batch_queue[i].valid) {
            continue;
        }
        if (strcmp(g_batch_queue[i].dest, dest) != 0) {
            continue;
        }

        /* "Send" the message (simulated) */
        g_stats.sent++;
        g_stats.batched++;
        g_batch_queue[i].valid = 0;
        flushed++;
    }

    g_stats.batch_flushes++;

    /* Compact the queue */
    int write_idx = 0;
    for (int read_idx = 0; read_idx < g_batch_count; read_idx++) {
        if (g_batch_queue[read_idx].valid) {
            if (write_idx != read_idx) {
                g_batch_queue[write_idx] = g_batch_queue[read_idx];
            }
            write_idx++;
        }
    }
    g_batch_count = write_idx;

    return flushed;
}

/* Flush all pending batches (all destinations). */
static void flush_all_batches(void) {
    /* Collect unique destinations */
    char dests[MAX_BATCH_QUEUE][MAX_BATCH_DEST_LEN];
    int dest_count = 0;

    for (int i = 0; i < g_batch_count; i++) {
        if (!g_batch_queue[i].valid) {
            continue;
        }

        int dup = 0;
        for (int j = 0; j < dest_count; j++) {
            if (strcmp(dests[j], g_batch_queue[i].dest) == 0) {
                dup = 1;
                break;
            }
        }
        if (!dup && dest_count < MAX_BATCH_QUEUE) {
            snprintf(dests[dest_count], MAX_BATCH_DEST_LEN, "%s",
                     g_batch_queue[i].dest);
            dest_count++;
        }
    }

    for (int i = 0; i < dest_count; i++) {
        flush_batch_for_dest(dests[i]);
    }

    g_last_batch_flush = time(NULL);
}

/* Check if the batch timer has expired and flush if so. */
static void check_batch_timer(void) {
    if (!g_batching_enabled || g_batch_interval_ms == 0) {
        return;
    }

    time_t now = time(NULL);
    time_t interval_s = (time_t)(g_batch_interval_ms / 1000);
    if (interval_s < 1) {
        interval_s = 1;
    }

    if ((now - g_last_batch_flush) >= interval_s && g_batch_count > 0) {
        flush_all_batches();
    }
}

/* Evict expired credential cache entries. */
static void evict_expired_creds(void) {
    time_t now = time(NULL);
    for (int i = 0; i < MAX_CRED_CACHE; i++) {
        if (g_cred_cache[i].valid &&
            (now - g_cred_cache[i].cached_at) > CRED_CACHE_TTL_S) {
            g_cred_cache[i].valid = 0;
            if (g_cred_count > 0) {
                g_cred_count--;
            }
        }
    }
}

/* Find a credential cache entry by sender name.  Returns index or -1. */
static int find_cred(const char *sender) {
    for (int i = 0; i < MAX_CRED_CACHE; i++) {
        if (g_cred_cache[i].valid &&
            strcmp(g_cred_cache[i].sender, sender) == 0) {
            return i;
        }
    }
    return -1;
}

/* Log stats periodically if monitoring is enabled. */
static void maybe_log_stats(void) {
    if (!g_monitor_enabled) {
        return;
    }

    time_t now = time(NULL);
    if ((now - g_last_stats_log) >= STATS_LOG_INTERVAL_S) {
        dbus_log("Stats: sent=%lu recv=%lu batched=%lu flushes=%lu "
                 "cache_hits=%lu misses=%lu shortcuts=%lu",
                 (unsigned long)g_stats.sent,
                 (unsigned long)g_stats.received,
                 (unsigned long)g_stats.batched,
                 (unsigned long)g_stats.batch_flushes,
                 (unsigned long)g_stats.cache_hits,
                 (unsigned long)g_stats.cache_misses,
                 (unsigned long)g_stats.shortcut_calls);
        g_last_stats_log = now;
    }
}

/* ======================================================================
 * Public API
 * ====================================================================== */

void dbus_opt_init(void) {
    if (g_initialized) {
        return;
    }

    dbus_log("Initializing D-Bus optimization subsystem");

    memset(g_batch_queue, 0, sizeof(g_batch_queue));
    memset(g_cred_cache, 0, sizeof(g_cred_cache));
    memset(g_local_services, 0, sizeof(g_local_services));
    memset(&g_stats, 0, sizeof(g_stats));

    g_batch_count = 0;
    g_batch_interval_ms = 0;
    g_max_batch_size = DEFAULT_MAX_BATCH_SIZE;
    g_batching_enabled = 0;
    g_cred_count = 0;
    g_cred_lru_idx = 0;
    g_local_count = 0;
    g_binary_shortcut_enabled = 0;
    g_monitor_enabled = 1;
    g_last_batch_flush = time(NULL);
    g_last_stats_log = time(NULL);

    g_initialized = 1;
    dbus_log("D-Bus optimization ready");
}

void dbus_opt_enable_batching(uint32_t interval_ms) {
    if (!g_initialized) {
        return;
    }

    if (interval_ms == 0) {
        /* Disable batching -- flush any pending */
        if (g_batch_count > 0) {
            flush_all_batches();
        }
        g_batching_enabled = 0;
        g_batch_interval_ms = 0;
        dbus_log("Message batching disabled");
        return;
    }

    g_batch_interval_ms = interval_ms;
    g_batching_enabled = 1;
    g_last_batch_flush = time(NULL);

    dbus_log("Message batching enabled: window=%u ms, max_size=%u",
             interval_ms, g_max_batch_size);
}

int dbus_opt_send_batched(const char *dest,
                          const struct dbus_opt_message *messages,
                          uint32_t count) {
    if (!dest || !messages || count == 0 || !g_initialized) {
        return -1;
    }

    check_batch_timer();
    maybe_log_stats();

    /* If binary shortcut is available for this destination, use it */
    if (g_binary_shortcut_enabled) {
        for (int i = 0; i < g_local_count; i++) {
            if (g_local_services[i].active &&
                strcmp(g_local_services[i].bus_name, dest) == 0 &&
                g_local_services[i].handler) {
                /* Direct in-process call for each message */
                for (uint32_t m = 0; m < count; m++) {
                    g_local_services[i].handler(
                        messages[m].interface,
                        messages[m].method,
                        messages[m].body,
                        messages[m].body_len);
                    g_stats.shortcut_calls++;
                    g_stats.sent++;
                }
                return 0;
            }
        }
    }

    /* If batching is disabled, send immediately */
    if (!g_batching_enabled) {
        for (uint32_t m = 0; m < count; m++) {
            /* Simulated immediate send */
            g_stats.sent++;
        }
        return 0;
    }

    /* Queue messages for batched send */
    for (uint32_t m = 0; m < count; m++) {
        if (g_batch_count >= MAX_BATCH_QUEUE) {
            /* Queue full -- flush everything first */
            flush_all_batches();
        }

        struct batch_entry *e = &g_batch_queue[g_batch_count];
        snprintf(e->dest, MAX_BATCH_DEST_LEN, "%s", dest);
        e->msg.interface = messages[m].interface;
        e->msg.method = messages[m].method;

        /* Copy body data inline */
        uint32_t copy_len = messages[m].body_len;
        if (copy_len > sizeof(e->body_data)) {
            copy_len = sizeof(e->body_data);
        }
        if (messages[m].body && copy_len > 0) {
            memcpy(e->body_data, messages[m].body, copy_len);
        }
        e->msg.body = e->body_data;
        e->msg.body_len = copy_len;
        e->valid = 1;
        g_batch_count++;

        /* Auto-flush if batch size limit reached for this destination */
        int dest_count = 0;
        for (int i = 0; i < g_batch_count; i++) {
            if (g_batch_queue[i].valid &&
                strcmp(g_batch_queue[i].dest, dest) == 0) {
                dest_count++;
            }
        }
        if ((uint32_t)dest_count >= g_max_batch_size) {
            flush_batch_for_dest(dest);
        }
    }

    return 0;
}

void dbus_opt_enable_binary_shortcut(void) {
    if (!g_initialized) {
        return;
    }

    g_binary_shortcut_enabled = 1;

    /* Pre-register well-known local services that Plasma components
     * frequently call within the same process.  In practice, these
     * handlers would be registered by the respective components
     * during startup. */

    /* Placeholder registrations -- real handlers set by components */
    if (g_local_count < MAX_LOCAL_SERVICES) {
        struct local_service *s = &g_local_services[g_local_count];
        snprintf(s->bus_name, MAX_BUS_NAME, "org.kde.KWin");
        s->handler = NULL;   /* set by KWin on init */
        s->active = 0;       /* inactive until handler registered */
        g_local_count++;
    }

    if (g_local_count < MAX_LOCAL_SERVICES) {
        struct local_service *s = &g_local_services[g_local_count];
        snprintf(s->bus_name, MAX_BUS_NAME, "org.kde.plasmashell");
        s->handler = NULL;
        s->active = 0;
        g_local_count++;
    }

    if (g_local_count < MAX_LOCAL_SERVICES) {
        struct local_service *s = &g_local_services[g_local_count];
        snprintf(s->bus_name, MAX_BUS_NAME, "org.kde.kded6");
        s->handler = NULL;
        s->active = 0;
        g_local_count++;
    }

    dbus_log("Binary protocol shortcut enabled (%d local services registered)",
             g_local_count);
}

void dbus_opt_cache_credentials(const char *sender) {
    if (!sender || !g_initialized) {
        return;
    }

    /* Check if already cached */
    int idx = find_cred(sender);
    if (idx >= 0) {
        /* Refresh TTL */
        g_cred_cache[idx].cached_at = time(NULL);
        g_stats.cache_hits++;
        return;
    }

    /* Evict expired entries first */
    evict_expired_creds();

    /* Find a free slot or use LRU eviction */
    int slot = -1;
    for (int i = 0; i < MAX_CRED_CACHE; i++) {
        if (!g_cred_cache[i].valid) {
            slot = i;
            break;
        }
    }

    if (slot < 0) {
        /* LRU eviction: use round-robin index */
        slot = g_cred_lru_idx;
        g_cred_lru_idx = (g_cred_lru_idx + 1) % MAX_CRED_CACHE;
    }

    struct cred_entry *ce = &g_cred_cache[slot];
    snprintf(ce->sender, MAX_BUS_NAME, "%s", sender);

    /* In a real implementation, we would extract UID/GID/PID from
     * the D-Bus daemon's GetConnectionCredentials() call.  Here we
     * use placeholder values. */
    ce->uid = 1000;   /* typical desktop user */
    ce->gid = 1000;
    ce->pid = 0;      /* would come from SCM_CREDENTIALS */
    ce->cached_at = time(NULL);
    ce->valid = 1;

    if (g_cred_count < MAX_CRED_CACHE) {
        g_cred_count++;
    }

    g_stats.cache_misses++;
    dbus_log("Cached credentials for '%s' (slot %d, total %d)",
             sender, slot, g_cred_count);
}

void dbus_opt_get_stats(uint64_t *sent, uint64_t *received,
                        uint64_t *batched, uint64_t *cache_hits) {
    if (sent) {
        *sent = g_stats.sent;
    }
    if (received) {
        *received = g_stats.received;
    }
    if (batched) {
        *batched = g_stats.batched;
    }
    if (cache_hits) {
        *cache_hits = g_stats.cache_hits;
    }
}

void dbus_opt_set_max_batch_size(uint32_t count) {
    if (!g_initialized) {
        return;
    }

    if (count == 0) {
        count = 1;
    }
    if (count > MAX_BATCH_QUEUE) {
        count = MAX_BATCH_QUEUE;
    }

    g_max_batch_size = count;
    dbus_log("Max batch size set to %u", count);
}

void dbus_opt_shutdown(void) {
    if (!g_initialized) {
        return;
    }

    dbus_log("Shutting down D-Bus optimization subsystem");

    /* Flush any pending batches */
    if (g_batch_count > 0) {
        dbus_log("Flushing %d pending batched messages", g_batch_count);
        flush_all_batches();
    }

    /* Log final stats */
    dbus_log("Final stats: sent=%lu recv=%lu batched=%lu "
             "flushes=%lu cache_hits=%lu misses=%lu shortcuts=%lu",
             (unsigned long)g_stats.sent,
             (unsigned long)g_stats.received,
             (unsigned long)g_stats.batched,
             (unsigned long)g_stats.batch_flushes,
             (unsigned long)g_stats.cache_hits,
             (unsigned long)g_stats.cache_misses,
             (unsigned long)g_stats.shortcut_calls);

    /* Clear all state */
    memset(g_batch_queue, 0, sizeof(g_batch_queue));
    memset(g_cred_cache, 0, sizeof(g_cred_cache));
    memset(g_local_services, 0, sizeof(g_local_services));
    memset(&g_stats, 0, sizeof(g_stats));

    g_batch_count = 0;
    g_cred_count = 0;
    g_local_count = 0;
    g_batching_enabled = 0;
    g_binary_shortcut_enabled = 0;
    g_monitor_enabled = 0;
    g_initialized = 0;

    dbus_log("D-Bus optimization shutdown complete");
}
