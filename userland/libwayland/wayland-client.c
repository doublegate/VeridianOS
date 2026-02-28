/*
 * wayland-client.c -- VeridianOS Wayland Client Library Implementation
 *
 * Implements the Wayland client API for user-space applications running on
 * VeridianOS. Uses raw syscalls to communicate with the kernel compositor.
 *
 * IMPORTANT: This file uses ONLY raw syscalls (no libc). It is linked as
 * a static library (libwayland-client.a) and can be used from both C and
 * Rust user-space programs.
 *
 * Protocol flow:
 *   1. wl_display_connect() -> SYS_WL_CONNECT -> kernel allocates client
 *   2. wl_display_get_registry() -> local registry object
 *   3. wl_registry_bind() -> local proxy objects (compositor, shm, shell)
 *   4. Surface/buffer creation -> SYS_WL_CREATE_SURFACE / SYS_WL_CREATE_POOL
 *   5. Rendering -> write to SHM pool, SYS_WL_ATTACH + SYS_WL_COMMIT
 *   6. Input -> SYS_WL_GET_EVENTS polls for input events
 */

#include "wayland-client.h"

/* ===================================================================
 * VeridianOS Syscall Numbers (from kernel/src/syscall/mod.rs)
 * =================================================================== */

#define SYS_WL_CONNECT        240
#define SYS_WL_CREATE_SURFACE 241
#define SYS_WL_CREATE_POOL    242
#define SYS_WL_CREATE_BUFFER  243
#define SYS_WL_ATTACH         244
#define SYS_WL_COMMIT         245
#define SYS_WL_DAMAGE         246
#define SYS_WL_GET_EVENTS     247

/* Memory management syscalls */
#define SYS_MEMORY_MAP        20
#define SYS_MEMORY_UNMAP      21

/* mmap flags */
#define PROT_READ   1
#define PROT_WRITE  2
#define MAP_PRIVATE 0x02
#define MAP_ANON    0x20

/* ===================================================================
 * Raw syscall wrappers (x86_64 SYSCALL convention)
 * =================================================================== */

static long syscall0(long nr)
{
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr)
        : "rcx", "r11", "memory");
    return ret;
}

static long syscall1(long nr, long a1)
{
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1)
        : "rcx", "r11", "memory");
    return ret;
}

static long syscall2(long nr, long a1, long a2)
{
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2)
        : "rcx", "r11", "memory");
    return ret;
}

static long syscall3(long nr, long a1, long a2, long a3)
{
    long ret;
    register long r10 __asm__("r10") = a3;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "r"(r10)
        : "rcx", "r11", "memory");
    return ret;
}

static long syscall4(long nr, long a1, long a2, long a3, long a4)
{
    long ret;
    register long r10 __asm__("r10") = a3;
    register long r8 __asm__("r8") = a4;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "r"(r10), "r"(r8)
        : "rcx", "r11", "memory");
    return ret;
}

static long syscall6(long nr, long a1, long a2, long a3,
                     long a4, long a5, long a6)
{
    long ret;
    register long r10 __asm__("r10") = a3;
    register long r8 __asm__("r8") = a4;
    register long r9 __asm__("r9") = a5;
    (void)a6; /* sixth arg via stack on some conventions; unused here */
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory");
    return ret;
}

/* ===================================================================
 * String utilities (no libc)
 * =================================================================== */

static int str_eq(const char *a, const char *b)
{
    while (*a && *b) {
        if (*a != *b)
            return 0;
        a++;
        b++;
    }
    return *a == *b;
}

static unsigned int str_len(const char *s)
{
    unsigned int len = 0;
    while (s[len])
        len++;
    return len;
}

static void mem_zero(void *dst, unsigned int n)
{
    unsigned char *p = (unsigned char *)dst;
    while (n--)
        *p++ = 0;
}

static void mem_copy(void *dst, const void *src, unsigned int n)
{
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    while (n--)
        *d++ = *s++;
}

/* ===================================================================
 * Maximum limits
 * =================================================================== */

