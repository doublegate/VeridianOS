/*
 * VeridianOS libc -- dbus.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libdbus-1 client API implementation for VeridianOS.
 * Provides connection lifecycle, message creation/iteration, bus name
 * ownership, signal matching, pending calls, error handling, memory,
 * threading, and signature validation.  Backed by static pools with
 * sensible defaults -- no actual Unix socket transport.
 */

#include <dbus/dbus.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/* ========================================================================= */
/* Pool sizes                                                                */
/* ========================================================================= */

#define MAX_CONNECTIONS    32
#define MAX_MESSAGES      512
#define MAX_PENDING        64
#define MAX_ARGS           32
#define MAX_STR           256

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct dbus_arg {
    int         type;
    union {
        unsigned char  byte_val;
        dbus_bool_t    bool_val;
        dbus_int16_t   i16_val;
        dbus_uint16_t  u16_val;
        dbus_int32_t   i32_val;
        dbus_uint32_t  u32_val;
        dbus_int64_t   i64_val;
        dbus_uint64_t  u64_val;
        double         dbl_val;
        char           str_val[MAX_STR];
    } value;
};

struct DBusMessage {
    int              in_use;
    int              refcount;
    int              msg_type;
    char             destination[MAX_STR];
    char             path[MAX_STR];
    char             interface[MAX_STR];
    char             member[MAX_STR];
    char             error_name[MAX_STR];
    char             sender[MAX_STR];
    char             signature[MAX_STR];
    dbus_uint32_t    serial;
    dbus_uint32_t    reply_serial;
    dbus_bool_t      no_reply;
    dbus_bool_t      auto_start;
    struct dbus_arg  args[MAX_ARGS];
    int              num_args;
};

struct DBusConnection {
    int              in_use;
    int              refcount;
    DBusBusType      bus_type;
    int              connected;
    int              exit_on_disconnect;
    char             unique_name[MAX_STR];
    dbus_uint32_t    next_serial;
    long             max_message_size;
    long             max_message_unix_fds;
    long             max_received_size;
    long             max_received_unix_fds;
};

struct DBusPendingCall {
    int              in_use;
    int              refcount;
    int              completed;
    int              reply_idx;    /* index into message pool, or -1 */
    DBusPendingCallNotifyFunction notify_fn;
    void            *notify_data;
    DBusFreeFunction notify_free;
};

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static struct DBusConnection g_connections[MAX_CONNECTIONS];
static struct DBusMessage    g_messages[MAX_MESSAGES];
static struct DBusPendingCall g_pending[MAX_PENDING];
static dbus_uint32_t         g_next_unique_id = 1;
static dbus_uint32_t         g_next_serial    = 1;
static int                   g_threads_initialized = 0;
static dbus_int32_t          g_next_data_slot = 0;

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static void safe_copy(char *dst, const char *src, int max)
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

static struct DBusConnection *alloc_connection(void)
{
    int i;
    for (i = 0; i < MAX_CONNECTIONS; i++) {
        if (!g_connections[i].in_use) {
            memset(&g_connections[i], 0, sizeof(struct DBusConnection));
            g_connections[i].in_use = 1;
            g_connections[i].refcount = 1;
            g_connections[i].connected = 1;
            g_connections[i].exit_on_disconnect = 1;
            g_connections[i].next_serial = 1;
            g_connections[i].max_message_size = DBUS_MAXIMUM_MESSAGE_LENGTH;
            g_connections[i].max_message_unix_fds = DBUS_MAXIMUM_MESSAGE_UNIX_FDS;
            g_connections[i].max_received_size = DBUS_MAXIMUM_MESSAGE_LENGTH * 4;
            g_connections[i].max_received_unix_fds = DBUS_MAXIMUM_MESSAGE_UNIX_FDS * 4;
            snprintf(g_connections[i].unique_name,
                     MAX_STR, ":1.%u", g_next_unique_id++);
            return &g_connections[i];
        }
    }
    return NULL;
}

static struct DBusMessage *alloc_message(void)
{
    int i;
    for (i = 0; i < MAX_MESSAGES; i++) {
        if (!g_messages[i].in_use) {
            memset(&g_messages[i], 0, sizeof(struct DBusMessage));
            g_messages[i].in_use = 1;
            g_messages[i].refcount = 1;
            g_messages[i].auto_start = TRUE;
            g_messages[i].serial = g_next_serial++;
            return &g_messages[i];
        }
    }
    return NULL;
}

static struct DBusPendingCall *alloc_pending(void)
{
    int i;
    for (i = 0; i < MAX_PENDING; i++) {
        if (!g_pending[i].in_use) {
            memset(&g_pending[i], 0, sizeof(struct DBusPendingCall));
            g_pending[i].in_use = 1;
            g_pending[i].refcount = 1;
            g_pending[i].reply_idx = -1;
            return &g_pending[i];
        }
    }
    return NULL;
}

/* ========================================================================= */
/* Error handling                                                            */
/* ========================================================================= */

void dbus_error_init(DBusError *error)
{
    if (error) {
        error->name = NULL;
        error->message = NULL;
        error->dummy1 = 0;
        error->dummy2 = 0;
        error->dummy3 = 0;
        error->dummy4 = 0;
        error->dummy5 = 0;
        error->padding1 = NULL;
    }
}

void dbus_error_free(DBusError *error)
{
    if (error) {
        /* Static strings -- nothing to free */
        error->name = NULL;
        error->message = NULL;
    }
}

void dbus_set_error(DBusError *error, const char *name,
                    const char *format, ...)
{
    if (error) {
        error->name = name;
        error->message = format ? format : name;
    }
}

void dbus_set_error_const(DBusError *error, const char *name,
                          const char *message)
{
    if (error) {
        error->name = name;
        error->message = message;
    }
}

void dbus_move_error(DBusError *src, DBusError *dest)
{
    if (dest) {
        dest->name = src ? src->name : NULL;
        dest->message = src ? src->message : NULL;
    }
    if (src)
        dbus_error_init(src);
}

dbus_bool_t dbus_error_has_name(const DBusError *error, const char *name)
{
    if (!error || !error->name || !name)
        return FALSE;
    return strcmp(error->name, name) == 0 ? TRUE : FALSE;
}

dbus_bool_t dbus_error_is_set(const DBusError *error)
{
    return (error && error->name) ? TRUE : FALSE;
}

/* ========================================================================= */
/* Memory                                                                    */
/* ========================================================================= */

void *dbus_malloc(size_t bytes)
{
    return malloc(bytes);
}

void *dbus_malloc0(size_t bytes)
{
    return calloc(1, bytes);
}

void *dbus_realloc(void *memory, size_t bytes)
{
    return realloc(memory, bytes);
}

void dbus_free(void *memory)
{
    free(memory);
}

void dbus_free_string_array(char **str_array)
{
    if (str_array) {
        int i;
        for (i = 0; str_array[i]; i++)
            free(str_array[i]);
        free(str_array);
    }
}

void dbus_shutdown(void)
{
    int i;
    for (i = 0; i < MAX_CONNECTIONS; i++)
        g_connections[i].in_use = 0;
    for (i = 0; i < MAX_MESSAGES; i++)
        g_messages[i].in_use = 0;
    for (i = 0; i < MAX_PENDING; i++)
        g_pending[i].in_use = 0;
    g_next_unique_id = 1;
    g_next_serial = 1;
    g_threads_initialized = 0;
    g_next_data_slot = 0;
}

