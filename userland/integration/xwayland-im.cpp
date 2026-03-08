/*
 * xwayland-im.cpp -- XIM (X Input Method) Bridge for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Implements the XIM server bridge that connects Wayland input methods
 * (via zwp_text_input_v3) to X11 applications running under XWayland.
 *
 * XIM protocol flow:
 *   1. X11 app connects to XIM server (XOpenIM)
 *   2. App creates Input Context (XCreateIC) with styles
 *   3. App sets focus (XSetICFocus)
 *   4. Bridge activates Wayland text-input-v3 for the seat
 *   5. User types -> key events forwarded to Wayland IM
 *   6. IM sends preedit/commit -> bridge forwards to X11 app
 *   7. App receives XIM_FORWARD_EVENT, XIM_COMMIT, preedit callbacks
 *
 * Supported IM frameworks (via Wayland text-input-v3):
 *   - IBus (used by GNOME, works with KDE too)
 *   - Fcitx5 (common for CJK input)
 *   - Any zwp_text_input_v3-compatible IM
 */

#include "xwayland-im.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ======================================================================
 * Internal helpers
 * ====================================================================== */

/*
 * Find a client slot by X11 window ID.
 * Returns the index, or -1 if not found.
 */
static int xim_find_client(const xim_bridge_t *bridge,
                            uint32_t x11_window)
{
    for (int i = 0; i < XIM_MAX_CLIENTS; i++) {
        if (bridge->clients[i].active &&
            bridge->clients[i].x11_window == x11_window) {
            return i;
        }
    }
    return -1;
}

/*
 * Allocate a new client slot.
 * Returns the index, or -1 if full.
 */
static int xim_alloc_client(xim_bridge_t *bridge)
{
    for (int i = 0; i < XIM_MAX_CLIENTS; i++) {
        if (!bridge->clients[i].active) {
            return i;
        }
    }
    return -1;
}

/*
 * Get the currently focused client, or NULL.
 */
static xim_client_t *xim_get_focused(xim_bridge_t *bridge)
{
    if (bridge->focused_idx < 0 ||
        bridge->focused_idx >= XIM_MAX_CLIENTS) {
        return NULL;
    }
    xim_client_t *client = &bridge->clients[bridge->focused_idx];
    return client->active ? client : NULL;
}

/*
 * Send a preedit draw event to an X11 client.
 * In the real implementation, this would send an XIM_FORWARD_EVENT
 * with the preedit draw callback data via the XIM protocol.
 */
static void xim_send_preedit_draw(xim_bridge_t *bridge,
                                   xim_client_t *client,
                                   const xim_preedit_draw_t *draw)
{
    (void)bridge;

    if (!client || !draw) {
        return;
    }

    /*
     * XIM protocol preedit draw:
     *   XIM_PREEDIT_DRAW message containing:
     *     - caret position
     *     - changed region (first, length)
     *     - new text and feedback (underline, highlight)
     *
     * For on-the-spot preedit (XIMPreeditCallbacks):
     *   The client's preedit draw callback is invoked with
     *   XIMPreeditDrawCallbackStruct containing the text change.
     *
     * For root-window preedit (XIMPreeditNothing):
     *   The XIM server renders the preedit in its own window.
     *   We handle this by positioning a preedit window near the
     *   client's focus point.
     */

    fprintf(stderr, "[xim] preedit_draw: window=0x%x caret=%d "
            "text='%.*s'\n",
            client->x11_window, draw->caret,
            draw->text_length, draw->text);
}

/*
 * Send a commit string to an X11 client.
 * Generates an XIM_COMMIT message with the committed text.
 */
static void xim_send_commit(xim_bridge_t *bridge,
                              xim_client_t *client,
                              const char *text, int text_len)
{
    (void)bridge;

    if (!client || !text || text_len <= 0) {
        return;
    }

    /*
     * XIM protocol commit:
     *   XIM_COMMIT message containing:
     *     - flag: XLookupChars
     *     - committed string (multi-byte, UTF-8)
     *
     * The X11 client receives this as an XKeyEvent with
     * XLookupString/Xutf8LookupString returning the committed text.
     * For CJK characters, this can be multi-byte (e.g., 3 bytes
     * for a single Unicode CJK character in UTF-8).
     *
     * Implementation steps:
     *   1. Encode the text as XIM_COMMIT packet
     *   2. Send via the XIM communication channel (X11 ClientMessage
     *      or property change, depending on transport)
     *   3. The X11 toolkit (Gtk, Qt) receives and processes
     */

    fprintf(stderr, "[xim] commit: window=0x%x text='%.*s' (%d bytes)\n",
            client->x11_window, text_len, text, text_len);
}

/*
 * Send a preedit start event to an X11 client.
 */
