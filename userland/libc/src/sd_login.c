/*
 * VeridianOS libc -- sd_login.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * systemd-logind session/seat/user query API and minimal sd-bus
 * implementation for VeridianOS.  Provides the API surface that KWin,
 * SDDM, and KDE Frameworks use for session management, seat
 * enumeration, and D-Bus method calls via sd-bus.
 *
 * Defaults: session "1", seat "seat0", type "wayland", class "user",
 * state "active", VT 1, active=true, graphical=true.
 */

#include <systemd/sd-login.h>
#include <systemd/sd-bus.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static char *strdup_safe(const char *s)
{
    if (!s)
        return NULL;
    size_t len = strlen(s) + 1;
    char *copy = (char *)malloc(len);
    if (copy)
        memcpy(copy, s, len);
    return copy;
}

/* ========================================================================= */
/* sd-login: session queries                                                 */
/* ========================================================================= */

int sd_pid_get_session(pid_t pid, char **session)
{
    (void)pid;
    if (!session)
        return -1;
    *session = strdup_safe("1");
    return *session ? 0 : -1;
}

int sd_pid_get_unit(pid_t pid, char **unit)
{
    (void)pid;
    if (!unit)
        return -1;
    *unit = strdup_safe("session-1.scope");
    return *unit ? 0 : -1;
}

int sd_pid_get_user_unit(pid_t pid, char **unit)
{
    (void)pid;
    if (!unit)
        return -1;
    *unit = strdup_safe("app.slice");
    return *unit ? 0 : -1;
}

int sd_pid_get_owner_uid(pid_t pid, uid_t *uid)
{
    (void)pid;
    if (uid)
        *uid = 1000;
    return 0;
}

int sd_pid_get_machine_name(pid_t pid, char **machine)
{
    (void)pid;
    if (!machine)
        return -1;
    *machine = NULL;
    return -1; /* not in a machine */
}

int sd_pid_get_slice(pid_t pid, char **slice)
{
    (void)pid;
    if (!slice)
        return -1;
    *slice = strdup_safe("user-1000.slice");
    return *slice ? 0 : -1;
}

int sd_pid_get_user_slice(pid_t pid, char **slice)
{
    (void)pid;
    if (!slice)
        return -1;
    *slice = strdup_safe("app.slice");
    return *slice ? 0 : -1;
}

int sd_pid_get_cgroup(pid_t pid, char **cgroup)
{
    (void)pid;
    if (!cgroup)
        return -1;
    *cgroup = strdup_safe("/user.slice/user-1000.slice/session-1.scope");
    return *cgroup ? 0 : -1;
}

int sd_session_get_seat(const char *session, char **seat)
{
    (void)session;
    if (!seat)
        return -1;
    *seat = strdup_safe("seat0");
    return *seat ? 0 : -1;
}

int sd_session_get_type(const char *session, char **type)
{
    (void)session;
    if (!type)
        return -1;
    *type = strdup_safe("wayland");
    return *type ? 0 : -1;
}

int sd_session_get_class(const char *session, char **class)
{
    (void)session;
    if (!class)
        return -1;
    *class = strdup_safe("user");
    return *class ? 0 : -1;
}

int sd_session_get_state(const char *session, char **state)
{
    (void)session;
    if (!state)
        return -1;
    *state = strdup_safe("active");
    return *state ? 0 : -1;
}

int sd_session_get_display(const char *session, char **display)
{
    (void)session;
    if (!display)
        return -1;
    *display = strdup_safe(":0");
    return *display ? 0 : -1;
}

int sd_session_get_tty(const char *session, char **tty)
{
    (void)session;
    if (!tty)
        return -1;
    *tty = strdup_safe("tty1");
    return *tty ? 0 : -1;
}

int sd_session_get_vt(const char *session, unsigned *vt)
{
    (void)session;
    if (vt)
        *vt = 1;
    return 0;
}

