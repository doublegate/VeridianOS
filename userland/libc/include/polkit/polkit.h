/*
 * VeridianOS libc -- <polkit/polkit.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PolicyKit (Polkit) authorization API.
 * Provides the API surface for KDE's authorization checks (KAuth,
 * Plasma system settings, package management).  Default policy is
 * permissive (always authorized) for initial porting.
 */

#ifndef _POLKIT_POLKIT_H
#define _POLKIT_POLKIT_H

#ifdef __cplusplus
extern "C" {
#endif

#include <sys/types.h>

/* ---- Opaque types ---- */

typedef struct _PolkitAuthority            PolkitAuthority;
typedef struct _PolkitAuthorizationResult  PolkitAuthorizationResult;
typedef struct _PolkitSubject              PolkitSubject;
typedef struct _PolkitDetails              PolkitDetails;
typedef struct _PolkitIdentity             PolkitIdentity;
typedef struct _PolkitPermission           PolkitPermission;

/* GObject-like type compatibility (GType = unsigned long) */
typedef unsigned long GType;
typedef void *GCancellable;
typedef void *GAsyncReadyCallback;
typedef void *GAsyncResult;
typedef struct _GError {
    unsigned int domain;
    int          code;
    char        *message;
} GError;

/* ---- Flags and enums ---- */

typedef enum {
    POLKIT_CHECK_AUTHORIZATION_FLAGS_NONE                        = 0,
    POLKIT_CHECK_AUTHORIZATION_FLAGS_ALLOW_USER_INTERACTION      = (1 << 0)
} PolkitCheckAuthorizationFlags;

typedef enum {
    POLKIT_AUTHORITY_FEATURES_NONE                    = 0,
    POLKIT_AUTHORITY_FEATURES_TEMPORARY_AUTHORIZATION = (1 << 0)
} PolkitAuthorityFeatures;

typedef enum {
    POLKIT_IMPLICIT_AUTHORIZATION_UNKNOWN              = -1,
    POLKIT_IMPLICIT_AUTHORIZATION_NOT_AUTHORIZED       = 0,
    POLKIT_IMPLICIT_AUTHORIZATION_AUTHENTICATION_REQUIRED = 1,
    POLKIT_IMPLICIT_AUTHORIZATION_ADMINISTRATOR_AUTHENTICATION_REQUIRED = 2,
    POLKIT_IMPLICIT_AUTHORIZATION_AUTHENTICATION_REQUIRED_RETAINED = 3,
    POLKIT_IMPLICIT_AUTHORIZATION_AUTHORIZED           = 4
} PolkitImplicitAuthorization;

/* ---- Authority ---- */

PolkitAuthority *polkit_authority_get_sync(GCancellable *cancellable,
                                           GError **error);
PolkitAuthority *polkit_authority_get(void);
void             polkit_authority_free(PolkitAuthority *authority);

PolkitAuthorizationResult *polkit_authority_check_authorization_sync(
    PolkitAuthority *authority,
    PolkitSubject *subject,
    const char *action_id,
    PolkitDetails *details,
    PolkitCheckAuthorizationFlags flags,
    GCancellable *cancellable,
    GError **error);

void polkit_authority_check_authorization(
    PolkitAuthority *authority,
    PolkitSubject *subject,
    const char *action_id,
    PolkitDetails *details,
    PolkitCheckAuthorizationFlags flags,
    GCancellable *cancellable,
    GAsyncReadyCallback callback,
    void *user_data);

PolkitAuthorizationResult *polkit_authority_check_authorization_finish(
    PolkitAuthority *authority,
    GAsyncResult *res,
    GError **error);

PolkitAuthorityFeatures polkit_authority_get_backend_features(
    PolkitAuthority *authority);

const char *polkit_authority_get_backend_name(PolkitAuthority *authority);
const char *polkit_authority_get_backend_version(PolkitAuthority *authority);

/* ---- Authorization result ---- */

int  polkit_authorization_result_get_is_authorized(
         PolkitAuthorizationResult *result);
int  polkit_authorization_result_get_is_challenge(
         PolkitAuthorizationResult *result);
int  polkit_authorization_result_get_retains_authorization(
         PolkitAuthorizationResult *result);
const char *polkit_authorization_result_get_temporary_authorization_id(
                PolkitAuthorizationResult *result);
PolkitDetails *polkit_authorization_result_get_details(
                   PolkitAuthorizationResult *result);
void polkit_authorization_result_free(PolkitAuthorizationResult *result);

/* ---- Subject constructors ---- */

PolkitSubject *polkit_system_bus_name_new(const char *name);
PolkitSubject *polkit_unix_process_new(pid_t pid);
PolkitSubject *polkit_unix_process_new_full(pid_t pid, unsigned long long start_time);
PolkitSubject *polkit_unix_process_new_for_owner(pid_t pid,
                                                  unsigned long long start_time,
                                                  uid_t uid);
PolkitSubject *polkit_unix_session_new(const char *session_id);
PolkitSubject *polkit_unix_session_new_for_process_sync(
                   pid_t pid, GCancellable *cancellable, GError **error);

/* Subject queries */
const char *polkit_system_bus_name_get_name(PolkitSubject *subject);
pid_t       polkit_unix_process_get_pid(PolkitSubject *subject);
uid_t       polkit_unix_process_get_uid(PolkitSubject *subject);

void polkit_subject_free(PolkitSubject *subject);

/* ---- Details ---- */

PolkitDetails *polkit_details_new(void);
void           polkit_details_free(PolkitDetails *details);
const char    *polkit_details_lookup(PolkitDetails *details, const char *key);
void           polkit_details_insert(PolkitDetails *details, const char *key,
                                     const char *value);

/* ---- Permission ---- */

PolkitPermission *polkit_permission_new_sync(const char *action_id,
                                              PolkitSubject *subject,
                                              GCancellable *cancellable,
                                              GError **error);
int  polkit_permission_get_allowed(PolkitPermission *permission);
int  polkit_permission_get_can_acquire(PolkitPermission *permission);
int  polkit_permission_get_can_release(PolkitPermission *permission);
void polkit_permission_free(PolkitPermission *permission);

/* ---- GType registration (for GObject introspection compat) ---- */

GType polkit_authority_get_type(void);
GType polkit_authorization_result_get_type(void);
GType polkit_subject_get_type(void);
GType polkit_details_get_type(void);

#ifdef __cplusplus
}
#endif

#endif /* _POLKIT_POLKIT_H */
