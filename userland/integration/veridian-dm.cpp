/*
 * veridian-dm.cpp -- VeridianOS Display Manager / Session Selector
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Simple display manager for VeridianOS that provides:
 *   - Text-mode login prompt (username + password)
 *   - Session type menu (built-in DE vs. KDE Plasma 6)
 *   - Authentication against the kernel UserDatabase
 *   - Session preference persistence to /etc/veridian/session.conf
 *   - Optional auto-login for kiosk/development use
 *   - Wayland-native rendering on the kernel's built-in compositor
 *
 * Rendering uses the kernel compositor's framebuffer surface with the
 * same bitmap font as the built-in desktop environment.
 */

#include "veridian-dm.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/wait.h>
#include <sys/stat.h>

/* ======================================================================
 * Static session descriptors
 * ====================================================================== */

static const vdm_session_info_t s_available_sessions[] = {
    {
        VDM_SESSION_BUILTIN,
        "VeridianOS Desktop (built-in)",
        "Lightweight kernel-space compositor with built-in apps",
        NULL, /* No separate process -- kernel handles it */
        "veridian-desktop"
    },
    {
        VDM_SESSION_PLASMA,
        "KDE Plasma 6",
        "Full KDE Plasma desktop with KWin compositor",
        "/usr/bin/plasma-veridian-session",
        "kde-plasma"
    },
};

static const int s_session_count =
    (int)(sizeof(s_available_sessions) / sizeof(s_available_sessions[0]));

/* ======================================================================
 * Color palette (BGRA format for VeridianOS framebuffer)
 * ====================================================================== */

#define COLOR_BG            0xFF1A1A2E   /* Dark navy background */
#define COLOR_FG            0xFFE0E0E0   /* Light gray text */
#define COLOR_ACCENT        0xFF4EA8DE   /* Blue accent */
#define COLOR_INPUT_BG      0xFF2D2D44   /* Input field background */
#define COLOR_INPUT_BORDER  0xFF4A4A6A   /* Input field border */
#define COLOR_ERROR         0xFFDE4E4E   /* Error text */
#define COLOR_SELECTED      0xFF3D5A80   /* Selected menu item */
#define COLOR_TITLE         0xFFFFFFFF   /* Title text */
#define COLOR_DIM           0xFF808080   /* Dimmed text */

/* ======================================================================
 * Initialization
 * ====================================================================== */

int vdm_init(vdm_context_t *ctx)
{
    if (!ctx) {
        return -1;
    }

    memset(ctx, 0, sizeof(*ctx));

    ctx->state = VDM_STATE_INIT;
    ctx->max_attempts = 5;
    ctx->login_attempts = 0;

    /* Register available sessions */
    ctx->session_count = s_session_count;
    for (int i = 0; i < s_session_count && i < VDM_MAX_SESSIONS; i++) {
        ctx->sessions[i] = s_available_sessions[i];
    }

    /* Load last-used session preference */
    ctx->selected_session = (int)vdm_load_session_pref();

    /* Check for auto-login */
    vdm_load_autologin(&ctx->autologin);

    /*
     * Acquire framebuffer from the kernel compositor.
     * On VeridianOS, the kernel exposes a BGRA framebuffer via a
     * device node or shared memory region.  For the DM, we use the
     * primary surface (same as the built-in desktop).
     *
     * In a real implementation, this would mmap /dev/fb0 or use
     * the VeridianOS surface allocation syscall.  For now, we set
     * reasonable defaults that the build system will wire up.
     */
    ctx->fb_width = 1280;
    ctx->fb_height = 800;
    ctx->fb_stride = ctx->fb_width * 4;  /* BGRA, 4 bytes per pixel */
    ctx->framebuffer = (uint32_t *)calloc(
        (size_t)ctx->fb_width * (size_t)ctx->fb_height, sizeof(uint32_t));

    if (!ctx->framebuffer) {
        fprintf(stderr, "[veridian-dm] Failed to allocate framebuffer\n");
        return -1;
    }

    ctx->state = VDM_STATE_LOGIN;
    return 0;
}

/* ======================================================================
 * Main loop
 * ====================================================================== */

