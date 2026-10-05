/* libudev_stubs.c - Functional libudev shim for VeridianOS
 *
 * Provides a minimal but functional libudev implementation that returns
 * valid (non-NULL) objects.  KWin and libinput call udev_new() during
 * initialization; returning NULL causes an immediate segfault.
 *
 * This stub:
 *   - udev_new() returns a valid context (heap-allocated)
 *   - udev_enumerate_*() returns empty device lists (no crash)
 *   - udev_monitor_*() returns a valid monitor with fd=-1 (no events)
 *   - udev_device_*() returns NULL / empty strings gracefully
 */

#include <stdlib.h>
#include <string.h>
#include <sys/types.h>

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

struct udev {
    int refcount;
};

struct udev_device {
    int refcount;
    struct udev *ctx;
    char syspath[256];
    char devnode[256];
    char subsystem[64];
    char sysname[64];
    char devtype[64];
    char action[32];
    struct udev_device *parent;
};

struct udev_list_entry {
    char name[256];
    char value[256];
    struct udev_list_entry *next;
};

struct udev_enumerate {
    int refcount;
    struct udev *ctx;
    struct udev_list_entry *head;
};

struct udev_monitor {
    int refcount;
    struct udev *ctx;
    int fd;
};

/* ========================================================================= */
/* Context lifecycle                                                         */
/* ========================================================================= */

struct udev *udev_new(void) {
    struct udev *u = (struct udev *)calloc(1, sizeof(struct udev));
    if (!u)
        return NULL;
    u->refcount = 1;
    return u;
}

struct udev *udev_ref(struct udev *udev) {
    if (udev)
        udev->refcount++;
    return udev;
}

struct udev *udev_unref(struct udev *udev) {
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
/* Device                                                                    */
/* ========================================================================= */

struct udev_device *udev_device_ref(struct udev_device *dev) {
    if (dev)
        dev->refcount++;
    return dev;
}

struct udev_device *udev_device_unref(struct udev_device *dev) {
    if (!dev)
        return NULL;
    dev->refcount--;
    if (dev->refcount <= 0) {
        free(dev);
        return NULL;
    }
    return dev;
}

struct udev *udev_device_get_udev(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->ctx;
}

const char *udev_device_get_devpath(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->syspath[0] ? dev->syspath : NULL;
}

const char *udev_device_get_subsystem(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->subsystem[0] ? dev->subsystem : NULL;
}

const char *udev_device_get_devtype(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->devtype[0] ? dev->devtype : NULL;
}

const char *udev_device_get_syspath(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->syspath[0] ? dev->syspath : NULL;
}

const char *udev_device_get_sysname(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->sysname[0] ? dev->sysname : NULL;
}

const char *udev_device_get_devnode(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->devnode[0] ? dev->devnode : NULL;
}

const char *udev_device_get_property_value(struct udev_device *dev,
                                            const char *key) {
    (void)key;
    if (!dev)
        return NULL;
    return NULL; /* no properties */
}

const char *udev_device_get_sysattr_value(struct udev_device *dev,
                                           const char *sysattr) {
    (void)sysattr;
    if (!dev)
        return NULL;
    return NULL; /* no sysattrs */
}

struct udev_device *udev_device_get_parent(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->parent;
}

struct udev_device *udev_device_get_parent_with_subsystem_devtype(
    struct udev_device *dev, const char *subsystem, const char *devtype) {
    (void)subsystem;
    (void)devtype;
    if (!dev)
        return NULL;
    return NULL; /* no parent chain */
}

struct udev_device *udev_device_new_from_syspath(struct udev *udev,
                                                   const char *syspath) {
    if (!udev || !syspath)
        return NULL;

    struct udev_device *dev = (struct udev_device *)calloc(1, sizeof(*dev));
    if (!dev)
        return NULL;

    dev->refcount = 1;
    dev->ctx = udev;
    strncpy(dev->syspath, syspath, sizeof(dev->syspath) - 1);

    /* Extract sysname from syspath (last component) */
    const char *last_slash = strrchr(syspath, '/');
    if (last_slash)
        strncpy(dev->sysname, last_slash + 1, sizeof(dev->sysname) - 1);

    return dev;
}

struct udev_device *udev_device_new_from_devnum(struct udev *udev,
                                                  char type, dev_t devnum) {
    (void)type;
    (void)devnum;
    if (!udev)
        return NULL;
    return NULL; /* device not found */
}

const char *udev_device_get_action(struct udev_device *dev) {
    if (!dev)
        return NULL;
    return dev->action[0] ? dev->action : NULL;
}

int udev_device_has_tag(struct udev_device *dev, const char *tag) {
    (void)tag;
    if (!dev)
        return 0;
    return 0; /* no tags */
}

dev_t udev_device_get_devnum(struct udev_device *dev) {
    (void)dev;
    return 0;
}

struct udev_list_entry *udev_device_get_properties_list_entry(
    struct udev_device *dev) {
    (void)dev;
    return NULL; /* empty property list */
}

int udev_device_get_is_initialized(struct udev_device *dev) {
    (void)dev;
    return 1; /* always initialized */
}

/* ========================================================================= */
/* Enumerate                                                                 */
/* ========================================================================= */

struct udev_enumerate *udev_enumerate_new(struct udev *udev) {
    if (!udev)
        return NULL;