/* ========================================================================= */
/* Threading                                                                 */
/* ========================================================================= */

dbus_bool_t dbus_threads_init_default(void)
{
    g_threads_initialized = 1;
    return TRUE;
}

/* ========================================================================= */
/* Bus connection                                                            */
/* ========================================================================= */

DBusConnection *dbus_bus_get(DBusBusType type, DBusError *error)
{
    struct DBusConnection *conn = alloc_connection();
    if (!conn) {
        dbus_set_error(error, DBUS_ERROR_NO_MEMORY,
                       "Connection pool exhausted");
        return NULL;
    }
    conn->bus_type = type;
    return conn;
}

DBusConnection *dbus_bus_get_private(DBusBusType type, DBusError *error)
{
    return dbus_bus_get(type, error);
}

dbus_bool_t dbus_bus_register(DBusConnection *connection, DBusError *error)
{
    (void)error;
    if (!connection)
        return FALSE;
    return TRUE;
}

void dbus_bus_set_unique_name(DBusConnection *connection,
                              const char *unique_name)
{
    if (connection)
        safe_copy(connection->unique_name, unique_name, MAX_STR);
}

const char *dbus_bus_get_unique_name(DBusConnection *connection)
{
    if (!connection)
        return NULL;
    return connection->unique_name;
}

unsigned long dbus_bus_get_unix_user(DBusConnection *connection,
                                    const char *name, DBusError *error)
{
    (void)connection;
    (void)name;
    (void)error;
    return 0; /* root */
}

char *dbus_bus_get_id(DBusConnection *connection, DBusError *error)
{
    (void)error;
    (void)connection;
    char *id = (char *)malloc(64);
    if (id)
        snprintf(id, 64, "veridian-dbus-%u", g_next_unique_id);
    return id;
}

/* ========================================================================= */
/* Name ownership                                                            */
/* ========================================================================= */

int dbus_bus_request_name(DBusConnection *connection, const char *name,
                          unsigned int flags, DBusError *error)
{
    (void)connection;
    (void)name;
    (void)flags;
    (void)error;
    return DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER;
}

int dbus_bus_release_name(DBusConnection *connection, const char *name,
                          DBusError *error)
{
    (void)connection;
    (void)name;
    (void)error;
    return DBUS_RELEASE_NAME_REPLY_RELEASED;
}

dbus_bool_t dbus_bus_name_has_owner(DBusConnection *connection,
                                    const char *name, DBusError *error)
{
    (void)connection;
    (void)name;
    (void)error;
    return TRUE;
}

dbus_bool_t dbus_bus_start_service_by_name(DBusConnection *connection,
                                           const char *name,
                                           dbus_uint32_t flags,
                                           dbus_uint32_t *result,
                                           DBusError *error)
{
    (void)connection;
    (void)name;
    (void)flags;
    (void)error;
    if (result)
        *result = DBUS_START_REPLY_ALREADY_RUNNING;
    return TRUE;
}

/* ========================================================================= */
/* Signal matching                                                           */
/* ========================================================================= */

void dbus_bus_add_match(DBusConnection *connection, const char *rule,
                        DBusError *error)
{
    (void)connection;
    (void)rule;
    (void)error;
}

void dbus_bus_remove_match(DBusConnection *connection, const char *rule,
                           DBusError *error)
{
    (void)connection;
    (void)rule;
    (void)error;
}

/* ========================================================================= */
/* Connection lifecycle                                                      */
/* ========================================================================= */

DBusConnection *dbus_connection_open(const char *address, DBusError *error)
{
    (void)address;
    return dbus_bus_get(DBUS_BUS_SESSION, error);
}

DBusConnection *dbus_connection_open_private(const char *address,
                                             DBusError *error)
{
    return dbus_connection_open(address, error);
}

DBusConnection *dbus_connection_ref(DBusConnection *connection)
{
    if (connection)
        connection->refcount++;
    return connection;
}

void dbus_connection_unref(DBusConnection *connection)
{
    if (!connection)
        return;
    connection->refcount--;
    if (connection->refcount <= 0) {
        connection->in_use = 0;
        connection->connected = 0;
    }
}

void dbus_connection_close(DBusConnection *connection)
{
    if (connection)
        connection->connected = 0;
}

dbus_bool_t dbus_connection_get_is_connected(DBusConnection *connection)
{
    if (!connection)
        return FALSE;
    return connection->connected ? TRUE : FALSE;
}

dbus_bool_t dbus_connection_get_is_authenticated(DBusConnection *connection)
{
    return dbus_connection_get_is_connected(connection);
}

dbus_bool_t dbus_connection_get_is_anonymous(DBusConnection *connection)
{
    (void)connection;
    return FALSE;
}

char *dbus_connection_get_server_id(DBusConnection *connection)
{
    (void)connection;
    return NULL;
}

dbus_bool_t dbus_connection_can_send_type(DBusConnection *connection, int type)
{
    (void)connection;
    (void)type;
    return TRUE;
}

/* ========================================================================= */
/* Sending                                                                   */
/* ========================================================================= */

dbus_bool_t dbus_connection_send(DBusConnection *connection,
                                 DBusMessage *message,
                                 dbus_uint32_t *serial)
{
    if (!connection || !message)
        return FALSE;
    message->serial = connection->next_serial++;
    if (serial)
        *serial = message->serial;
    return TRUE;
}

dbus_bool_t dbus_connection_send_with_reply(DBusConnection *connection,
                                            DBusMessage *message,
                                            DBusPendingCall **pending_return,
                                            int timeout_milliseconds)
{
    (void)timeout_milliseconds;
    if (!connection || !message)
        return FALSE;

    message->serial = connection->next_serial++;

    if (pending_return) {
        struct DBusPendingCall *pc = alloc_pending();
        if (!pc)
            return FALSE;
        /* Create an auto-reply */
        struct DBusMessage *reply = alloc_message();
        if (reply) {
            reply->msg_type = DBUS_MESSAGE_TYPE_METHOD_RETURN;
            reply->reply_serial = message->serial;
            pc->reply_idx = (int)(reply - g_messages);
            pc->completed = 1;
        }
        *pending_return = pc;
    }
    return TRUE;
}

DBusMessage *dbus_connection_send_with_reply_and_block(
    DBusConnection *connection, DBusMessage *message,
    int timeout_milliseconds, DBusError *error)
{
    (void)timeout_milliseconds;
    (void)error;
    if (!connection || !message)
        return NULL;

    message->serial = connection->next_serial++;

    /* Return a valid empty reply */
    struct DBusMessage *reply = alloc_message();
    if (reply) {
        reply->msg_type = DBUS_MESSAGE_TYPE_METHOD_RETURN;
        reply->reply_serial = message->serial;
        safe_copy(reply->sender, DBUS_SERVICE_DBUS, MAX_STR);
    }
    return reply;
}

void dbus_connection_flush(DBusConnection *connection)
{
    (void)connection;
}

/* ========================================================================= */
/* Dispatching                                                               */
/* ========================================================================= */

dbus_bool_t dbus_connection_read_write_dispatch(DBusConnection *connection,
                                                int timeout_milliseconds)
{
    (void)timeout_milliseconds;
    if (!connection || !connection->connected)
        return FALSE;
    return TRUE;
}

dbus_bool_t dbus_connection_read_write(DBusConnection *connection,
                                       int timeout_milliseconds)
{
    return dbus_connection_read_write_dispatch(connection,
                                              timeout_milliseconds);
}