int sd_session_get_service(const char *session, char **service)
{
    (void)session;
    if (!service)
        return -1;
    *service = strdup_safe("login");
    return *service ? 0 : -1;
}

int sd_session_get_desktop(const char *session, char **desktop)
{
    (void)session;
    if (!desktop)
        return -1;
    *desktop = strdup_safe("KDE");
    return *desktop ? 0 : -1;
}

int sd_session_get_remote_host(const char *session, char **remote_host)
{
    (void)session;
    if (!remote_host)
        return -1;
    *remote_host = NULL;
    return -1; /* not remote */
}

int sd_session_get_remote_user(const char *session, char **remote_user)
{
    (void)session;
    if (!remote_user)
        return -1;
    *remote_user = NULL;
    return -1; /* not remote */
}

int sd_session_get_leader(const char *session, pid_t *leader)
{
    (void)session;
    if (leader)
        *leader = 1;
    return 0;
}

int sd_session_is_active(const char *session)
{
    (void)session;
    return 1; /* always active */
}

int sd_session_is_remote(const char *session)
{
    (void)session;
    return 0; /* not remote */
}

/* ========================================================================= */
/* sd-login: seat queries                                                    */
/* ========================================================================= */

int sd_seat_get_active(const char *seat, char **session, uid_t *uid)
{
    (void)seat;
    if (session)
        *session = strdup_safe("1");
    if (uid)
        *uid = 1000;
    return 0;
}

int sd_seat_get_sessions(const char *seat, char ***sessions,
                         uid_t **uid, unsigned *n_uids)
{
    (void)seat;
    if (sessions) {
        *sessions = (char **)calloc(2, sizeof(char *));
        if (*sessions)
            (*sessions)[0] = strdup_safe("1");
    }
    if (uid) {
        *uid = (uid_t *)malloc(sizeof(uid_t));
        if (*uid)
            (*uid)[0] = 1000;
    }
    if (n_uids)
        *n_uids = 1;
    return 1; /* number of sessions */
}

int sd_seat_can_multi_session(const char *seat)
{
    (void)seat;
    return 1;
}

int sd_seat_can_tty(const char *seat)
{
    (void)seat;
    return 1;
}

int sd_seat_can_graphical(const char *seat)
{
    (void)seat;
    return 1;
}

/* ========================================================================= */
/* sd-login: enumeration                                                     */
/* ========================================================================= */

int sd_get_sessions(char ***sessions)
{
    if (!sessions)
        return -1;
    *sessions = (char **)calloc(2, sizeof(char *));
    if (*sessions)
        (*sessions)[0] = strdup_safe("1");
    return 1;
}

int sd_get_seats(char ***seats)
{
    if (!seats)
        return -1;
    *seats = (char **)calloc(2, sizeof(char *));
    if (*seats)
        (*seats)[0] = strdup_safe("seat0");
    return 1;
}

int sd_get_uids(uid_t **uids)
{
    if (!uids)
        return -1;
    *uids = (uid_t *)malloc(sizeof(uid_t));
    if (*uids)
        (*uids)[0] = 1000;
    return 1;
}

int sd_get_machine_names(char ***machines)
{
    if (!machines)
        return -1;
    *machines = (char **)calloc(1, sizeof(char *));
    return 0;
}

/* ========================================================================= */
/* sd-login: user queries                                                    */
/* ========================================================================= */

int sd_uid_get_state(uid_t uid, char **state)
{
    (void)uid;
    if (!state)
        return -1;
    *state = strdup_safe("active");
    return *state ? 0 : -1;
}

int sd_uid_get_display(uid_t uid, char **session)
{
    (void)uid;
    if (!session)
        return -1;
    *session = strdup_safe("1");
    return *session ? 0 : -1;
}

