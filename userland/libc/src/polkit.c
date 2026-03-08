/*
 * VeridianOS libc -- polkit.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PolicyKit (Polkit) authorization API for VeridianOS.
 * Default policy is permissive (always authorized) for initial porting.
 * Provides the API surface for KDE's KAuth framework, Plasma system
 * settings, and package management authorization checks.
 */

#include <polkit/polkit.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

#define MAX_DETAILS_ENTRIES 16
#define MAX_DETAIL_KEY      64
#define MAX_DETAIL_VALUE   256

struct _PolkitAuthority {
    int initialized;
};

struct _PolkitAuthorizationResult {
    int is_authorized;
    int is_challenge;
    int retains_authorization;
};

typedef enum {
    POLKIT_SUBJECT_TYPE_SYSTEM_BUS_NAME,
    POLKIT_SUBJECT_TYPE_UNIX_PROCESS,
    POLKIT_SUBJECT_TYPE_UNIX_SESSION
} PolkitSubjectType;

struct _PolkitSubject {
    PolkitSubjectType type;
    char              bus_name[MAX_DETAIL_VALUE];
    pid_t             pid;
    uid_t             uid;
    unsigned long long start_time;
    char              session_id[MAX_DETAIL_KEY];
};

struct detail_entry {
    char key[MAX_DETAIL_KEY];
    char value[MAX_DETAIL_VALUE];
    int  in_use;
};

struct _PolkitDetails {
    struct detail_entry entries[MAX_DETAILS_ENTRIES];
    int count;
};

struct _PolkitPermission {
    int allowed;
    int can_acquire;
    int can_release;
};

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static struct _PolkitAuthority g_authority = { .initialized = 1 };

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static void polkit_safe_copy(char *dst, const char *src, int max)
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
/* Authority                                                                 */
/* ========================================================================= */

PolkitAuthority *polkit_authority_get_sync(GCancellable *cancellable,
                                           GError **error)
{
    (void)cancellable;
    (void)error;
    return &g_authority;
}

PolkitAuthority *polkit_authority_get(void)
{
    return &g_authority;
}

void polkit_authority_free(PolkitAuthority *authority)
{
    (void)authority;
    /* Static singleton -- nothing to free */
}

PolkitAuthorizationResult *polkit_authority_check_authorization_sync(
    PolkitAuthority *authority, PolkitSubject *subject,
    const char *action_id, PolkitDetails *details,
    PolkitCheckAuthorizationFlags flags,
    GCancellable *cancellable, GError **error)
{
    (void)authority;
    (void)subject;
    (void)action_id;
    (void)details;
    (void)flags;
    (void)cancellable;
    (void)error;

    PolkitAuthorizationResult *result =
        (PolkitAuthorizationResult *)malloc(sizeof(PolkitAuthorizationResult));
    if (result) {
        result->is_authorized = 1;       /* permissive */
        result->is_challenge = 0;
        result->retains_authorization = 0;
    }
    return result;
}

void polkit_authority_check_authorization(
    PolkitAuthority *authority, PolkitSubject *subject,
    const char *action_id, PolkitDetails *details,
    PolkitCheckAuthorizationFlags flags,
    GCancellable *cancellable,
    GAsyncReadyCallback callback, void *user_data)
{
    (void)authority;
    (void)subject;
    (void)action_id;
    (void)details;
    (void)flags;
    (void)cancellable;
    (void)callback;
    (void)user_data;
    /* Async stub -- callback never fired in shim */
}

PolkitAuthorizationResult *polkit_authority_check_authorization_finish(
    PolkitAuthority *authority, GAsyncResult *res, GError **error)
{
    (void)authority;
    (void)res;
    (void)error;

    PolkitAuthorizationResult *result =
        (PolkitAuthorizationResult *)malloc(sizeof(PolkitAuthorizationResult));
    if (result) {
        result->is_authorized = 1;
        result->is_challenge = 0;
        result->retains_authorization = 0;
    }
    return result;
}

PolkitAuthorityFeatures polkit_authority_get_backend_features(
    PolkitAuthority *authority)
{
    (void)authority;
    return POLKIT_AUTHORITY_FEATURES_TEMPORARY_AUTHORIZATION;
}

const char *polkit_authority_get_backend_name(PolkitAuthority *authority)
{
    (void)authority;
    return "veridian-polkit";
}

const char *polkit_authority_get_backend_version(PolkitAuthority *authority)
{
    (void)authority;
    return "124.0";
}

/* ========================================================================= */
/* Authorization result                                                      */
/* ========================================================================= */

int polkit_authorization_result_get_is_authorized(
    PolkitAuthorizationResult *result)
{
    return result ? result->is_authorized : 0;
}

int polkit_authorization_result_get_is_challenge(
    PolkitAuthorizationResult *result)
{
    return result ? result->is_challenge : 0;
}

int polkit_authorization_result_get_retains_authorization(
    PolkitAuthorizationResult *result)
{
    return result ? result->retains_authorization : 0;
}

const char *polkit_authorization_result_get_temporary_authorization_id(
    PolkitAuthorizationResult *result)
{
    (void)result;
    return NULL;
}

PolkitDetails *polkit_authorization_result_get_details(
    PolkitAuthorizationResult *result)
{
    (void)result;
    return NULL;
}