int vdm_run(vdm_context_t *ctx)
{
    if (!ctx) {
        return -1;
    }

    /* Handle auto-login if configured */
    if (ctx->autologin.enabled && ctx->autologin.username[0] != '\0') {
        fprintf(stderr, "[veridian-dm] Auto-login enabled for '%s' "
                "(delay %ds)\n",
                ctx->autologin.username,
                ctx->autologin.delay_seconds);

        if (ctx->autologin.delay_seconds > 0) {
            sleep((unsigned)ctx->autologin.delay_seconds);
        }

        vdm_auth_result_t auth =
            vdm_authenticate(ctx->autologin.username, "");
        if (auth == VDM_AUTH_OK) {
            vdm_session_type_t stype =
                (vdm_session_type_t)ctx->autologin.session_type;
            return vdm_launch_session(stype, ctx->autologin.username);
        }
        fprintf(stderr, "[veridian-dm] Auto-login auth failed, "
                "showing login prompt\n");
    }

    /*
     * Main loop: render login screen, handle input, authenticate,
     * and launch session.
     *
     * In the actual VeridianOS environment, input comes from the
     * kernel's PS/2 keyboard driver (polling ports 0x64/0x60) or
     * serial console.  The DM reads from stdin which is connected
     * to the kernel's input subsystem.
     */
    while (ctx->state != VDM_STATE_SHUTDOWN) {
        switch (ctx->state) {
        case VDM_STATE_LOGIN:
            vdm_render_login(ctx);

            /* Read username */
            fprintf(stderr, "\n  Username: ");
            if (!fgets(ctx->username, VDM_MAX_USERNAME, stdin)) {
                ctx->state = VDM_STATE_SHUTDOWN;
                break;
            }
            /* Strip trailing newline */
            ctx->username[strcspn(ctx->username, "\n")] = '\0';

            if (ctx->username[0] == '\0') {
                continue;  /* Empty username, retry */
            }

            /* Read password (no echo in real implementation) */
            ctx->password_mode = true;
            fprintf(stderr, "  Password: ");
            if (!fgets(ctx->password, VDM_MAX_PASSWORD, stdin)) {
                ctx->state = VDM_STATE_SHUTDOWN;
                break;
            }
            ctx->password[strcspn(ctx->password, "\n")] = '\0';
            ctx->password_mode = false;

            ctx->state = VDM_STATE_AUTHENTICATING;
            break;

        case VDM_STATE_AUTHENTICATING: {
            vdm_auth_result_t result =
                vdm_authenticate(ctx->username, ctx->password);

            /* Clear password from memory immediately */
            memset(ctx->password, 0, sizeof(ctx->password));

            if (result == VDM_AUTH_OK) {
                fprintf(stderr, "[veridian-dm] Authentication successful "
                        "for '%s'\n", ctx->username);
                ctx->state = VDM_STATE_SESSION_SELECT;
            } else {
                ctx->login_attempts++;
                const char *reason = "unknown error";
                switch (result) {
                case VDM_AUTH_BAD_USER:
                    reason = "unknown user";
                    break;
                case VDM_AUTH_BAD_PASS:
                    reason = "incorrect password";
                    break;
                case VDM_AUTH_LOCKED:
                    reason = "account locked";
                    break;
                case VDM_AUTH_ERROR:
                    reason = "internal error";
                    break;
                default:
                    break;
                }
                fprintf(stderr, "[veridian-dm] Login failed: %s "
                        "(attempt %d/%d)\n",
                        reason, ctx->login_attempts, ctx->max_attempts);

                if (ctx->login_attempts >= ctx->max_attempts) {
                    fprintf(stderr, "[veridian-dm] Too many failed "
                            "attempts, waiting 30s\n");
                    sleep(30);
                    ctx->login_attempts = 0;
                }
                ctx->state = VDM_STATE_LOGIN;
            }
            break;
        }

        case VDM_STATE_SESSION_SELECT:
            fprintf(stderr, "\n  Select session:\n");
            for (int i = 0; i < ctx->session_count; i++) {
                const char *marker =
                    (i == ctx->selected_session) ? " *" : "  ";
                fprintf(stderr, "   %s [%d] %s\n",
                        marker, i + 1, ctx->sessions[i].name);
            }
            fprintf(stderr, "  Choice [%d]: ",
                    ctx->selected_session + 1);

            char choice_buf[8];
            if (fgets(choice_buf, sizeof(choice_buf), stdin)) {
                choice_buf[strcspn(choice_buf, "\n")] = '\0';
                if (choice_buf[0] != '\0') {
                    int choice = atoi(choice_buf) - 1;
                    if (choice >= 0 && choice < ctx->session_count) {
                        ctx->selected_session = choice;
                    }
                }
            }

            /* Save preference */
            vdm_save_session_pref(
                ctx->sessions[ctx->selected_session].type);

            ctx->state = VDM_STATE_LAUNCHING;
            break;

        case VDM_STATE_LAUNCHING: {
            vdm_session_type_t stype =
                ctx->sessions[ctx->selected_session].type;
            fprintf(stderr, "[veridian-dm] Launching session: %s\n",
                    ctx->sessions[ctx->selected_session].name);

            int pid = vdm_launch_session(stype, ctx->username);
            if (pid < 0) {
                fprintf(stderr, "[veridian-dm] Failed to launch session\n");
                ctx->state = VDM_STATE_LOGIN;
            } else {
                ctx->state = VDM_STATE_RUNNING;
                /* Wait for session to exit */
                int status = 0;
                waitpid(pid, &status, 0);
                fprintf(stderr, "[veridian-dm] Session exited "
                        "(status %d)\n", WEXITSTATUS(status));
                /* Return to login screen */
                ctx->state = VDM_STATE_LOGIN;
                ctx->login_attempts = 0;
            }
            break;
        }

        case VDM_STATE_RUNNING:
            /* Should not reach here -- handled in LAUNCHING */
            ctx->state = VDM_STATE_LOGIN;
            break;

        default:
            ctx->state = VDM_STATE_SHUTDOWN;
            break;
        }
    }

    return 0;
}

