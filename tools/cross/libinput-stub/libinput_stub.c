/* Minimal libinput API stub for VeridianOS KWin
 *
 * Provides the libinput API surface that KWin needs to compile and link.
 * Input events are delivered through VeridianOS's kernel input subsystem
 * (PS/2 keyboard + VirtIO mouse) rather than libinput's evdev backend.
 *
 * This stub implements the following strategy:
 * - All creation functions return valid (but inert) opaque pointers
 * - All event query functions return "no events"
 * - All configuration functions succeed silently
 *
 * When real input needs to reach KWin, the VeridianOS KWin platform
 * backend (userland/kwin/kwin_input_veridian.cpp) injects synthetic
 * events into KWin's input pipeline directly.
 */
#include <stdlib.h>
#include <stdint.h>

/* Opaque types matching libinput's public API */
struct libinput { int dummy; };
struct libinput_device { int dummy; };
struct libinput_event { int type; };
struct libinput_event_keyboard { struct libinput_event base; uint32_t key; int state; };
struct libinput_event_pointer { struct libinput_event base; double dx, dy; uint32_t button; int state; };
struct libinput_seat { int dummy; };
struct udev { int dummy; };

enum libinput_event_type {
    LIBINPUT_EVENT_NONE = 0,
    LIBINPUT_EVENT_DEVICE_ADDED = 1,
    LIBINPUT_EVENT_DEVICE_REMOVED = 2,
    LIBINPUT_EVENT_KEYBOARD_KEY = 300,
    LIBINPUT_EVENT_POINTER_MOTION = 400,
    LIBINPUT_EVENT_POINTER_MOTION_ABSOLUTE = 401,
    LIBINPUT_EVENT_POINTER_BUTTON = 402,
    LIBINPUT_EVENT_POINTER_AXIS = 403,
    LIBINPUT_EVENT_TOUCH_DOWN = 500,
    LIBINPUT_EVENT_TOUCH_UP = 501,
    LIBINPUT_EVENT_TOUCH_MOTION = 502,
};

enum libinput_key_state {
    LIBINPUT_KEY_STATE_RELEASED = 0,
    LIBINPUT_KEY_STATE_PRESSED = 1,
};

enum libinput_button_state {
    LIBINPUT_BUTTON_STATE_RELEASED = 0,
    LIBINPUT_BUTTON_STATE_PRESSED = 1,
};

/* Context creation */
struct libinput *libinput_udev_create_context(
    const void *interface, void *user_data, struct udev *udev) {
    (void)interface; (void)user_data; (void)udev;
    return calloc(1, sizeof(struct libinput));
}

int libinput_udev_assign_seat(struct libinput *li, const char *seat) {
    (void)li; (void)seat;
    return 0;
}

/* Event loop integration */
int libinput_get_fd(struct libinput *li) {
    (void)li;
    return -1; /* No real fd -- input comes from kernel */
}

int libinput_dispatch(struct libinput *li) {
    (void)li;
    return 0;
}

struct libinput_event *libinput_get_event(struct libinput *li) {
    (void)li;
    return NULL; /* No events from this stub */
}

/* Event inspection */
enum libinput_event_type libinput_event_get_type(struct libinput_event *event) {
    if (!event) return LIBINPUT_EVENT_NONE;
    return (enum libinput_event_type)event->type;
}

struct libinput_device *libinput_event_get_device(struct libinput_event *event) {
    (void)event;
    return NULL;
}

/* Cleanup */
void libinput_event_destroy(struct libinput_event *event) {
    (void)event;
}

struct libinput *libinput_unref(struct libinput *li) {
    free(li);
    return NULL;
}

struct libinput *libinput_ref(struct libinput *li) {
    return li;
}

/* Suspend/resume */
void libinput_suspend(struct libinput *li) { (void)li; }
int libinput_resume(struct libinput *li) { (void)li; return 0; }

/* Device properties */
const char *libinput_device_get_name(struct libinput_device *device) {
    (void)device;
    return "VeridianOS Virtual Input";
}

const char *libinput_device_get_sysname(struct libinput_device *device) {
    (void)device;
    return "veridian0";
}

unsigned int libinput_device_get_id_vendor(struct libinput_device *device) {
    (void)device;
    return 0;
}

unsigned int libinput_device_get_id_product(struct libinput_device *device) {
    (void)device;
    return 0;
}

int libinput_device_has_capability(struct libinput_device *device, int capability) {
    (void)device; (void)capability;
    return 0;
}