static void xim_send_preedit_start(xim_bridge_t *bridge,
                                     xim_client_t *client)
{
    (void)bridge;
    if (!client) {
        return;
    }

    /*
     * XIM_PREEDIT_START: notifies the client that preedit has begun.
     * The client may allocate resources for displaying preedit text.
     */

    fprintf(stderr, "[xim] preedit_start: window=0x%x\n",
            client->x11_window);
}

/*
 * Send a preedit done event to an X11 client.
 */
static void xim_send_preedit_done(xim_bridge_t *bridge,
                                    xim_client_t *client)
{
    (void)bridge;
    if (!client) {
        return;
    }

    /*
     * XIM_PREEDIT_DONE: notifies the client that preedit is finished.
     * The client should clean up any preedit display state.
     */

    fprintf(stderr, "[xim] preedit_done: window=0x%x\n",
            client->x11_window);
}

/* ======================================================================
 * XIM bridge API implementation
 * ====================================================================== */

int xim_bridge_init(xim_bridge_t *bridge, void *wl_text_input,
                    void *wl_seat)
{
    if (!bridge) {
        return -1;
    }

    memset(bridge, 0, sizeof(*bridge));
    bridge->wl_text_input = wl_text_input;
    bridge->wl_seat = wl_seat;
    bridge->focused_idx = -1;

    /* Supported XIM styles */
    bridge->supported_styles =
        XIM_PREEDIT_CALLBACKS | XIM_STATUS_NOTHING |   /* on-the-spot */
        XIM_PREEDIT_POSITION  | XIM_STATUS_NOTHING |   /* over-the-spot */
        XIM_PREEDIT_NOTHING   | XIM_STATUS_NOTHING;    /* root-window */

    /*
     * Register as XIM server:
     *   1. Set the XIM_SERVERS atom on the X11 root window
     *   2. Create the XIM communication window
     *   3. Announce supported styles
     *
     * The XIM server name is typically "@server=veridian-xim".
     * X11 clients discover us via the XMODIFIERS env var:
     *   export XMODIFIERS="@im=veridian-xim"
     */

    bridge->initialized = true;
    bridge->active = true;

    fprintf(stderr, "[xim] Bridge initialized (styles=0x%x)\n",
            bridge->supported_styles);

    return 0;
}

int xim_bridge_set_focus(xim_bridge_t *bridge, uint32_t x11_window)
{
    if (!bridge || !bridge->initialized || x11_window == 0) {
        return -1;
    }

    /* Unfocus the previously focused client */
    xim_client_t *prev = xim_get_focused(bridge);
    if (prev) {
        prev->focused = false;

        /* Clear any active preedit */
        if (prev->preedit_len > 0) {
            xim_send_preedit_done(bridge, prev);
            prev->preedit_len = 0;
            prev->preedit[0] = '\0';
        }
    }

    /* Find or create a client entry for this window */
    int idx = xim_find_client(bridge, x11_window);
    if (idx < 0) {
        idx = xim_alloc_client(bridge);
        if (idx < 0) {
            fprintf(stderr, "[xim] set_focus: no free client slots\n");
            return -1;
        }

        xim_client_t *client = &bridge->clients[idx];
        memset(client, 0, sizeof(*client));
        client->x11_window = x11_window;
        client->style = XIM_PREEDIT_CALLBACKS | XIM_STATUS_NOTHING;
        client->active = true;
        bridge->client_count++;
    }

    bridge->clients[idx].focused = true;
    bridge->focused_idx = idx;

    /*
     * Activate Wayland text-input-v3 for this seat:
     *   zwp_text_input_v3_enable(bridge->wl_text_input);
     *   zwp_text_input_v3_set_content_type(bridge->wl_text_input,
     *       ZWP_TEXT_INPUT_V3_CONTENT_HINT_NONE,
     *       ZWP_TEXT_INPUT_V3_CONTENT_PURPOSE_NORMAL);
     *   zwp_text_input_v3_commit(bridge->wl_text_input);
     *   wl_surface_commit(focused_surface);
     */

    fprintf(stderr, "[xim] set_focus: window=0x%x (slot %d)\n",
            x11_window, idx);

    return 0;
}

void xim_bridge_unset_focus(xim_bridge_t *bridge)
{
    if (!bridge || !bridge->initialized) {
        return;
    }

    xim_client_t *client = xim_get_focused(bridge);
    if (client) {
        /* Clear preedit */
        if (client->preedit_len > 0) {
            xim_send_preedit_done(bridge, client);
            client->preedit_len = 0;
            client->preedit[0] = '\0';
        }

        client->focused = false;

        fprintf(stderr, "[xim] unset_focus: window=0x%x\n",
                client->x11_window);
    }

    bridge->focused_idx = -1;

    /*
     * Deactivate Wayland text-input-v3:
     *   zwp_text_input_v3_disable(bridge->wl_text_input);
     *   zwp_text_input_v3_commit(bridge->wl_text_input);
     */
}