int sd_uid_get_sessions(uid_t uid, int require_active, char ***sessions)
{
    (void)uid;
    (void)require_active;
    if (!sessions)
        return -1;
    *sessions = (char **)calloc(2, sizeof(char *));
    if (*sessions)
        (*sessions)[0] = strdup_safe("1");
    return 1;
}

int sd_uid_get_seats(uid_t uid, int require_active, char ***seats)
{
    (void)uid;
    (void)require_active;
    if (!seats)
        return -1;
    *seats = (char **)calloc(2, sizeof(char *));
    if (*seats)
        (*seats)[0] = strdup_safe("seat0");
    return 1;
}

int sd_uid_is_on_seat(uid_t uid, int require_active, const char *seat)
{
    (void)uid;
    (void)require_active;
    (void)seat;
    return 1;
}

/* ========================================================================= */
/* sd-login: monitor                                                         */
/* ========================================================================= */

struct sd_login_monitor {
    int fd;
};

static struct sd_login_monitor g_monitor = { .fd = 42 };

int sd_login_monitor_new(const char *category, sd_login_monitor **ret)
{
    (void)category;
    if (!ret)
        return -1;
    *ret = &g_monitor;
    return 0;
}

sd_login_monitor *sd_login_monitor_unref(sd_login_monitor *m)
{
    (void)m;
    return NULL;
}

int sd_login_monitor_flush(sd_login_monitor *m)
{
    (void)m;
    return 0;
}

int sd_login_monitor_get_fd(sd_login_monitor *m)
{
    return m ? m->fd : -1;
}

int sd_login_monitor_get_events(sd_login_monitor *m)
{
    (void)m;
    return 1; /* POLLIN */
}

int sd_login_monitor_get_timeout(sd_login_monitor *m,
                                 unsigned long long *timeout_usec)
{
    (void)m;
    if (timeout_usec)
        *timeout_usec = (unsigned long long)-1; /* infinite */
    return 0;
}

/* ========================================================================= */
/* sd-bus: internal state                                                    */
/* ========================================================================= */

#define MAX_SD_BUSES    8
#define MAX_SD_MESSAGES 64
#define MAX_SD_STR    256

struct sd_bus {
    int in_use;
    int refcount;
    int is_open;
    char unique_name[MAX_SD_STR];
};

struct sd_bus_message {
    int in_use;
    int refcount;
    char path[MAX_SD_STR];
    char interface[MAX_SD_STR];
    char member[MAX_SD_STR];
    char destination[MAX_SD_STR];
    char sender[MAX_SD_STR];
    sd_bus_error error;
};

struct sd_bus_slot {
    int in_use;
    int refcount;
};

static struct sd_bus         g_sd_buses[MAX_SD_BUSES];
static struct sd_bus_message g_sd_messages[MAX_SD_MESSAGES];
static struct sd_bus_slot    g_sd_slots[8];
static unsigned int          g_sd_bus_unique_id = 1;

static struct sd_bus *alloc_sd_bus(void)
{
    int i;
    for (i = 0; i < MAX_SD_BUSES; i++) {
        if (!g_sd_buses[i].in_use) {
            memset(&g_sd_buses[i], 0, sizeof(struct sd_bus));
            g_sd_buses[i].in_use = 1;
            g_sd_buses[i].refcount = 1;
            g_sd_buses[i].is_open = 1;
            snprintf(g_sd_buses[i].unique_name, MAX_SD_STR,
                     ":1.%u", g_sd_bus_unique_id++);
            return &g_sd_buses[i];
        }
    }
    return NULL;
}

static struct sd_bus_message *alloc_sd_message(void)
{
    int i;
    for (i = 0; i < MAX_SD_MESSAGES; i++) {
        if (!g_sd_messages[i].in_use) {
            memset(&g_sd_messages[i], 0, sizeof(struct sd_bus_message));
            g_sd_messages[i].in_use = 1;
            g_sd_messages[i].refcount = 1;
            return &g_sd_messages[i];
        }
    }
    return NULL;
}

