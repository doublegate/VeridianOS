/*
 * VeridianOS -- udev-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * udev device event daemon for VeridianOS.  Monitors kernel device
 * events (USB hotplug, block devices, network interfaces, input
 * devices) and provides a udev-compatible event interface for
 * userland consumers.
 *
 * Publishes events on D-Bus (org.freedesktop.UDev) for integration
 * with KDE Solid and other desktop components.
 */

#ifndef UDEV_VERIDIAN_H
#define UDEV_VERIDIAN_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Event types                                                               */
/* ========================================================================= */

/**
 * Device event action.
 */
typedef enum {
    UDEV_ACTION_ADD     = 0,    /**< Device added */
    UDEV_ACTION_REMOVE  = 1,    /**< Device removed */
    UDEV_ACTION_CHANGE  = 2,    /**< Device properties changed */
    UDEV_ACTION_BIND    = 3,    /**< Driver bound to device */
    UDEV_ACTION_UNBIND  = 4,    /**< Driver unbound from device */
} UdevEventAction;

/**
 * Maximum length for string fields in UdevEvent.
 */
#define UDEV_MAX_PATH  256
#define UDEV_MAX_NAME   64
#define UDEV_MAX_PROPS  16

/**
 * Property key-value pair.
 */
typedef struct {
    char key[UDEV_MAX_NAME];
    char value[UDEV_MAX_PATH];
} UdevProperty;

/**
 * Device event descriptor.
 *
 * Carries all information about a single device add/remove/change event.
 */
typedef struct UdevEvent {
    UdevEventAction  action;                        /**< Event action */
    char             subsystem[UDEV_MAX_NAME];      /**< Subsystem (usb, block, net, input) */
    char             devpath[UDEV_MAX_PATH];        /**< Sysfs device path */
    char             devnode[UDEV_MAX_PATH];        /**< Device node (/dev/...) */
    UdevProperty     properties[UDEV_MAX_PROPS];    /**< Device properties */
    int              num_properties;                /**< Number of valid properties */
    uint64_t         seqnum;                        /**< Sequence number */
} UdevEvent;

/* ========================================================================= */
/* Rule types                                                                */
/* ========================================================================= */

/**
 * Maximum number of match conditions per rule.
 */
#define UDEV_MAX_MATCHES  4

/**
 * Rule match condition.
 */
typedef struct {
    char key[UDEV_MAX_NAME];        /**< Match key (SUBSYSTEM, ATTR{x}, ACTION) */
    char value[UDEV_MAX_PATH];      /**< Expected value */
} UdevRuleMatch;

/**
 * Rule action types.
 */
typedef enum {
    UDEV_RULE_RUN      = 0,    /**< Run a program */
    UDEV_RULE_SYMLINK  = 1,    /**< Create a symlink */
    UDEV_RULE_ENV      = 2,    /**< Set environment variable */
    UDEV_RULE_LABEL    = 3,    /**< Set a label */
} UdevRuleActionType;

/**
 * udev rule: match conditions + action.
 */
typedef struct {
    UdevRuleMatch       matches[UDEV_MAX_MATCHES];  /**< Match conditions */
    int                 num_matches;                /**< Number of active matches */
    UdevRuleActionType  action_type;                /**< Action to take */
    char                action_value[UDEV_MAX_PATH]; /**< Action argument */
} UdevRule;

/* ========================================================================= */
/* Opaque monitor handle                                                     */
/* ========================================================================= */

typedef struct UdevMonitor UdevMonitor;

/* ========================================================================= */
/* Daemon lifecycle                                                          */
/* ========================================================================= */

/**
 * Start the udev daemon.
 *
 * Initializes kernel event monitoring, loads rules from
 * /etc/udev/rules.d/, and registers on D-Bus.
 *
 * @return 0 on success, negative errno on failure.
 */
int udev_daemon_start(void);

/**
 * Stop the udev daemon and clean up resources.
 */
void udev_daemon_stop(void);

/* ========================================================================= */
/* Monitor interface                                                         */
/* ========================================================================= */

/**
 * Create a new event monitor.
 *
 * @return Monitor handle, or NULL on failure.
 */
UdevMonitor *udev_monitor_new(void);

/**
 * Add a subsystem filter to a monitor.
 *
 * @param monitor    Monitor handle.
 * @param subsystem  Subsystem name (e.g., "usb", "block", "net").
 * @return 0 on success, negative errno on failure.
 */
int udev_monitor_add_filter(UdevMonitor *monitor, const char *subsystem);

/**
 * Receive the next event from a monitor (blocking).
 *
 * Caller must free the returned event with udev_event_free().
 *
 * @param monitor  Monitor handle.
 * @return Event pointer, or NULL on error.
 */
UdevEvent *udev_monitor_receive_event(UdevMonitor *monitor);

/**
 * Destroy a monitor and free its resources.
 */
void udev_monitor_destroy(UdevMonitor *monitor);

/* ========================================================================= */
/* Device enumeration                                                        */
/* ========================================================================= */

/**
 * Enumerate devices in a given subsystem.
 *
 * @param subsystem  Subsystem to scan (e.g., "usb", "block").
 * @param results    Output array of device paths.
 * @param max        Maximum number of results.
 * @return Number of devices found, or negative errno on error.
 */
int udev_enumerate_devices(const char *subsystem, char results[][UDEV_MAX_PATH], int max);

/**
 * Get a property value for a device.
 *
 * @param devpath  Sysfs device path.
 * @param key      Property name.
 * @return Property value string (static buffer), or NULL if not found.
 */
const char *udev_device_get_property(const char *devpath, const char *key);

/* ========================================================================= */
/* Cleanup                                                                   */
/* ========================================================================= */

/**
 * Free a UdevEvent returned by udev_monitor_receive_event().
 */
void udev_event_free(UdevEvent *event);

#ifdef __cplusplus
}
#endif

#endif /* UDEV_VERIDIAN_H */
