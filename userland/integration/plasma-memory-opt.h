/*
 * plasma-memory-opt.h -- Plasma Memory Optimization for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Memory optimization infrastructure for reducing KDE Plasma 6 session
 * footprint on VeridianOS.  Targets ~800MB -> ~500MB RSS reduction via:
 *   - Lazy KF6 plugin loading (dlopen on first use)
 *   - Shared library deduplication analysis
 *   - Periodic cache cleanup (QPixmapCache, font cache, icon theme)
 *   - RSS monitoring with threshold warnings
 *   - Detailed per-component memory reporting
 *
 * Usage:
 *   plasma_mem_init();
 *   plasma_mem_enable_monitoring(5000);   // log RSS every 5s
 *   void *h = plasma_mem_lazy_load_plugin("kio_file");
 *   plasma_mem_cleanup_caches();
 *   plasma_mem_report();
 */

#ifndef VERIDIAN_PLASMA_MEMORY_OPT_H
#define VERIDIAN_PLASMA_MEMORY_OPT_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Initialize the memory optimization subsystem.  Must be called once
 * before any other plasma_mem_* function.  Sets up the plugin registry,
 * baseline RSS measurement, and internal bookkeeping. */
void plasma_mem_init(void);

/* Load a KF6 plugin on demand via dlopen().  Returns a handle suitable
 * for dlsym(), or NULL on failure.  Subsequent calls with the same
 * name return the cached handle without re-loading.
 * @param name  Plugin base name (e.g. "kio_file", "plasma_applet_taskbar") */
void *plasma_mem_lazy_load_plugin(const char *name);

/* Unload a previously loaded plugin via dlclose().  The cached handle
 * is invalidated; the next lazy_load_plugin() call will re-open it.
 * @param handle  Handle returned by plasma_mem_lazy_load_plugin() */
void plasma_mem_unload_plugin(void *handle);

/* Scan /proc/self/maps for duplicate shared library mappings (same
 * inode mapped at multiple virtual addresses) and log potential
 * consolidation savings.  Does not modify mappings at runtime. */
void plasma_mem_dedup_libs(void);

/* Flush transient caches to reclaim memory:
 *   - QPixmapCache (rendered pixmap cache, typically 20-50 MB)
 *   - Font glyph cache (trim to max 64 entries)
 *   - Icon theme cache (remove stale entries older than 24 hours)
 *   - KConfig object cache (release unused KConfig instances) */
void plasma_mem_cleanup_caches(void);

/* Read current RSS (Resident Set Size) in bytes from /proc/self/statm.
 * Returns 0 on failure (e.g. procfs unavailable). */
uint64_t plasma_mem_get_rss(void);

/* Estimated bytes saved since plasma_mem_init(), based on plugins
 * that were deferred and caches that were flushed. */
uint64_t plasma_mem_get_savings(void);

/* Enable periodic RSS monitoring.  A background timer logs current
 * RSS every interval_ms milliseconds and warns if RSS exceeds the
 * configured threshold (default 600 MB).
 * @param interval_ms  Monitoring interval in milliseconds */
void plasma_mem_enable_monitoring(uint32_t interval_ms);

/* Print a detailed memory breakdown to stderr:
 *   - Per-category: libraries, heap, stack, mmap, shared
 *   - Per-component: KWin compositor, Plasma shell, KF6, Qt6
 *   - Savings summary and optimization recommendations */
void plasma_mem_report(void);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_PLASMA_MEMORY_OPT_H */
