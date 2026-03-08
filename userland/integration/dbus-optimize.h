/*
 * dbus-optimize.h -- D-Bus Performance Optimization for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * D-Bus optimization layer for KDE Plasma 6 on VeridianOS.  Reduces
 * context switches and syscall overhead for chatty Plasma components
 * via:
 *   - Message batching (accumulate + flush to same destination)
 *   - Binary protocol shortcut for same-process D-Bus calls
 *   - Credential caching (avoid per-message SCM_CREDENTIALS)
 *   - Performance counter tracking
 *
 * Usage:
 *   dbus_opt_init();
 *   dbus_opt_enable_batching(5);   // 5ms batch window
 *   dbus_opt_enable_binary_shortcut();
 *   dbus_opt_cache_credentials("org.kde.plasmashell");
 *   dbus_opt_shutdown();
 */

#ifndef VERIDIAN_DBUS_OPTIMIZE_H
#define VERIDIAN_DBUS_OPTIMIZE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque message handle for batched sends. */
struct dbus_opt_message {
    const char *interface;
    const char *method;
    const char *body;        /* serialized argument blob */
    uint32_t body_len;
};

/* Initialize the D-Bus optimization subsystem.  Must be called once
 * before any other dbus_opt_* function. */
void dbus_opt_init(void);

/* Enable message batching.  Messages destined for the same service
 * are accumulated during a batch window of interval_ms milliseconds,
 * then flushed as a single D-Bus call.
 * @param interval_ms  Batch window in milliseconds (0 to disable) */
void dbus_opt_enable_batching(uint32_t interval_ms);

/* Send multiple messages to the same destination as a single batched
 * D-Bus call (org.freedesktop.DBus.BatchedCall).  Falls back to
 * individual sends if the receiver does not support batching.
 * @param dest      D-Bus bus name of the receiver
 * @param messages  Array of messages to send
 * @param count     Number of messages in the array
 * @return 0 on success, -1 on error */
int dbus_opt_send_batched(const char *dest,
                          const struct dbus_opt_message *messages,
                          uint32_t count);

/* Enable binary protocol shortcut for same-process D-Bus calls.
 * Registers an in-process handler that skips the socket roundtrip
 * for calls where both sender and receiver share the same address
 * space. */
void dbus_opt_enable_binary_shortcut(void);

/* Cache the UID/GID/PID credentials for a D-Bus sender.  Subsequent
 * messages from the same sender skip the SCM_CREDENTIALS ancillary
 * message lookup for 60 seconds or until disconnect.
 * @param sender  D-Bus unique name (e.g. ":1.42") or well-known name */
void dbus_opt_cache_credentials(const char *sender);

/* Retrieve performance counters.  Any pointer may be NULL if the
 * caller is not interested in that counter.
 * @param sent         Total messages sent (including batched)
 * @param received     Total messages received
 * @param batched      Messages sent via batching
 * @param cache_hits   Credential cache hits */
void dbus_opt_get_stats(uint64_t *sent, uint64_t *received,
                        uint64_t *batched, uint64_t *cache_hits);

/* Set the maximum number of messages that can be accumulated in a
 * single batch before an automatic flush is triggered.
 * @param count  Maximum batch size (default 32) */
void dbus_opt_set_max_batch_size(uint32_t count);

/* Shut down the D-Bus optimization subsystem.  Flushes any pending
 * batched messages and releases all resources. */
void dbus_opt_shutdown(void);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_DBUS_OPTIMIZE_H */