static struct sd_bus_slot *alloc_sd_slot(void)
{
    int i;
    for (i = 0; i < 8; i++) {
        if (!g_sd_slots[i].in_use) {
            memset(&g_sd_slots[i], 0, sizeof(struct sd_bus_slot));
            g_sd_slots[i].in_use = 1;
            g_sd_slots[i].refcount = 1;
            return &g_sd_slots[i];
        }
    }
    return NULL;
}

static void sd_safe_copy(char *dst, const char *src, int max)
{
    if (src) {
        int i;
        for (i = 0; i < max - 1 && src[i]; i++)
            dst[i] = src[i];
        dst[i] = '\0';
    } else {
        dst[0] = '\0';
    }
}

/* ========================================================================= */
/* sd-bus: lifecycle                                                         */
/* ========================================================================= */

int sd_bus_open_system(sd_bus **bus)
{
    if (!bus)
        return -1;
    *bus = alloc_sd_bus();
    return *bus ? 0 : -1;
}

int sd_bus_open_user(sd_bus **bus)
{
    return sd_bus_open_system(bus);
}

int sd_bus_open_system_remote(sd_bus **bus, const char *host)
{
    (void)host;
    return sd_bus_open_system(bus);
}

int sd_bus_open_system_machine(sd_bus **bus, const char *machine)
{
    (void)machine;
    return sd_bus_open_system(bus);
}

int sd_bus_default_system(sd_bus **bus)
{
    return sd_bus_open_system(bus);
}

int sd_bus_default_user(sd_bus **bus)
{
    return sd_bus_open_user(bus);
}

int sd_bus_default(sd_bus **bus)
{
    return sd_bus_open_user(bus);
}

int sd_bus_new(sd_bus **bus)
{
    if (!bus)
        return -1;
    *bus = alloc_sd_bus();
    if (*bus)
        (*bus)->is_open = 0; /* not connected yet */
    return *bus ? 0 : -1;
}

sd_bus *sd_bus_ref(sd_bus *bus)
{
    if (bus)
        bus->refcount++;
    return bus;
}

sd_bus *sd_bus_unref(sd_bus *bus)
{
    if (!bus)
        return NULL;
    bus->refcount--;
    if (bus->refcount <= 0) {
        bus->in_use = 0;
        bus->is_open = 0;
    }
    return NULL;
}

sd_bus *sd_bus_flush_close_unref(sd_bus *bus)
{
    if (bus)
        bus->is_open = 0;
    return sd_bus_unref(bus);
}

int sd_bus_start(sd_bus *bus)
{
    if (!bus)
        return -1;
    bus->is_open = 1;
    return 0;
}

void sd_bus_close(sd_bus *bus)
{
    if (bus)
        bus->is_open = 0;
}

int sd_bus_flush(sd_bus *bus)
{
    (void)bus;
    return 0;
}

int sd_bus_is_open(sd_bus *bus)
{
    return bus ? bus->is_open : 0;
}

int sd_bus_get_bus_id(sd_bus *bus, void *id)
{
    (void)bus;
    if (id)
        memset(id, 0, 16);
    return 0;
}

int sd_bus_get_unique_name(sd_bus *bus, const char **unique)
{
    if (!bus || !unique)
        return -1;
    *unique = bus->unique_name;
    return 0;
}

/* ========================================================================= */
/* sd-bus: method calls                                                      */
/* ========================================================================= */

int sd_bus_call_method(sd_bus *bus, const char *destination,
                       const char *path, const char *interface,
                       const char *member, sd_bus_error *error,
                       sd_bus_message **reply, const char *types, ...)
{
    (void)bus;
    (void)destination;
    (void)path;
    (void)interface;
    (void)member;
    (void)error;
    (void)types;
    if (reply) {
        *reply = alloc_sd_message();
        if (!*reply)
            return -1;
    }
    return 0;
}