void polkit_authorization_result_free(PolkitAuthorizationResult *result)
{
    free(result);
}

/* ========================================================================= */
/* Subject constructors                                                      */
/* ========================================================================= */

PolkitSubject *polkit_system_bus_name_new(const char *name)
{
    PolkitSubject *s = (PolkitSubject *)calloc(1, sizeof(PolkitSubject));
    if (s) {
        s->type = POLKIT_SUBJECT_TYPE_SYSTEM_BUS_NAME;
        polkit_safe_copy(s->bus_name, name, MAX_DETAIL_VALUE);
    }
    return s;
}

PolkitSubject *polkit_unix_process_new(pid_t pid)
{
    PolkitSubject *s = (PolkitSubject *)calloc(1, sizeof(PolkitSubject));
    if (s) {
        s->type = POLKIT_SUBJECT_TYPE_UNIX_PROCESS;
        s->pid = pid;
    }
    return s;
}

PolkitSubject *polkit_unix_process_new_full(pid_t pid,
                                            unsigned long long start_time)
{
    PolkitSubject *s = polkit_unix_process_new(pid);
    if (s)
        s->start_time = start_time;
    return s;
}

PolkitSubject *polkit_unix_process_new_for_owner(pid_t pid,
                                                  unsigned long long start_time,
                                                  uid_t uid)
{
    PolkitSubject *s = polkit_unix_process_new_full(pid, start_time);
    if (s)
        s->uid = uid;
    return s;
}

PolkitSubject *polkit_unix_session_new(const char *session_id)
{
    PolkitSubject *s = (PolkitSubject *)calloc(1, sizeof(PolkitSubject));
    if (s) {
        s->type = POLKIT_SUBJECT_TYPE_UNIX_SESSION;
        polkit_safe_copy(s->session_id, session_id, MAX_DETAIL_KEY);
    }
    return s;
}

PolkitSubject *polkit_unix_session_new_for_process_sync(
    pid_t pid, GCancellable *cancellable, GError **error)
{
    (void)cancellable;
    (void)error;
    PolkitSubject *s = polkit_unix_session_new("1");
    if (s)
        s->pid = pid;
    return s;
}

const char *polkit_system_bus_name_get_name(PolkitSubject *subject)
{
    if (!subject || subject->type != POLKIT_SUBJECT_TYPE_SYSTEM_BUS_NAME)
        return NULL;
    return subject->bus_name;
}

pid_t polkit_unix_process_get_pid(PolkitSubject *subject)
{
    return subject ? subject->pid : 0;
}

uid_t polkit_unix_process_get_uid(PolkitSubject *subject)
{
    return subject ? subject->uid : 0;
}

void polkit_subject_free(PolkitSubject *subject)
{
    free(subject);
}

/* ========================================================================= */
/* Details                                                                   */
/* ========================================================================= */

PolkitDetails *polkit_details_new(void)
{
    return (PolkitDetails *)calloc(1, sizeof(PolkitDetails));
}

void polkit_details_free(PolkitDetails *details)
{
    free(details);
}

const char *polkit_details_lookup(PolkitDetails *details, const char *key)
{
    int i;
    if (!details || !key)
        return NULL;
    for (i = 0; i < details->count; i++) {
        if (details->entries[i].in_use &&
            strcmp(details->entries[i].key, key) == 0)
            return details->entries[i].value;
    }
    return NULL;
}

void polkit_details_insert(PolkitDetails *details, const char *key,
                           const char *value)
{
    if (!details || !key || details->count >= MAX_DETAILS_ENTRIES)
        return;
    int idx = details->count;
    polkit_safe_copy(details->entries[idx].key, key, MAX_DETAIL_KEY);
    polkit_safe_copy(details->entries[idx].value, value, MAX_DETAIL_VALUE);
    details->entries[idx].in_use = 1;
    details->count++;
}

/* ========================================================================= */
/* Permission                                                                */
/* ========================================================================= */

PolkitPermission *polkit_permission_new_sync(const char *action_id,
                                              PolkitSubject *subject,
                                              GCancellable *cancellable,
                                              GError **error)
{
    (void)action_id;
    (void)subject;
    (void)cancellable;
    (void)error;
    PolkitPermission *p =
        (PolkitPermission *)calloc(1, sizeof(PolkitPermission));
    if (p) {
        p->allowed = 1;
        p->can_acquire = 1;
        p->can_release = 1;
    }
    return p;
}

int polkit_permission_get_allowed(PolkitPermission *permission)
{
    return permission ? permission->allowed : 0;
}

int polkit_permission_get_can_acquire(PolkitPermission *permission)
{
    return permission ? permission->can_acquire : 0;
}

int polkit_permission_get_can_release(PolkitPermission *permission)
{
    return permission ? permission->can_release : 0;
}

void polkit_permission_free(PolkitPermission *permission)
{
    free(permission);
}

/* ========================================================================= */
/* GType stubs (for GObject introspection compatibility)                     */
/* ========================================================================= */

GType polkit_authority_get_type(void)
{
    return 0;
}

GType polkit_authorization_result_get_type(void)
{
    return 0;
}

GType polkit_subject_get_type(void)
{
    return 0;
}

GType polkit_details_get_type(void)
{
    return 0;
}