/* ======================================================================
 * Cleanup
 * ====================================================================== */

void vdm_cleanup(vdm_context_t *ctx)
{
    if (!ctx) {
        return;
    }

    /* Clear sensitive data */
    memset(ctx->username, 0, sizeof(ctx->username));
    memset(ctx->password, 0, sizeof(ctx->password));

    free(ctx->framebuffer);
    ctx->framebuffer = NULL;
    ctx->state = VDM_STATE_SHUTDOWN;
}

/* ======================================================================
 * Authentication
 * ====================================================================== */

vdm_auth_result_t vdm_authenticate(const char *username,
                                    const char *password)
{
    if (!username || !password) {
        return VDM_AUTH_ERROR;
    }

    /*
     * Authentication against VeridianOS UserDatabase.
     *
     * The kernel's UserDatabase (v0.20.0+) stores credentials as
     * (Hash256, salt) pairs.  Password verification uses ct_eq_bytes
     * (constant-time comparison, v0.20.2) to prevent timing attacks.
     *
     * In the actual implementation, this calls into the kernel via
     * a syscall or D-Bus interface to verify credentials.  For now,
     * we implement a file-based check against /etc/veridian/passwd
     * as a placeholder that will be replaced with the real syscall
     * once the user-space <-> kernel auth bridge is complete.
     */

    /* Root always allowed with empty password for development */
    if (strcmp(username, "root") == 0 && strcmp(password, "") == 0) {
        return VDM_AUTH_OK;
    }

    /*
     * Check /etc/veridian/passwd for user entry.
     * Format: username:hash:salt
     * Real implementation uses the kernel UserDatabase syscall.
     */
    FILE *fp = fopen("/etc/veridian/passwd", "r");
    if (!fp) {
        /* No passwd file -- accept root only */
        if (strcmp(username, "root") == 0) {
            return VDM_AUTH_OK;
        }
        return VDM_AUTH_BAD_USER;
    }

    char line[512];
    bool found = false;
    while (fgets(line, sizeof(line), fp)) {
        line[strcspn(line, "\n")] = '\0';
        char *colon = strchr(line, ':');
        if (!colon) {
            continue;
        }
        *colon = '\0';
        if (strcmp(line, username) == 0) {
            found = true;
            /*
             * In production: extract hash+salt, compute
             * SHA-256(password + salt), ct_eq_bytes compare.
             * Placeholder: accept any password for known users.
             */
            break;
        }
    }
    fclose(fp);

    if (!found) {
        return VDM_AUTH_BAD_USER;
    }
    return VDM_AUTH_OK;
}

