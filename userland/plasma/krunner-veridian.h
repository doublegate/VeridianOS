/*
 * VeridianOS -- krunner-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KRunner search and launch framework for KDE Plasma on VeridianOS.
 *
 * Provides a universal search interface with pluggable runners for
 * applications, files, calculator, commands, web search, and bookmarks.
 * Results are ranked by relevance and returned as KRunnerMatch structs.
 *
 * D-Bus service: org.kde.krunner
 */

#ifndef KRUNNER_VERIDIAN_H
#define KRUNNER_VERIDIAN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Match types                                                               */
/* ========================================================================= */

/**
 * Classification of a KRunner match result.
 */
typedef enum {
    KRUNNER_EXACT_MATCH         = 0,  /* Query matches result exactly */
    KRUNNER_POSSIBLE_MATCH      = 1,  /* Strong but not exact match */
    KRUNNER_INFORMATIONAL_MATCH = 2,  /* Display-only result (e.g., calculator) */
    KRUNNER_HELPER_MATCH        = 3,  /* Helper/action match */
    KRUNNER_COMPLETION_MATCH    = 4   /* Auto-complete suggestion */
} KRunnerMatchType;

/* ========================================================================= */
/* KRunnerMatch -- a single search result                                    */
/* ========================================================================= */

/**
 * A single match returned by a KRunner query.
 *
 * Matches are sorted by relevance (descending).  The `data` field carries
 * runner-specific payload used to execute the match (e.g., a .desktop path,
 * a URL, or a shell command).
 */
typedef struct {
    char text[256];          /* Primary display text */
    char subtext[256];       /* Secondary description */
    char icon_name[128];     /* Icon name (freedesktop icon spec) */
    int  relevance;          /* 0-100 relevance score */
    KRunnerMatchType match_type;
    char data[512];          /* Runner-specific execution payload */
} KRunnerMatch;

/* ========================================================================= */
/* KRunnerQuery -- search query parameters                                   */
/* ========================================================================= */

/**
 * Input query for a KRunner search.
 *
 * If runner_filter is non-empty, only the named runner is queried.
 * Otherwise all enabled runners participate.
 */
typedef struct {
    char query_string[512];  /* User-typed search text */
    char runner_filter[64];  /* Optional: restrict to single runner */
} KRunnerQuery;

/* ========================================================================= */
/* Built-in runners                                                          */
/* ========================================================================= */

/*
 * Available runner names (passed to krunner_enable_runner /
 * krunner_disable_runner):
 *
 *   "applications"  -- .desktop file search
 *   "files"         -- File name / content search (Baloo delegate)
 *   "calculator"    -- Simple arithmetic evaluation
 *   "commands"      -- Shell command execution (prefix ">")
 *   "websearch"     -- Web search URL construction
 *   "bookmarks"     -- Browser bookmark search
 */

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

/**
 * Initialize KRunner subsystem.  Loads runner configuration and
 * registers the org.kde.krunner D-Bus service.
 *
 * Returns 0 on success, -1 on error.
 */
int krunner_init(void);

/**
 * Shut down KRunner and release all resources.
 */
void krunner_destroy(void);

/* ========================================================================= */
/* Query interface                                                           */
/* ========================================================================= */

/**
 * Execute a search query across all enabled runners.
 *
 * @param query_string  Search text typed by the user.
 * @param matches_out   Caller-allocated array to receive results.
 * @param max_matches   Size of matches_out array.
 * @return              Number of matches written (0..max_matches), or -1 on error.
 *
 * Results are sorted by relevance descending, capped at 10 per runner.
 */
int krunner_query(const char *query_string,
                  KRunnerMatch *matches_out,
                  int max_matches);

/**
 * Execute the selected match.
 *
 * The action depends on the originating runner (launch app, open file,
 * run command, open URL, etc.).
 */
void krunner_run(const char *match_data);

/* ========================================================================= */
/* Runner management                                                         */
/* ========================================================================= */

/**
 * Return a NULL-terminated list of available runner names.
 * The returned pointer is valid until krunner_destroy().
 */
const char **krunner_get_runners(void);

/**
 * Enable a runner by name.  Enabled runners participate in queries.
 */
void krunner_enable_runner(const char *name);

/**
 * Disable a runner by name.  Disabled runners are skipped during queries.
 */
void krunner_disable_runner(const char *name);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* KRUNNER_VERIDIAN_H */