DBusDispatchStatus dbus_connection_dispatch(DBusConnection *connection)
{
    (void)connection;
    return DBUS_DISPATCH_COMPLETE;
}

DBusDispatchStatus dbus_connection_get_dispatch_status(
    DBusConnection *connection)
{
    (void)connection;
    return DBUS_DISPATCH_COMPLETE;
}

/* ========================================================================= */
/* Message pop / borrow                                                      */
/* ========================================================================= */

DBusMessage *dbus_connection_borrow_message(DBusConnection *connection)
{
    (void)connection;
    return NULL;
}

void dbus_connection_return_message(DBusConnection *connection,
                                   DBusMessage *message)
{
    (void)connection;
    (void)message;
}

void dbus_connection_steal_borrowed_message(DBusConnection *connection,
                                            DBusMessage *message)
{
    (void)connection;
    (void)message;
}

DBusMessage *dbus_connection_pop_message(DBusConnection *connection)
{
    (void)connection;
    return NULL;
}

/* ========================================================================= */
/* Watches and timeouts                                                      */
/* ========================================================================= */

dbus_bool_t dbus_connection_set_watch_functions(
    DBusConnection *connection,
    DBusAddWatchFunction add_function,
    DBusRemoveWatchFunction remove_function,
    DBusWatchToggledFunction toggled_function,
    void *data,
    DBusFreeFunction free_data_function)
{
    (void)connection;
    (void)add_function;
    (void)remove_function;
    (void)toggled_function;
    (void)data;
    (void)free_data_function;
    return TRUE;
}

dbus_bool_t dbus_connection_set_timeout_functions(
    DBusConnection *connection,
    DBusAddTimeoutFunction add_function,
    DBusRemoveTimeoutFunction remove_function,
    DBusTimeoutToggledFunction toggled_function,
    void *data,
    DBusFreeFunction free_data_function)
{
    (void)connection;
    (void)add_function;
    (void)remove_function;
    (void)toggled_function;
    (void)data;
    (void)free_data_function;
    return TRUE;
}

/* Watch API stubs */
int dbus_watch_get_unix_fd(DBusWatch *watch)
{
    (void)watch;
    return -1;
}

int dbus_watch_get_socket(DBusWatch *watch)
{
    (void)watch;
    return -1;
}

unsigned int dbus_watch_get_flags(DBusWatch *watch)
{
    (void)watch;
    return 0;
}

void *dbus_watch_get_data(DBusWatch *watch)
{
    (void)watch;
    return NULL;
}

void dbus_watch_set_data(DBusWatch *watch, void *data,
                         DBusFreeFunction free_data_function)
{
    (void)watch;
    (void)data;
    (void)free_data_function;
}

dbus_bool_t dbus_watch_handle(DBusWatch *watch, unsigned int flags)
{
    (void)watch;
    (void)flags;
    return TRUE;
}

dbus_bool_t dbus_watch_get_enabled(DBusWatch *watch)
{
    (void)watch;
    return TRUE;
}

/* Timeout API stubs */
int dbus_timeout_get_interval(DBusTimeout *timeout)
{
    (void)timeout;
    return -1;
}

void *dbus_timeout_get_data(DBusTimeout *timeout)
{
    (void)timeout;
    return NULL;
}

void dbus_timeout_set_data(DBusTimeout *timeout, void *data,
                           DBusFreeFunction free_data_function)
{
    (void)timeout;
    (void)data;
    (void)free_data_function;
}

dbus_bool_t dbus_timeout_handle(DBusTimeout *timeout)
{
    (void)timeout;
    return TRUE;
}

dbus_bool_t dbus_timeout_get_enabled(DBusTimeout *timeout)
{
    (void)timeout;
    return TRUE;
}

/* ========================================================================= */
/* Filters                                                                   */
/* ========================================================================= */

dbus_bool_t dbus_connection_add_filter(DBusConnection *connection,
                                       DBusHandleMessageFunction function,
                                       void *user_data,
                                       DBusFreeFunction free_data_function)
{
    (void)connection;
    (void)function;
    (void)user_data;
    (void)free_data_function;
    return TRUE;
}

void dbus_connection_remove_filter(DBusConnection *connection,
                                   DBusHandleMessageFunction function,
                                   void *user_data)
{
    (void)connection;
    (void)function;
    (void)user_data;
}

/* ========================================================================= */
/* Dispatch status / wakeup                                                  */
/* ========================================================================= */

void dbus_connection_set_dispatch_status_function(
    DBusConnection *connection,
    DBusDispatchStatusFunction function,
    void *data, DBusFreeFunction free_data_function)
{
    (void)connection;
    (void)function;
    (void)data;
    (void)free_data_function;
}

void dbus_connection_set_wakeup_main_function(
    DBusConnection *connection,
    DBusWakeupMainFunction wakeup_main_function,
    void *data, DBusFreeFunction free_data_function)
{
    (void)connection;
    (void)wakeup_main_function;
    (void)data;
    (void)free_data_function;
}

/* ========================================================================= */
/* Unix fd / process / user                                                  */
/* ========================================================================= */

dbus_bool_t dbus_connection_get_unix_fd(DBusConnection *connection, int *fd)
{
    (void)connection;
    if (fd)
        *fd = -1;
    return FALSE;
}

dbus_bool_t dbus_connection_get_unix_process_id(DBusConnection *connection,
                                                unsigned long *pid)
{
    (void)connection;
    if (pid)
        *pid = 1;
    return TRUE;
}

dbus_bool_t dbus_connection_get_unix_user(DBusConnection *connection,
                                          unsigned long *uid)
{
    (void)connection;
    if (uid)
        *uid = 0;
    return TRUE;
}

/* ========================================================================= */
/* Data slots                                                                */
/* ========================================================================= */

dbus_bool_t dbus_connection_set_data(DBusConnection *connection,
                                     dbus_int32_t slot,
                                     void *data,
                                     DBusFreeFunction free_data_func)
{
    (void)connection;
    (void)slot;
    (void)data;
    (void)free_data_func;
    return TRUE;
}

void *dbus_connection_get_data(DBusConnection *connection, dbus_int32_t slot)
{
    (void)connection;
    (void)slot;
    return NULL;
}

dbus_bool_t dbus_connection_allocate_data_slot(dbus_int32_t *slot_p)
{
    if (slot_p)
        *slot_p = g_next_data_slot++;
    return TRUE;
}

void dbus_connection_free_data_slot(dbus_int32_t *slot_p)
{
    (void)slot_p;
}

/* ========================================================================= */
/* Max message / fd limits                                                   */
/* ========================================================================= */

void dbus_connection_set_max_message_size(DBusConnection *connection,
                                          long size)
{
    if (connection)
        connection->max_message_size = size;
}

long dbus_connection_get_max_message_size(DBusConnection *connection)
{
    return connection ? connection->max_message_size
                      : DBUS_MAXIMUM_MESSAGE_LENGTH;
}

void dbus_connection_set_max_message_unix_fds(DBusConnection *connection,
                                              long n)
{
    if (connection)
        connection->max_message_unix_fds = n;
}

long dbus_connection_get_max_message_unix_fds(DBusConnection *connection)
{
    return connection ? connection->max_message_unix_fds
                      : DBUS_MAXIMUM_MESSAGE_UNIX_FDS;
}