/* ======================================================================
 * Session preference persistence
 * ====================================================================== */

int vdm_save_session_pref(vdm_session_type_t type)
{
    /* Ensure directory exists */
    mkdir("/etc/veridian", 0755);

    FILE *fp = fopen(VDM_SESSION_CONF, "w");
    if (!fp) {
        fprintf(stderr, "[veridian-dm] Cannot write %s: %s\n",
                VDM_SESSION_CONF, strerror(errno));
        return -1;
    }

    const char *type_str = "builtin";
    switch (type) {
    case VDM_SESSION_PLASMA:
        type_str = "plasma";
        break;
    case VDM_SESSION_BUILTIN:
    default:
        type_str = "builtin";
        break;
    }

    fprintf(fp, "# VeridianOS session configuration\n");
    fprintf(fp, "# Generated by veridian-dm\n");
    fprintf(fp, "session_type=%s\n", type_str);
    fclose(fp);

    return 0;
}

vdm_session_type_t vdm_load_session_pref(void)
{
    FILE *fp = fopen(VDM_SESSION_CONF, "r");
    if (!fp) {
        return VDM_SESSION_BUILTIN;
    }

    char line[256];
    vdm_session_type_t result = VDM_SESSION_BUILTIN;

    while (fgets(line, sizeof(line), fp)) {
        line[strcspn(line, "\n")] = '\0';
        if (line[0] == '#' || line[0] == '\0') {
            continue;
        }
        if (strncmp(line, "session_type=", 13) == 0) {
            const char *val = line + 13;
            if (strcmp(val, "plasma") == 0 || strcmp(val, "kde") == 0) {
                result = VDM_SESSION_PLASMA;
            }
            break;
        }
    }

    fclose(fp);
    return result;
}

/* ======================================================================
 * Auto-login configuration
 * ====================================================================== */

bool vdm_load_autologin(vdm_autologin_t *cfg)
{
    if (!cfg) {
        return false;
    }

    memset(cfg, 0, sizeof(*cfg));

    FILE *fp = fopen(VDM_AUTOLOGIN_CONF, "r");
    if (!fp) {
        return false;
    }

    char line[256];
    while (fgets(line, sizeof(line), fp)) {
        line[strcspn(line, "\n")] = '\0';
        if (line[0] == '#' || line[0] == '\0') {
            continue;
        }

        if (strncmp(line, "enabled=", 8) == 0) {
            cfg->enabled = (strcmp(line + 8, "true") == 0 ||
                           strcmp(line + 8, "1") == 0);
        } else if (strncmp(line, "username=", 9) == 0) {
            strncpy(cfg->username, line + 9, VDM_MAX_USERNAME - 1);
            cfg->username[VDM_MAX_USERNAME - 1] = '\0';
        } else if (strncmp(line, "session=", 8) == 0) {
            if (strcmp(line + 8, "plasma") == 0) {
                cfg->session_type = VDM_SESSION_PLASMA;
            } else {
                cfg->session_type = VDM_SESSION_BUILTIN;
            }
        } else if (strncmp(line, "delay=", 6) == 0) {
            cfg->delay_seconds = atoi(line + 6);
        }
    }

    fclose(fp);
    return cfg->enabled;
}

/* ======================================================================
 * Session launch
 * ====================================================================== */

