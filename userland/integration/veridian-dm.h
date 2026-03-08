/*
 * veridian-dm.h -- VeridianOS Display Manager / Session Selector
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Simple display manager providing:
 *   - Login prompt (username/password against UserDatabase)
 *   - Session type menu (VeridianOS built-in DE or KDE Plasma 6)
 *   - Session preference persistence (/etc/veridian/session.conf)
 *   - Auto-login support (configurable)
 *   - Wayland-native rendering on the kernel compositor
 *
 * This runs as a user-space process using the kernel's built-in
 * compositor for rendering the login screen.  After successful
 * authentication, it writes the session config and exec's the
 * appropriate session launcher.
 */

#ifndef VERIDIAN_DM_H
#define VERIDIAN_DM_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ======================================================================
 * Constants
 * ====================================================================== */

#define VDM_MAX_USERNAME    64
#define VDM_MAX_PASSWORD    128
#define VDM_MAX_SESSIONS    8
#define VDM_SESSION_CONF    "/etc/veridian/session.conf"
#define VDM_AUTOLOGIN_CONF  "/etc/veridian/autologin.conf"

/* Session types */
typedef enum {
    VDM_SESSION_BUILTIN = 0,   /* VeridianOS built-in kernel compositor */
    VDM_SESSION_PLASMA  = 1,   /* KDE Plasma 6 (KWin + plasmashell) */
    VDM_SESSION_COUNT
} vdm_session_type_t;

/* Authentication result */
typedef enum {
    VDM_AUTH_OK = 0,
    VDM_AUTH_BAD_USER,
    VDM_AUTH_BAD_PASS,
    VDM_AUTH_LOCKED,
    VDM_AUTH_ERROR
} vdm_auth_result_t;

/* DM state machine */
typedef enum {
    VDM_STATE_INIT = 0,
    VDM_STATE_LOGIN,           /* Showing username/password prompt */
    VDM_STATE_SESSION_SELECT,  /* Showing session type menu */
    VDM_STATE_AUTHENTICATING,  /* Verifying credentials */
    VDM_STATE_LAUNCHING,       /* Starting selected session */
    VDM_STATE_RUNNING,         /* Session is active */
    VDM_STATE_SHUTDOWN         /* Cleaning up */
} vdm_state_t;

/* Session descriptor */
typedef struct {
    vdm_session_type_t type;
    const char        *name;
    const char        *description;
    const char        *exec_path;
    const char        *icon_name;
} vdm_session_info_t;

/* Auto-login configuration */
typedef struct {
    bool    enabled;
    char    username[VDM_MAX_USERNAME];
    int     session_type;      /* vdm_session_type_t */
    int     delay_seconds;     /* 0 = immediate */
} vdm_autologin_t;

/* Display manager context */
typedef struct {
    vdm_state_t         state;
    char                username[VDM_MAX_USERNAME];
    char                password[VDM_MAX_PASSWORD];
    int                 selected_session;
    vdm_session_info_t  sessions[VDM_MAX_SESSIONS];
    int                 session_count;
    vdm_autologin_t     autologin;
    int                 login_attempts;
    int                 max_attempts;
    /* Framebuffer for rendering (kernel compositor surface) */
    uint32_t           *framebuffer;
    int                 fb_width;
    int                 fb_height;
    int                 fb_stride;
    /* Cursor position for text input */
    int                 cursor_x;
    int                 cursor_y;
    bool                password_mode;  /* Hide input characters */
} vdm_context_t;

/* ======================================================================
 * API
 * ====================================================================== */

/*
 * Initialize the display manager context.
 * Sets up available sessions, loads autologin config, acquires framebuffer.
 * Returns 0 on success, -1 on error.
 */
int vdm_init(vdm_context_t *ctx);

/*
 * Run the display manager main loop.
 * Handles input, rendering, authentication, and session launch.
 * Returns only when the session exits or on error.
 * Returns the exit code of the launched session.
 */
int vdm_run(vdm_context_t *ctx);

/*
 * Clean up display manager resources.
 */
void vdm_cleanup(vdm_context_t *ctx);

/*
 * Authenticate a user against the VeridianOS UserDatabase.
 * The UserDatabase stores (Hash256, salt) pairs with ct_eq_bytes
 * comparison (constant-time, introduced in v0.20.2).
 * Returns VDM_AUTH_OK on success.
 */
vdm_auth_result_t vdm_authenticate(const char *username,
                                    const char *password);

/*
 * Save session preference to /etc/veridian/session.conf.
 * Returns 0 on success, -1 on error.
 */
int vdm_save_session_pref(vdm_session_type_t type);

/*
 * Load session preference from /etc/veridian/session.conf.
 * Returns the saved session type, or VDM_SESSION_BUILTIN if
 * the config file doesn't exist.
 */
vdm_session_type_t vdm_load_session_pref(void);

/*
 * Load auto-login configuration from /etc/veridian/autologin.conf.
 * Returns true if auto-login is enabled and valid.
 */
bool vdm_load_autologin(vdm_autologin_t *cfg);

/*
 * Launch the selected session.
 * For "builtin": returns immediately (kernel compositor is already running).
 * For "plasma": exec's plasma-veridian-session.
 * Returns child PID on success, -1 on error.
 */
int vdm_launch_session(vdm_session_type_t type, const char *username);

/* ======================================================================
 * Rendering (uses kernel compositor framebuffer)
 * ====================================================================== */

/*
 * Render the login screen (username/password prompt + session menu).
 */
void vdm_render_login(vdm_context_t *ctx);

/*
 * Render a text string at (x, y) in the framebuffer.
 * Uses the kernel's built-in bitmap font (same as built-in DE).
 */
void vdm_render_text(vdm_context_t *ctx, int x, int y,
                     const char *text, uint32_t color);

/*
 * Render a filled rectangle.
 */
void vdm_render_rect(vdm_context_t *ctx, int x, int y,
                     int w, int h, uint32_t color);

/*
 * Handle a keyboard event from the kernel input subsystem.
 * Returns true if the event was consumed.
 */
bool vdm_handle_key(vdm_context_t *ctx, int keycode, bool pressed);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_DM_H */
