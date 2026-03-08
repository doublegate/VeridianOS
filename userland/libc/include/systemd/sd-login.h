/*
 * VeridianOS libc -- <systemd/sd-login.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * systemd-logind session/seat/user query API.
 * Provides the API surface that KWin, SDDM, and other desktop components
 * use for session management, seat enumeration, and VT switching.
 */

#ifndef _SYSTEMD_SD_LOGIN_H
#define _SYSTEMD_SD_LOGIN_H

#ifdef __cplusplus
extern "C" {
#endif

#include <sys/types.h>

/* ---- Monitor ---- */

typedef struct sd_login_monitor sd_login_monitor;

/* ---- Session queries ---- */

int sd_pid_get_session(pid_t pid, char **session);
int sd_pid_get_unit(pid_t pid, char **unit);
int sd_pid_get_user_unit(pid_t pid, char **unit);
int sd_pid_get_owner_uid(pid_t pid, uid_t *uid);
int sd_pid_get_machine_name(pid_t pid, char **machine);
int sd_pid_get_slice(pid_t pid, char **slice);
int sd_pid_get_user_slice(pid_t pid, char **slice);
int sd_pid_get_cgroup(pid_t pid, char **cgroup);

int sd_session_get_seat(const char *session, char **seat);
int sd_session_get_type(const char *session, char **type);
int sd_session_get_class(const char *session, char **class);
int sd_session_get_state(const char *session, char **state);
int sd_session_get_display(const char *session, char **display);
int sd_session_get_tty(const char *session, char **tty);
int sd_session_get_vt(const char *session, unsigned *vt);
int sd_session_get_service(const char *session, char **service);
int sd_session_get_desktop(const char *session, char **desktop);
int sd_session_get_remote_host(const char *session, char **remote_host);
int sd_session_get_remote_user(const char *session, char **remote_user);
int sd_session_get_leader(const char *session, pid_t *leader);
int sd_session_is_active(const char *session);
int sd_session_is_remote(const char *session);

/* ---- Seat queries ---- */

int sd_seat_get_active(const char *seat, char **session, uid_t *uid);
int sd_seat_get_sessions(const char *seat, char ***sessions,
                         uid_t **uid, unsigned *n_uids);
int sd_seat_can_multi_session(const char *seat);
int sd_seat_can_tty(const char *seat);
int sd_seat_can_graphical(const char *seat);

/* ---- Enumeration ---- */

int sd_get_sessions(char ***sessions);
int sd_get_seats(char ***seats);
int sd_get_uids(uid_t **uids);
int sd_get_machine_names(char ***machines);

/* ---- User queries ---- */

int sd_uid_get_state(uid_t uid, char **state);
int sd_uid_get_display(uid_t uid, char **session);
int sd_uid_get_sessions(uid_t uid, int require_active,
                        char ***sessions);
int sd_uid_get_seats(uid_t uid, int require_active, char ***seats);
int sd_uid_is_on_seat(uid_t uid, int require_active, const char *seat);

/* ---- Monitor ---- */

int sd_login_monitor_new(const char *category, sd_login_monitor **ret);
sd_login_monitor *sd_login_monitor_unref(sd_login_monitor *m);
int sd_login_monitor_flush(sd_login_monitor *m);
int sd_login_monitor_get_fd(sd_login_monitor *m);
int sd_login_monitor_get_events(sd_login_monitor *m);
int sd_login_monitor_get_timeout(sd_login_monitor *m, unsigned long long *timeout_usec);

#ifdef __cplusplus
}
#endif

#endif /* _SYSTEMD_SD_LOGIN_H */