#define MAX_GLOBALS     16
#define MAX_SURFACES    32
#define MAX_POOLS       16
#define MAX_BUFFERS     64
#define MAX_TITLE_LEN   128
#define EVENT_BUF_SIZE  4096

/* ===================================================================
 * Internal object structures
 *
 * Since we cannot use malloc (no libc), all objects are statically
 * allocated from fixed-size arrays. Object IDs index into these arrays.
 * =================================================================== */

/* Global interface binding */
struct wl_global_entry {
    uint32_t name;
    char interface[32];
    uint32_t version;
    int active;
};

/* Display (singleton) */
struct wl_display {
    int client_id;
    int connected;
    uint32_t serial;
    struct wl_global_entry globals[MAX_GLOBALS];
    int num_globals;
    uint8_t event_buf[EVENT_BUF_SIZE];
};

/* Registry */
struct wl_registry {
    wl_display *display;
    const wl_registry_listener *listener;
    void *listener_data;
};

/* Compositor proxy */
struct wl_compositor {
    wl_display *display;
    uint32_t global_name;
};

/* Surface */
struct wl_surface {
    uint32_t id;         /* kernel-assigned surface ID */
    wl_display *display;
    wl_buffer *attached_buffer;
    int32_t attach_x;
    int32_t attach_y;
    int active;
};

/* SHM proxy */
struct wl_shm {
    wl_display *display;
    uint32_t global_name;
    const wl_shm_listener *listener;
    void *listener_data;
};

/* SHM pool */
struct wl_shm_pool {
    uint32_t id;         /* kernel-assigned pool ID */
    wl_display *display;
    int32_t size;
    void *data;          /* mmap'd pool data (if available) */
    int active;
};

/* Buffer */
struct wl_buffer {
    uint32_t id;         /* buffer ID within pool */
    wl_shm_pool *pool;
    int32_t offset;
    int32_t width;
    int32_t height;
    int32_t stride;
    uint32_t format;
    int active;
};

/* Callback */
struct wl_callback {
    uint32_t id;
    const wl_callback_listener *listener;
    void *listener_data;
};

/* Seat */
struct wl_seat {
    wl_display *display;
    uint32_t global_name;
    uint32_t capabilities;
    const wl_seat_listener *listener;
    void *listener_data;
};

/* Keyboard */
struct wl_keyboard {
    wl_seat *seat;
    const wl_keyboard_listener *listener;
    void *listener_data;
};

/* Pointer */
struct wl_pointer {
    wl_seat *seat;
    const wl_pointer_listener *listener;
    void *listener_data;
};

/* Output */
struct wl_output {
    wl_display *display;
    uint32_t global_name;
    int32_t scale;
    const wl_output_listener *listener;
    void *listener_data;
};

/* XDG shell */
struct xdg_wm_base {
    wl_display *display;
    uint32_t global_name;
    const xdg_wm_base_listener *listener;
    void *listener_data;
};

/* XDG surface */
struct xdg_surface {
    wl_surface *wl_surface;
    xdg_wm_base *shell;
    uint32_t configure_serial;
    const xdg_surface_listener *listener;
    void *listener_data;
};

/* XDG toplevel */
struct xdg_toplevel {
    xdg_surface *xdg_surface;
    char title[MAX_TITLE_LEN];
    char app_id[MAX_TITLE_LEN];
    int32_t min_width;
    int32_t min_height;
    int32_t max_width;
    int32_t max_height;
    const xdg_toplevel_listener *listener;
    void *listener_data;
};

/* ===================================================================
 * Static object pools
 * =================================================================== */

static struct wl_display    s_display;
static struct wl_registry   s_registry;
static struct wl_compositor s_compositor;
static struct wl_shm        s_shm;
static struct wl_seat       s_seat;
static struct wl_keyboard   s_keyboard;
static struct wl_pointer    s_pointer;
static struct wl_output     s_output;
static struct xdg_wm_base   s_xdg_wm_base;

