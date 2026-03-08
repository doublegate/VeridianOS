/*
 * session-save-restore.h -- KDE Plasma session save/restore for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Provides session save and restore functionality for KDE Plasma 6 on
 * VeridianOS.  On logout/shutdown, the current window layout is saved
 * to the user's config directory.  On the next login, the saved state
 * is restored by re-launching applications and repositioning windows.
 */

#ifndef SESSION_SAVE_RESTORE_H
#define SESSION_SAVE_RESTORE_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Maximum number of windows that can be saved in a session */
#define SESSION_MAX_WINDOWS     64

/* Maximum command length for an application */
#define SESSION_MAX_COMMAND     256

/* Maximum activity ID length */
#define SESSION_MAX_ACTIVITY_ID 64

/* ======================================================================
 * Data types
 * ====================================================================== */

/* State of a single window at save time */
typedef struct {
    char     app_command[SESSION_MAX_COMMAND];
    int      x;
    int      y;
    int      width;
    int      height;
    int      desktop_number;
    bool     is_maximized;
    bool     is_minimized;
    char     activity_id[SESSION_MAX_ACTIVITY_ID];
} session_window_state_t;

/* Saved session data */
typedef struct {
    session_window_state_t  window_states[SESSION_MAX_WINDOWS];
    int                     window_count;
    int                     session_type;   /* 0 = plasma, 1 = builtin */
    int64_t                 last_save_time; /* Unix timestamp */
} session_data_t;

/* ======================================================================
 * API
 * ====================================================================== */

/*
 * Save the current session state (window positions, running apps).
 * Called on logout or shutdown.  Writes to config_dir/plasma-session/.
 * Returns 0 on success, -1 on error.
 */
int session_save(const char *config_dir);

/*
 * Restore a previously saved session.
 * Called on login.  Reads from config_dir/plasma-session/ and
 * re-launches applications with their saved positions.
 * Returns the number of windows restored, or -1 on error.
 */
int session_restore(const char *config_dir);

/*
 * Query the current window states from KWin via D-Bus.
 * Fills the states array (up to max entries).
 * Returns the number of windows found.
 */
int session_get_window_states(session_window_state_t *states, int max);

/*
 * Clear all saved session data for the given config directory.
 * Returns 0 on success, -1 on error.
 */
int session_clear_saved(const char *config_dir);

/*
 * Mark an application for autostart on next login.
 * Creates/removes a .desktop file in config_dir/autostart/.
 * Returns 0 on success, -1 on error.
 */
int session_set_autostart(const char *app_command, bool enabled);

#ifdef __cplusplus
}
#endif

#endif /* SESSION_SAVE_RESTORE_H */
