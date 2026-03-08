/*
 * xwayland-veridian.h -- XWayland Integration for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Manages the XWayland server for X11 application compatibility under
 * KWin on VeridianOS.  Provides:
 *   - Xwayland process lifecycle (on-demand launch via KWin)
 *   - X11 socket management (/tmp/.X11-unix/X0)
 *   - Xauthority file management
 *   - Window reparenting bridge (X11 windows as Wayland subsurfaces)
 *   - Clipboard sharing (X11 CLIPBOARD <-> Wayland wl_data_device)
 *   - Input forwarding (Wayland keyboard/pointer -> X11 events)
 *   - DISPLAY environment variable management
 *
 * This module is used by KWin's VeridianOS platform backend to manage
 * X11 compatibility.  KWin calls xwl_start() when an X11 application
 * is detected, and xwl_stop() on session shutdown.
 */

#ifndef XWAYLAND_VERIDIAN_H
#define XWAYLAND_VERIDIAN_H

#include <stdint.h>
#include <stdbool.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ======================================================================
 * Constants
 * ====================================================================== */

#define XWL_DISPLAY_DEFAULT     0        /* :0 */
#define XWL_DISPLAY_MAX         63       /* Maximum display number */
#define XWL_SOCKET_DIR          "/tmp/.X11-unix"
#define XWL_XAUTH_FILE         "/tmp/.Xauthority-veridian"
#define XWL_XWAYLAND_PATH      "/usr/bin/Xwayland"
#define XWL_MAX_WINDOWS         256
#define XWL_CLIPBOARD_MAX_SIZE  (16 * 1024 * 1024)  /* 16 MB */

/* ======================================================================
 * Types
 * ====================================================================== */

/* XWayland server state */
typedef enum {
    XWL_STATE_STOPPED = 0,
    XWL_STATE_STARTING,
    XWL_STATE_RUNNING,
    XWL_STATE_STOPPING,
    XWL_STATE_ERROR
} xwl_state_t;

/* Clipboard direction */
typedef enum {
    XWL_CLIP_X11_TO_WAYLAND = 0,
    XWL_CLIP_WAYLAND_TO_X11
} xwl_clipboard_dir_t;

/* Clipboard data type */
typedef enum {
    XWL_CLIP_TEXT_PLAIN = 0,
    XWL_CLIP_TEXT_HTML,
    XWL_CLIP_IMAGE_PNG,
    XWL_CLIP_IMAGE_BMP,
    XWL_CLIP_URI_LIST
} xwl_clip_type_t;

/* Forward declarations */
struct wl_surface;
struct wl_seat;

/* X11 window mapping to Wayland surface */
typedef struct {
    uint32_t    x11_window_id;      /* X11 Window XID */
    uint32_t    wayland_surface_id; /* Wayland wl_surface ID */
    int         x, y;               /* Position */
    int         width, height;      /* Size */
    bool        mapped;             /* Whether the window is visible */
    bool        override_redirect;  /* X11 override-redirect flag */
    char        title[256];         /* Window title (WM_NAME) */
    char        wm_class[128];      /* WM_CLASS for app matching */
} xwl_window_t;

/* Clipboard buffer */
typedef struct {
    xwl_clip_type_t type;
    uint8_t        *data;
    size_t          size;
    bool            valid;
} xwl_clipboard_t;

/* XWayland server context */
typedef struct {
    xwl_state_t     state;
    pid_t           server_pid;         /* Xwayland process PID */
    int             display_number;     /* X11 display number (:0, :1, ...) */
    int             wm_fd;              /* Window manager socket fd */
    int             x11_fd;             /* X11 connection socket fd */

    /* Window tracking */
    xwl_window_t    windows[XWL_MAX_WINDOWS];
    int             window_count;

    /* Clipboard */
    xwl_clipboard_t clipboard;

    /* Wayland integration (opaque pointers to KWin internals) */
    void           *wayland_display;    /* struct wl_display * */
    void           *wayland_compositor; /* struct wl_compositor * */
    void           *wayland_seat;       /* struct wl_seat * */

    /* Socket paths */
    char            socket_path[128];   /* /tmp/.X11-unix/X0 */
    char            xauth_path[128];    /* Xauthority file path */
    char            display_str[16];    /* ":0" string */

    /* Configuration */
    bool            enable_clipboard;
    bool            enable_dri3;        /* DRI3 for GPU acceleration */
    bool            rootless;           /* Rootless mode (no root window) */
} xwl_context_t;

/* ======================================================================
 * Server Lifecycle
 * ====================================================================== */

/*
 * Initialize the XWayland integration context.
 * Must be called before xwl_start().
 * Returns 0 on success, -1 on error.
 */
int xwl_init(xwl_context_t *ctx);