int sd_bus_call_method_async(sd_bus *bus, sd_bus_slot **slot,
                             const char *destination, const char *path,
                             const char *interface, const char *member,
                             void *callback, void *userdata,
                             const char *types, ...)
{
    (void)bus;
    (void)destination;
    (void)path;
    (void)interface;
    (void)member;
    (void)callback;
    (void)userdata;
    (void)types;
    if (slot) {
        *slot = alloc_sd_slot();
        if (!*slot)
            return -1;
    }
    return 0;
}

/* ========================================================================= */
/* sd-bus: property access                                                   */
/* ========================================================================= */

int sd_bus_get_property(sd_bus *bus, const char *destination,
                        const char *path, const char *interface,
                        const char *member, sd_bus_error *error,
                        sd_bus_message **reply, const char *type)
{
    (void)bus;
    (void)destination;
    (void)path;
    (void)interface;
    (void)member;
    (void)error;
    (void)type;
    if (reply) {
        *reply = alloc_sd_message();
        if (!*reply)
            return -1;
    }
    return 0;
}

int sd_bus_get_property_string(sd_bus *bus, const char *destination,
                               const char *path, const char *interface,
                               const char *member, sd_bus_error *error,
                               char **ret)
{
    (void)bus;
    (void)destination;
    (void)path;
    (void)interface;
    (void)member;
    (void)error;
    if (ret)
        *ret = strdup_safe("");
    return 0;
}

int sd_bus_get_property_trivial(sd_bus *bus, const char *destination,
                                const char *path, const char *interface,
                                const char *member, sd_bus_error *error,
                                char type, void *ret)
{
    (void)bus;
    (void)destination;
    (void)path;
    (void)interface;
    (void)member;
    (void)error;
    (void)type;
    if (ret)
        memset(ret, 0, 8);
    return 0;
}

int sd_bus_set_property(sd_bus *bus, const char *destination,
                        const char *path, const char *interface,
                        const char *member, sd_bus_error *error,
                        const char *type, ...)
{
    (void)bus;
    (void)destination;
    (void)path;
    (void)interface;
    (void)member;
    (void)error;
    (void)type;
    return 0;
}

/* ========================================================================= */
/* sd-bus: message handling                                                  */
/* ========================================================================= */

int sd_bus_message_new_method_call(sd_bus *bus, sd_bus_message **m,
                                   const char *destination,
                                   const char *path,
                                   const char *interface,
                                   const char *member)
{
    (void)bus;
    if (!m)
        return -1;
    *m = alloc_sd_message();
    if (!*m)
        return -1;
    sd_safe_copy((*m)->destination, destination, MAX_SD_STR);
    sd_safe_copy((*m)->path, path, MAX_SD_STR);
    sd_safe_copy((*m)->interface, interface, MAX_SD_STR);
    sd_safe_copy((*m)->member, member, MAX_SD_STR);
    return 0;
}

int sd_bus_message_new_signal(sd_bus *bus, sd_bus_message **m,
                               const char *path, const char *interface,
                               const char *member)
{
    (void)bus;
    if (!m)
        return -1;
    *m = alloc_sd_message();
    if (!*m)
        return -1;
    sd_safe_copy((*m)->path, path, MAX_SD_STR);
    sd_safe_copy((*m)->interface, interface, MAX_SD_STR);
    sd_safe_copy((*m)->member, member, MAX_SD_STR);
    return 0;
}

int sd_bus_message_new_method_return(sd_bus_message *call, sd_bus_message **m)
{
    (void)call;
    if (!m)
        return -1;
    *m = alloc_sd_message();
    return *m ? 0 : -1;
}

int sd_bus_message_new_method_error(sd_bus_message *call, sd_bus_message **m,
                                    const sd_bus_error *e)
{
    (void)call;
    (void)e;
    if (!m)
        return -1;
    *m = alloc_sd_message();
    return *m ? 0 : -1;
}

