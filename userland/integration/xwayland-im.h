/*
 * xwayland-im.h -- XIM (X Input Method) Bridge for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Bridges the X Input Method (XIM) protocol with the Wayland
 * text-input-v3 protocol for input method support under XWayland.
 *
 * Architecture:
 *   Wayland IM framework (IBus/Fcitx via zwp_text_input_v3)
 *     -> xim_bridge (this module)
 *       -> XIM server protocol to X11 clients
 *
 * The bridge acts as an XIM server for X11 applications, forwarding
 * input method events (preedit, commit) from the Wayland-side input
 * method to X11 clients via the XIM protocol.
 */

#ifndef XWAYLAND_IM_H
#define XWAYLAND_IM_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ======================================================================
 * Constants
 * ====================================================================== */

#define XIM_MAX_PREEDIT_LEN     256     /* Max preedit string (UTF-8 bytes) */
#define XIM_MAX_COMMIT_LEN      128     /* Max commit string (UTF-8 bytes) */
#define XIM_MAX_CLIENTS         32      /* Max simultaneous XIM clients */

/* XIM interaction styles (from X11/Xlib.h) */
#define XIM_PREEDIT_NOTHING     0x0004
#define XIM_PREEDIT_CALLBACKS   0x0002
#define XIM_PREEDIT_POSITION    0x0001
#define XIM_STATUS_NOTHING      0x0400
#define XIM_STATUS_CALLBACKS    0x0200

/* ======================================================================
 * Types
 * ====================================================================== */

/* XIM style bitmask */
typedef uint32_t XIMStyle;

/* Preedit draw callback data */
typedef struct {
    int         caret;          /* Cursor position in preedit string */
    int         change_first;   /* Start of changed region */
    int         change_length;  /* Length of changed region */
    char        text[XIM_MAX_PREEDIT_LEN];  /* UTF-8 preedit text */
    int         text_length;    /* Length of text in bytes */
} xim_preedit_draw_t;

/* XIM client state (one per X11 window with IM focus) */
typedef struct {
    uint32_t    x11_window;     /* X11 Window XID */
    XIMStyle    style;          /* Negotiated interaction style */
    bool        focused;        /* Whether this IC has input focus */
    bool        active;         /* Whether this client slot is in use */

    /* Current preedit state */
    char        preedit[XIM_MAX_PREEDIT_LEN];
    int         preedit_len;
    int         preedit_caret;

    /* Spot location for on-the-spot preedit rendering */
    int         spot_x;
    int         spot_y;
} xim_client_t;

/* XIM bridge state */
typedef struct {
    bool            initialized;
    bool            active;         /* Whether the XIM server is running */

    /* Connected XIM clients */
    xim_client_t    clients[XIM_MAX_CLIENTS];
    int             client_count;

    /* Currently focused client */
    int             focused_idx;    /* Index into clients[], or -1 */

    /* Wayland text-input connection (opaque) */
    void           *wl_text_input;  /* zwp_text_input_v3 * */
    void           *wl_seat;        /* wl_seat * */

    /* Supported styles */
    XIMStyle        supported_styles;
} xim_bridge_t;

/* ======================================================================
 * Functions
 * ====================================================================== */

/*
 * Initialize the XIM bridge.
 * Sets up the XIM server that X11 applications will connect to.
 * wl_text_input: Wayland text-input-v3 object (opaque pointer).
 * wl_seat: Wayland seat for input events.
 * Returns 0 on success, -1 on error.
 */
int xim_bridge_init(xim_bridge_t *bridge, void *wl_text_input,
                    void *wl_seat);

/*
 * Set input focus to a specific X11 window.
 * Activates the Wayland text-input for the given window.
 * x11_window: the X11 Window XID that now has IM focus.
 * Returns 0 on success, -1 on error.
 */
int xim_bridge_set_focus(xim_bridge_t *bridge, uint32_t x11_window);

/*
 * Remove input focus from the currently focused window.
 * Deactivates the Wayland text-input.
 */
void xim_bridge_unset_focus(xim_bridge_t *bridge);

/*
 * Forward a key event from Wayland to the input method.
 * If the IM consumes the key, returns true (caller should not
 * forward the key to the X11 client directly).
 * keycode: Linux evdev keycode.
 * pressed: true for key-down, false for key-up.
 * modifiers: bitmask of active modifiers.
 */
bool xim_bridge_forward_key(xim_bridge_t *bridge, uint32_t keycode,
                             bool pressed, uint32_t modifiers);

/*
 * Commit a composed string to the focused X11 client.
 * Called when the Wayland IM commits text (e.g., user selects
 * a CJK character from the candidate list).
 * text: UTF-8 encoded string.
 * Returns 0 on success, -1 on error.
 */
int xim_bridge_commit_string(xim_bridge_t *bridge, const char *text);

/*
 * Update the preedit string for the focused X11 client.
 * Called when the Wayland IM updates the preedit (composing) text.
 * text: UTF-8 encoded preedit string (NULL to clear).
 * cursor_pos: cursor position within the preedit string (bytes).
 * Returns 0 on success, -1 on error.
 */
int xim_bridge_set_preedit(xim_bridge_t *bridge, const char *text,
                            int cursor_pos);

/*
 * Destroy the XIM bridge and release all resources.
 * Disconnects all XIM clients.
 */
void xim_bridge_destroy(xim_bridge_t *bridge);

#ifdef __cplusplus
}
#endif

#endif /* XWAYLAND_IM_H */
