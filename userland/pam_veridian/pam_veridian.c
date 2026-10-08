/*
 * pam_veridian -- PAM authentication against the VeridianOS account store.
 *
 * Passwords live in the kernel (PBKDF2, security::auth); there is no
 * /etc/shadow. This module asks the kernel through veridian_auth
 * (kernel/src/syscall/veridian_auth.rs):
 *
 *   auth      the password the conversation supplies (PAM_AUTHTOK)
 *   account   whether the account is usable (not locked or expired)
 *   password  change it (PAM_OLDAUTHTOK -> PAM_AUTHTOK); root may set it
 *             without the old one
 *
 * The kernel lets root act on any account and anyone else only on its own,
 * so the screen locker (running as the user) can check the user's password
 * and a setuid helper (polkit-agent-helper-1) can check an administrator's.
 * Failed attempts lock an account for a while, in the kernel.
 *
 * Built by tools/cross/build-deps.sh (after Linux-PAM), with SYS_* from the
 * generated <veridian/sysno.h>.
 */

#define PAM_SM_AUTH
#define PAM_SM_ACCOUNT
#define PAM_SM_PASSWORD

#include <errno.h>
#include <security/pam_ext.h>
#include <security/pam_modules.h>
#include <stddef.h>
#include <syslog.h>
#include <unistd.h>

#ifndef SYS_VERIDIAN_AUTH
#error "build with -include userland/libc/include/veridian/sysno.h"
#endif

/* Operations and results (kernel/src/syscall/veridian_auth.rs). */
enum { AUTH_CHECK = 0, AUTH_ACCOUNT = 1, AUTH_CHANGE = 2 };
enum {
    AUTH_OK = 0,
    AUTH_DENIED = 1,
    AUTH_LOCKED = 2,
    AUTH_EXPIRED = 3,
    AUTH_NEW_PASSWORD = 4,
    AUTH_MFA_REQUIRED = 5,
};

static long veridian_auth(long op, const char *name, const char *secret, const char *new_secret)
{
    return syscall(SYS_VERIDIAN_AUTH, op, name, secret, new_secret);
}

/* The PAM code for a failed veridian_auth call. */
static int errno_to_pam(pam_handle_t *pamh, const char *what)
{
    switch (errno) {
    case EPERM:
        return PAM_PERM_DENIED;
    case ENOSYS:
        pam_syslog(pamh, LOG_ERR, "%s: the kernel has no veridian_auth", what);
        return PAM_AUTHINFO_UNAVAIL;
    case ENAMETOOLONG:
        return PAM_USER_UNKNOWN;
    default:
        pam_syslog(pamh, LOG_ERR, "%s: veridian_auth failed: errno %d", what, errno);
        return PAM_SYSTEM_ERR;
    }
}

PAM_EXTERN int pam_sm_authenticate(pam_handle_t *pamh, int flags, int argc, const char **argv)
{
    (void)flags;
    (void)argc;
    (void)argv;
    const char *user = NULL, *password = NULL;
    int ret = pam_get_user(pamh, &user, NULL);
    if (ret != PAM_SUCCESS)
        return ret;
    ret = pam_get_authtok(pamh, PAM_AUTHTOK, &password, NULL);
    if (ret != PAM_SUCCESS)
        return ret;

    long r = veridian_auth(AUTH_CHECK, user, password, NULL);
    if (r < 0)
        return errno_to_pam(pamh, "auth");
    switch (r) {
    case AUTH_OK:
    case AUTH_NEW_PASSWORD: /* right password; account management asks for a new one */
        return PAM_SUCCESS;
    case AUTH_LOCKED:
        pam_syslog(pamh, LOG_NOTICE, "account %s is locked", user);
        return PAM_MAXTRIES;
    case AUTH_EXPIRED:
        return PAM_ACCT_EXPIRED;
    case AUTH_MFA_REQUIRED:
        pam_syslog(pamh, LOG_NOTICE, "account %s needs a second factor this module cannot check",
                   user);
        return PAM_AUTH_ERR;
    default:
        return PAM_AUTH_ERR;
    }
}

PAM_EXTERN int pam_sm_setcred(pam_handle_t *pamh, int flags, int argc, const char **argv)
{
    (void)pamh;
    (void)flags;
    (void)argc;
    (void)argv;
    return PAM_SUCCESS;
}

PAM_EXTERN int pam_sm_acct_mgmt(pam_handle_t *pamh, int flags, int argc, const char **argv)
{
    (void)flags;
    (void)argc;
    (void)argv;
    const char *user = NULL;
    int ret = pam_get_user(pamh, &user, NULL);
    if (ret != PAM_SUCCESS)
        return ret;

    long r = veridian_auth(AUTH_ACCOUNT, user, NULL, NULL);
    if (r < 0)
        return errno_to_pam(pamh, "account");
    switch (r) {
    case AUTH_OK:
        return PAM_SUCCESS;
    case AUTH_LOCKED:
        return PAM_PERM_DENIED;
    case AUTH_EXPIRED:
        return PAM_ACCT_EXPIRED;
    case AUTH_NEW_PASSWORD:
        return PAM_NEW_AUTHTOK_REQD;
    default:
        return PAM_USER_UNKNOWN;
    }
}

PAM_EXTERN int pam_sm_chauthtok(pam_handle_t *pamh, int flags, int argc, const char **argv)
{
    (void)argc;
    (void)argv;
    const char *user = NULL, *old = NULL, *new_password = NULL;
    int ret = pam_get_user(pamh, &user, NULL);
    if (ret != PAM_SUCCESS)
        return ret;
    /* Root changes a password without the old one, as pam_unix lets it. */
    int need_old = getuid() != 0;

    if (flags & PAM_PRELIM_CHECK) {
        if (!need_old)
            return PAM_SUCCESS;
        ret = pam_get_authtok(pamh, PAM_OLDAUTHTOK, &old, NULL);
        if (ret != PAM_SUCCESS)
            return ret;
        long r = veridian_auth(AUTH_CHECK, user, old, NULL);
        if (r < 0)
            return errno_to_pam(pamh, "password");
        return r == AUTH_OK || r == AUTH_NEW_PASSWORD ? PAM_SUCCESS : PAM_AUTH_ERR;
    }
    if (!(flags & PAM_UPDATE_AUTHTOK))
        return PAM_SERVICE_ERR;

    if (need_old) {
        ret = pam_get_item(pamh, PAM_OLDAUTHTOK, (const void **)&old);
        if (ret != PAM_SUCCESS || old == NULL)
            return PAM_AUTHTOK_ERR;
    }
    ret = pam_get_authtok(pamh, PAM_AUTHTOK, &new_password, NULL);
    if (ret != PAM_SUCCESS)
        return ret;

    if (veridian_auth(AUTH_CHANGE, user, need_old ? old : NULL, new_password) < 0) {
        switch (errno) {
        case EACCES:
            return PAM_AUTH_ERR;
        case EINVAL:
            pam_error(pamh, "The new password does not meet the password policy.");
            return PAM_AUTHTOK_ERR;
        default:
            return errno_to_pam(pamh, "password");
        }
    }
    return PAM_SUCCESS;
}
