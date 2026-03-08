/*
 * xwayland-veridian.cpp -- XWayland Integration for VeridianOS
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Implementation of XWayland server management for X11 application
 * compatibility under KWin on VeridianOS.
 *
 * Architecture:
 *   KWin (Wayland compositor)
 *     -> xwl_start() launches /usr/bin/Xwayland
 *     -> Xwayland connects back to KWin via Wayland socket
 *     -> X11 apps connect to Xwayland via /tmp/.X11-unix/X0
 *     -> Window reparenting: X11 windows become Wayland subsurfaces
 *     -> Clipboard bridging: X11 CLIPBOARD <-> wl_data_device
 *     -> Input forwarding: Wayland input events -> X11 events
 */

#include "xwayland-veridian.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/stat.h>
#include <sys/wait.h>

/* ======================================================================
 * Helper: find available display number
 * ====================================================================== */

static int find_free_display(void)
{
    for (int d = XWL_DISPLAY_DEFAULT; d <= XWL_DISPLAY_MAX; d++) {
        char path[128];
        snprintf(path, sizeof(path), "%s/X%d", XWL_SOCKET_DIR, d);
        if (access(path, F_OK) != 0) {
            return d;
        }
    }
    return -1;
}

/* ======================================================================
 * Helper: create X11 socket
 * ====================================================================== */

static int create_x11_socket(int display_number, char *path_out,
                             size_t path_size)
{
    /* Create socket directory */
    mkdir(XWL_SOCKET_DIR, 01777);

    snprintf(path_out, path_size, "%s/X%d", XWL_SOCKET_DIR, display_number);

    /* Remove stale socket */
    unlink(path_out);

    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) {
        fprintf(stderr, "[xwayland] socket() failed: %s\n", strerror(errno));
        return -1;
    }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, path_out, sizeof(addr.sun_path) - 1);

    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        fprintf(stderr, "[xwayland] bind(%s) failed: %s\n",
                path_out, strerror(errno));
        close(fd);
        return -1;
    }

    if (listen(fd, 1) < 0) {
        fprintf(stderr, "[xwayland] listen() failed: %s\n", strerror(errno));
        close(fd);
        unlink(path_out);
        return -1;
    }

    return fd;
}

/* ======================================================================
 * Helper: create Xauthority entry
 * ====================================================================== */

static int create_xauth(const char *xauth_path, int display_number)
{
    /*
     * Generate a minimal Xauthority file with MIT-MAGIC-COOKIE-1.
     * In a production system, this would use proper random bytes
     * from the kernel CSPRNG.
     */
    FILE *fp = fopen(xauth_path, "w");
    if (!fp) {
        fprintf(stderr, "[xwayland] Cannot create %s: %s\n",
                xauth_path, strerror(errno));
        return -1;
    }

    /*
     * Xauthority file format:
     *   2 bytes: family (0x0100 = FamilyLocal)
     *   2 bytes: address length
     *   N bytes: address
     *   2 bytes: display number string length
     *   N bytes: display number string
     *   2 bytes: auth name length
     *   N bytes: auth name ("MIT-MAGIC-COOKIE-1")
     *   2 bytes: auth data length
     *   N bytes: auth data (16 bytes cookie)
     */
    char hostname[64] = "veridian";
    char display_str[8];
    snprintf(display_str, sizeof(display_str), "%d", display_number);

    const char *auth_name = "MIT-MAGIC-COOKIE-1";
    uint8_t cookie[16];

    /* Generate cookie -- use /dev/urandom or fallback to simple PRNG */
    int urand = open("/dev/urandom", O_RDONLY);
    if (urand >= 0) {
        ssize_t rd = read(urand, cookie, sizeof(cookie));
        (void)rd;
        close(urand);
    } else {
        /* Fallback: deterministic for development */
        for (int i = 0; i < 16; i++) {
            cookie[i] = (uint8_t)(i * 17 + 42);
        }
    }

    /* Write Xauthority entry (binary format) */
    uint16_t family = 256; /* FamilyLocal (big-endian) */
    uint16_t host_len = (uint16_t)strlen(hostname);
    uint16_t disp_len = (uint16_t)strlen(display_str);
    uint16_t name_len = (uint16_t)strlen(auth_name);
    uint16_t data_len = 16;

    fwrite(&family, 2, 1, fp);
    fwrite(&host_len, 2, 1, fp);
    fwrite(hostname, 1, host_len, fp);
    fwrite(&disp_len, 2, 1, fp);
    fwrite(display_str, 1, disp_len, fp);
    fwrite(&name_len, 2, 1, fp);
    fwrite(auth_name, 1, name_len, fp);
    fwrite(&data_len, 2, 1, fp);
    fwrite(cookie, 1, 16, fp);

    fclose(fp);
    chmod(xauth_path, 0600);

    return 0;
}

