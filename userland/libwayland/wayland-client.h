/*
 * wayland-client.h -- VeridianOS Wayland Client Library
 *
 * Provides the user-space Wayland client API for GUI applications running
 * on VeridianOS. Communicates with the kernel Wayland compositor via
 * system calls (SYS_WL_CONNECT through SYS_WL_GET_EVENTS).
 *
 * Usage:
 *   #include "wayland-client.h"
 *
 *   wl_display *display = wl_display_connect(NULL);
 *   wl_registry *registry = wl_display_get_registry(display);
 *   wl_registry_add_listener(registry, &registry_listener, data);
 *   wl_display_roundtrip(display);
 *   // ... bind globals, create surfaces, render ...
 *   wl_display_disconnect(display);
 *
 * Build: Link against libwayland-client.a (static library, no libc dependency)
 */

#ifndef WAYLAND_CLIENT_H
#define WAYLAND_CLIENT_H

#include <stdint.h>
#include <stddef.h>

/* ===================================================================
 * Opaque handle types
 * =================================================================== */

typedef struct wl_display wl_display;
typedef struct wl_registry wl_registry;
typedef struct wl_compositor wl_compositor;
typedef struct wl_surface wl_surface;
typedef struct wl_shm wl_shm;
typedef struct wl_shm_pool wl_shm_pool;
typedef struct wl_buffer wl_buffer;
typedef struct wl_callback wl_callback;
typedef struct wl_seat wl_seat;
typedef struct wl_keyboard wl_keyboard;
typedef struct wl_pointer wl_pointer;
typedef struct wl_output wl_output;
typedef struct xdg_wm_base xdg_wm_base;
typedef struct xdg_surface xdg_surface;
typedef struct xdg_toplevel xdg_toplevel;

/* ===================================================================
 * Pixel format constants (matches kernel wl_shm.format)
 * =================================================================== */

#define WL_SHM_FORMAT_ARGB8888  0
#define WL_SHM_FORMAT_XRGB8888  1

/* ===================================================================
 * Seat capability flags
 * =================================================================== */

#define WL_SEAT_CAPABILITY_POINTER   1
#define WL_SEAT_CAPABILITY_KEYBOARD  2
#define WL_SEAT_CAPABILITY_TOUCH     4

/* ===================================================================
 * Key state constants
 * =================================================================== */

#define WL_KEYBOARD_KEY_STATE_RELEASED  0
#define WL_KEYBOARD_KEY_STATE_PRESSED   1

/* ===================================================================
 * Pointer button state constants
 * =================================================================== */

#define WL_POINTER_BUTTON_STATE_RELEASED  0
#define WL_POINTER_BUTTON_STATE_PRESSED   1

/* ===================================================================
 * Output transform (matches kernel OutputTransform)
 * =================================================================== */

#define WL_OUTPUT_TRANSFORM_NORMAL        0
#define WL_OUTPUT_TRANSFORM_90            1
#define WL_OUTPUT_TRANSFORM_180           2
#define WL_OUTPUT_TRANSFORM_270           3
#define WL_OUTPUT_TRANSFORM_FLIPPED       4
#define WL_OUTPUT_TRANSFORM_FLIPPED_90    5
#define WL_OUTPUT_TRANSFORM_FLIPPED_180   6
#define WL_OUTPUT_TRANSFORM_FLIPPED_270   7

/* ===================================================================
 * Output mode flags
 * =================================================================== */

#define WL_OUTPUT_MODE_CURRENT    0x1
#define WL_OUTPUT_MODE_PREFERRED  0x2

/* ===================================================================
 * Listener structures
 *
 * Each Wayland interface sends events back to the client via listener
 * callbacks. Register a listener with the appropriate add_listener()
 * function. The `data` pointer is passed through unchanged.
 * =================================================================== */

/* wl_registry events */
typedef struct {
    void (*global)(void *data, wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version);
    void (*global_remove)(void *data, wl_registry *registry, uint32_t name);
} wl_registry_listener;

/* xdg_wm_base events */
typedef struct {
    void (*ping)(void *data, xdg_wm_base *shell, uint32_t serial);
} xdg_wm_base_listener;

/* xdg_surface events */
typedef struct {
    void (*configure)(void *data, xdg_surface *surface, uint32_t serial);
} xdg_surface_listener;

/* xdg_toplevel events */
typedef struct {
    void (*configure)(void *data, xdg_toplevel *toplevel,
                      int32_t width, int32_t height,
                      void *states, int states_count);
    void (*close)(void *data, xdg_toplevel *toplevel);
} xdg_toplevel_listener;