sd_bus_message *sd_bus_message_ref(sd_bus_message *m)
{
    if (m)
        m->refcount++;
    return m;
}

sd_bus_message *sd_bus_message_unref(sd_bus_message *m)
{
    if (!m)
        return NULL;
    m->refcount--;
    if (m->refcount <= 0)
        m->in_use = 0;
    return NULL;
}

int sd_bus_message_read(sd_bus_message *m, const char *types, ...)
{
    (void)m;
    (void)types;
    return 0;
}

int sd_bus_message_read_basic(sd_bus_message *m, char type, void *p)
{
    (void)m;
    (void)type;
    if (p)
        memset(p, 0, 8);
    return 0;
}

int sd_bus_message_append(sd_bus_message *m, const char *types, ...)
{
    (void)m;
    (void)types;
    return 0;
}

int sd_bus_message_append_basic(sd_bus_message *m, char type, const void *p)
{
    (void)m;
    (void)type;
    (void)p;
    return 0;
}

int sd_bus_message_open_container(sd_bus_message *m, char type,
                                  const char *contents)
{
    (void)m;
    (void)type;
    (void)contents;
    return 0;
}

int sd_bus_message_close_container(sd_bus_message *m)
{
    (void)m;
    return 0;
}

int sd_bus_message_enter_container(sd_bus_message *m, char type,
                                   const char *contents)
{
    (void)m;
    (void)type;
    (void)contents;
    return 0;
}

int sd_bus_message_exit_container(sd_bus_message *m)
{
    (void)m;
    return 0;
}

int sd_bus_message_peek_type(sd_bus_message *m, char *type,
                             const char **contents)
{
    (void)m;
    if (type)
        *type = 0;
    if (contents)
        *contents = NULL;
    return 0;
}

int sd_bus_message_at_end(sd_bus_message *m, int complete)
{
    (void)m;
    (void)complete;
    return 1; /* at end */
}

int sd_bus_message_skip(sd_bus_message *m, const char *types)
{
    (void)m;
    (void)types;
    return 0;
}

int sd_bus_message_get_errno(sd_bus_message *m)
{
    (void)m;
    return 0;
}

int sd_bus_message_is_signal(sd_bus_message *m, const char *interface,
                             const char *member)
{
    if (!m)
        return 0;
    if (interface && strcmp(m->interface, interface) != 0)
        return 0;
    if (member && strcmp(m->member, member) != 0)
        return 0;
    return 1;
}

int sd_bus_message_is_method_call(sd_bus_message *m, const char *interface,
                                  const char *member)
{
    if (!m)
        return 0;
    if (interface && strcmp(m->interface, interface) != 0)
        return 0;
    if (member && strcmp(m->member, member) != 0)
        return 0;
    return 1;
}

int sd_bus_message_is_method_error(sd_bus_message *m, const char *name)
{
    (void)m;
    (void)name;
    return 0;
}

const char *sd_bus_message_get_path(sd_bus_message *m)
{
    return m ? m->path : NULL;
}

const char *sd_bus_message_get_interface(sd_bus_message *m)
{
    return m ? m->interface : NULL;
}

const char *sd_bus_message_get_member(sd_bus_message *m)
{
    return m ? m->member : NULL;
}

const char *sd_bus_message_get_destination(sd_bus_message *m)
{
    return m ? m->destination : NULL;
}

const char *sd_bus_message_get_sender(sd_bus_message *m)
{
    return m ? m->sender : NULL;
}

const sd_bus_error *sd_bus_message_get_error(sd_bus_message *m)
{
    return m ? &m->error : NULL;
}

int sd_bus_send(sd_bus *bus, sd_bus_message *m, uint64_t *cookie)
{
    (void)bus;
    (void)m;
    if (cookie)
        *cookie = 1;
    return 0;
}

