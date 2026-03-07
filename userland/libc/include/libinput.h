/*
 * VeridianOS libc -- <libinput.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal libinput shim.
 * Reads from /dev/input/event* devices and provides a higher-level
 * input event API compatible with libinput consumers (KWin, etc.).
 */

#ifndef _LIBINPUT_H
#define _LIBINPUT_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>

/* ========================================================================= */
/* Event types                                                               */
/* ========================================================================= */

enum libinput_event_type {
    LIBINPUT_EVENT_NONE = 0,

    /* Device events */
    LIBINPUT_EVENT_DEVICE_ADDED = 1,
    LIBINPUT_EVENT_DEVICE_REMOVED = 2,

    /* Keyboard events */
    LIBINPUT_EVENT_KEYBOARD_KEY = 300,

    /* Pointer events */
    LIBINPUT_EVENT_POINTER_MOTION = 400,
    LIBINPUT_EVENT_POINTER_MOTION_ABSOLUTE = 401,
    LIBINPUT_EVENT_POINTER_BUTTON = 402,
    LIBINPUT_EVENT_POINTER_AXIS = 403,
    LIBINPUT_EVENT_POINTER_SCROLL_WHEEL = 404,
    LIBINPUT_EVENT_POINTER_SCROLL_FINGER = 405,
    LIBINPUT_EVENT_POINTER_SCROLL_CONTINUOUS = 406,

    /* Touch events */
    LIBINPUT_EVENT_TOUCH_DOWN = 500,
    LIBINPUT_EVENT_TOUCH_UP = 501,
    LIBINPUT_EVENT_TOUCH_MOTION = 502,
    LIBINPUT_EVENT_TOUCH_CANCEL = 503,
    LIBINPUT_EVENT_TOUCH_FRAME = 504,
};

/** Key state */
enum libinput_key_state {
    LIBINPUT_KEY_STATE_RELEASED = 0,
    LIBINPUT_KEY_STATE_PRESSED = 1,
};

/** Button state */
enum libinput_button_state {
    LIBINPUT_BUTTON_STATE_RELEASED = 0,
    LIBINPUT_BUTTON_STATE_PRESSED = 1,
};

/** Pointer axis */
enum libinput_pointer_axis {
    LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL = 0,
    LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL = 1,
};

/** Pointer axis source */
enum libinput_pointer_axis_source {
    LIBINPUT_POINTER_AXIS_SOURCE_WHEEL = 1,
    LIBINPUT_POINTER_AXIS_SOURCE_FINGER = 2,
    LIBINPUT_POINTER_AXIS_SOURCE_CONTINUOUS = 3,
};

/** Device capabilities */
enum libinput_device_capability {
    LIBINPUT_DEVICE_CAP_KEYBOARD = 0,
    LIBINPUT_DEVICE_CAP_POINTER = 1,
    LIBINPUT_DEVICE_CAP_TOUCH = 2,
    LIBINPUT_DEVICE_CAP_TABLET_TOOL = 3,
    LIBINPUT_DEVICE_CAP_TABLET_PAD = 4,
    LIBINPUT_DEVICE_CAP_GESTURE = 5,
    LIBINPUT_DEVICE_CAP_SWITCH = 6,
};

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

struct libinput;
struct libinput_event;
struct libinput_event_keyboard;
struct libinput_event_pointer;
struct libinput_event_touch;
struct libinput_device;
struct libinput_seat;

/* ========================================================================= */
/* Interface (for udev/path backends)                                        */
/* ========================================================================= */

/**
 * libinput interface for open/close device callbacks.
 * The path backend uses these to open /dev/input/event* devices.
 */
struct libinput_interface {
    /**
     * Open a device at the given path.
     * Returns an fd on success, or a negative errno on failure.
     */
    int (*open_restricted)(const char *path, int flags, void *user_data);

    /**
     * Close a previously opened device fd.
     */
    void (*close_restricted)(int fd, void *user_data);
};

/* ========================================================================= */
/* Context management                                                        */
/* ========================================================================= */

/**
 * Create a libinput context using the path backend.
 * Opens specific device paths (e.g., "/dev/input/event0").
 */
struct libinput *libinput_path_create_context(
    const struct libinput_interface *interface,
    void *user_data);

/**
 * Add a device to a path-based context.
 * Returns the libinput_device or NULL on failure.
 */
