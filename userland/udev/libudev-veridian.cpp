/*
 * VeridianOS -- libudev-veridian.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libudev API shim implementation for VeridianOS.
 *
 * Provides the standard libudev interface by delegating to the
 * udev-veridian daemon.  Used by PipeWire, libinput, Mesa, and
 * other system components that depend on libudev.
 */

#include "libudev-veridian.h"
#include "udev-veridian.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

/* Concrete definition for the opaque UdevMonitor handle from udev-veridian.h.
 * The daemon side creates these; the libudev shim accesses pipeFd for poll. */
struct UdevMonitor {
    int pipeFd[2];          /* pipe for event notification */
    char filter[UDEV_MAX_NAME]; /* subsystem filter */
};

struct udev {
    int refcount;
};

struct udev_monitor {
    struct udev *ctx;
    UdevMonitor *internal;      /* daemon-side monitor handle */
    int fd;                     /* pipe read fd for poll */
    int refcount;
};

#define UDEV_DEV_MAX_PROPS 16

struct udev_device {
    struct udev *ctx;
    char devnode[UDEV_MAX_PATH];
    char subsystem[UDEV_MAX_NAME];
    char action[UDEV_MAX_NAME];
    char syspath[UDEV_MAX_PATH];
    struct {
        char key[UDEV_MAX_NAME];
        char value[UDEV_MAX_PATH];
    } properties[UDEV_DEV_MAX_PROPS];
    int num_properties;
    struct udev_device *parent;
    int refcount;
};

struct udev_list_entry {
    char name[UDEV_MAX_PATH];
    struct udev_list_entry *next;
};

struct udev_enumerate {
    struct udev *ctx;
    char subsystem[UDEV_MAX_NAME];
    struct udev_list_entry *head;
    int refcount;
};

/* ========================================================================= */
/* Context                                                                   */
/* ========================================================================= */

struct udev *udev_new(void)
{
    struct udev *u = (struct udev *)calloc(1, sizeof(*u));
    if (!u)
        return NULL;
    u->refcount = 1;
    return u;
}

struct udev *udev_unref(struct udev *udev)
{
    if (!udev)
        return NULL;
    udev->refcount--;
    if (udev->refcount <= 0) {
        free(udev);
        return NULL;
    }
    return udev;
}

/* ========================================================================= */
/* Monitor                                                                   */
/* ========================================================================= */

struct udev_monitor *udev_monitor_new_from_netlink(struct udev *udev,
                                                    const char *name)
{
    if (!udev)
        return NULL;

    (void)name; /* both "udev" and "kernel" sources are treated the same */

    struct udev_monitor *mon = (struct udev_monitor *)calloc(1, sizeof(*mon));
    if (!mon)
        return NULL;

    mon->ctx = udev;
    mon->refcount = 1;
    mon->fd = -1;

    /* Create daemon-side monitor */
    mon->internal = udev_monitor_new();
    if (!mon->internal) {
        free(mon);
        return NULL;
    }

    return mon;
}

int udev_monitor_enable_receiving(struct udev_monitor *udev_monitor)
{
    if (!udev_monitor || !udev_monitor->internal)
        return -EINVAL;

    /* The daemon-side monitor is already active once created.
     * Store the pipe fd for poll(). */
    udev_monitor->fd = udev_monitor->internal->pipeFd[0];
    return 0;
}

int udev_monitor_get_fd(struct udev_monitor *udev_monitor)
{
    if (!udev_monitor)
        return -1;
    return udev_monitor->fd;
}

struct udev_device *udev_monitor_receive_device(
    struct udev_monitor *udev_monitor)
{
    if (!udev_monitor || !udev_monitor->internal)
        return NULL;

    UdevEvent *ev = udev_monitor_receive_event(udev_monitor->internal);
    if (!ev)
        return NULL;

    struct udev_device *dev = (struct udev_device *)calloc(1, sizeof(*dev));
    if (!dev) {
        udev_event_free(ev);
        return NULL;
    }

    dev->ctx = udev_monitor->ctx;
    dev->refcount = 1;
    dev->parent = NULL;

    strncpy(dev->devnode, ev->devnode, UDEV_MAX_PATH - 1);
    strncpy(dev->subsystem, ev->subsystem, UDEV_MAX_NAME - 1);
    strncpy(dev->syspath, ev->devpath, UDEV_MAX_PATH - 1);