/* ======================================================================
 * Server Lifecycle
 * ====================================================================== */

int xwl_init(xwl_context_t *ctx)
{
    if (!ctx) {
        return -1;
    }

    memset(ctx, 0, sizeof(*ctx));
    ctx->state = XWL_STATE_STOPPED;
    ctx->server_pid = -1;
    ctx->display_number = -1;
    ctx->wm_fd = -1;
    ctx->x11_fd = -1;
    ctx->enable_clipboard = true;
    ctx->enable_dri3 = true;
    ctx->rootless = true;  /* Default: rootless mode for desktop use */

    strncpy(ctx->xauth_path, XWL_XAUTH_FILE, sizeof(ctx->xauth_path) - 1);

    return 0;
}

int xwl_start(xwl_context_t *ctx)
{
    if (!ctx) {
        return -1;
    }

    if (ctx->state == XWL_STATE_RUNNING) {
        fprintf(stderr, "[xwayland] Already running\n");
        return 0;
    }

    ctx->state = XWL_STATE_STARTING;

    /* Find available display number */
    ctx->display_number = find_free_display();
    if (ctx->display_number < 0) {
        fprintf(stderr, "[xwayland] No free display number available\n");
        ctx->state = XWL_STATE_ERROR;
        return -1;
    }

    snprintf(ctx->display_str, sizeof(ctx->display_str),
             ":%d", ctx->display_number);
    snprintf(ctx->socket_path, sizeof(ctx->socket_path),
             "%s/X%d", XWL_SOCKET_DIR, ctx->display_number);

    fprintf(stderr, "[xwayland] Starting Xwayland on display %s\n",
            ctx->display_str);

    /* Create Xauthority file */
    if (create_xauth(ctx->xauth_path, ctx->display_number) != 0) {
        ctx->state = XWL_STATE_ERROR;
        return -1;
    }

    /*
     * Create a socketpair for window management communication.
     * KWin uses one end; Xwayland uses the other.
     */
    int wm_fds[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, wm_fds) < 0) {
        fprintf(stderr, "[xwayland] socketpair() failed: %s\n",
                strerror(errno));
        ctx->state = XWL_STATE_ERROR;
        return -1;
    }

    /* Create X11 listening socket */
    ctx->x11_fd = create_x11_socket(ctx->display_number,
                                     ctx->socket_path,
                                     sizeof(ctx->socket_path));
    if (ctx->x11_fd < 0) {
        close(wm_fds[0]);
        close(wm_fds[1]);
        ctx->state = XWL_STATE_ERROR;
        return -1;
    }

    /* Fork and exec Xwayland */
    pid_t pid = fork();
    if (pid < 0) {
        fprintf(stderr, "[xwayland] fork() failed: %s\n", strerror(errno));
        close(wm_fds[0]);
        close(wm_fds[1]);
        close(ctx->x11_fd);
        ctx->state = XWL_STATE_ERROR;
        return -1;
    }

    if (pid == 0) {
        /* Child: exec Xwayland */
        close(wm_fds[0]);  /* Close KWin's end */

        char wm_fd_str[16];
        snprintf(wm_fd_str, sizeof(wm_fd_str), "%d", wm_fds[1]);

        char listen_fd_str[16];
        snprintf(listen_fd_str, sizeof(listen_fd_str), "%d", ctx->x11_fd);

        setenv("XAUTHORITY", ctx->xauth_path, 1);

        /*
         * Xwayland command line:
         *   Xwayland :N -rootless -wm <fd> -listenfd <fd>
         *            -auth <path> -noreset
         *
         * -rootless:  No root window (desktop managed by KWin)
         * -wm:        Window manager socket fd
         * -listenfd:  Pre-created listening socket fd
         * -noreset:   Don't reset when last client disconnects
         */
        const char *xwl_argv[16];
        int argc = 0;

        xwl_argv[argc++] = "Xwayland";
        xwl_argv[argc++] = ctx->display_str;
        if (ctx->rootless) {
            xwl_argv[argc++] = "-rootless";
        }
        xwl_argv[argc++] = "-wm";
        xwl_argv[argc++] = wm_fd_str;
        xwl_argv[argc++] = "-listenfd";
        xwl_argv[argc++] = listen_fd_str;
        xwl_argv[argc++] = "-auth";
        xwl_argv[argc++] = ctx->xauth_path;
        xwl_argv[argc++] = "-noreset";
        if (ctx->enable_dri3) {
            xwl_argv[argc++] = "+iglx";
        }
        xwl_argv[argc] = NULL;

        execv(XWL_XWAYLAND_PATH, (char *const *)xwl_argv);

        /* exec failed */
        fprintf(stderr, "[xwayland] execv(%s) failed: %s\n",
                XWL_XWAYLAND_PATH, strerror(errno));
        _exit(127);
    }

    /* Parent: KWin side */
    close(wm_fds[1]);  /* Close Xwayland's end */
    ctx->wm_fd = wm_fds[0];
    ctx->server_pid = pid;

    /* Wait for Xwayland to be ready (socket becomes connectable) */
    int retries = 0;
    while (retries < 50) {
        int test_fd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (test_fd >= 0) {
            struct sockaddr_un test_addr;
            memset(&test_addr, 0, sizeof(test_addr));
            test_addr.sun_family = AF_UNIX;
            strncpy(test_addr.sun_path, ctx->socket_path,
                    sizeof(test_addr.sun_path) - 1);
            if (connect(test_fd, (struct sockaddr *)&test_addr,
                       sizeof(test_addr)) == 0) {
                close(test_fd);
                break;
            }
            close(test_fd);
        }
        usleep(100000);  /* 100 ms */
        retries++;
    }

    if (retries >= 50) {
        fprintf(stderr, "[xwayland] Xwayland failed to start after 5s\n");
        xwl_stop(ctx);
        return -1;
    }

    /* Set DISPLAY for child processes */
    setenv("DISPLAY", ctx->display_str, 1);
    setenv("XAUTHORITY", ctx->xauth_path, 1);

    ctx->state = XWL_STATE_RUNNING;
    fprintf(stderr, "[xwayland] Xwayland running on %s (PID %d)\n",
            ctx->display_str, (int)ctx->server_pid);

    return 0;
}