static struct wl_surface    s_surfaces[MAX_SURFACES];
static struct wl_shm_pool   s_pools[MAX_POOLS];
static struct wl_buffer     s_buffers[MAX_BUFFERS];
static struct xdg_surface   s_xdg_surfaces[MAX_SURFACES];
static struct xdg_toplevel  s_xdg_toplevels[MAX_SURFACES];
static struct wl_callback   s_callback;

static int s_initialized = 0;

/* ===================================================================
 * Internal helpers
 * =================================================================== */

static struct wl_surface *alloc_surface(void)
{
    for (int i = 0; i < MAX_SURFACES; i++) {
        if (!s_surfaces[i].active) {
            mem_zero(&s_surfaces[i], sizeof(struct wl_surface));
            s_surfaces[i].active = 1;
            s_surfaces[i].display = &s_display;
            return &s_surfaces[i];
        }
    }
    return (struct wl_surface *)0;
}

static struct wl_shm_pool *alloc_pool(void)
{
    for (int i = 0; i < MAX_POOLS; i++) {
        if (!s_pools[i].active) {
            mem_zero(&s_pools[i], sizeof(struct wl_shm_pool));
            s_pools[i].active = 1;
            s_pools[i].display = &s_display;
            return &s_pools[i];
        }
    }
    return (struct wl_shm_pool *)0;
}

static struct wl_buffer *alloc_buffer(void)
{
    for (int i = 0; i < MAX_BUFFERS; i++) {
        if (!s_buffers[i].active) {
            mem_zero(&s_buffers[i], sizeof(struct wl_buffer));
            s_buffers[i].active = 1;
            return &s_buffers[i];
        }
    }
    return (struct wl_buffer *)0;
}

/* ===================================================================
 * Core Display API
 * =================================================================== */

wl_display *wl_display_connect(const char *name)
{
    (void)name;

    if (s_initialized)
        return (wl_display *)0;

    mem_zero(&s_display, sizeof(s_display));
    mem_zero(&s_registry, sizeof(s_registry));
    mem_zero(&s_compositor, sizeof(s_compositor));
    mem_zero(&s_shm, sizeof(s_shm));
    mem_zero(&s_seat, sizeof(s_seat));
    mem_zero(&s_keyboard, sizeof(s_keyboard));
    mem_zero(&s_pointer, sizeof(s_pointer));
    mem_zero(&s_output, sizeof(s_output));
    mem_zero(&s_xdg_wm_base, sizeof(s_xdg_wm_base));
    mem_zero(s_surfaces, sizeof(s_surfaces));
    mem_zero(s_pools, sizeof(s_pools));
    mem_zero(s_buffers, sizeof(s_buffers));
    mem_zero(s_xdg_surfaces, sizeof(s_xdg_surfaces));
    mem_zero(s_xdg_toplevels, sizeof(s_xdg_toplevels));

    /* Connect to the kernel compositor */
    long ret = syscall0(SYS_WL_CONNECT);
    if (ret < 0)
        return (wl_display *)0;

    s_display.client_id = (int)ret;
    s_display.connected = 1;
    s_display.serial = 1;

    /* Pre-populate globals (matching kernel's WaylandDisplay::new()) */
    s_display.globals[0].name = 1;
    mem_copy(s_display.globals[0].interface, "wl_compositor", 14);
    s_display.globals[0].version = 4;
    s_display.globals[0].active = 1;

    s_display.globals[1].name = 2;
    mem_copy(s_display.globals[1].interface, "wl_shm", 7);
    s_display.globals[1].version = 1;
    s_display.globals[1].active = 1;

    s_display.globals[2].name = 3;
    mem_copy(s_display.globals[2].interface, "xdg_wm_base", 12);
    s_display.globals[2].version = 2;
    s_display.globals[2].active = 1;

    s_display.globals[3].name = 4;
    mem_copy(s_display.globals[3].interface, "wl_seat", 8);
    s_display.globals[3].version = 5;
    s_display.globals[3].active = 1;

    s_display.globals[4].name = 5;
    mem_copy(s_display.globals[4].interface, "wl_output", 10);
    s_display.globals[4].version = 4;
    s_display.globals[4].active = 1;

    s_display.num_globals = 5;

    s_initialized = 1;
    return &s_display;
}