bool xim_bridge_forward_key(xim_bridge_t *bridge, uint32_t keycode,
                             bool pressed, uint32_t modifiers)
{
    if (!bridge || !bridge->initialized || !bridge->active) {
        return false;
    }

    xim_client_t *client = xim_get_focused(bridge);
    if (!client) {
        return false;
    }

    /*
     * Forward key event to the Wayland input method:
     *
     * The key goes through this path:
     *   1. Wayland compositor receives the key event
     *   2. Key is forwarded to zwp_text_input_v3 via
     *      zwp_input_method_v2_key()
     *   3. The IM (IBus/Fcitx) decides whether to consume it
     *   4. If consumed: IM sends preedit_string or commit_string
     *   5. If not consumed: key is forwarded to the X11 client
     *
     * For CJK input:
     *   - Typing 'n', 'i', 'h', 'a', 'o' builds preedit
     *   - Preedit shows candidate characters
     *   - Pressing Enter/Space commits the selected character
     *   - The bridge converts to XIM events for the X11 client
     *
     * For dead key composition (European languages):
     *   - Dead key starts composition (e.g., dead_acute)
     *   - Next key completes it (e.g., 'a' -> 'a with acute')
     *   - commit_string sends the composed character
     */

    (void)keycode;
    (void)pressed;
    (void)modifiers;

    /* Return false to indicate the key was not consumed by the IM.
     * In the real implementation, this would return true if the
     * IM is actively composing and consumed the key. */
    return false;
}

int xim_bridge_commit_string(xim_bridge_t *bridge, const char *text)
{
    if (!bridge || !bridge->initialized || !text) {
        return -1;
    }

    xim_client_t *client = xim_get_focused(bridge);
    if (!client) {
        fprintf(stderr, "[xim] commit_string: no focused client\n");
        return -1;
    }

    int text_len = (int)strlen(text);
    if (text_len <= 0 || text_len >= XIM_MAX_COMMIT_LEN) {
        fprintf(stderr, "[xim] commit_string: invalid length %d\n",
                text_len);
        return -1;
    }

    /* Clear any active preedit first */
    if (client->preedit_len > 0) {
        xim_send_preedit_done(bridge, client);
        client->preedit_len = 0;
        client->preedit[0] = '\0';
    }

    /* Send the committed text to the X11 client */
    xim_send_commit(bridge, client, text, text_len);

    fprintf(stderr, "[xim] commit_string: '%s' -> window 0x%x\n",
            text, client->x11_window);

    return 0;
}

int xim_bridge_set_preedit(xim_bridge_t *bridge, const char *text,
                            int cursor_pos)
{
    if (!bridge || !bridge->initialized) {
        return -1;
    }

    xim_client_t *client = xim_get_focused(bridge);
    if (!client) {
        return -1;
    }

    /* Handle preedit clear */
    if (!text || strlen(text) == 0) {
        if (client->preedit_len > 0) {
            /* Send preedit done */
            xim_send_preedit_done(bridge, client);
            client->preedit_len = 0;
            client->preedit[0] = '\0';
            client->preedit_caret = 0;
        }
        return 0;
    }

    int text_len = (int)strlen(text);
    if (text_len >= XIM_MAX_PREEDIT_LEN) {
        text_len = XIM_MAX_PREEDIT_LEN - 1;
    }

    /* Send preedit start if this is a new preedit session */
    if (client->preedit_len == 0) {
        xim_send_preedit_start(bridge, client);
    }

    /* Build preedit draw data */
    xim_preedit_draw_t draw;
    memset(&draw, 0, sizeof(draw));
    draw.caret = cursor_pos;
    draw.change_first = 0;
    draw.change_length = client->preedit_len;
    memcpy(draw.text, text, (size_t)text_len);
    draw.text[text_len] = '\0';
    draw.text_length = text_len;

    /* Update client state */
    memcpy(client->preedit, text, (size_t)text_len);
    client->preedit[text_len] = '\0';
    client->preedit_len = text_len;
    client->preedit_caret = cursor_pos;

    /* Send to X11 client */
    xim_send_preedit_draw(bridge, client, &draw);

    return 0;
}

void xim_bridge_destroy(xim_bridge_t *bridge)
{
    if (!bridge) {
        return;
    }

    /* Unfocus and clean up all clients */
    if (bridge->focused_idx >= 0) {
        xim_bridge_unset_focus(bridge);
    }

    for (int i = 0; i < XIM_MAX_CLIENTS; i++) {
        if (bridge->clients[i].active) {
            /* Send preedit done if active */
            if (bridge->clients[i].preedit_len > 0) {
                xim_send_preedit_done(bridge, &bridge->clients[i]);
            }
            bridge->clients[i].active = false;
        }
    }

    /*
     * Unregister XIM server:
     *   1. Remove XIM_SERVERS atom from root window
     *   2. Destroy communication window
     *   3. Deactivate text-input-v3
     */

    bridge->active = false;
    bridge->initialized = false;
    bridge->client_count = 0;

    fprintf(stderr, "[xim] Bridge destroyed\n");
}