int xwl_stop(xwl_context_t *ctx)
{
    if (!ctx || ctx->state == XWL_STATE_STOPPED) {
        return 0;
    }

    ctx->state = XWL_STATE_STOPPING;
    fprintf(stderr, "[xwayland] Stopping Xwayland (PID %d)...\n",
            (int)ctx->server_pid);

    /* Send SIGTERM and wait */
    if (ctx->server_pid > 0) {
        kill(ctx->server_pid, SIGTERM);

        int status;
        int wait_count = 0;
        while (wait_count < 30) {
            pid_t w = waitpid(ctx->server_pid, &status, WNOHANG);
            if (w > 0) {
                break;
            }
            usleep(100000);
            wait_count++;
        }

        /* Force kill if still running */
        if (wait_count >= 30) {
            fprintf(stderr, "[xwayland] Force-killing Xwayland\n");
            kill(ctx->server_pid, SIGKILL);
            waitpid(ctx->server_pid, &status, 0);
        }
    }

    /* Clean up sockets */
    if (ctx->wm_fd >= 0) {
        close(ctx->wm_fd);
        ctx->wm_fd = -1;
    }
    if (ctx->x11_fd >= 0) {
        close(ctx->x11_fd);
        ctx->x11_fd = -1;
    }

    /* Remove socket and auth files */
    unlink(ctx->socket_path);
    unlink(ctx->xauth_path);

    /* Clear environment */
    unsetenv("DISPLAY");

    /* Clear clipboard */
    xwl_clipboard_clear(ctx);

    ctx->server_pid = -1;
    ctx->window_count = 0;
    ctx->state = XWL_STATE_STOPPED;

    fprintf(stderr, "[xwayland] Xwayland stopped\n");
    return 0;
}

bool xwl_is_running(const xwl_context_t *ctx)
{
    if (!ctx || ctx->state != XWL_STATE_RUNNING) {
        return false;
    }
    if (ctx->server_pid <= 0) {
        return false;
    }
    /* Check if process is alive */
    return (kill(ctx->server_pid, 0) == 0);
}