    switch (ev->action) {
    case UDEV_ACTION_ADD:    strncpy(dev->action, "add", UDEV_MAX_NAME - 1); break;
    case UDEV_ACTION_REMOVE: strncpy(dev->action, "remove", UDEV_MAX_NAME - 1); break;
    case UDEV_ACTION_CHANGE: strncpy(dev->action, "change", UDEV_MAX_NAME - 1); break;
    case UDEV_ACTION_BIND:   strncpy(dev->action, "bind", UDEV_MAX_NAME - 1); break;
    case UDEV_ACTION_UNBIND: strncpy(dev->action, "unbind", UDEV_MAX_NAME - 1); break;
    }

    /* Copy properties */
    dev->num_properties = 0;
    for (int i = 0; i < ev->num_properties && i < UDEV_DEV_MAX_PROPS; i++) {
        strncpy(dev->properties[i].key, ev->properties[i].key,
                UDEV_MAX_NAME - 1);
        strncpy(dev->properties[i].value, ev->properties[i].value,
                UDEV_MAX_PATH - 1);
        dev->num_properties++;
    }

    udev_event_free(ev);
    return dev;
}

int udev_monitor_filter_add_match_subsystem_devtype(
    struct udev_monitor *udev_monitor,
    const char *subsystem,
    const char *devtype)
{
    if (!udev_monitor || !udev_monitor->internal || !subsystem)
        return -EINVAL;

    (void)devtype; /* devtype filtering not yet supported */

    return udev_monitor_add_filter(udev_monitor->internal, subsystem);
}

struct udev_monitor *udev_monitor_unref(struct udev_monitor *udev_monitor)
{
    if (!udev_monitor)
        return NULL;

    udev_monitor->refcount--;
    if (udev_monitor->refcount <= 0) {
        if (udev_monitor->internal)
            udev_monitor_destroy(udev_monitor->internal);
        free(udev_monitor);
        return NULL;
    }
    return udev_monitor;
}

/* ========================================================================= */
/* Device                                                                    */
/* ========================================================================= */

const char *udev_device_get_devnode(struct udev_device *udev_device)
{
    if (!udev_device)
        return NULL;
    return udev_device->devnode[0] ? udev_device->devnode : NULL;
}

const char *udev_device_get_subsystem(struct udev_device *udev_device)
{
    if (!udev_device)
        return NULL;
    return udev_device->subsystem[0] ? udev_device->subsystem : NULL;
}

const char *udev_device_get_action(struct udev_device *udev_device)
{
    if (!udev_device)
        return NULL;
    return udev_device->action[0] ? udev_device->action : NULL;
}

const char *udev_device_get_property_value(struct udev_device *udev_device,
                                            const char *key)
{
    if (!udev_device || !key)
        return NULL;

    for (int i = 0; i < udev_device->num_properties; i++) {
        if (strcmp(udev_device->properties[i].key, key) == 0)
            return udev_device->properties[i].value;
    }

    /* Fallback: query daemon */
    return udev_device_get_property(udev_device->syspath, key);
}

const char *udev_device_get_sysattr_value(struct udev_device *udev_device,
                                           const char *sysattr)
{
    if (!udev_device || !sysattr)
        return NULL;

    /* Read sysfs attribute file: <syspath>/<sysattr> */
    static char attr_path[UDEV_MAX_PATH * 2];
    static char attr_value[UDEV_MAX_PATH];

    snprintf(attr_path, sizeof(attr_path), "%s/%s",
             udev_device->syspath, sysattr);

    int fd = open(attr_path, O_RDONLY);
    if (fd < 0)
        return NULL;

    ssize_t n = read(fd, attr_value, sizeof(attr_value) - 1);
    close(fd);

    if (n <= 0)
        return NULL;

    attr_value[n] = '\0';
    /* Strip trailing newline */
    if (n > 0 && attr_value[n - 1] == '\n')
        attr_value[n - 1] = '\0';

    return attr_value;
}

struct udev_device *udev_device_get_parent(struct udev_device *udev_device)
{
    if (!udev_device)
        return NULL;
    return udev_device->parent; /* may be NULL */
}

