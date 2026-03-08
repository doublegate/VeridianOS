/*
 * VeridianOS -- libudev-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libudev API shim for VeridianOS.  Provides the standard libudev
 * interface used by system daemons (PipeWire, libinput, Mesa) and
 * desktop components.  Delegates to the udev-veridian daemon.
 */

#ifndef LIBUDEV_VERIDIAN_H
#define LIBUDEV_VERIDIAN_H

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

struct udev;
struct udev_monitor;
struct udev_device;
struct udev_enumerate;
struct udev_list_entry;

/* ========================================================================= */
/* Context lifecycle                                                         */
/* ========================================================================= */

/**
 * Create a new udev context.
 * Connects to the udev-veridian daemon.
 */
struct udev *udev_new(void);

/**
 * Drop a reference to a udev context.
 * Frees when refcount reaches zero.
 */
struct udev *udev_unref(struct udev *udev);

/* ========================================================================= */
/* Monitor                                                                   */
/* ========================================================================= */

/**
 * Create a new monitor that receives device events.
 *
 * @param udev  Context handle.
 * @param name  Source name, typically "udev" or "kernel".
 * @return Monitor handle, or NULL on failure.
 */
struct udev_monitor *udev_monitor_new_from_netlink(struct udev *udev,
                                                    const char *name);

/**
 * Enable event receiving on the monitor.
 *
 * @return 0 on success, negative errno on failure.
 */
int udev_monitor_enable_receiving(struct udev_monitor *udev_monitor);

/**
 * Get the file descriptor for polling.
 *
 * @return fd suitable for poll()/select(), or -1 on error.
 */
int udev_monitor_get_fd(struct udev_monitor *udev_monitor);

/**
 * Receive the next device event (blocking).
 *
 * @return Device handle (caller must udev_device_unref), or NULL.
 */
struct udev_device *udev_monitor_receive_device(
    struct udev_monitor *udev_monitor);

/**
 * Add a subsystem/devtype filter.
 *
 * @param subsystem  Subsystem name (e.g., "input", "usb").
 * @param devtype    Device type (may be NULL).
 * @return 0 on success, negative errno on failure.
 */
int udev_monitor_filter_add_match_subsystem_devtype(
    struct udev_monitor *udev_monitor,
    const char *subsystem,
    const char *devtype);

/**
 * Destroy a monitor.
 */
struct udev_monitor *udev_monitor_unref(struct udev_monitor *udev_monitor);

/* ========================================================================= */
/* Device                                                                    */
/* ========================================================================= */

/**
 * Get the device node path (e.g., "/dev/input/event0").
 */
const char *udev_device_get_devnode(struct udev_device *udev_device);

/**
 * Get the device subsystem (e.g., "input", "usb").
 */
const char *udev_device_get_subsystem(struct udev_device *udev_device);

/**
 * Get the action string ("add", "remove", "change").
 */
const char *udev_device_get_action(struct udev_device *udev_device);

/**
 * Get a device property value by key.
 */
const char *udev_device_get_property_value(struct udev_device *udev_device,
                                            const char *key);

/**
 * Get a sysfs attribute value.
 */
const char *udev_device_get_sysattr_value(struct udev_device *udev_device,
                                           const char *sysattr);

/**
 * Get the parent device.
 *
 * The returned device is NOT ref'd; do not unref it.
 */
struct udev_device *udev_device_get_parent(struct udev_device *udev_device);

/**
 * Drop a reference to a device.
 */
struct udev_device *udev_device_unref(struct udev_device *udev_device);

/* ========================================================================= */
/* Enumerate                                                                 */
/* ========================================================================= */

/**
 * Create a new enumerator.
 */
struct udev_enumerate *udev_enumerate_new(struct udev *udev);

/**
 * Add a subsystem match to the enumerator.
 */
int udev_enumerate_add_match_subsystem(struct udev_enumerate *udev_enumerate,
                                        const char *subsystem);

/**
 * Scan for matching devices.
 *
 * @return 0 on success, negative errno on failure.
 */
int udev_enumerate_scan_devices(struct udev_enumerate *udev_enumerate);

/**
 * Get the first list entry of enumerated devices.
 */
struct udev_list_entry *udev_enumerate_get_list_entry(
    struct udev_enumerate *udev_enumerate);

/**
 * Destroy an enumerator.
 */
struct udev_enumerate *udev_enumerate_unref(
    struct udev_enumerate *udev_enumerate);

/* ========================================================================= */
/* List entry                                                                */
/* ========================================================================= */

/**
 * Get the next list entry.
 */
struct udev_list_entry *udev_list_entry_get_next(
    struct udev_list_entry *list_entry);

/**
 * Get the name (sysfs path) of a list entry.
 */
const char *udev_list_entry_get_name(struct udev_list_entry *list_entry);

#ifdef __cplusplus
}
#endif

#endif /* LIBUDEV_VERIDIAN_H */