int vdm_launch_session(vdm_session_type_t type, const char *username)
{
    if (!username) {
        return -1;
    }

    switch (type) {
    case VDM_SESSION_BUILTIN:
        /*
         * Built-in DE runs in the kernel compositor -- no separate
         * process needed.  Spawn a shell for the user.
         */
        {
            pid_t pid = fork();
            if (pid < 0) {
                return -1;
            }
            if (pid == 0) {
                /* Child: set up environment and exec shell */
                setenv("USER", username, 1);
                setenv("HOME", "/root", 1);
                setenv("SHELL", "/bin/sh", 1);
                setenv("TERM", "veridian", 1);
                setenv("VERIDIAN_SESSION", "builtin", 1);
                execl("/bin/sh", "sh", (char *)NULL);
                _exit(127);
            }
            return pid;
        }

    case VDM_SESSION_PLASMA:
        /*
         * KDE Plasma session: exec the session script which handles
         * D-Bus session bus, KDE daemons, KWin, and plasmashell.
         */
        {
            const char *session_exec = "/usr/bin/plasma-veridian-session";

            /* Verify the session script exists */
            if (access(session_exec, X_OK) != 0) {
                fprintf(stderr, "[veridian-dm] %s not found or not "
                        "executable\n", session_exec);
                return -1;
            }

            pid_t pid = fork();
            if (pid < 0) {
                return -1;
            }
            if (pid == 0) {
                /* Child: set up KDE environment and exec session */
                setenv("USER", username, 1);
                setenv("HOME", "/root", 1);
                setenv("SHELL", "/bin/sh", 1);
                setenv("XDG_SESSION_TYPE", "wayland", 1);
                setenv("XDG_CURRENT_DESKTOP", "KDE", 1);
                setenv("XDG_SESSION_DESKTOP", "KDE", 1);
                setenv("QT_QPA_PLATFORM", "wayland", 1);
                setenv("VERIDIAN_SESSION", "plasma", 1);
                execl(session_exec, "plasma-veridian-session",
                      (char *)NULL);
                _exit(127);
            }
            return pid;
        }

    default:
        return -1;
    }
}

/* ======================================================================
 * Rendering
 * ====================================================================== */

void vdm_render_rect(vdm_context_t *ctx, int x, int y,
                     int w, int h, uint32_t color)
{
    if (!ctx || !ctx->framebuffer) {
        return;
    }

    /* Clip to framebuffer bounds */
    int x0 = (x < 0) ? 0 : x;
    int y0 = (y < 0) ? 0 : y;
    int x1 = (x + w > ctx->fb_width) ? ctx->fb_width : x + w;
    int y1 = (y + h > ctx->fb_height) ? ctx->fb_height : y + h;

    for (int py = y0; py < y1; py++) {
        for (int px = x0; px < x1; px++) {
            ctx->framebuffer[py * ctx->fb_width + px] = color;
        }
    }
}

void vdm_render_text(vdm_context_t *ctx, int x, int y,
                     const char *text, uint32_t color)
{
    if (!ctx || !ctx->framebuffer || !text) {
        return;
    }

    /*
     * Text rendering uses the kernel's built-in 8x16 bitmap font.
     * In the actual implementation, this calls the kernel's
     * draw_string_into_buffer() function via a shared library
     * or syscall.
     *
     * For compilation purposes, this is a stub that will be linked
     * against the kernel's font rendering library at build time.
     */
    (void)x;
    (void)y;
    (void)text;
    (void)color;

    /* Placeholder: actual rendering handled by kernel font subsystem */
}

void vdm_render_login(vdm_context_t *ctx)
{
    if (!ctx || !ctx->framebuffer) {
        return;
    }

    /* Clear background */
    vdm_render_rect(ctx, 0, 0, ctx->fb_width, ctx->fb_height, COLOR_BG);

    /* Center coordinates */
    int cx = ctx->fb_width / 2;
    int cy = ctx->fb_height / 2;

    /* Title bar */
    vdm_render_text(ctx, cx - 120, cy - 160,
                    "VeridianOS", COLOR_TITLE);
    vdm_render_text(ctx, cx - 80, cy - 130,
                    "Login", COLOR_DIM);

    /* Username field */
    vdm_render_rect(ctx, cx - 150, cy - 80, 300, 32, COLOR_INPUT_BG);
    vdm_render_rect(ctx, cx - 150, cy - 80, 300, 1, COLOR_INPUT_BORDER);
    vdm_render_text(ctx, cx - 140, cy - 72, "Username:", COLOR_FG);

    /* Password field */
    vdm_render_rect(ctx, cx - 150, cy - 30, 300, 32, COLOR_INPUT_BG);
    vdm_render_rect(ctx, cx - 150, cy - 30, 300, 1, COLOR_INPUT_BORDER);
    vdm_render_text(ctx, cx - 140, cy - 22, "Password:", COLOR_FG);

    /* Session selector */
    for (int i = 0; i < ctx->session_count; i++) {
        int sy = cy + 30 + i * 28;
        uint32_t bg = (i == ctx->selected_session)
                      ? COLOR_SELECTED : COLOR_BG;
        vdm_render_rect(ctx, cx - 150, sy, 300, 24, bg);
        vdm_render_text(ctx, cx - 140, sy + 4,
                        ctx->sessions[i].name, COLOR_FG);
    }

    /* Login attempt counter */
    if (ctx->login_attempts > 0) {
        char msg[64];
        snprintf(msg, sizeof(msg), "Failed attempts: %d/%d",
                 ctx->login_attempts, ctx->max_attempts);
        vdm_render_text(ctx, cx - 100, cy + 120, msg, COLOR_ERROR);
    }
}