int sd_bus_call(sd_bus *bus, sd_bus_message *m, uint64_t usec,
                sd_bus_error *error, sd_bus_message **reply)
{
    (void)bus;
    (void)m;
    (void)usec;
    (void)error;
    if (reply) {
        *reply = alloc_sd_message();
        if (!*reply)
            return -1;
    }
    return 0;
}

/* ========================================================================= */
/* sd-bus: signal matching                                                   */
/* ========================================================================= */

int sd_bus_match_signal(sd_bus *bus, sd_bus_slot **slot,
                        const char *sender, const char *path,
                        const char *interface, const char *member,
                        void *callback, void *userdata)
{
    (void)bus;
    (void)sender;
    (void)path;
    (void)interface;
    (void)member;
    (void)callback;
    (void)userdata;
    if (slot)
        *slot = alloc_sd_slot();
    return 0;
}

int sd_bus_match_signal_async(sd_bus *bus, sd_bus_slot **slot,
                              const char *sender, const char *path,
                              const char *interface, const char *member,
                              void *callback, void *install_callback,
                              void *userdata)
{
    (void)bus;
    (void)sender;
    (void)path;
    (void)interface;
    (void)member;
    (void)callback;
    (void)install_callback;
    (void)userdata;
    if (slot)
        *slot = alloc_sd_slot();
    return 0;
}

/* ========================================================================= */
/* sd-bus: name ownership                                                    */
/* ========================================================================= */

int sd_bus_request_name(sd_bus *bus, const char *name, uint64_t flags)
{
    (void)bus;
    (void)name;
    (void)flags;
    return 0;
}

int sd_bus_release_name(sd_bus *bus, const char *name)
{
    (void)bus;
    (void)name;
    return 0;
}

/* ========================================================================= */
/* sd-bus: event loop                                                        */
/* ========================================================================= */

int sd_bus_get_fd(sd_bus *bus)
{
    (void)bus;
    return 43;
}

int sd_bus_get_events(sd_bus *bus)
{
    (void)bus;
    return 1; /* POLLIN */
}

int sd_bus_get_timeout(sd_bus *bus, uint64_t *timeout_usec)
{
    (void)bus;
    if (timeout_usec)
        *timeout_usec = (uint64_t)-1;
    return 0;
}

int sd_bus_process(sd_bus *bus, sd_bus_message **r)
{
    (void)bus;
    if (r)
        *r = NULL;
    return 0;
}

int sd_bus_wait(sd_bus *bus, uint64_t timeout_usec)
{
    (void)bus;
    (void)timeout_usec;
    return 0;
}

/* ========================================================================= */
/* sd-bus: slot                                                              */
/* ========================================================================= */

sd_bus_slot *sd_bus_slot_ref(sd_bus_slot *slot)
{
    if (slot)
        slot->refcount++;
    return slot;
}

sd_bus_slot *sd_bus_slot_unref(sd_bus_slot *slot)
{
    if (!slot)
        return NULL;
    slot->refcount--;
    if (slot->refcount <= 0)
        slot->in_use = 0;
    return NULL;
}

/* ========================================================================= */
/* sd-bus: error helpers                                                     */
/* ========================================================================= */

void sd_bus_error_free(sd_bus_error *e)
{
    if (e) {
        e->name = NULL;
        e->message = NULL;
        e->_need_free = 0;
    }
}

int sd_bus_error_set(sd_bus_error *e, const char *name, const char *message)
{
    if (e) {
        e->name = name;
        e->message = message;
        e->_need_free = 0;
    }
    return -1; /* convention: return negative errno */
}

int sd_bus_error_set_const(sd_bus_error *e, const char *name,
                           const char *message)
{
    return sd_bus_error_set(e, name, message);
}

int sd_bus_error_is_set(const sd_bus_error *e)
{
    return (e && e->name) ? 1 : 0;
}

int sd_bus_error_has_name(const sd_bus_error *e, const char *name)
{
    if (!e || !e->name || !name)
        return 0;
    return strcmp(e->name, name) == 0 ? 1 : 0;
}