void dbus_connection_set_max_received_size(DBusConnection *connection,
                                           long size)
{
    if (connection)
        connection->max_received_size = size;
}

long dbus_connection_get_max_received_size(DBusConnection *connection)
{
    return connection ? connection->max_received_size
                      : DBUS_MAXIMUM_MESSAGE_LENGTH * 4;
}

void dbus_connection_set_max_received_unix_fds(DBusConnection *connection,
                                               long n)
{
    if (connection)
        connection->max_received_unix_fds = n;
}

long dbus_connection_get_max_received_unix_fds(DBusConnection *connection)
{
    return connection ? connection->max_received_unix_fds
                      : DBUS_MAXIMUM_MESSAGE_UNIX_FDS * 4;
}

long dbus_connection_get_outgoing_size(DBusConnection *connection)
{
    (void)connection;
    return 0;
}

long dbus_connection_get_outgoing_unix_fds(DBusConnection *connection)
{
    (void)connection;
    return 0;
}

/* ========================================================================= */
/* Exit on disconnect                                                        */
/* ========================================================================= */

void dbus_connection_set_exit_on_disconnect(DBusConnection *connection,
                                            dbus_bool_t exit_on_disconnect)
{
    if (connection)
        connection->exit_on_disconnect = exit_on_disconnect ? 1 : 0;
}

/* ========================================================================= */
/* Object path registration (stubs)                                          */
/* ========================================================================= */

dbus_bool_t dbus_connection_try_register_object_path(
    DBusConnection *connection, const char *path,
    const DBusObjectPathVTable *vtable, void *user_data,
    DBusError *error)
{
    (void)connection;
    (void)path;
    (void)vtable;
    (void)user_data;
    (void)error;
    return TRUE;
}

dbus_bool_t dbus_connection_register_object_path(
    DBusConnection *connection, const char *path,
    const DBusObjectPathVTable *vtable, void *user_data)
{
    (void)connection;
    (void)path;
    (void)vtable;
    (void)user_data;
    return TRUE;
}

dbus_bool_t dbus_connection_try_register_fallback(
    DBusConnection *connection, const char *path,
    const DBusObjectPathVTable *vtable, void *user_data,
    DBusError *error)
{
    (void)connection;
    (void)path;
    (void)vtable;
    (void)user_data;
    (void)error;
    return TRUE;
}

dbus_bool_t dbus_connection_register_fallback(
    DBusConnection *connection, const char *path,
    const DBusObjectPathVTable *vtable, void *user_data)
{
    (void)connection;
    (void)path;
    (void)vtable;
    (void)user_data;
    return TRUE;
}

dbus_bool_t dbus_connection_unregister_object_path(
    DBusConnection *connection, const char *path)
{
    (void)connection;
    (void)path;
    return TRUE;
}

dbus_bool_t dbus_connection_get_object_path_data(
    DBusConnection *connection, const char *path, void **data_p)
{
    (void)connection;
    (void)path;
    if (data_p)
        *data_p = NULL;
    return TRUE;
}

dbus_bool_t dbus_connection_list_registered(
    DBusConnection *connection, const char *parent_path,
    char ***child_entries)
{
    (void)connection;
    (void)parent_path;
    if (child_entries) {
        *child_entries = (char **)dbus_malloc0(sizeof(char *));
    }
    return TRUE;
}

/* ========================================================================= */
/* Message lifecycle                                                         */
/* ========================================================================= */

DBusMessage *dbus_message_new(int message_type)
{
    struct DBusMessage *msg = alloc_message();
    if (msg)
        msg->msg_type = message_type;
    return msg;
}

DBusMessage *dbus_message_new_method_call(const char *destination,
                                          const char *path,
                                          const char *iface,
                                          const char *method)
{
    struct DBusMessage *msg = alloc_message();
    if (!msg)
        return NULL;
    msg->msg_type = DBUS_MESSAGE_TYPE_METHOD_CALL;
    safe_copy(msg->destination, destination, MAX_STR);
    safe_copy(msg->path, path, MAX_STR);
    safe_copy(msg->interface, iface, MAX_STR);
    safe_copy(msg->member, method, MAX_STR);
    return msg;
}

DBusMessage *dbus_message_new_method_return(DBusMessage *method_call)
{
    struct DBusMessage *msg = alloc_message();
    if (!msg)
        return NULL;
    msg->msg_type = DBUS_MESSAGE_TYPE_METHOD_RETURN;
    if (method_call) {
        msg->reply_serial = method_call->serial;
        safe_copy(msg->destination, method_call->sender, MAX_STR);
    }
    return msg;
}

DBusMessage *dbus_message_new_signal(const char *path, const char *iface,
                                     const char *name)
{
    struct DBusMessage *msg = alloc_message();
    if (!msg)
        return NULL;
    msg->msg_type = DBUS_MESSAGE_TYPE_SIGNAL;
    safe_copy(msg->path, path, MAX_STR);
    safe_copy(msg->interface, iface, MAX_STR);
    safe_copy(msg->member, name, MAX_STR);
    return msg;
}

DBusMessage *dbus_message_new_error(DBusMessage *reply_to,
                                    const char *error_name,
                                    const char *error_message)
{
    struct DBusMessage *msg = alloc_message();
    if (!msg)
        return NULL;
    msg->msg_type = DBUS_MESSAGE_TYPE_ERROR;
    safe_copy(msg->error_name, error_name, MAX_STR);
    if (reply_to) {
        msg->reply_serial = reply_to->serial;
        safe_copy(msg->destination, reply_to->sender, MAX_STR);
    }
    /* Store error message as a string argument */
    if (error_message && msg->num_args < MAX_ARGS) {
        msg->args[0].type = DBUS_TYPE_STRING;
        safe_copy(msg->args[0].value.str_val, error_message, MAX_STR);
        msg->num_args = 1;
    }
    return msg;
}

DBusMessage *dbus_message_new_error_printf(DBusMessage *reply_to,
                                           const char *error_name,
                                           const char *error_format, ...)
{
    /* Simplified: just use the format string as-is */
    return dbus_message_new_error(reply_to, error_name, error_format);
}

DBusMessage *dbus_message_ref(DBusMessage *message)
{
    if (message)
        message->refcount++;
    return message;
}

void dbus_message_unref(DBusMessage *message)
{
    if (!message)
        return;
    message->refcount--;
    if (message->refcount <= 0)
        message->in_use = 0;
}

DBusMessage *dbus_message_copy(const DBusMessage *message)
{
    if (!message)
        return NULL;
    struct DBusMessage *copy = alloc_message();
    if (!copy)
        return NULL;
    int saved_serial = copy->serial;
    memcpy(copy, message, sizeof(struct DBusMessage));
    copy->serial = saved_serial;
    copy->refcount = 1;
    copy->in_use = 1;
    return copy;
}

/* ========================================================================= */
/* Message metadata                                                          */
/* ========================================================================= */

int dbus_message_get_type(DBusMessage *message)
{
    return message ? message->msg_type : DBUS_MESSAGE_TYPE_INVALID;
}

dbus_bool_t dbus_message_set_path(DBusMessage *message,
                                  const char *object_path)
{
    if (!message)
        return FALSE;
    safe_copy(message->path, object_path, MAX_STR);
    return TRUE;
}

const char *dbus_message_get_path(DBusMessage *message)
{
    if (!message || message->path[0] == '\0')
        return NULL;
    return message->path;
}