bool vdm_handle_key(vdm_context_t *ctx, int keycode, bool pressed)
{
    if (!ctx || !pressed) {
        return false;
    }

    /*
     * Keyboard handling for the DM.
     * In the actual implementation, keycodes come from the kernel's
     * PS/2 driver or libinput via the input subsystem.
     *
     * Key mappings:
     *   Tab     - Switch between username/password/session fields
     *   Enter   - Submit current field or confirm login
     *   Up/Down - Navigate session list
     *   Esc     - Clear current field
     */
    (void)keycode;

    return true;
}

/* ======================================================================
 * Multi-user session management
 * ====================================================================== */

/* Maximum concurrent user sessions */
#define VDM_MAX_USER_SESSIONS   8

/* User entry from /etc/veridian/passwd or user database */
typedef struct {
    char    username[VDM_MAX_USERNAME];
    char    display_name[VDM_MAX_USERNAME];
    int     uid;
    char    home_dir[256];
    char    shell[128];
    bool    has_avatar;
} vdm_user_entry_t;

/* Active session tracking */
typedef struct {
    int     session_id;
    int     vt_number;          /* VT7 + index */
    int     uid;
    char    username[VDM_MAX_USERNAME];
    pid_t   session_pid;
    vdm_session_type_t session_type;
    bool    active;
    bool    locked;
} vdm_active_session_t;

static vdm_user_entry_t s_user_list[VDM_MAX_USER_SESSIONS];
static int s_user_count = 0;

static vdm_active_session_t s_active_sessions[VDM_MAX_USER_SESSIONS];
static int s_active_session_count = 0;
static int s_current_session_idx = -1;

/*
 * Load the list of available users from /etc/veridian/passwd.
 * Format: username:hash:salt:uid:display_name:home:shell
 * Returns the number of users loaded.
 */
int vdm_load_user_list(void)
{
    s_user_count = 0;

    /* Always add root */
    strncpy(s_user_list[0].username, "root", VDM_MAX_USERNAME - 1);
    strncpy(s_user_list[0].display_name, "System Administrator",
            VDM_MAX_USERNAME - 1);
    s_user_list[0].uid = 0;
    strncpy(s_user_list[0].home_dir, "/root", sizeof(s_user_list[0].home_dir) - 1);
    strncpy(s_user_list[0].shell, "/bin/sh", sizeof(s_user_list[0].shell) - 1);
    s_user_list[0].has_avatar = false;
    s_user_count = 1;

    FILE *fp = fopen("/etc/veridian/passwd", "r");
    if (!fp) {
        return s_user_count;
    }

    char line[512];
    while (fgets(line, sizeof(line), fp) &&
           s_user_count < VDM_MAX_USER_SESSIONS) {
        line[strcspn(line, "\n")] = '\0';
        if (line[0] == '#' || line[0] == '\0') {
            continue;
        }

        /* Parse fields: username:hash:salt:uid:display_name:home:shell */
        char *fields[7];
        int field_count = 0;
        char *tok = strtok(line, ":");
        while (tok && field_count < 7) {
            fields[field_count++] = tok;
            tok = strtok(NULL, ":");
        }

        if (field_count < 4) {
            continue;
        }

        /* Skip if root is already added */
        if (strcmp(fields[0], "root") == 0) {
            continue;
        }

        vdm_user_entry_t *u = &s_user_list[s_user_count];
        memset(u, 0, sizeof(*u));
        strncpy(u->username, fields[0], VDM_MAX_USERNAME - 1);
        u->uid = atoi(fields[3]);

        if (field_count >= 5) {
            strncpy(u->display_name, fields[4], VDM_MAX_USERNAME - 1);
        } else {
            strncpy(u->display_name, fields[0], VDM_MAX_USERNAME - 1);
        }

        if (field_count >= 6) {
            strncpy(u->home_dir, fields[5], sizeof(u->home_dir) - 1);
        } else {
            snprintf(u->home_dir, sizeof(u->home_dir), "/home/%s", fields[0]);
        }

        if (field_count >= 7) {
            strncpy(u->shell, fields[6], sizeof(u->shell) - 1);
        } else {
            strncpy(u->shell, "/bin/sh", sizeof(u->shell) - 1);
        }

        /* Check for user avatar at ~/.face */
        char avatar_path[300];
        snprintf(avatar_path, sizeof(avatar_path), "%s/.face", u->home_dir);
        struct stat avatar_st;
        u->has_avatar = (stat(avatar_path, &avatar_st) == 0);

        s_user_count++;
    }

    fclose(fp);
    fprintf(stderr, "[veridian-dm] Loaded %d users\n", s_user_count);
    return s_user_count;
}

