/*
 * VeridianOS -- activities-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KDE Activities backend for VeridianOS.
 *
 * Activities provide virtual workspace grouping: each activity has its
 * own set of associated windows, allowing users to organize work by
 * context (e.g., "Development", "Communication", "Design").
 *
 * Activity configurations are persisted to /etc/veridian/activities/.
 * Window-to-activity associations are tracked in memory and survive
 * activity switches.
 *
 * D-Bus service: org.kde.ActivityManager
 */

#ifndef ACTIVITIES_VERIDIAN_H
#define ACTIVITIES_VERIDIAN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Activity state                                                            */
/* ========================================================================= */

/**
 * Lifecycle state of an activity.
 */
typedef enum {
    ACTIVITY_STOPPED  = 0,  /* Not running */
    ACTIVITY_STARTING = 1,  /* Transitioning to running */
    ACTIVITY_RUNNING  = 2,  /* Active and usable */
    ACTIVITY_STOPPING = 3   /* Transitioning to stopped */
} ActivityState;

/* ========================================================================= */
/* Activity descriptor                                                       */
/* ========================================================================= */

/**
 * Describes a single activity.
 *
 * The `id` is a unique identifier generated on creation (timestamp-based
 * UUID-style string).
 */
typedef struct {
    char id[64];            /* Unique activity identifier */
    char name[128];         /* Human-readable name */
    char description[256];  /* Optional description */
    char icon[128];         /* Icon name (freedesktop icon spec) */
    ActivityState state;    /* Current lifecycle state */
    int  is_current;        /* Non-zero if this is the active activity */
} Activity;

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

/**
 * Initialize the activities subsystem.
 * Loads saved activity configs from /etc/veridian/activities/.
 * Creates the "Default" activity if none exist.
 *
 * @return 0 on success, -1 on error.
 */
int activities_init(void);

/**
 * Shut down activities and save state.
 */
void activities_destroy(void);

/* ========================================================================= */
/* Activity management                                                       */
/* ========================================================================= */

/**
 * Create a new activity.
 *
 * @param name         Display name for the activity.
 * @param description  Optional description (may be NULL).
 * @param icon         Icon name (may be NULL for default).
 * @return             Activity ID string, or NULL on error.
 *                     Valid until activities_destroy().
 */
const char *activities_create(const char *name,
                              const char *description,
                              const char *icon);

/**
 * Delete an activity by ID.
 *
 * Windows assigned to the deleted activity are reassigned to the
 * current activity.  Cannot delete the last remaining activity.
 */
void activities_delete(const char *id);

/**
 * Switch to a different activity (make it current).
 *
 * Windows not belonging to the new activity are hidden;
 * windows belonging to it are shown.
 */
void activities_switch(const char *id);

/**
 * Return the ID of the currently active activity.
 * Valid until activities_destroy().
 */
const char *activities_get_current(void);

/**
 * List all activities.
 *
 * @param out  Caller-allocated array to receive Activity descriptors.
 * @param max  Size of the out array.
 * @return     Number of activities written (0..max).
 */
int activities_list(Activity *out, int max);

/* ========================================================================= */
/* Activity properties                                                       */
/* ========================================================================= */

/**
 * Update the display name of an activity.
 */
void activities_set_name(const char *id, const char *name);

/**
 * Update the icon of an activity.
 */
void activities_set_icon(const char *id, const char *icon);

/* ========================================================================= */
/* Window association                                                        */
/* ========================================================================= */

/**
 * Associate a window with an activity.
 * A window can be associated with multiple activities.
 */
void activities_add_window(const char *id, uint32_t window_id);

/**
 * Remove a window's association with an activity.
 */
void activities_remove_window(const char *id, uint32_t window_id);

/**
 * List windows associated with an activity.
 *
 * @param id   Activity ID.
 * @param out  Caller-allocated array to receive window IDs.
 * @param max  Size of the out array.
 * @return     Number of window IDs written.
 */
int activities_get_windows(const char *id, uint32_t *out, int max);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* ACTIVITIES_VERIDIAN_H */