void wl_display_disconnect(wl_display *display)
{
    if (!display || !display->connected)
        return;

    /* Free active pools */
    for (int i = 0; i < MAX_POOLS; i++) {
        if (s_pools[i].active && s_pools[i].data) {
            syscall2(SYS_MEMORY_UNMAP, (long)s_pools[i].data, (long)s_pools[i].size);
            s_pools[i].data = (void *)0;
        }
    }

    display->connected = 0;
    s_initialized = 0;
}

int wl_display_dispatch(wl_display *display)
{
    if (!display || !display->connected)
        return -1;

    /* Poll for events from kernel */
    long ret = syscall3(SYS_WL_GET_EVENTS,
                        (long)display->client_id,
                        (long)display->event_buf,
                        (long)EVENT_BUF_SIZE);
    if (ret < 0)
        return -1;

    /* TODO(phase7): Parse returned event buffer and dispatch to listeners */
    return 0;
}

int wl_display_roundtrip(wl_display *display)
{
    if (!display || !display->connected)
        return -1;

    /* Flush outgoing, dispatch incoming, increment serial */
    wl_display_flush(display);
    int rc = wl_display_dispatch(display);
    display->serial++;
    return rc;
}

int wl_display_flush(wl_display *display)
{
    if (!display || !display->connected)
        return -1;

    /* In the kernel-compositor model, requests are synchronous syscalls.
     * No buffered data to flush. */
    return 0;
}

int wl_display_get_fd(wl_display *display)
{
    (void)display;
    /* VeridianOS uses syscalls, not Unix domain sockets. No fd. */
    return -1;
}

/* ===================================================================
 * Registry API
 * =================================================================== */

wl_registry *wl_display_get_registry(wl_display *display)
{
    if (!display || !display->connected)
        return (wl_registry *)0;

    s_registry.display = display;
    return &s_registry;
}

int wl_registry_add_listener(wl_registry *registry,
                             const wl_registry_listener *listener,
                             void *data)
{
    if (!registry)
        return -1;

    registry->listener = listener;
    registry->listener_data = data;

    /* Fire global events for all known globals */
    if (listener && listener->global) {
        wl_display *display = registry->display;
        for (int i = 0; i < display->num_globals; i++) {
            if (display->globals[i].active) {
                listener->global(data, registry,
                                 display->globals[i].name,
                                 display->globals[i].interface,
                                 display->globals[i].version);
            }
        }
    }

    return 0;
}

void *wl_registry_bind(wl_registry *registry, uint32_t name,
                       const char *interface, uint32_t version)
{
    (void)version;

    if (!registry || !registry->display)
        return (void *)0;

    if (str_eq(interface, "wl_compositor")) {
        s_compositor.display = registry->display;
        s_compositor.global_name = name;
        return &s_compositor;
    }
    if (str_eq(interface, "wl_shm")) {
        s_shm.display = registry->display;
        s_shm.global_name = name;
        return &s_shm;
    }
    if (str_eq(interface, "xdg_wm_base")) {
        s_xdg_wm_base.display = registry->display;
        s_xdg_wm_base.global_name = name;
        return &s_xdg_wm_base;
    }
    if (str_eq(interface, "wl_seat")) {
        s_seat.display = registry->display;
        s_seat.global_name = name;
        s_seat.capabilities = WL_SEAT_CAPABILITY_POINTER
                            | WL_SEAT_CAPABILITY_KEYBOARD;
        return &s_seat;
    }
    if (str_eq(interface, "wl_output")) {
        s_output.display = registry->display;
        s_output.global_name = name;
        s_output.scale = 1;
        return &s_output;
    }

    return (void *)0;
}

/* ===================================================================
 * Compositor / Surface API
 * =================================================================== */