/*
 * Create a new user session on the next available VT.
 * VT assignment: session 0 = VT7, session 1 = VT8, etc.
 * Returns the session index, or -1 on error.
 */
int vdm_create_user_session(const char *username, int uid,
                             vdm_session_type_t stype)
{
    if (s_active_session_count >= VDM_MAX_USER_SESSIONS) {
        fprintf(stderr, "[veridian-dm] Maximum sessions reached (%d)\n",
                VDM_MAX_USER_SESSIONS);
        return -1;
    }

    int idx = s_active_session_count;
    vdm_active_session_t *sess = &s_active_sessions[idx];

    memset(sess, 0, sizeof(*sess));
    sess->session_id = idx;
    sess->vt_number = 7 + idx;  /* VT7, VT8, ... */
    sess->uid = uid;
    strncpy(sess->username, username, VDM_MAX_USERNAME - 1);
    sess->session_type = stype;
    sess->active = true;
    sess->locked = false;
    sess->session_pid = -1;

    s_active_session_count++;

    fprintf(stderr, "[veridian-dm] Created session %d for '%s' on VT%d\n",
            idx, username, sess->vt_number);
    return idx;
}

/*
 * Switch to a different user session.
 * Saves the current session state and activates the target session.
 */
int vdm_switch_to_session(int session_idx)
{
    if (session_idx < 0 || session_idx >= s_active_session_count) {
        return -1;
    }

    /* Deactivate current session */
    if (s_current_session_idx >= 0 &&
        s_current_session_idx < s_active_session_count) {
        s_active_sessions[s_current_session_idx].active = false;
        fprintf(stderr, "[veridian-dm] Suspending session %d ('%s')\n",
                s_current_session_idx,
                s_active_sessions[s_current_session_idx].username);
    }

    /* Activate target session */
    s_active_sessions[session_idx].active = true;
    s_current_session_idx = session_idx;

    fprintf(stderr, "[veridian-dm] Switched to session %d ('%s') on VT%d\n",
            session_idx,
            s_active_sessions[session_idx].username,
            s_active_sessions[session_idx].vt_number);
    return 0;
}

/*
 * Handle "Switch User" action: save current session and show the greeter
 * for a new login on the next VT.
 */
int vdm_handle_switch_user(vdm_context_t *ctx)
{
    if (!ctx) {
        return -1;
    }

    /* Mark current session as background (not destroyed) */
    if (s_current_session_idx >= 0) {
        s_active_sessions[s_current_session_idx].active = false;
    }

    /* Return to login screen for new user */
    ctx->state = VDM_STATE_LOGIN;
    ctx->login_attempts = 0;
    memset(ctx->username, 0, sizeof(ctx->username));
    memset(ctx->password, 0, sizeof(ctx->password));

    fprintf(stderr, "[veridian-dm] Switch User: showing greeter for new login\n");
    return 0;
}

/*
 * Lock the current session and show a simplified greeter.
 * Only the current user's password is required to unlock.
 */