/* wl_callback events */
typedef struct {
    void (*done)(void *data, wl_callback *callback, uint32_t callback_data);
} wl_callback_listener;

/* wl_shm events */
typedef struct {
    void (*format)(void *data, wl_shm *shm, uint32_t format);
} wl_shm_listener;

/* wl_keyboard events */
typedef struct {
    void (*keymap)(void *data, wl_keyboard *kb, uint32_t format,
                   int32_t fd, uint32_t size);
    void (*enter)(void *data, wl_keyboard *kb, uint32_t serial,
                  wl_surface *surface, void *keys);
    void (*leave)(void *data, wl_keyboard *kb, uint32_t serial,
                  wl_surface *surface);
    void (*key)(void *data, wl_keyboard *kb, uint32_t serial,
                uint32_t time, uint32_t key, uint32_t state);
    void (*modifiers)(void *data, wl_keyboard *kb, uint32_t serial,
                      uint32_t mods_depressed, uint32_t mods_latched,
                      uint32_t mods_locked, uint32_t group);
} wl_keyboard_listener;

/* wl_pointer events */
typedef struct {
    void (*enter)(void *data, wl_pointer *ptr, uint32_t serial,
                  wl_surface *surface, int32_t sx, int32_t sy);
    void (*leave)(void *data, wl_pointer *ptr, uint32_t serial,
                  wl_surface *surface);
    void (*motion)(void *data, wl_pointer *ptr, uint32_t time,
                   int32_t sx, int32_t sy);
    void (*button)(void *data, wl_pointer *ptr, uint32_t serial,
                   uint32_t time, uint32_t button, uint32_t state);
    void (*axis)(void *data, wl_pointer *ptr, uint32_t time,
                 uint32_t axis, int32_t value);
} wl_pointer_listener;

/* wl_seat events */
typedef struct {
    void (*capabilities)(void *data, wl_seat *seat, uint32_t caps);
    void (*name)(void *data, wl_seat *seat, const char *name);
} wl_seat_listener;

/* wl_output events */
typedef struct {
    void (*geometry)(void *data, wl_output *output, int32_t x, int32_t y,
                     int32_t physical_width, int32_t physical_height,
                     int32_t subpixel, const char *make, const char *model,
                     int32_t transform);
    void (*mode)(void *data, wl_output *output, uint32_t flags,
                 int32_t width, int32_t height, int32_t refresh);
    void (*done)(void *data, wl_output *output);
    void (*scale)(void *data, wl_output *output, int32_t factor);
    void (*name)(void *data, wl_output *output, const char *name);
    void (*description)(void *data, wl_output *output, const char *description);
} wl_output_listener;

/* ===================================================================
 * Core display API
 * =================================================================== */

/* Connect to the Wayland display server.
 * `name` is currently ignored (reserved for future named sockets).
 * Returns NULL on failure. */
wl_display *wl_display_connect(const char *name);

/* Disconnect from the display server and free resources. */
void wl_display_disconnect(wl_display *display);

/* Process incoming events and dispatch to listeners. Returns 0 on success,
 * -1 on error. */
int wl_display_dispatch(wl_display *display);

/* Send a sync request, dispatch all pending events, and block until
 * the compositor acknowledges. Returns 0 on success, -1 on error. */
int wl_display_roundtrip(wl_display *display);

/* Flush buffered outgoing requests to the compositor. Returns number of
 * bytes sent, or -1 on error. */
int wl_display_flush(wl_display *display);

/* Get the file descriptor for the display connection (for poll/epoll).
 * Returns -1 if display is kernel-internal (no fd-based transport). */
int wl_display_get_fd(wl_display *display);

/* ===================================================================
 * Registry API
 * =================================================================== */

/* Get the global registry from the display. */
wl_registry *wl_display_get_registry(wl_display *display);

/* Register a listener for registry global events. */
int wl_registry_add_listener(wl_registry *registry,
                             const wl_registry_listener *listener,
                             void *data);

/* Bind a global object by name. Returns an opaque pointer to the bound
 * interface (cast to wl_compositor*, wl_shm*, etc.). */
void *wl_registry_bind(wl_registry *registry, uint32_t name,
                       const char *interface, uint32_t version);

/* ===================================================================
 * Compositor / Surface API
 * =================================================================== */

/* Create a new wl_surface. */
wl_surface *wl_compositor_create_surface(wl_compositor *compositor);

/* Destroy a surface. */
void wl_surface_destroy(wl_surface *surface);

/* Attach a buffer to the surface at the given offset. */
void wl_surface_attach(wl_surface *surface, wl_buffer *buffer,
                       int32_t x, int32_t y);