    struct udev_enumerate *en = (struct udev_enumerate *)calloc(1, sizeof(*en));
    if (!en)
        return NULL;

    en->refcount = 1;
    en->ctx = udev;
    en->head = NULL;
    return en;
}

struct udev_enumerate *udev_enumerate_ref(struct udev_enumerate *en) {
    if (en)
        en->refcount++;
    return en;
}

struct udev_enumerate *udev_enumerate_unref(struct udev_enumerate *en) {
    if (!en)
        return NULL;
    en->refcount--;
    if (en->refcount <= 0) {
        /* Free list entries */
        struct udev_list_entry *entry = en->head;
        while (entry) {
            struct udev_list_entry *next = entry->next;
            free(entry);
            entry = next;
        }
        free(en);
        return NULL;
    }
    return en;
}

int udev_enumerate_add_match_subsystem(struct udev_enumerate *en,
                                        const char *subsystem) {
    (void)subsystem;
    if (!en)
        return -1;
    return 0; /* accepted (no-op) */
}

int udev_enumerate_add_match_sysname(struct udev_enumerate *en,
                                      const char *sysname) {
    (void)sysname;
    if (!en)
        return -1;
    return 0; /* accepted (no-op) */
}

int udev_enumerate_scan_devices(struct udev_enumerate *en) {
    if (!en)
        return -1;
    return 0; /* success, but no devices found */
}

struct udev_list_entry *udev_enumerate_get_list_entry(
    struct udev_enumerate *en) {
    if (!en)
        return NULL;
    return en->head; /* NULL = empty list */
}

/* ========================================================================= */
/* List entry                                                                */
/* ========================================================================= */

struct udev_list_entry *udev_list_entry_get_next(
    struct udev_list_entry *entry) {
    if (!entry)
        return NULL;
    return entry->next;
}

const char *udev_list_entry_get_name(struct udev_list_entry *entry) {
    if (!entry)
        return NULL;
    return entry->name;
}

const char *udev_list_entry_get_value(struct udev_list_entry *entry) {
    if (!entry)
        return NULL;
    return entry->value;
}

/* ========================================================================= */
/* Monitor                                                                   */
/* ========================================================================= */

struct udev_monitor *udev_monitor_new_from_netlink(struct udev *udev,
                                                     const char *name) {
    (void)name;
    if (!udev)
        return NULL;

    struct udev_monitor *mon = (struct udev_monitor *)calloc(1, sizeof(*mon));
    if (!mon)
        return NULL;

    mon->refcount = 1;
    mon->ctx = udev;
    mon->fd = -1; /* no real fd -- no events will be delivered */
    return mon;
}

int udev_monitor_enable_receiving(struct udev_monitor *mon) {
    if (!mon)
        return -1;
    return 0; /* success (no-op) */
}

int udev_monitor_get_fd(struct udev_monitor *mon) {
    if (!mon)
        return -1;
    return mon->fd; /* -1 */
}

struct udev_device *udev_monitor_receive_device(struct udev_monitor *mon) {
    (void)mon;
    return NULL; /* no devices */
}

struct udev_monitor *udev_monitor_unref(struct udev_monitor *mon) {
    if (!mon)
        return NULL;
    mon->refcount--;
    if (mon->refcount <= 0) {
        free(mon);
        return NULL;
    }
    return mon;
}

int udev_monitor_filter_add_match_subsystem_devtype(
    struct udev_monitor *mon, const char *subsystem, const char *devtype) {
    (void)subsystem;
    (void)devtype;
    if (!mon)
        return -1;
    return 0; /* accepted (no-op) */
}