dbus_bool_t dbus_message_has_path(DBusMessage *message, const char *path)
{
    if (!message || !path)
        return FALSE;
    return strcmp(message->path, path) == 0 ? TRUE : FALSE;
}

dbus_bool_t dbus_message_set_interface(DBusMessage *message, const char *iface)
{
    if (!message)
        return FALSE;
    safe_copy(message->interface, iface, MAX_STR);
    return TRUE;
}

const char *dbus_message_get_interface(DBusMessage *message)
{
    if (!message || message->interface[0] == '\0')
        return NULL;
    return message->interface;
}

dbus_bool_t dbus_message_has_interface(DBusMessage *message, const char *iface)
{
    if (!message || !iface)
        return FALSE;
    return strcmp(message->interface, iface) == 0 ? TRUE : FALSE;
}

dbus_bool_t dbus_message_set_member(DBusMessage *message, const char *member)
{
    if (!message)
        return FALSE;
    safe_copy(message->member, member, MAX_STR);
    return TRUE;
}

const char *dbus_message_get_member(DBusMessage *message)
{
    if (!message || message->member[0] == '\0')
        return NULL;
    return message->member;
}

dbus_bool_t dbus_message_has_member(DBusMessage *message, const char *member)
{
    if (!message || !member)
        return FALSE;
    return strcmp(message->member, member) == 0 ? TRUE : FALSE;
}

dbus_bool_t dbus_message_set_error_name(DBusMessage *message,
                                        const char *error_name)
{
    if (!message)
        return FALSE;
    safe_copy(message->error_name, error_name, MAX_STR);
    return TRUE;
}

const char *dbus_message_get_error_name(DBusMessage *message)
{
    if (!message || message->error_name[0] == '\0')
        return NULL;
    return message->error_name;
}

dbus_bool_t dbus_message_set_destination(DBusMessage *message,
                                         const char *destination)
{
    if (!message)
        return FALSE;
    safe_copy(message->destination, destination, MAX_STR);
    return TRUE;
}

const char *dbus_message_get_destination(DBusMessage *message)
{
    if (!message || message->destination[0] == '\0')
        return NULL;
    return message->destination;
}

dbus_bool_t dbus_message_set_sender(DBusMessage *message, const char *sender)
{
    if (!message)
        return FALSE;
    safe_copy(message->sender, sender, MAX_STR);
    return TRUE;
}

const char *dbus_message_get_sender(DBusMessage *message)
{
    if (!message || message->sender[0] == '\0')
        return NULL;
    return message->sender;
}

const char *dbus_message_get_signature(DBusMessage *message)
{
    if (!message)
        return "";
    return message->signature;
}

void dbus_message_set_no_reply(DBusMessage *message, dbus_bool_t no_reply)
{
    if (message)
        message->no_reply = no_reply;
}

dbus_bool_t dbus_message_get_no_reply(DBusMessage *message)
{
    return message ? message->no_reply : FALSE;
}

void dbus_message_set_auto_start(DBusMessage *message, dbus_bool_t auto_start)
{
    if (message)
        message->auto_start = auto_start;
}

dbus_bool_t dbus_message_get_auto_start(DBusMessage *message)
{
    return message ? message->auto_start : FALSE;
}

dbus_bool_t dbus_message_set_reply_serial(DBusMessage *message,
                                          dbus_uint32_t serial)
{
    if (!message)
        return FALSE;
    message->reply_serial = serial;
    return TRUE;
}

dbus_uint32_t dbus_message_get_reply_serial(DBusMessage *message)
{
    return message ? message->reply_serial : 0;
}

dbus_uint32_t dbus_message_get_serial(DBusMessage *message)
{
    return message ? message->serial : 0;
}

dbus_bool_t dbus_message_is_method_call(DBusMessage *message,
                                        const char *iface,
                                        const char *method)
{
    if (!message || message->msg_type != DBUS_MESSAGE_TYPE_METHOD_CALL)
        return FALSE;
    if (iface && strcmp(message->interface, iface) != 0)
        return FALSE;
    if (method && strcmp(message->member, method) != 0)
        return FALSE;
    return TRUE;
}

dbus_bool_t dbus_message_is_signal(DBusMessage *message, const char *iface,
                                   const char *signal_name)
{
    if (!message || message->msg_type != DBUS_MESSAGE_TYPE_SIGNAL)
        return FALSE;
    if (iface && strcmp(message->interface, iface) != 0)
        return FALSE;
    if (signal_name && strcmp(message->member, signal_name) != 0)
        return FALSE;
    return TRUE;
}

dbus_bool_t dbus_message_is_error(DBusMessage *message,
                                  const char *error_name)
{
    if (!message || message->msg_type != DBUS_MESSAGE_TYPE_ERROR)
        return FALSE;
    if (error_name && strcmp(message->error_name, error_name) != 0)
        return FALSE;
    return TRUE;
}

dbus_bool_t dbus_message_has_destination(DBusMessage *message,
                                         const char *name)
{
    if (!message || !name)
        return FALSE;
    return strcmp(message->destination, name) == 0 ? TRUE : FALSE;
}

dbus_bool_t dbus_message_has_sender(DBusMessage *message, const char *name)
{
    if (!message || !name)
        return FALSE;
    return strcmp(message->sender, name) == 0 ? TRUE : FALSE;
}

dbus_bool_t dbus_message_has_signature(DBusMessage *message,
                                       const char *signature)
{
    if (!message || !signature)
        return FALSE;
    return strcmp(message->signature, signature) == 0 ? TRUE : FALSE;
}

/* ========================================================================= */
/* Message argument convenience                                              */
/* ========================================================================= */

dbus_bool_t dbus_message_get_args(DBusMessage *message, DBusError *error,
                                  int first_arg_type, ...)
{
    va_list ap;
    dbus_bool_t result;
    va_start(ap, first_arg_type);
    result = dbus_message_get_args_valist(message, error, first_arg_type, ap);
    va_end(ap);
    return result;
}

dbus_bool_t dbus_message_get_args_valist(DBusMessage *message,
                                         DBusError *error,
                                         int first_arg_type, va_list var_args)
{
    int idx = 0;
    int type = first_arg_type;

    if (!message) {
        dbus_set_error(error, DBUS_ERROR_INVALID_ARGS, "NULL message");
        return FALSE;
    }

    while (type != DBUS_TYPE_INVALID) {
        if (idx >= message->num_args)
            break;

        void *value = va_arg(var_args, void *);
        if (value && message->args[idx].type == type) {
            switch (type) {
            case DBUS_TYPE_BYTE:
                *(unsigned char *)value = message->args[idx].value.byte_val;
                break;
            case DBUS_TYPE_BOOLEAN:
                *(dbus_bool_t *)value = message->args[idx].value.bool_val;
                break;
            case DBUS_TYPE_INT16:
                *(dbus_int16_t *)value = message->args[idx].value.i16_val;
                break;
            case DBUS_TYPE_UINT16:
                *(dbus_uint16_t *)value = message->args[idx].value.u16_val;
                break;
            case DBUS_TYPE_INT32:
                *(dbus_int32_t *)value = message->args[idx].value.i32_val;
                break;
            case DBUS_TYPE_UINT32:
                *(dbus_uint32_t *)value = message->args[idx].value.u32_val;
                break;
            case DBUS_TYPE_INT64:
                *(dbus_int64_t *)value = message->args[idx].value.i64_val;
                break;
            case DBUS_TYPE_UINT64:
                *(dbus_uint64_t *)value = message->args[idx].value.u64_val;
                break;
            case DBUS_TYPE_DOUBLE:
                *(double *)value = message->args[idx].value.dbl_val;
                break;
            case DBUS_TYPE_STRING:
            case DBUS_TYPE_OBJECT_PATH:
            case DBUS_TYPE_SIGNATURE:
                *(const char **)value = message->args[idx].value.str_val;
                break;
            default:
                break;
            }
        }
        idx++;
        type = va_arg(var_args, int);
    }
    return TRUE;
}