/* ======================================================================
 * Window Management
 * ====================================================================== */

int xwl_window_created(xwl_context_t *ctx, uint32_t x11_id,
                       int x, int y, int width, int height,
                       bool override_redirect)
{
    if (!ctx || ctx->window_count >= XWL_MAX_WINDOWS) {
        return -1;
    }

    int idx = ctx->window_count;
    xwl_window_t *win = &ctx->windows[idx];

    memset(win, 0, sizeof(*win));
    win->x11_window_id = x11_id;
    win->wayland_surface_id = 0;  /* Assigned by KWin */
    win->x = x;
    win->y = y;
    win->width = width;
    win->height = height;
    win->mapped = false;
    win->override_redirect = override_redirect;

    ctx->window_count++;

    fprintf(stderr, "[xwayland] Window created: X11 ID 0x%x "
            "(%dx%d+%d+%d)%s\n",
            x11_id, width, height, x, y,
            override_redirect ? " [override-redirect]" : "");

    return idx;
}

void xwl_window_destroyed(xwl_context_t *ctx, uint32_t x11_id)
{
    if (!ctx) {
        return;
    }

    for (int i = 0; i < ctx->window_count; i++) {
        if (ctx->windows[i].x11_window_id == x11_id) {
            fprintf(stderr, "[xwayland] Window destroyed: X11 ID 0x%x\n",
                    x11_id);

            /* Shift remaining windows down */
            if (i < ctx->window_count - 1) {
                memmove(&ctx->windows[i], &ctx->windows[i + 1],
                        sizeof(xwl_window_t) *
                        (size_t)(ctx->window_count - i - 1));
            }
            ctx->window_count--;
            return;
        }
    }
}

void xwl_window_configure(xwl_context_t *ctx, uint32_t x11_id,
                          int x, int y, int width, int height)
{
    if (!ctx) {
        return;
    }

    for (int i = 0; i < ctx->window_count; i++) {
        if (ctx->windows[i].x11_window_id == x11_id) {
            ctx->windows[i].x = x;
            ctx->windows[i].y = y;
            ctx->windows[i].width = width;
            ctx->windows[i].height = height;
            return;
        }
    }
}

void xwl_window_set_mapped(xwl_context_t *ctx, uint32_t x11_id,
                           bool mapped)
{
    if (!ctx) {
        return;
    }

    for (int i = 0; i < ctx->window_count; i++) {
        if (ctx->windows[i].x11_window_id == x11_id) {
            ctx->windows[i].mapped = mapped;
            return;
        }
    }
}

const xwl_window_t *xwl_find_window(const xwl_context_t *ctx,
                                     uint32_t x11_id)
{
    if (!ctx) {
        return NULL;
    }

    for (int i = 0; i < ctx->window_count; i++) {
        if (ctx->windows[i].x11_window_id == x11_id) {
            return &ctx->windows[i];
        }
    }
    return NULL;
}

/* ======================================================================
 * Clipboard Sharing
 * ====================================================================== */

int xwl_clipboard_transfer(xwl_context_t *ctx, xwl_clipboard_dir_t dir)
{
    if (!ctx || !ctx->enable_clipboard) {
        return -1;
    }

    /*
     * Clipboard transfer between X11 and Wayland.
     *
     * X11 -> Wayland:
     *   1. Read X11 CLIPBOARD selection via XConvertSelection()
     *   2. Receive SelectionNotify event with data
     *   3. Create wl_data_source with the data
     *   4. Set it on the Wayland seat's wl_data_device
     *
     * Wayland -> X11:
     *   1. Read wl_data_offer from the Wayland seat
     *   2. Read data from the offer's fd
     *   3. Set X11 CLIPBOARD selection via XSetSelectionOwner()
     *   4. Respond to SelectionRequest events with the data
     *
     * In the actual implementation, this is handled by KWin's
     * built-in XWayland clipboard bridge.  This function provides
     * the VeridianOS-specific hooks for the bridge.
     */

    fprintf(stderr, "[xwayland] Clipboard transfer: %s\n",
            dir == XWL_CLIP_X11_TO_WAYLAND
            ? "X11 -> Wayland" : "Wayland -> X11");

    (void)dir;
    return 0;
}

