/*
 * VeridianOS libc -- libinput.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal libinput shim for VeridianOS.
 * Reads from /dev/input/event* devices and converts evdev events
 * to the libinput event API.
 */

#include <libinput.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>

/* ========================================================================= */
/* Internal: evdev event structure (must match kernel InputEvent)            */
/* ========================================================================= */

struct input_event {
    uint64_t timestamp;
    uint16_t type;
    uint16_t code;
    int32_t  value;
};

/* evdev event types */
#define EV_KEY  0x01
#define EV_REL  0x02

/* Relative axis codes */
#define REL_X  0x00
#define REL_Y  0x01

/* Mouse button codes */
#define BTN_LEFT   0x110
#define BTN_RIGHT  0x111
#define BTN_MIDDLE 0x112

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

#define MAX_DEVICES  4
#define MAX_EVENTS  64

/** Internal device representation */
struct libinput_device {
    int         fd;
    char        path[128];
    char        name[64];
    char        sysname[32];
    int         caps;        /* Bitmask of libinput_device_capability */
    int         refcount;
    struct libinput *li;
};

/** Internal seat (single seat: "seat0") */
struct libinput_seat {
    char logical_name[32];
    char physical_name[32];
};

/** Generic event (with union for typed sub-events) */
struct libinput_event {
    enum libinput_event_type type;
    struct libinput_device  *device;
    uint64_t                 time_usec;

    /* Sub-event data */
    union {
        struct {
            uint32_t key;
            enum libinput_key_state state;
        } keyboard;
        struct {
            int32_t  dx;    /* Fixed-point 16.16 for sub-pixel, stored as int */
            int32_t  dy;
            uint32_t button;
            enum libinput_button_state button_state;
            int      has_axis_v;
            int      has_axis_h;
            int32_t  axis_v;
            int32_t  axis_h;
        } pointer;
    } data;
};

/** Main libinput context */
struct libinput {
    const struct libinput_interface *interface;
    void                           *user_data;
    int                             refcount;

    struct libinput_device          *devices[MAX_DEVICES];
    int                              device_count;

    struct libinput_event           *events[MAX_EVENTS];
    int                              event_head;
    int                              event_tail;
    int                              event_count;

    struct libinput_seat              seat;

    /* Pointer acceleration state (simple linear) */
    int32_t                          accel_numerator;   /* 16.16 fixed-point */
    int32_t                          accel_denominator; /* 16.16 fixed-point */
};

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

/** Push an event into the context's event queue. */
static void push_event(struct libinput *li, struct libinput_event *ev)
{
    if (li->event_count >= MAX_EVENTS) {
        /* Queue full: drop oldest */
        free(li->events[li->event_tail]);
        li->event_tail = (li->event_tail + 1) % MAX_EVENTS;
        li->event_count--;
    }

    li->events[li->event_head] = ev;
    li->event_head = (li->event_head + 1) % MAX_EVENTS;
    li->event_count++;
}

/** Pop an event from the context's event queue. */
static struct libinput_event *pop_event(struct libinput *li)
{
    struct libinput_event *ev;

    if (li->event_count <= 0)
        return NULL;

    ev = li->events[li->event_tail];
    li->events[li->event_tail] = NULL;
    li->event_tail = (li->event_tail + 1) % MAX_EVENTS;
    li->event_count--;
    return ev;
}

/** Apply simple pointer acceleration. Returns accelerated value. */
static int32_t apply_accel(struct libinput *li, int32_t raw)
{
    /* Simple 1:1 mapping with configurable multiplier.
     * Default: accel_numerator = 1 << 16, denominator = 1 << 16 (1.0x)
     * For higher sensitivity, increase numerator.
     */
    int64_t result = (int64_t)raw * li->accel_numerator;
    return (int32_t)(result >> 16);
}

/* ========================================================================= */
/* Context management                                                        */
/* ========================================================================= */

struct libinput *libinput_path_create_context(
    const struct libinput_interface *interface,
    void *user_data)
{
    struct libinput *li;

    if (!interface)
        return NULL;

    li = calloc(1, sizeof(*li));
    if (!li)
        return NULL;

    li->interface  = interface;
    li->user_data  = user_data;
    li->refcount   = 1;

    /* Default acceleration: 1.0x (16.16 fixed-point) */
    li->accel_numerator   = 1 << 16;
    li->accel_denominator = 1 << 16;

    /* Default seat */
    strcpy(li->seat.logical_name, "default");
    strcpy(li->seat.physical_name, "seat0");

    return li;
}