dbus_bool_t dbus_message_append_args(DBusMessage *message,
                                     int first_arg_type, ...)
{
    va_list ap;
    dbus_bool_t result;
    va_start(ap, first_arg_type);
    result = dbus_message_append_args_valist(message, first_arg_type, ap);
    va_end(ap);
    return result;
}

dbus_bool_t dbus_message_append_args_valist(DBusMessage *message,
                                            int first_arg_type,
                                            va_list var_args)
{
    int type = first_arg_type;

    if (!message)
        return FALSE;

    while (type != DBUS_TYPE_INVALID) {
        if (message->num_args >= MAX_ARGS)
            return FALSE;

        int idx = message->num_args;
        message->args[idx].type = type;

        switch (type) {
        case DBUS_TYPE_BYTE: {
            /* Promoted to int in va_arg */
            unsigned char v = (unsigned char)va_arg(var_args, int);
            message->args[idx].value.byte_val = v;
            break;
        }
        case DBUS_TYPE_BOOLEAN: {
            dbus_bool_t v = (dbus_bool_t)va_arg(var_args, int);
            message->args[idx].value.bool_val = v;
            break;
        }
        case DBUS_TYPE_INT16: {
            dbus_int16_t v = (dbus_int16_t)va_arg(var_args, int);
            message->args[idx].value.i16_val = v;
            break;
        }
        case DBUS_TYPE_UINT16: {
            dbus_uint16_t v = (dbus_uint16_t)va_arg(var_args, int);
            message->args[idx].value.u16_val = v;
            break;
        }
        case DBUS_TYPE_INT32: {
            dbus_int32_t v = va_arg(var_args, dbus_int32_t);
            message->args[idx].value.i32_val = v;
            break;
        }
        case DBUS_TYPE_UINT32: {
            dbus_uint32_t v = va_arg(var_args, dbus_uint32_t);
            message->args[idx].value.u32_val = v;
            break;
        }
        case DBUS_TYPE_INT64: {
            dbus_int64_t v = va_arg(var_args, dbus_int64_t);
            message->args[idx].value.i64_val = v;
            break;
        }
        case DBUS_TYPE_UINT64: {
            dbus_uint64_t v = va_arg(var_args, dbus_uint64_t);
            message->args[idx].value.u64_val = v;
            break;
        }
        case DBUS_TYPE_DOUBLE: {
            double v = va_arg(var_args, double);
            message->args[idx].value.dbl_val = v;
            break;
        }
        case DBUS_TYPE_STRING:
        case DBUS_TYPE_OBJECT_PATH:
        case DBUS_TYPE_SIGNATURE: {
            const char *v = va_arg(var_args, const char *);
            safe_copy(message->args[idx].value.str_val, v, MAX_STR);
            break;
        }
        default:
            return FALSE;
        }
        message->num_args++;
        type = va_arg(var_args, int);
    }
    return TRUE;
}

/* ========================================================================= */
/* Message iterator                                                          */
/* ========================================================================= */

/*
 * Iterator internals:
 *   dummy4 = current argument index
 *   dummy5 = total args (or container element count)
 *   dummy1 = pointer to message (for read) or message (for append)
 *   dummy6 = container type (0=top-level, or DBUS_TYPE_ARRAY etc.)
 *   dummy7 = 1 if append mode, 0 if read mode
 */

dbus_bool_t dbus_message_iter_init(DBusMessage *message,
                                   DBusMessageIter *iter)
{
    if (!message || !iter)
        return FALSE;
    memset(iter, 0, sizeof(DBusMessageIter));
    iter->dummy1 = message;
    iter->dummy4 = 0;
    iter->dummy5 = message->num_args;
    iter->dummy6 = 0;
    iter->dummy7 = 0;
    return message->num_args > 0 ? TRUE : FALSE;
}

dbus_bool_t dbus_message_iter_has_next(DBusMessageIter *iter)
{
    if (!iter)
        return FALSE;
    return (iter->dummy4 + 1 < iter->dummy5) ? TRUE : FALSE;
}

dbus_bool_t dbus_message_iter_next(DBusMessageIter *iter)
{
    if (!iter)
        return FALSE;
    if (iter->dummy4 + 1 < iter->dummy5) {
        iter->dummy4++;
        return TRUE;
    }
    return FALSE;
}

int dbus_message_iter_get_arg_type(DBusMessageIter *iter)
{
    if (!iter || !iter->dummy1)
        return DBUS_TYPE_INVALID;
    struct DBusMessage *msg = (struct DBusMessage *)iter->dummy1;
    int idx = iter->dummy4;
    if (idx < 0 || idx >= msg->num_args)
        return DBUS_TYPE_INVALID;
    return msg->args[idx].type;
}

int dbus_message_iter_get_element_type(DBusMessageIter *iter)
{
    (void)iter;
    return DBUS_TYPE_INVALID;
}

void dbus_message_iter_recurse(DBusMessageIter *iter, DBusMessageIter *sub)
{
    if (!iter || !sub)
        return;
    memset(sub, 0, sizeof(DBusMessageIter));
    sub->dummy1 = iter->dummy1;
    sub->dummy4 = 0;
    sub->dummy5 = 0;
    sub->dummy6 = dbus_message_iter_get_arg_type(iter);
    sub->dummy7 = 0;
}

int dbus_message_iter_get_element_count(DBusMessageIter *iter)
{
    (void)iter;
    return 0;
}

void dbus_message_iter_get_basic(DBusMessageIter *iter, void *value)
{
    if (!iter || !value || !iter->dummy1)
        return;
    struct DBusMessage *msg = (struct DBusMessage *)iter->dummy1;
    int idx = iter->dummy4;
    if (idx < 0 || idx >= msg->num_args)
        return;

    struct dbus_arg *arg = &msg->args[idx];
    switch (arg->type) {
    case DBUS_TYPE_BYTE:
        *(unsigned char *)value = arg->value.byte_val;
        break;
    case DBUS_TYPE_BOOLEAN:
        *(dbus_bool_t *)value = arg->value.bool_val;
        break;
    case DBUS_TYPE_INT16:
        *(dbus_int16_t *)value = arg->value.i16_val;
        break;
    case DBUS_TYPE_UINT16:
        *(dbus_uint16_t *)value = arg->value.u16_val;
        break;
    case DBUS_TYPE_INT32:
        *(dbus_int32_t *)value = arg->value.i32_val;
        break;
    case DBUS_TYPE_UINT32:
        *(dbus_uint32_t *)value = arg->value.u32_val;
        break;
    case DBUS_TYPE_INT64:
        *(dbus_int64_t *)value = arg->value.i64_val;
        break;
    case DBUS_TYPE_UINT64:
        *(dbus_uint64_t *)value = arg->value.u64_val;
        break;
    case DBUS_TYPE_DOUBLE:
        *(double *)value = arg->value.dbl_val;
        break;
    case DBUS_TYPE_STRING:
    case DBUS_TYPE_OBJECT_PATH:
    case DBUS_TYPE_SIGNATURE:
        *(const char **)value = arg->value.str_val;
        break;
    default:
        break;
    }
}

