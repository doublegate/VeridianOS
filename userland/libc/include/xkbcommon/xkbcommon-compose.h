/*
 * VeridianOS libc -- xkbcommon/xkbcommon-compose.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * xkbcommon compose (dead key) API.
 */

#ifndef _XKBCOMMON_XKBCOMMON_COMPOSE_H
#define _XKBCOMMON_XKBCOMMON_COMPOSE_H

#include <xkbcommon/xkbcommon.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

struct xkb_compose_table;
struct xkb_compose_state;

enum xkb_compose_compile_flags {
    XKB_COMPOSE_COMPILE_NO_FLAGS = 0
};

enum xkb_compose_format {
    XKB_COMPOSE_FORMAT_TEXT_V1 = 1
};

enum xkb_compose_state_flags {
    XKB_COMPOSE_STATE_NO_FLAGS = 0
};

enum xkb_compose_status {
    XKB_COMPOSE_NOTHING,
    XKB_COMPOSE_COMPOSING,
    XKB_COMPOSE_COMPOSED,
    XKB_COMPOSE_CANCELLED
};

enum xkb_compose_feed_result {
    XKB_COMPOSE_FEED_IGNORED,
    XKB_COMPOSE_FEED_ACCEPTED
};

/* ========================================================================= */
/* Compose table API                                                         */
/* ========================================================================= */

struct xkb_compose_table *
xkb_compose_table_new_from_locale(struct xkb_context *context,
                                    const char *locale,
                                    enum xkb_compose_compile_flags flags);

struct xkb_compose_table *
xkb_compose_table_new_from_file(struct xkb_context *context,
                                  void *file, /* FILE* */
                                  const char *locale,
                                  enum xkb_compose_format format,
                                  enum xkb_compose_compile_flags flags);

struct xkb_compose_table *
xkb_compose_table_new_from_buffer(struct xkb_context *context,
                                     const char *buffer, size_t length,
                                     const char *locale,
                                     enum xkb_compose_format format,
                                     enum xkb_compose_compile_flags flags);

struct xkb_compose_table *
xkb_compose_table_ref(struct xkb_compose_table *table);

void xkb_compose_table_unref(struct xkb_compose_table *table);

/* ========================================================================= */
/* Compose state API                                                         */
/* ========================================================================= */

struct xkb_compose_state *
xkb_compose_state_new(struct xkb_compose_table *table,
                        enum xkb_compose_state_flags flags);

struct xkb_compose_state *
xkb_compose_state_ref(struct xkb_compose_state *state);

void xkb_compose_state_unref(struct xkb_compose_state *state);

struct xkb_compose_table *
xkb_compose_state_get_compose_table(struct xkb_compose_state *state);

enum xkb_compose_feed_result
xkb_compose_state_feed(struct xkb_compose_state *state,
                         xkb_keysym_t keysym);

void xkb_compose_state_reset(struct xkb_compose_state *state);

enum xkb_compose_status
xkb_compose_state_get_status(struct xkb_compose_state *state);

int xkb_compose_state_get_utf8(struct xkb_compose_state *state,
                                  char *buffer, size_t size);

xkb_keysym_t
xkb_compose_state_get_one_sym(struct xkb_compose_state *state);

#ifdef __cplusplus
}
#endif

#endif /* _XKBCOMMON_XKBCOMMON_COMPOSE_H */