struct libinput_device *libinput_path_add_device(
    struct libinput *li,
    const char *path)
{
    struct libinput_device *dev;
    int fd;

    if (!li || !path || li->device_count >= MAX_DEVICES)
        return NULL;

    /* Open device through the interface callback */
    fd = li->interface->open_restricted(path, O_RDONLY | O_NONBLOCK,
                                        li->user_data);
    if (fd < 0)
        return NULL;

    dev = calloc(1, sizeof(*dev));
    if (!dev) {
        li->interface->close_restricted(fd, li->user_data);
        return NULL;
    }

    dev->fd = fd;
    dev->refcount = 1;
    dev->li = li;
    strncpy(dev->path, path, sizeof(dev->path) - 1);

    /* Determine device type from path */
    if (strstr(path, "event0")) {
        strcpy(dev->name, "VeridianOS PS/2 Keyboard");
        strcpy(dev->sysname, "event0");
        dev->caps = (1 << LIBINPUT_DEVICE_CAP_KEYBOARD);
    } else if (strstr(path, "event1")) {
        strcpy(dev->name, "VeridianOS PS/2 Mouse");
        strcpy(dev->sysname, "event1");
        dev->caps = (1 << LIBINPUT_DEVICE_CAP_POINTER);
    } else {
        strcpy(dev->name, "Unknown Input Device");
        strcpy(dev->sysname, "eventN");
    }

    li->devices[li->device_count++] = dev;

    /* Generate DEVICE_ADDED event */
    {
        struct libinput_event *ev = calloc(1, sizeof(*ev));
        if (ev) {
            ev->type = LIBINPUT_EVENT_DEVICE_ADDED;
            ev->device = dev;
            ev->time_usec = 0;
            push_event(li, ev);
        }
    }

    return dev;
}

void libinput_path_remove_device(struct libinput_device *device)
{
    if (!device || !device->li)
        return;

    /* Generate DEVICE_REMOVED event */
    {
        struct libinput_event *ev = calloc(1, sizeof(*ev));
        if (ev) {
            ev->type = LIBINPUT_EVENT_DEVICE_REMOVED;
            ev->device = device;
            push_event(device->li, ev);
        }
    }

    /* Remove from context's device list */
    struct libinput *li = device->li;
    for (int i = 0; i < li->device_count; i++) {
        if (li->devices[i] == device) {
            /* Shift remaining devices down */
            for (int j = i; j < li->device_count - 1; j++)
                li->devices[j] = li->devices[j + 1];
            li->devices[li->device_count - 1] = NULL;
            li->device_count--;
            break;
        }
    }

    /* Close fd */
    if (device->fd >= 0 && li->interface)
        li->interface->close_restricted(device->fd, li->user_data);

    device->fd = -1;
}

void libinput_unref(struct libinput *li)
{
    if (!li)
        return;

    li->refcount--;
    if (li->refcount > 0)
        return;

    /* Remove all devices */
    while (li->device_count > 0)
        libinput_path_remove_device(li->devices[0]);

    /* Free queued events */
    while (li->event_count > 0) {
        struct libinput_event *ev = pop_event(li);
        free(ev);
    }

    free(li);
}

struct libinput *libinput_ref(struct libinput *li)
{
    if (li)
        li->refcount++;
    return li;
}

/* ========================================================================= */
/* Event dispatching                                                         */
/* ========================================================================= */

int libinput_dispatch(struct libinput *li)
{
    struct input_event raw;
    int i;

    if (!li)
        return -1;

    /* Read events from all device fds */
    for (i = 0; i < li->device_count; i++) {
        struct libinput_device *dev = li->devices[i];
        if (!dev || dev->fd < 0)
            continue;

        /* Read as many events as available (non-blocking) */
        while (1) {
            int n = (int)read(dev->fd, &raw, sizeof(raw));
            if (n < (int)sizeof(raw))
                break;

            struct libinput_event *ev = calloc(1, sizeof(*ev));
            if (!ev)
                break;

            ev->device = dev;
            ev->time_usec = raw.timestamp;

            switch (raw.type) {
            case EV_KEY:
                if (raw.code >= BTN_LEFT && raw.code <= BTN_MIDDLE) {
                    /* Mouse button */
                    ev->type = LIBINPUT_EVENT_POINTER_BUTTON;
                    ev->data.pointer.button = raw.code;
                    ev->data.pointer.button_state =
                        raw.value ? LIBINPUT_BUTTON_STATE_PRESSED
                                  : LIBINPUT_BUTTON_STATE_RELEASED;
                } else {
                    /* Keyboard key */
                    ev->type = LIBINPUT_EVENT_KEYBOARD_KEY;
                    ev->data.keyboard.key = raw.code;
                    ev->data.keyboard.state =
                        raw.value ? LIBINPUT_KEY_STATE_PRESSED
                                  : LIBINPUT_KEY_STATE_RELEASED;
                }
                break;

            case EV_REL:
                ev->type = LIBINPUT_EVENT_POINTER_MOTION;
                if (raw.code == REL_X) {
                    ev->data.pointer.dx = apply_accel(li, raw.value);
                } else if (raw.code == REL_Y) {
                    ev->data.pointer.dy = apply_accel(li, raw.value);
                }
                break;

            default:
                free(ev);
                ev = NULL;
                break;
            }

            if (ev)
                push_event(li, ev);
        }
    }

    return 0;
}

struct libinput_event *libinput_get_event(struct libinput *li)
{
    if (!li)
        return NULL;
    return pop_event(li);
}