void dbus_message_iter_get_fixed_array(DBusMessageIter *iter,
                                       void *value, int *n_elements)
{
    (void)iter;
    if (value)
        *(void **)value = NULL;
    if (n_elements)
        *n_elements = 0;
}

char *dbus_message_iter_get_signature(DBusMessageIter *iter)
{
    (void)iter;
    char *sig = (char *)dbus_malloc(2);
    if (sig) {
        sig[0] = '\0';
        sig[1] = '\0';
    }
    return sig;
}

void dbus_message_iter_init_append(DBusMessage *message,
                                   DBusMessageIter *iter)
{
    if (!message || !iter)
        return;
    memset(iter, 0, sizeof(DBusMessageIter));
    iter->dummy1 = message;
    iter->dummy4 = message->num_args;
    iter->dummy5 = message->num_args;
    iter->dummy6 = 0;
    iter->dummy7 = 1; /* append mode */
}

dbus_bool_t dbus_message_iter_append_basic(DBusMessageIter *iter, int type,
                                           const void *value)
{
    if (!iter || !value || !iter->dummy1)
        return FALSE;
    struct DBusMessage *msg = (struct DBusMessage *)iter->dummy1;
    if (msg->num_args >= MAX_ARGS)
        return FALSE;

    int idx = msg->num_args;
    msg->args[idx].type = type;

    switch (type) {
    case DBUS_TYPE_BYTE:
        msg->args[idx].value.byte_val = *(const unsigned char *)value;
        break;
    case DBUS_TYPE_BOOLEAN:
        msg->args[idx].value.bool_val = *(const dbus_bool_t *)value;
        break;
    case DBUS_TYPE_INT16:
        msg->args[idx].value.i16_val = *(const dbus_int16_t *)value;
        break;
    case DBUS_TYPE_UINT16:
        msg->args[idx].value.u16_val = *(const dbus_uint16_t *)value;
        break;
    case DBUS_TYPE_INT32:
        msg->args[idx].value.i32_val = *(const dbus_int32_t *)value;
        break;
    case DBUS_TYPE_UINT32:
        msg->args[idx].value.u32_val = *(const dbus_uint32_t *)value;
        break;
    case DBUS_TYPE_INT64:
        msg->args[idx].value.i64_val = *(const dbus_int64_t *)value;
        break;
    case DBUS_TYPE_UINT64:
        msg->args[idx].value.u64_val = *(const dbus_uint64_t *)value;
        break;
    case DBUS_TYPE_DOUBLE:
        msg->args[idx].value.dbl_val = *(const double *)value;
        break;
    case DBUS_TYPE_STRING:
    case DBUS_TYPE_OBJECT_PATH:
    case DBUS_TYPE_SIGNATURE:
        safe_copy(msg->args[idx].value.str_val,
                  *(const char *const *)value, MAX_STR);
        break;
    default:
        return FALSE;
    }
    msg->num_args++;
    iter->dummy4 = msg->num_args;
    iter->dummy5 = msg->num_args;
    return TRUE;
}

dbus_bool_t dbus_message_iter_append_fixed_array(DBusMessageIter *iter,
                                                 int element_type,
                                                 const void *value,
                                                 int n_elements)
{
    (void)iter;
    (void)element_type;
    (void)value;
    (void)n_elements;
    return TRUE;
}

dbus_bool_t dbus_message_iter_open_container(DBusMessageIter *iter, int type,
                                             const char *contained_signature,
                                             DBusMessageIter *sub)
{
    (void)contained_signature;
    if (!iter || !sub)
        return FALSE;
    memset(sub, 0, sizeof(DBusMessageIter));
    sub->dummy1 = iter->dummy1;
    sub->dummy6 = type;
    sub->dummy7 = 1;
    struct DBusMessage *msg = (struct DBusMessage *)iter->dummy1;
    if (msg) {
        sub->dummy4 = msg->num_args;
        sub->dummy5 = msg->num_args;
    }
    return TRUE;
}

dbus_bool_t dbus_message_iter_close_container(DBusMessageIter *iter,
                                              DBusMessageIter *sub)
{
    if (!iter || !sub)
        return FALSE;
    /* Sync parent iterator position */
    struct DBusMessage *msg = (struct DBusMessage *)iter->dummy1;
    if (msg) {
        iter->dummy4 = msg->num_args;
        iter->dummy5 = msg->num_args;
    }
    return TRUE;
}

void dbus_message_iter_abandon_container(DBusMessageIter *iter,
                                         DBusMessageIter *sub)
{
    (void)iter;
    (void)sub;
}

dbus_bool_t dbus_message_iter_abandon_container_if_open(
    DBusMessageIter *iter, DBusMessageIter *sub)
{
    (void)iter;
    (void)sub;
    return TRUE;
}

/* ========================================================================= */
/* Pending calls                                                             */
/* ========================================================================= */

DBusPendingCall *dbus_pending_call_ref(DBusPendingCall *pending)
{
    if (pending)
        pending->refcount++;
    return pending;
}

void dbus_pending_call_unref(DBusPendingCall *pending)
{
    if (!pending)
        return;
    pending->refcount--;
    if (pending->refcount <= 0) {
        if (pending->notify_free && pending->notify_data)
            pending->notify_free(pending->notify_data);
        pending->in_use = 0;
    }
}

dbus_bool_t dbus_pending_call_set_notify(DBusPendingCall *pending,
                                         DBusPendingCallNotifyFunction function,
                                         void *user_data,
                                         DBusFreeFunction free_user_data)
{
    if (!pending)
        return FALSE;
    pending->notify_fn = function;
    pending->notify_data = user_data;
    pending->notify_free = free_user_data;
    /* If already completed, fire immediately */
    if (pending->completed && function)
        function(pending, user_data);
    return TRUE;
}

void dbus_pending_call_cancel(DBusPendingCall *pending)
{
    if (pending)
        pending->completed = 1;
}

dbus_bool_t dbus_pending_call_get_completed(DBusPendingCall *pending)
{
    return (pending && pending->completed) ? TRUE : FALSE;
}

DBusMessage *dbus_pending_call_steal_reply(DBusPendingCall *pending)
{
    if (!pending || !pending->completed)
        return NULL;
    if (pending->reply_idx >= 0 && pending->reply_idx < MAX_MESSAGES) {
        struct DBusMessage *msg = &g_messages[pending->reply_idx];
        if (msg->in_use) {
            msg->refcount++;
            pending->reply_idx = -1;
            return msg;
        }
    }
    /* Return a generic empty reply */
    struct DBusMessage *reply = alloc_message();
    if (reply)
        reply->msg_type = DBUS_MESSAGE_TYPE_METHOD_RETURN;
    return reply;
}

void dbus_pending_call_block(DBusPendingCall *pending)
{
    if (pending)
        pending->completed = 1;
}

dbus_bool_t dbus_pending_call_set_data(DBusPendingCall *pending,
                                       dbus_int32_t slot,
                                       void *data,
                                       DBusFreeFunction free_data_func)
{
    (void)pending;
    (void)slot;
    (void)data;
    (void)free_data_func;
    return TRUE;
}