wl_surface *wl_compositor_create_surface(wl_compositor *compositor)
{
    if (!compositor || !compositor->display)
        return (wl_surface *)0;

    struct wl_surface *surface = alloc_surface();
    if (!surface)
        return (wl_surface *)0;

    /* Ask kernel to create a compositor surface */
    long ret = syscall4(SYS_WL_CREATE_SURFACE,
                        (long)compositor->display->client_id,
                        0, 0, 0);
    if (ret < 0) {
        surface->active = 0;
        return (wl_surface *)0;
    }

    surface->id = (uint32_t)ret;
    return surface;
}

void wl_surface_destroy(wl_surface *surface)
{
    if (!surface)
        return;
    /* TODO(phase7): Send destroy request to kernel */
    surface->active = 0;
}

void wl_surface_attach(wl_surface *surface, wl_buffer *buffer,
                       int32_t x, int32_t y)
{
    if (!surface)
        return;

    surface->attached_buffer = buffer;
    surface->attach_x = x;
    surface->attach_y = y;

    if (buffer && surface->display) {
        syscall4(SYS_WL_ATTACH,
                 (long)surface->display->client_id,
                 (long)surface->id,
                 (long)buffer->id,
                 0);
    }
}

void wl_surface_damage(wl_surface *surface, int32_t x, int32_t y,
                       int32_t width, int32_t height)
{
    if (!surface || !surface->display)
        return;

    /* Pack damage rect: x | y in first arg, w | h in second */
    uint64_t xy = ((uint64_t)(uint32_t)x << 32) | (uint32_t)y;
    uint64_t wh = ((uint64_t)(uint32_t)width << 32) | (uint32_t)height;
    syscall4(SYS_WL_DAMAGE,
             (long)surface->display->client_id,
             (long)surface->id,
             (long)xy,
             (long)wh);
}

void wl_surface_commit(wl_surface *surface)
{
    if (!surface || !surface->display)
        return;

    syscall2(SYS_WL_COMMIT,
             (long)surface->display->client_id,
             (long)surface->id);
}

wl_callback *wl_surface_frame(wl_surface *surface)
{
    (void)surface;
    /* Frame callbacks are not yet supported by the kernel compositor.
     * Return a stub callback object. */
    mem_zero(&s_callback, sizeof(s_callback));
    s_callback.id = 0;
    return &s_callback;
}

/* ===================================================================
 * SHM / Buffer API
 * =================================================================== */

int wl_shm_add_listener(wl_shm *shm, const wl_shm_listener *listener,
                        void *data)
{
    if (!shm)
        return -1;

    shm->listener = listener;
    shm->listener_data = data;

    /* Announce supported formats */
    if (listener && listener->format) {
        listener->format(data, shm, WL_SHM_FORMAT_ARGB8888);
        listener->format(data, shm, WL_SHM_FORMAT_XRGB8888);
    }
    return 0;
}

wl_shm_pool *wl_shm_create_pool(wl_shm *shm, int32_t fd, int32_t size)
{
    (void)fd; /* fd is ignored; kernel allocates backing memory */

    if (!shm || !shm->display || size <= 0)
        return (wl_shm_pool *)0;

    struct wl_shm_pool *pool = alloc_pool();
    if (!pool)
        return (wl_shm_pool *)0;

    /* Ask kernel to create a SHM pool */
    long ret = syscall2(SYS_WL_CREATE_POOL,
                        (long)shm->display->client_id,
                        (long)size);
    if (ret < 0) {
        pool->active = 0;
        return (wl_shm_pool *)0;
    }

    pool->id = (uint32_t)ret;
    pool->size = size;

    /* Attempt to mmap the pool for direct pixel access.
     * This may fail if the kernel does not support user-mappable SHM pools. */
    long map_ret = syscall6(SYS_MEMORY_MAP,
                            0,
                            (long)size,
                            (long)(PROT_READ | PROT_WRITE),
                            (long)(MAP_PRIVATE | MAP_ANON),
                            -1,
                            0);
    if (map_ret > 0) {
        pool->data = (void *)map_ret;
    }

    return pool;
}