struct udev_device *udev_device_unref(struct udev_device *udev_device)
{
    if (!udev_device)
        return NULL;

    udev_device->refcount--;
    if (udev_device->refcount <= 0) {
        /* Note: parent is not ref'd, so don't free it */
        free(udev_device);
        return NULL;
    }
    return udev_device;
}

/* ========================================================================= */
/* Enumerate                                                                 */
/* ========================================================================= */

struct udev_enumerate *udev_enumerate_new(struct udev *udev)
{
    if (!udev)
        return NULL;

    struct udev_enumerate *en = (struct udev_enumerate *)calloc(1, sizeof(*en));
    if (!en)
        return NULL;

    en->ctx = udev;
    en->refcount = 1;
    en->head = NULL;
    en->subsystem[0] = '\0';

    return en;
}

int udev_enumerate_add_match_subsystem(struct udev_enumerate *udev_enumerate,
                                        const char *subsystem)
{
    if (!udev_enumerate || !subsystem)
        return -EINVAL;

    strncpy(udev_enumerate->subsystem, subsystem, UDEV_MAX_NAME - 1);
    return 0;
}

int udev_enumerate_add_match_property(struct udev_enumerate *udev_enumerate,
                                       const char *property,
                                       const char *value)
{
    if (!udev_enumerate)
        return -EINVAL;

    /* VeridianOS stub: property matching not implemented, accept all */
    (void)property;
    (void)value;
    return 0;
}

int udev_enumerate_add_match_sysattr(struct udev_enumerate *udev_enumerate,
                                      const char *sysattr,
                                      const char *value)
{
    if (!udev_enumerate)
        return -EINVAL;

    /* VeridianOS stub: sysattr matching not implemented, accept all */
    (void)sysattr;
    (void)value;
    return 0;
}

int udev_enumerate_add_match_is_initialized(struct udev_enumerate *udev_enumerate)
{
    if (!udev_enumerate)
        return -EINVAL;

    /* VeridianOS stub: all devices considered initialized */
    return 0;
}

int udev_enumerate_scan_devices(struct udev_enumerate *udev_enumerate)
{
    if (!udev_enumerate)
        return -EINVAL;

    /* Free previous results */
    struct udev_list_entry *entry = udev_enumerate->head;
    while (entry) {
        struct udev_list_entry *next = entry->next;
        free(entry);
        entry = next;
    }
    udev_enumerate->head = NULL;

    /* Query daemon for devices matching subsystem */
    char results[64][UDEV_MAX_PATH];
    int count = udev_enumerate_devices(udev_enumerate->subsystem,
                                        results, 64);
    if (count < 0)
        return count;

    /* Build linked list (reverse order for simplicity) */
    struct udev_list_entry *prev = NULL;
    for (int i = count - 1; i >= 0; i--) {
        struct udev_list_entry *e =
            (struct udev_list_entry *)calloc(1, sizeof(*e));
        if (!e)
            continue;
        strncpy(e->name, results[i], UDEV_MAX_PATH - 1);
        e->next = prev;
        prev = e;
    }
    udev_enumerate->head = prev;

    return 0;
}

struct udev_list_entry *udev_enumerate_get_list_entry(
    struct udev_enumerate *udev_enumerate)
{
    if (!udev_enumerate)
        return NULL;
    return udev_enumerate->head;
}

struct udev_enumerate *udev_enumerate_unref(
    struct udev_enumerate *udev_enumerate)
{
    if (!udev_enumerate)
        return NULL;

    udev_enumerate->refcount--;
    if (udev_enumerate->refcount <= 0) {
        struct udev_list_entry *entry = udev_enumerate->head;
        while (entry) {
            struct udev_list_entry *next = entry->next;
            free(entry);
            entry = next;
        }
        free(udev_enumerate);
        return NULL;
    }
    return udev_enumerate;
}

/* ========================================================================= */
/* List entry                                                                */
/* ========================================================================= */

struct udev_list_entry *udev_list_entry_get_next(
    struct udev_list_entry *list_entry)
{
    if (!list_entry)
        return NULL;
    return list_entry->next;
}

const char *udev_list_entry_get_name(struct udev_list_entry *list_entry)
{
    if (!list_entry)
        return NULL;
    return list_entry->name;
}