void *dbus_pending_call_get_data(DBusPendingCall *pending, dbus_int32_t slot)
{
    (void)pending;
    (void)slot;
    return NULL;
}

dbus_bool_t dbus_pending_call_allocate_data_slot(dbus_int32_t *slot_p)
{
    if (slot_p)
        *slot_p = g_next_data_slot++;
    return TRUE;
}

void dbus_pending_call_free_data_slot(dbus_int32_t *slot_p)
{
    (void)slot_p;
}

/* ========================================================================= */
/* Address parsing (stubs)                                                   */
/* ========================================================================= */

dbus_bool_t dbus_parse_address(const char *address,
                               DBusAddressEntry ***entry,
                               int *array_len, DBusError *error)
{
    (void)address;
    (void)error;
    if (entry)
        *entry = NULL;
    if (array_len)
        *array_len = 0;
    return TRUE;
}

const char *dbus_address_entry_get_value(DBusAddressEntry *entry,
                                         const char *key)
{
    (void)entry;
    (void)key;
    return NULL;
}

const char *dbus_address_entry_get_method(DBusAddressEntry *entry)
{
    (void)entry;
    return "unix";
}

void dbus_address_entries_free(DBusAddressEntry **entries)
{
    (void)entries;
}

char *dbus_address_escape_value(const char *value)
{
    if (!value)
        return NULL;
    size_t len = strlen(value);
    char *escaped = (char *)dbus_malloc(len + 1);
    if (escaped)
        memcpy(escaped, value, len + 1);
    return escaped;
}

char *dbus_address_unescape_value(const char *value, DBusError *error)
{
    (void)error;
    return dbus_address_escape_value(value);
}

/* ========================================================================= */
/* Signature validation                                                      */
/* ========================================================================= */

void dbus_signature_iter_init(DBusSignatureIter *iter, const char *signature)
{
    if (!iter)
        return;
    memset(iter, 0, sizeof(DBusSignatureIter));
    iter->dummy1 = (void *)signature;
    iter->dummy8 = 0; /* current position */
}

int dbus_signature_iter_get_current_type(const DBusSignatureIter *iter)
{
    if (!iter || !iter->dummy1)
        return DBUS_TYPE_INVALID;
    const char *sig = (const char *)iter->dummy1;
    unsigned int pos = iter->dummy8;
    if (sig[pos] == '\0')
        return DBUS_TYPE_INVALID;
    return (int)(unsigned char)sig[pos];
}

char *dbus_signature_iter_get_signature(const DBusSignatureIter *iter)
{
    if (!iter || !iter->dummy1)
        return NULL;
    const char *sig = (const char *)iter->dummy1;
    size_t len = strlen(sig);
    char *copy = (char *)dbus_malloc(len + 1);
    if (copy)
        memcpy(copy, sig, len + 1);
    return copy;
}

int dbus_signature_iter_get_element_type(const DBusSignatureIter *iter)
{
    if (!iter || !iter->dummy1)
        return DBUS_TYPE_INVALID;
    const char *sig = (const char *)iter->dummy1;
    unsigned int pos = iter->dummy8;
    if (sig[pos] == 'a' && sig[pos + 1] != '\0')
        return (int)(unsigned char)sig[pos + 1];
    return DBUS_TYPE_INVALID;
}

dbus_bool_t dbus_signature_iter_next(DBusSignatureIter *iter)
{
    if (!iter || !iter->dummy1)
        return FALSE;
    const char *sig = (const char *)iter->dummy1;
    unsigned int pos = iter->dummy8;
    if (sig[pos] == '\0')
        return FALSE;
    /* Skip current type character (simplified -- does not handle containers) */
    iter->dummy8 = pos + 1;
    return sig[pos + 1] != '\0' ? TRUE : FALSE;
}

void dbus_signature_iter_recurse(const DBusSignatureIter *iter,
                                 DBusSignatureIter *sub_iter)
{
    if (!iter || !sub_iter)
        return;
    memset(sub_iter, 0, sizeof(DBusSignatureIter));
    if (iter->dummy1) {
        const char *sig = (const char *)iter->dummy1;
        unsigned int pos = iter->dummy8;
        /* Point sub-iter at the contained signature */
        sub_iter->dummy1 = (void *)&sig[pos + 1];
        sub_iter->dummy8 = 0;
    }
}

dbus_bool_t dbus_signature_validate(const char *signature, DBusError *error)
{
    (void)error;
    if (!signature)
        return FALSE;
    if (strlen(signature) > DBUS_MAXIMUM_SIGNATURE_LENGTH)
        return FALSE;
    return TRUE;
}

dbus_bool_t dbus_signature_validate_single(const char *signature,
                                           DBusError *error)
{
    (void)error;
    if (!signature || signature[0] == '\0')
        return FALSE;
    return TRUE;
}

dbus_bool_t dbus_type_is_valid(int typecode)
{
    switch (typecode) {
    case DBUS_TYPE_BYTE:
    case DBUS_TYPE_BOOLEAN:
    case DBUS_TYPE_INT16:
    case DBUS_TYPE_UINT16:
    case DBUS_TYPE_INT32:
    case DBUS_TYPE_UINT32:
    case DBUS_TYPE_INT64:
    case DBUS_TYPE_UINT64:
    case DBUS_TYPE_DOUBLE:
    case DBUS_TYPE_STRING:
    case DBUS_TYPE_OBJECT_PATH:
    case DBUS_TYPE_SIGNATURE:
    case DBUS_TYPE_UNIX_FD:
    case DBUS_TYPE_ARRAY:
    case DBUS_TYPE_VARIANT:
    case DBUS_TYPE_STRUCT:
    case DBUS_TYPE_DICT_ENTRY:
        return TRUE;
    default:
        return FALSE;
    }
}

dbus_bool_t dbus_type_is_basic(int typecode)
{
    switch (typecode) {
    case DBUS_TYPE_BYTE:
    case DBUS_TYPE_BOOLEAN:
    case DBUS_TYPE_INT16:
    case DBUS_TYPE_UINT16:
    case DBUS_TYPE_INT32:
    case DBUS_TYPE_UINT32:
    case DBUS_TYPE_INT64:
    case DBUS_TYPE_UINT64:
    case DBUS_TYPE_DOUBLE:
    case DBUS_TYPE_STRING:
    case DBUS_TYPE_OBJECT_PATH:
    case DBUS_TYPE_SIGNATURE:
    case DBUS_TYPE_UNIX_FD:
        return TRUE;
    default:
        return FALSE;
    }
}

dbus_bool_t dbus_type_is_container(int typecode)
{
    switch (typecode) {
    case DBUS_TYPE_ARRAY:
    case DBUS_TYPE_VARIANT:
    case DBUS_TYPE_STRUCT:
    case DBUS_TYPE_DICT_ENTRY:
        return TRUE;
    default:
        return FALSE;
    }
}

dbus_bool_t dbus_type_is_fixed(int typecode)
{
    switch (typecode) {
    case DBUS_TYPE_BYTE:
    case DBUS_TYPE_BOOLEAN:
    case DBUS_TYPE_INT16:
    case DBUS_TYPE_UINT16:
    case DBUS_TYPE_INT32:
    case DBUS_TYPE_UINT32:
    case DBUS_TYPE_INT64:
    case DBUS_TYPE_UINT64:
    case DBUS_TYPE_DOUBLE:
    case DBUS_TYPE_UNIX_FD:
        return TRUE;
    default:
        return FALSE;
    }
}