void wl_shm_pool_destroy(wl_shm_pool *pool)
{
    if (!pool)
        return;

    if (pool->data) {
        syscall2(SYS_MEMORY_UNMAP, (long)pool->data, (long)pool->size);
        pool->data = (void *)0;
    }
    pool->active = 0;
}

wl_buffer *wl_shm_pool_create_buffer(wl_shm_pool *pool, int32_t offset,
                                     int32_t width, int32_t height,
                                     int32_t stride, uint32_t format)
{
    if (!pool || !pool->active)
        return (wl_buffer *)0;

    struct wl_buffer *buffer = alloc_buffer();
    if (!buffer)
        return (wl_buffer *)0;

    /* Ask kernel to create a buffer within the pool */
    long ret = syscall4(SYS_WL_CREATE_BUFFER,
                        (long)pool->id,
                        (long)((uint64_t)width << 32 | (uint32_t)height),
                        (long)stride,
                        (long)format);

    buffer->id = (ret > 0) ? (uint32_t)ret : 0;
    buffer->pool = pool;
    buffer->offset = offset;
    buffer->width = width;
    buffer->height = height;
    buffer->stride = stride;
    buffer->format = format;

    return buffer;
}

void wl_buffer_destroy(wl_buffer *buffer)
{
    if (!buffer)
        return;
    buffer->active = 0;
}

void *wl_shm_pool_get_data(wl_shm_pool *pool)
{
    if (!pool)
        return (void *)0;
    return pool->data;
}

/* ===================================================================
 * XDG Shell API
 * =================================================================== */

int xdg_wm_base_add_listener(xdg_wm_base *shell,
                              const xdg_wm_base_listener *listener,
                              void *data)
{
    if (!shell)
        return -1;

    shell->listener = listener;
    shell->listener_data = data;
    return 0;
}

void xdg_wm_base_pong(xdg_wm_base *shell, uint32_t serial)
{
    (void)shell;
    (void)serial;
    /* Pong is handled by the kernel; no-op in user space */
}

xdg_surface *xdg_wm_base_get_xdg_surface(xdg_wm_base *shell,
                                          wl_surface *surface)
{
    if (!shell || !surface)
        return (xdg_surface *)0;

    /* Find a free xdg_surface slot */
    for (int i = 0; i < MAX_SURFACES; i++) {
        if (!s_xdg_surfaces[i].wl_surface) {
            mem_zero(&s_xdg_surfaces[i], sizeof(struct xdg_surface));
            s_xdg_surfaces[i].wl_surface = surface;
            s_xdg_surfaces[i].shell = shell;
            return &s_xdg_surfaces[i];
        }
    }
    return (xdg_surface *)0;
}

int xdg_surface_add_listener(xdg_surface *surface,
                              const xdg_surface_listener *listener,
                              void *data)
{
    if (!surface)
        return -1;

    surface->listener = listener;
    surface->listener_data = data;
    return 0;
}

void xdg_surface_ack_configure(xdg_surface *surface, uint32_t serial)
{
    if (!surface)
        return;
    surface->configure_serial = serial;
}

xdg_toplevel *xdg_surface_get_toplevel(xdg_surface *surface)
{
    if (!surface)
        return (xdg_toplevel *)0;

    /* Find a free toplevel slot */
    for (int i = 0; i < MAX_SURFACES; i++) {
        if (!s_xdg_toplevels[i].xdg_surface) {
            mem_zero(&s_xdg_toplevels[i], sizeof(struct xdg_toplevel));
            s_xdg_toplevels[i].xdg_surface = surface;
            return &s_xdg_toplevels[i];
        }
    }
    return (xdg_toplevel *)0;
}

int xdg_toplevel_add_listener(xdg_toplevel *toplevel,
                               const xdg_toplevel_listener *listener,
                               void *data)
{
    if (!toplevel)
        return -1;

    toplevel->listener = listener;
    toplevel->listener_data = data;

    /* Send initial configure event */
    if (listener && listener->configure) {
        listener->configure(data, toplevel, 0, 0, (void *)0, 0);
    }
    return 0;
}