int libinput_get_fd(struct libinput *li)
{
    /* Return first device fd (for simple poll integration) */
    if (!li || li->device_count == 0)
        return -1;
    return li->devices[0]->fd;
}

/* ========================================================================= */
/* Event accessors                                                           */
/* ========================================================================= */

enum libinput_event_type libinput_event_get_type(
    struct libinput_event *event)
{
    return event ? event->type : LIBINPUT_EVENT_NONE;
}

struct libinput_device *libinput_event_get_device(
    struct libinput_event *event)
{
    return event ? event->device : NULL;
}

void libinput_event_destroy(struct libinput_event *event)
{
    free(event);
}

/* ========================================================================= */
/* Keyboard event accessors                                                  */
/* ========================================================================= */

struct libinput_event_keyboard *libinput_event_get_keyboard_event(
    struct libinput_event *event)
{
    if (!event || event->type != LIBINPUT_EVENT_KEYBOARD_KEY)
        return NULL;
    return (struct libinput_event_keyboard *)event;
}

uint32_t libinput_event_keyboard_get_key(
    struct libinput_event_keyboard *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? ev->data.keyboard.key : 0;
}

enum libinput_key_state libinput_event_keyboard_get_key_state(
    struct libinput_event_keyboard *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? ev->data.keyboard.state : LIBINPUT_KEY_STATE_RELEASED;
}

uint64_t libinput_event_keyboard_get_time_usec(
    struct libinput_event_keyboard *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? ev->time_usec : 0;
}

/* ========================================================================= */
/* Pointer event accessors                                                   */
/* ========================================================================= */

struct libinput_event_pointer *libinput_event_get_pointer_event(
    struct libinput_event *event)
{
    if (!event)
        return NULL;
    if (event->type != LIBINPUT_EVENT_POINTER_MOTION &&
        event->type != LIBINPUT_EVENT_POINTER_BUTTON &&
        event->type != LIBINPUT_EVENT_POINTER_AXIS)
        return NULL;
    return (struct libinput_event_pointer *)event;
}

double libinput_event_pointer_get_dx(
    struct libinput_event_pointer *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? (double)ev->data.pointer.dx : 0.0;
}

double libinput_event_pointer_get_dy(
    struct libinput_event_pointer *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? (double)ev->data.pointer.dy : 0.0;
}

uint32_t libinput_event_pointer_get_button(
    struct libinput_event_pointer *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? ev->data.pointer.button : 0;
}

enum libinput_button_state libinput_event_pointer_get_button_state(
    struct libinput_event_pointer *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? ev->data.pointer.button_state
              : LIBINPUT_BUTTON_STATE_RELEASED;
}

int libinput_event_pointer_has_axis(
    struct libinput_event_pointer *event,
    enum libinput_pointer_axis axis)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    if (!ev)
        return 0;
    if (axis == LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL)
        return ev->data.pointer.has_axis_v;
    if (axis == LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL)
        return ev->data.pointer.has_axis_h;
    return 0;
}

double libinput_event_pointer_get_axis_value(
    struct libinput_event_pointer *event,
    enum libinput_pointer_axis axis)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    if (!ev)
        return 0.0;
    if (axis == LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL)
        return (double)ev->data.pointer.axis_v;
    if (axis == LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL)
        return (double)ev->data.pointer.axis_h;
    return 0.0;
}

uint64_t libinput_event_pointer_get_time_usec(
    struct libinput_event_pointer *event)
{
    struct libinput_event *ev = (struct libinput_event *)event;
    return ev ? ev->time_usec : 0;
}

/* ========================================================================= */
/* Device accessors                                                          */
/* ========================================================================= */

const char *libinput_device_get_name(struct libinput_device *device)
{
    return device ? device->name : NULL;
}

int libinput_device_has_capability(
    struct libinput_device *device,
    enum libinput_device_capability cap)
{
    if (!device)
        return 0;
    return (device->caps >> cap) & 1;
}

const char *libinput_device_get_sysname(struct libinput_device *device)
{
    return device ? device->sysname : NULL;
}

struct libinput_device *libinput_device_ref(struct libinput_device *device)
{
    if (device)
        device->refcount++;
    return device;
}

struct libinput_device *libinput_device_unref(struct libinput_device *device)
{
    if (!device)
        return NULL;

    device->refcount--;
    if (device->refcount <= 0) {
        free(device);
        return NULL;
    }
    return device;
}

/* ========================================================================= */
/* Seat                                                                      */
/* ========================================================================= */

struct libinput_seat *libinput_device_get_seat(
    struct libinput_device *device)
{
    if (!device || !device->li)
        return NULL;
    return &device->li->seat;
}

const char *libinput_seat_get_logical_name(struct libinput_seat *seat)
{
    return seat ? seat->logical_name : NULL;
}

const char *libinput_seat_get_physical_name(struct libinput_seat *seat)
{
    return seat ? seat->physical_name : NULL;
}
