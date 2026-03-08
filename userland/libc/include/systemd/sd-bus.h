/*
 * VeridianOS libc -- <systemd/sd-bus.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Simplified sd-bus API for D-Bus client operations.
 * Used by KDE components that interface with logind, UPower,
 * NetworkManager, and other system services via sd-bus.
 */

#ifndef _SYSTEMD_SD_BUS_H
#define _SYSTEMD_SD_BUS_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>
#include <sys/types.h>

/* ---- Opaque types ---- */

typedef struct sd_bus sd_bus;
typedef struct sd_bus_message sd_bus_message;
typedef struct sd_bus_slot sd_bus_slot;
typedef struct sd_bus_creds sd_bus_creds;
typedef struct sd_bus_track sd_bus_track;

/* ---- Error ---- */

typedef struct {
    const char *name;
    const char *message;
    int _need_free;
} sd_bus_error;

#define SD_BUS_ERROR_NULL ((sd_bus_error){NULL, NULL, 0})

/* Common error names */
#define SD_BUS_ERROR_FAILED               "org.freedesktop.DBus.Error.Failed"
#define SD_BUS_ERROR_NO_MEMORY            "org.freedesktop.DBus.Error.NoMemory"
#define SD_BUS_ERROR_SERVICE_UNKNOWN      "org.freedesktop.DBus.Error.ServiceUnknown"
#define SD_BUS_ERROR_NAME_HAS_NO_OWNER    "org.freedesktop.DBus.Error.NameHasNoOwner"
#define SD_BUS_ERROR_NO_REPLY             "org.freedesktop.DBus.Error.NoReply"
#define SD_BUS_ERROR_ACCESS_DENIED        "org.freedesktop.DBus.Error.AccessDenied"
#define SD_BUS_ERROR_NOT_SUPPORTED        "org.freedesktop.DBus.Error.NotSupported"
#define SD_BUS_ERROR_UNKNOWN_METHOD       "org.freedesktop.DBus.Error.UnknownMethod"
#define SD_BUS_ERROR_UNKNOWN_OBJECT       "org.freedesktop.DBus.Error.UnknownObject"
#define SD_BUS_ERROR_UNKNOWN_INTERFACE    "org.freedesktop.DBus.Error.UnknownInterface"
#define SD_BUS_ERROR_UNKNOWN_PROPERTY     "org.freedesktop.DBus.Error.UnknownProperty"
#define SD_BUS_ERROR_PROPERTY_READ_ONLY   "org.freedesktop.DBus.Error.PropertyReadOnly"
#define SD_BUS_ERROR_INCONSISTENT_MESSAGE "org.freedesktop.DBus.Error.InconsistentMessage"
#define SD_BUS_ERROR_TIMEOUT              "org.freedesktop.DBus.Error.Timeout"

/* ---- Bus lifecycle ---- */

int sd_bus_open_system(sd_bus **bus);
int sd_bus_open_user(sd_bus **bus);
int sd_bus_open_system_remote(sd_bus **bus, const char *host);
int sd_bus_open_system_machine(sd_bus **bus, const char *machine);
int sd_bus_default_system(sd_bus **bus);
int sd_bus_default_user(sd_bus **bus);
int sd_bus_default(sd_bus **bus);
int sd_bus_new(sd_bus **bus);
sd_bus *sd_bus_ref(sd_bus *bus);
sd_bus *sd_bus_unref(sd_bus *bus);
sd_bus *sd_bus_flush_close_unref(sd_bus *bus);
int sd_bus_start(sd_bus *bus);
void sd_bus_close(sd_bus *bus);
int sd_bus_flush(sd_bus *bus);
int sd_bus_is_open(sd_bus *bus);
int sd_bus_get_bus_id(sd_bus *bus, void *id);
int sd_bus_get_unique_name(sd_bus *bus, const char **unique);

/* ---- Method calls ---- */

int sd_bus_call_method(sd_bus *bus,
                       const char *destination,
                       const char *path,
                       const char *interface,
                       const char *member,
                       sd_bus_error *error,
                       sd_bus_message **reply,
                       const char *types, ...);

int sd_bus_call_method_async(sd_bus *bus,
                             sd_bus_slot **slot,
                             const char *destination,
                             const char *path,
                             const char *interface,
                             const char *member,
                             void *callback,
                             void *userdata,
                             const char *types, ...);

/* ---- Property access ---- */

int sd_bus_get_property(sd_bus *bus,
                        const char *destination,
                        const char *path,
                        const char *interface,
                        const char *member,
                        sd_bus_error *error,
                        sd_bus_message **reply,
                        const char *type);

int sd_bus_get_property_string(sd_bus *bus,
                               const char *destination,
                               const char *path,
                               const char *interface,
                               const char *member,
                               sd_bus_error *error,
                               char **ret);