struct libinput_device *libinput_path_add_device(
    struct libinput *li,
    const char *path);

/**
 * Remove a device from a path-based context.
 */
void libinput_path_remove_device(struct libinput_device *device);

/**
 * Destroy a libinput context.
 */
void libinput_unref(struct libinput *li);

/**
 * Add a reference to a libinput context.
 */
struct libinput *libinput_ref(struct libinput *li);

/* ========================================================================= */
/* Event dispatching                                                         */
/* ========================================================================= */

/**
 * Read events from the device fds.
 * Returns 0 on success, negative errno on failure.
 */
int libinput_dispatch(struct libinput *li);

/**
 * Get the next event from the internal queue.
 * Returns NULL if no events are available.
 */
struct libinput_event *libinput_get_event(struct libinput *li);

/**
 * Get the file descriptor for poll/epoll integration.
 */
int libinput_get_fd(struct libinput *li);

/* ========================================================================= */
/* Event accessors                                                           */
/* ========================================================================= */

/** Get event type. */
enum libinput_event_type libinput_event_get_type(
    struct libinput_event *event);

/** Get the device that generated the event. */
struct libinput_device *libinput_event_get_device(
    struct libinput_event *event);

/** Destroy an event. */
void libinput_event_destroy(struct libinput_event *event);

/* ========================================================================= */
/* Keyboard event accessors                                                  */
/* ========================================================================= */

/** Get keyboard event from generic event. */
struct libinput_event_keyboard *libinput_event_get_keyboard_event(
    struct libinput_event *event);

/** Get the key code. */
uint32_t libinput_event_keyboard_get_key(
    struct libinput_event_keyboard *event);

/** Get key state (pressed/released). */
enum libinput_key_state libinput_event_keyboard_get_key_state(
    struct libinput_event_keyboard *event);

/** Get event timestamp in microseconds. */
uint64_t libinput_event_keyboard_get_time_usec(
    struct libinput_event_keyboard *event);

/* ========================================================================= */
/* Pointer event accessors                                                   */
/* ========================================================================= */

/** Get pointer event from generic event. */
struct libinput_event_pointer *libinput_event_get_pointer_event(
    struct libinput_event *event);

/** Get relative X motion (unaccelerated). */
double libinput_event_pointer_get_dx(
    struct libinput_event_pointer *event);

/** Get relative Y motion (unaccelerated). */
double libinput_event_pointer_get_dy(
    struct libinput_event_pointer *event);

/** Get button code. */
uint32_t libinput_event_pointer_get_button(
    struct libinput_event_pointer *event);

/** Get button state. */
enum libinput_button_state libinput_event_pointer_get_button_state(
    struct libinput_event_pointer *event);

/** Check if axis has value in this event. */
int libinput_event_pointer_has_axis(
    struct libinput_event_pointer *event,
    enum libinput_pointer_axis axis);

/** Get scroll axis value. */
double libinput_event_pointer_get_axis_value(
    struct libinput_event_pointer *event,
    enum libinput_pointer_axis axis);

/** Get event timestamp in microseconds. */
uint64_t libinput_event_pointer_get_time_usec(
    struct libinput_event_pointer *event);

/* ========================================================================= */
/* Device accessors                                                          */
/* ========================================================================= */

/** Get device name. */
const char *libinput_device_get_name(struct libinput_device *device);

/** Check if device has a capability. */
int libinput_device_has_capability(
    struct libinput_device *device,
    enum libinput_device_capability cap);

/** Get device sysname (e.g. "event0"). */
const char *libinput_device_get_sysname(struct libinput_device *device);

/** Reference a device. */
struct libinput_device *libinput_device_ref(struct libinput_device *device);

/** Unreference a device. */
struct libinput_device *libinput_device_unref(struct libinput_device *device);

/* ========================================================================= */
/* Seat                                                                      */
/* ========================================================================= */

/** Get the seat a device belongs to. */
struct libinput_seat *libinput_device_get_seat(
    struct libinput_device *device);

/** Get seat name. */
const char *libinput_seat_get_logical_name(struct libinput_seat *seat);

/** Get physical seat name. */
const char *libinput_seat_get_physical_name(struct libinput_seat *seat);

#ifdef __cplusplus
}
#endif

#endif /* _LIBINPUT_H */