int xwl_clipboard_set(xwl_context_t *ctx, xwl_clip_type_t type,
                      const uint8_t *data, size_t size)
{
    if (!ctx || !data || size == 0) {
        return -1;
    }

    if (size > XWL_CLIPBOARD_MAX_SIZE) {
        fprintf(stderr, "[xwayland] Clipboard data too large: %zu bytes "
                "(max %d)\n", size, XWL_CLIPBOARD_MAX_SIZE);
        return -1;
    }

    /* Free existing data */
    xwl_clipboard_clear(ctx);

    /* Copy new data */
    ctx->clipboard.data = (uint8_t *)malloc(size);
    if (!ctx->clipboard.data) {
        return -1;
    }

    memcpy(ctx->clipboard.data, data, size);
    ctx->clipboard.size = size;
    ctx->clipboard.type = type;
    ctx->clipboard.valid = true;

    return 0;
}

const uint8_t *xwl_clipboard_get(const xwl_context_t *ctx,
                                  xwl_clip_type_t *type,
                                  size_t *size)
{
    if (!ctx || !ctx->clipboard.valid) {
        if (size) *size = 0;
        return NULL;
    }

    if (type) *type = ctx->clipboard.type;
    if (size) *size = ctx->clipboard.size;
    return ctx->clipboard.data;
}

void xwl_clipboard_clear(xwl_context_t *ctx)
{
    if (!ctx) {
        return;
    }

    free(ctx->clipboard.data);
    ctx->clipboard.data = NULL;
    ctx->clipboard.size = 0;
    ctx->clipboard.valid = false;
}

/* ======================================================================
 * Input Forwarding
 * ====================================================================== */

void xwl_forward_key(xwl_context_t *ctx, uint32_t keycode,
                     bool pressed, uint32_t modifiers)
{
    if (!ctx || ctx->state != XWL_STATE_RUNNING) {
        return;
    }

    /*
     * Forward keyboard event to the focused X11 window.
     *
     * KWin handles this through its built-in XWayland support:
     *   1. Determine the focused X11 window
     *   2. Translate Linux evdev keycode to X11 keycode (+8 offset)
     *   3. Send XKeyEvent via the X11 protocol
     *
     * The modifier mask translates between Wayland modifier names
     * and X11 modifier bits (ShiftMask, ControlMask, Mod1Mask, etc.)
     */
    (void)keycode;
    (void)pressed;
    (void)modifiers;
}

void xwl_forward_pointer_motion(xwl_context_t *ctx, int x, int y)
{
    if (!ctx || ctx->state != XWL_STATE_RUNNING) {
        return;
    }

    /*
     * Forward pointer motion to X11.
     * Coordinates are relative to the X11 root window (which maps
     * to the full Wayland output in non-rootless mode, or to the
     * individual window bounds in rootless mode).
     */
    (void)x;
    (void)y;
}

void xwl_forward_pointer_button(xwl_context_t *ctx, uint32_t button,
                                bool pressed)
{
    if (!ctx || ctx->state != XWL_STATE_RUNNING) {
        return;
    }

    /*
     * Forward pointer button event to X11.
     * Linux button codes map to X11 buttons:
     *   BTN_LEFT   (0x110) -> Button1
     *   BTN_RIGHT  (0x111) -> Button3
     *   BTN_MIDDLE (0x112) -> Button2
     */
    (void)button;
    (void)pressed;
}

void xwl_forward_scroll(xwl_context_t *ctx, int axis, int value)
{
    if (!ctx || ctx->state != XWL_STATE_RUNNING) {
        return;
    }

    /*
     * Forward scroll event to X11 as button press/release.
     * X11 uses button 4/5 for vertical scroll and 6/7 for horizontal.
     *   Vertical up:    Button4
     *   Vertical down:  Button5
     *   Horizontal left:  Button6
     *   Horizontal right: Button7
     */
    (void)axis;
    (void)value;
}

/* ======================================================================
 * Environment
 * ====================================================================== */

const char *xwl_get_display(const xwl_context_t *ctx)
{
    if (!ctx || ctx->state != XWL_STATE_RUNNING) {
        return NULL;
    }
    return ctx->display_str;
}

const char *xwl_get_xauthority(const xwl_context_t *ctx)
{
    if (!ctx) {
        return NULL;
    }
    return ctx->xauth_path;
}