int sd_bus_get_property_trivial(sd_bus *bus,
                                const char *destination,
                                const char *path,
                                const char *interface,
                                const char *member,
                                sd_bus_error *error,
                                char type,
                                void *ret);

int sd_bus_set_property(sd_bus *bus,
                        const char *destination,
                        const char *path,
                        const char *interface,
                        const char *member,
                        sd_bus_error *error,
                        const char *type, ...);

/* ---- Message handling ---- */

int sd_bus_message_new_method_call(sd_bus *bus, sd_bus_message **m,
                                   const char *destination,
                                   const char *path,
                                   const char *interface,
                                   const char *member);
int sd_bus_message_new_signal(sd_bus *bus, sd_bus_message **m,
                               const char *path,
                               const char *interface,
                               const char *member);
int sd_bus_message_new_method_return(sd_bus_message *call,
                                     sd_bus_message **m);
int sd_bus_message_new_method_error(sd_bus_message *call,
                                    sd_bus_message **m,
                                    const sd_bus_error *e);

sd_bus_message *sd_bus_message_ref(sd_bus_message *m);
sd_bus_message *sd_bus_message_unref(sd_bus_message *m);

int sd_bus_message_read(sd_bus_message *m, const char *types, ...);
int sd_bus_message_read_basic(sd_bus_message *m, char type, void *p);
int sd_bus_message_append(sd_bus_message *m, const char *types, ...);
int sd_bus_message_append_basic(sd_bus_message *m, char type,
                                const void *p);
int sd_bus_message_open_container(sd_bus_message *m, char type,
                                  const char *contents);
int sd_bus_message_close_container(sd_bus_message *m);
int sd_bus_message_enter_container(sd_bus_message *m, char type,
                                   const char *contents);
int sd_bus_message_exit_container(sd_bus_message *m);
int sd_bus_message_peek_type(sd_bus_message *m, char *type,
                             const char **contents);
int sd_bus_message_at_end(sd_bus_message *m, int complete);
int sd_bus_message_skip(sd_bus_message *m, const char *types);
int sd_bus_message_get_errno(sd_bus_message *m);
int sd_bus_message_is_signal(sd_bus_message *m, const char *interface,
                             const char *member);
int sd_bus_message_is_method_call(sd_bus_message *m, const char *interface,
                                  const char *member);
int sd_bus_message_is_method_error(sd_bus_message *m, const char *name);
const char *sd_bus_message_get_path(sd_bus_message *m);
const char *sd_bus_message_get_interface(sd_bus_message *m);
const char *sd_bus_message_get_member(sd_bus_message *m);
const char *sd_bus_message_get_destination(sd_bus_message *m);
const char *sd_bus_message_get_sender(sd_bus_message *m);
const sd_bus_error *sd_bus_message_get_error(sd_bus_message *m);

int sd_bus_send(sd_bus *bus, sd_bus_message *m, uint64_t *cookie);
int sd_bus_call(sd_bus *bus, sd_bus_message *m, uint64_t usec,
                sd_bus_error *error, sd_bus_message **reply);

/* ---- Signal matching ---- */

int sd_bus_match_signal(sd_bus *bus, sd_bus_slot **slot,
                        const char *sender,
                        const char *path,
                        const char *interface,
                        const char *member,
                        void *callback,
                        void *userdata);
int sd_bus_match_signal_async(sd_bus *bus, sd_bus_slot **slot,
                              const char *sender,
                              const char *path,
                              const char *interface,
                              const char *member,
                              void *callback,
                              void *install_callback,
                              void *userdata);

/* ---- Name ownership ---- */

int sd_bus_request_name(sd_bus *bus, const char *name, uint64_t flags);
int sd_bus_release_name(sd_bus *bus, const char *name);

/* ---- Event loop integration ---- */

int sd_bus_get_fd(sd_bus *bus);
int sd_bus_get_events(sd_bus *bus);
int sd_bus_get_timeout(sd_bus *bus, uint64_t *timeout_usec);
int sd_bus_process(sd_bus *bus, sd_bus_message **r);
int sd_bus_wait(sd_bus *bus, uint64_t timeout_usec);

/* ---- Slot ---- */

sd_bus_slot *sd_bus_slot_ref(sd_bus_slot *slot);
sd_bus_slot *sd_bus_slot_unref(sd_bus_slot *slot);

/* ---- Error helpers ---- */

void sd_bus_error_free(sd_bus_error *e);
int sd_bus_error_set(sd_bus_error *e, const char *name, const char *message);
int sd_bus_error_set_const(sd_bus_error *e, const char *name,
                           const char *message);
int sd_bus_error_is_set(const sd_bus_error *e);
int sd_bus_error_has_name(const sd_bus_error *e, const char *name);

#ifdef __cplusplus
}
#endif

#endif /* _SYSTEMD_SD_BUS_H */