void xdg_toplevel_set_title(xdg_toplevel *toplevel, const char *title)
{
    if (!toplevel || !title)
        return;

    unsigned int len = str_len(title);
    if (len >= MAX_TITLE_LEN)
        len = MAX_TITLE_LEN - 1;
    mem_copy(toplevel->title, title, len);
    toplevel->title[len] = '\0';
}

void xdg_toplevel_set_app_id(xdg_toplevel *toplevel, const char *app_id)
{
    if (!toplevel || !app_id)
        return;

    unsigned int len = str_len(app_id);
    if (len >= MAX_TITLE_LEN)
        len = MAX_TITLE_LEN - 1;
    mem_copy(toplevel->app_id, app_id, len);
    toplevel->app_id[len] = '\0';
}

void xdg_toplevel_set_min_size(xdg_toplevel *toplevel,
                                int32_t width, int32_t height)
{
    if (!toplevel)
        return;
    toplevel->min_width = width;
    toplevel->min_height = height;
}

void xdg_toplevel_set_max_size(xdg_toplevel *toplevel,
                                int32_t width, int32_t height)
{
    if (!toplevel)
        return;
    toplevel->max_width = width;
    toplevel->max_height = height;
}

void xdg_toplevel_destroy(xdg_toplevel *toplevel)
{
    if (!toplevel)
        return;
    toplevel->xdg_surface = (xdg_surface *)0;
}

/* ===================================================================
 * Seat / Input API
 * =================================================================== */

int wl_seat_add_listener(wl_seat *seat, const wl_seat_listener *listener,
                         void *data)
{
    if (!seat)
        return -1;

    seat->listener = listener;
    seat->listener_data = data;

    /* Report capabilities immediately */
    if (listener && listener->capabilities) {
        listener->capabilities(data, seat, seat->capabilities);
    }
    if (listener && listener->name) {
        listener->name(data, seat, "default");
    }
    return 0;
}

wl_keyboard *wl_seat_get_keyboard(wl_seat *seat)
{
    if (!seat)
        return (wl_keyboard *)0;

    s_keyboard.seat = seat;
    return &s_keyboard;
}

wl_pointer *wl_seat_get_pointer(wl_seat *seat)
{
    if (!seat)
        return (wl_pointer *)0;

    s_pointer.seat = seat;
    return &s_pointer;
}

int wl_keyboard_add_listener(wl_keyboard *kb,
                              const wl_keyboard_listener *listener,
                              void *data)
{
    if (!kb)
        return -1;

    kb->listener = listener;
    kb->listener_data = data;
    return 0;
}

int wl_pointer_add_listener(wl_pointer *ptr,
                             const wl_pointer_listener *listener,
                             void *data)
{
    if (!ptr)
        return -1;

    ptr->listener = listener;
    ptr->listener_data = data;
    return 0;
}

/* ===================================================================
 * Output API
 * =================================================================== */

int wl_output_add_listener(wl_output *output,
                           const wl_output_listener *listener,
                           void *data)
{
    if (!output)
        return -1;

    output->listener = listener;
    output->listener_data = data;

    /* Send initial output events */
    if (listener) {
        if (listener->geometry) {
            listener->geometry(data, output,
                               0, 0,        /* x, y */
                               0, 0,        /* physical size (unknown) */
                               0,           /* subpixel: unknown */
                               "VeridianOS", "Virtual Display",
                               WL_OUTPUT_TRANSFORM_NORMAL);
        }
        if (listener->mode) {
            listener->mode(data, output,
                           WL_OUTPUT_MODE_CURRENT | WL_OUTPUT_MODE_PREFERRED,
                           1280, 800,   /* width, height */
                           60000);      /* refresh: 60Hz in mHz */
        }
        if (listener->scale) {
            listener->scale(data, output, output->scale);
        }
        if (listener->done) {
            listener->done(data, output);
        }
    }
    return 0;
}

int32_t wl_output_get_scale(wl_output *output)
{
    if (!output)
        return 1;
    return output->scale;
}