/* Mark a rectangular region as damaged (needs redraw). */
void wl_surface_damage(wl_surface *surface, int32_t x, int32_t y,
                       int32_t width, int32_t height);

/* Commit pending surface state (buffer, damage) atomically. */
void wl_surface_commit(wl_surface *surface);

/* Request a frame callback (for animation timing). */
wl_callback *wl_surface_frame(wl_surface *surface);

/* ===================================================================
 * SHM / Buffer API
 * =================================================================== */

/* Register a listener for SHM format events. */
int wl_shm_add_listener(wl_shm *shm, const wl_shm_listener *listener,
                        void *data);

/* Create a shared memory pool. `fd` is ignored on VeridianOS (kernel
 * allocates the backing memory). `size` is the pool size in bytes. */
wl_shm_pool *wl_shm_create_pool(wl_shm *shm, int32_t fd, int32_t size);

/* Destroy a shared memory pool. */
void wl_shm_pool_destroy(wl_shm_pool *pool);

/* Create a buffer within a pool at the given offset. */
wl_buffer *wl_shm_pool_create_buffer(wl_shm_pool *pool, int32_t offset,
                                     int32_t width, int32_t height,
                                     int32_t stride, uint32_t format);

/* Destroy a buffer. */
void wl_buffer_destroy(wl_buffer *buffer);

/* Get a pointer to the pool's raw pixel memory for direct rendering.
 * Returns NULL if the pool is kernel-managed and not memory-mapped. */
void *wl_shm_pool_get_data(wl_shm_pool *pool);

/* ===================================================================
 * XDG Shell API
 * =================================================================== */

/* Create an xdg_surface from a wl_surface. */
xdg_surface *xdg_wm_base_get_xdg_surface(xdg_wm_base *shell,
                                          wl_surface *surface);

/* Register a listener for xdg_wm_base ping events. */
int xdg_wm_base_add_listener(xdg_wm_base *shell,
                              const xdg_wm_base_listener *listener,
                              void *data);

/* Respond to a ping from the compositor. */
void xdg_wm_base_pong(xdg_wm_base *shell, uint32_t serial);

/* Get a toplevel role for an xdg_surface. */
xdg_toplevel *xdg_surface_get_toplevel(xdg_surface *surface);

/* Register a listener for xdg_surface configure events. */
int xdg_surface_add_listener(xdg_surface *surface,
                              const xdg_surface_listener *listener,
                              void *data);

/* Acknowledge a configure event. */
void xdg_surface_ack_configure(xdg_surface *surface, uint32_t serial);

/* Set the window title. */
void xdg_toplevel_set_title(xdg_toplevel *toplevel, const char *title);

/* Set the application ID (for desktop integration). */
void xdg_toplevel_set_app_id(xdg_toplevel *toplevel, const char *app_id);

/* Set minimum window size (0,0 = no minimum). */
void xdg_toplevel_set_min_size(xdg_toplevel *toplevel,
                                int32_t width, int32_t height);

/* Set maximum window size (0,0 = no maximum). */
void xdg_toplevel_set_max_size(xdg_toplevel *toplevel,
                                int32_t width, int32_t height);

/* Register a listener for toplevel configure/close events. */
int xdg_toplevel_add_listener(xdg_toplevel *toplevel,
                               const xdg_toplevel_listener *listener,
                               void *data);

/* Destroy the toplevel role. */
void xdg_toplevel_destroy(xdg_toplevel *toplevel);

/* ===================================================================
 * Seat / Input API
 * =================================================================== */

/* Register a listener for seat capability events. */
int wl_seat_add_listener(wl_seat *seat, const wl_seat_listener *listener,
                         void *data);

/* Get a keyboard object from the seat. */
wl_keyboard *wl_seat_get_keyboard(wl_seat *seat);

/* Get a pointer object from the seat. */
wl_pointer *wl_seat_get_pointer(wl_seat *seat);

/* Register a listener for keyboard events. */
int wl_keyboard_add_listener(wl_keyboard *kb,
                              const wl_keyboard_listener *listener,
                              void *data);

/* Register a listener for pointer events. */
int wl_pointer_add_listener(wl_pointer *ptr,
                             const wl_pointer_listener *listener,
                             void *data);

/* ===================================================================
 * Output API
 * =================================================================== */

/* Register a listener for output events. */
int wl_output_add_listener(wl_output *output,
                           const wl_output_listener *listener,
                           void *data);

/* Get the output's current scale factor. */
int32_t wl_output_get_scale(wl_output *output);

#endif /* WAYLAND_CLIENT_H */