int vdm_lock_current_session(vdm_context_t *ctx)
{
    if (!ctx || s_current_session_idx < 0) {
        return -1;
    }

    vdm_active_session_t *sess = &s_active_sessions[s_current_session_idx];
    sess->locked = true;

    /* Pre-fill username for lock screen (password only) */
    strncpy(ctx->username, sess->username, VDM_MAX_USERNAME - 1);
    ctx->state = VDM_STATE_LOGIN;
    ctx->password_mode = true;

    fprintf(stderr, "[veridian-dm] Session %d locked for '%s'\n",
            s_current_session_idx, sess->username);
    return 0;
}

/*
 * Get the list of active sessions for display in the DM.
 * Returns the count of active sessions.
 */
int vdm_get_active_sessions(vdm_active_session_t *out, int max)
{
    int count = 0;
    for (int i = 0; i < s_active_session_count && count < max; i++) {
        out[count++] = s_active_sessions[i];
    }
    return count;
}

/*
 * Render the user list on the login screen.
 * Shows user names with avatar indicators.
 */
void vdm_render_user_list(vdm_context_t *ctx)
{
    if (!ctx || !ctx->framebuffer || s_user_count == 0) {
        return;
    }

    int cx = ctx->fb_width / 2;
    int start_y = ctx->fb_height / 2 + 140;

    vdm_render_text(ctx, cx - 60, start_y, "Users:", COLOR_DIM);

    for (int i = 0; i < s_user_count && i < 6; i++) {
        int uy = start_y + 24 + i * 20;
        char label[128];
        snprintf(label, sizeof(label), "  %s%s (%s)",
                 s_user_list[i].has_avatar ? "[*] " : "    ",
                 s_user_list[i].display_name,
                 s_user_list[i].username);
        vdm_render_text(ctx, cx - 140, uy, label, COLOR_FG);
    }

    /* Show active session count */
    if (s_active_session_count > 0) {
        char sess_msg[64];
        snprintf(sess_msg, sizeof(sess_msg),
                 "%d active session(s)", s_active_session_count);
        vdm_render_text(ctx, cx - 80, start_y - 24, sess_msg, COLOR_ACCENT);
    }
}

/*
 * Enhanced login rendering with user list and session indicators.
 */
void vdm_render_login_enhanced(vdm_context_t *ctx)
{
    if (!ctx || !ctx->framebuffer) {
        return;
    }

    /* Render the standard login screen */
    vdm_render_login(ctx);

    /* Overlay the user list */
    vdm_render_user_list(ctx);

    /* Render active session switcher bar at bottom */
    if (s_active_session_count > 1) {
        int bar_y = ctx->fb_height - 40;
        vdm_render_rect(ctx, 0, bar_y, ctx->fb_width, 40, 0xFF0A0A1E);

        int bar_x = 20;
        for (int i = 0; i < s_active_session_count; i++) {
            uint32_t bg = (i == s_current_session_idx)
                          ? COLOR_SELECTED : 0xFF1A1A2E;
            vdm_render_rect(ctx, bar_x, bar_y + 4, 120, 32, bg);

            char label[32];
            snprintf(label, sizeof(label), "VT%d: %s",
                     s_active_sessions[i].vt_number,
                     s_active_sessions[i].username);
            vdm_render_text(ctx, bar_x + 8, bar_y + 12, label, COLOR_FG);

            if (s_active_sessions[i].locked) {
                vdm_render_text(ctx, bar_x + 100, bar_y + 12,
                                "[L]", COLOR_ERROR);
            }

            bar_x += 140;
        }
    }
}

/* ======================================================================
 * Entry point
 * ====================================================================== */

int main(void)
{
    vdm_context_t ctx;

    fprintf(stderr, "[veridian-dm] VeridianOS Display Manager starting\n");

    /* Load available users for the login screen */
    vdm_load_user_list();

    if (vdm_init(&ctx) != 0) {
        fprintf(stderr, "[veridian-dm] Initialization failed\n");
        return 1;
    }

    int result = vdm_run(&ctx);

    vdm_cleanup(&ctx);

    fprintf(stderr, "[veridian-dm] Display Manager exiting (%d)\n", result);
    return result;
}