/*
 * Start the Xwayland server.
 * Finds an available display number, sets up sockets, launches
 * the Xwayland binary, and waits for it to be ready.
 * Returns 0 on success, -1 on error.
 */
int xwl_start(xwl_context_t *ctx);

/*
 * Stop the Xwayland server.
 * Sends SIGTERM, waits for graceful exit, then SIGKILL if needed.
 * Cleans up sockets and Xauthority file.
 * Returns 0 on success, -1 on error.
 */
int xwl_stop(xwl_context_t *ctx);

/*
 * Check if the Xwayland server is running and healthy.
 * Returns true if the server is responsive.
 */
bool xwl_is_running(const xwl_context_t *ctx);

/* ======================================================================
 * Window Management
 * ====================================================================== */

/*
 * Register a new X11 window and create a corresponding Wayland subsurface.
 * Called by KWin when a new X11 window is created.
 * Returns the window slot index, or -1 on error.
 */
int xwl_window_created(xwl_context_t *ctx, uint32_t x11_id,
                       int x, int y, int width, int height,
                       bool override_redirect);

/*
 * Handle X11 window destruction.
 * Removes the window mapping and destroys the Wayland subsurface.
 */
void xwl_window_destroyed(xwl_context_t *ctx, uint32_t x11_id);

/*
 * Handle X11 window resize/move.
 * Updates the Wayland subsurface position and size.
 */
void xwl_window_configure(xwl_context_t *ctx, uint32_t x11_id,
                          int x, int y, int width, int height);

/*
 * Handle X11 window map/unmap (show/hide).
 */
void xwl_window_set_mapped(xwl_context_t *ctx, uint32_t x11_id,
                           bool mapped);

/*
 * Find a window by X11 window ID.
 * Returns NULL if not found.
 */
const xwl_window_t *xwl_find_window(const xwl_context_t *ctx,
                                     uint32_t x11_id);

/* ======================================================================
 * Clipboard Sharing
 * ====================================================================== */

/*
 * Transfer clipboard data between X11 and Wayland.
 * direction: XWL_CLIP_X11_TO_WAYLAND or XWL_CLIP_WAYLAND_TO_X11
 * Returns 0 on success, -1 on error.
 */
int xwl_clipboard_transfer(xwl_context_t *ctx, xwl_clipboard_dir_t dir);

/*
 * Set clipboard data (from Wayland side, to be offered to X11 apps).
 * Makes a copy of the data.
 * Returns 0 on success, -1 on error.
 */
int xwl_clipboard_set(xwl_context_t *ctx, xwl_clip_type_t type,
                      const uint8_t *data, size_t size);

/*
 * Get clipboard data (from X11 side, previously set by an X11 app).
 * Returns a pointer to internal buffer (valid until next set/transfer).
 * Sets *size to the data size.  Returns NULL if no data available.
 */
const uint8_t *xwl_clipboard_get(const xwl_context_t *ctx,
                                  xwl_clip_type_t *type,
                                  size_t *size);

/*
 * Clear clipboard data and free buffer.
 */
void xwl_clipboard_clear(xwl_context_t *ctx);

/* ======================================================================
 * Input Forwarding
 * ====================================================================== */

/*
 * Forward a Wayland keyboard event to the focused X11 window.
 * keycode: Linux evdev keycode
 * pressed: true for key down, false for key up
 * modifiers: bitmask of active modifiers
 */
void xwl_forward_key(xwl_context_t *ctx, uint32_t keycode,
                     bool pressed, uint32_t modifiers);

/*
 * Forward a Wayland pointer motion event to X11.
 * x, y: pointer position relative to the X11 root window
 */
void xwl_forward_pointer_motion(xwl_context_t *ctx, int x, int y);

/*
 * Forward a Wayland pointer button event to X11.
 * button: Linux evdev button code
 * pressed: true for button down, false for button up
 */
void xwl_forward_pointer_button(xwl_context_t *ctx, uint32_t button,
                                bool pressed);

/*
 * Forward a Wayland scroll (axis) event to X11.
 * axis: 0 = vertical, 1 = horizontal
 * value: scroll amount (positive = down/right)
 */
void xwl_forward_scroll(xwl_context_t *ctx, int axis, int value);

/* ======================================================================
 * Environment
 * ====================================================================== */

/*
 * Set the DISPLAY environment variable for child processes.
 * Uses the display number from the context.
 * Returns the DISPLAY string (e.g., ":0").
 */
const char *xwl_get_display(const xwl_context_t *ctx);

/*
 * Set the XAUTHORITY environment variable for child processes.
 * Returns the Xauthority file path.
 */
const char *xwl_get_xauthority(const xwl_context_t *ctx);

#ifdef __cplusplus
}
#endif

#endif /* XWAYLAND_VERIDIAN_H */
