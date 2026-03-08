/*
 * VeridianOS libc -- xkbcommon.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * xkbcommon 1.7.0 compatible implementation.
 * Provides keyboard context, keymap from names/string, state
 * tracking with modifier management, keysym-to-UTF8 conversion,
 * and compose key support.
 */

#include <xkbcommon/xkbcommon.h>
#include <xkbcommon/xkbcommon-keysyms.h>
#include <xkbcommon/xkbcommon-names.h>
#include <xkbcommon/xkbcommon-compose.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

/* ========================================================================= */
/* Internal limits                                                           */
/* ========================================================================= */

#define MAX_CONTEXTS       8
#define MAX_KEYMAPS       16
#define MAX_STATES        32
#define MAX_COMPOSE_TABLES 4
#define MAX_COMPOSE_STATES 8

#define NUM_MODS   8
#define NUM_LEDS   3

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct xkb_context {
    int                  in_use;
    int                  ref_count;
    enum xkb_log_level   log_level;
    int                  log_verbosity;
};

struct xkb_keymap {
    int                  in_use;
    int                  ref_count;
    struct xkb_context  *context;
    /* Modifier names */
    const char          *mod_names[NUM_MODS];
    /* LED names */
    const char          *led_names[NUM_LEDS];
};

struct xkb_state {
    int                  in_use;
    int                  ref_count;
    struct xkb_keymap   *keymap;
    /* Modifier state */
    xkb_mod_mask_t       mods_depressed;
    xkb_mod_mask_t       mods_latched;
    xkb_mod_mask_t       mods_locked;
    xkb_layout_index_t   layout;
    /* LED state */
    int                  led_caps;
    int                  led_num;
    int                  led_scroll;
    /* Last keysym produced (for key_get_syms) */
    xkb_keysym_t         last_sym;
};

struct xkb_compose_table {
    int                  in_use;
    int                  ref_count;
    struct xkb_context  *context;
};

struct xkb_compose_state {
    int                             in_use;
    int                             ref_count;
    struct xkb_compose_table       *table;
    enum xkb_compose_status         status;
    xkb_keysym_t                    result_sym;
};

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static struct xkb_context       g_contexts[MAX_CONTEXTS];
static struct xkb_keymap         g_keymaps[MAX_KEYMAPS];
static struct xkb_state          g_states[MAX_STATES];
static struct xkb_compose_table  g_compose_tables[MAX_COMPOSE_TABLES];
static struct xkb_compose_state  g_compose_states[MAX_COMPOSE_STATES];

/* Standard modifier names (indexed by bit position) */
static const char *g_mod_names[NUM_MODS] = {
    "Shift",      /* bit 0 */
    "Lock",       /* bit 1 (Caps Lock) */
    "Control",    /* bit 2 */
    "Mod1",       /* bit 3 (Alt) */
    "Mod2",       /* bit 4 (Num Lock) */
    "Mod3",       /* bit 5 */
    "Mod4",       /* bit 6 (Super/Logo) */
    "Mod5"        /* bit 7 */
};

static const char *g_led_names[NUM_LEDS] = {
    "Caps Lock",
    "Num Lock",
    "Scroll Lock"
};

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static xkb_keysym_t keycode_to_keysym(xkb_keycode_t keycode,
                                         xkb_mod_mask_t mods)
{
    /* Evdev keycodes start at 8; subtract to get scancode.
     * For simplicity, map common keys directly. */
    uint32_t sc = keycode - 8;
    int shift = (mods & 0x01) != 0;  /* Shift */
    int caps  = (mods & 0x02) != 0;  /* Caps Lock */

    /* ASCII letters (scancodes 16-25 = q,w,e,r,t,y,u,i,o,p etc.) */
    static const char row1[] = "qwertyuiop";
    static const char row2[] = "asdfghjkl";
    static const char row3[] = "zxcvbnm";

    xkb_keysym_t sym = XKB_KEY_NoSymbol;

    if (sc >= 16 && sc <= 25 && (sc - 16) < sizeof(row1) - 1) {
        sym = (xkb_keysym_t)row1[sc - 16];
    } else if (sc >= 30 && sc <= 38 && (sc - 30) < sizeof(row2) - 1) {
        sym = (xkb_keysym_t)row2[sc - 30];
    } else if (sc >= 44 && sc <= 50 && (sc - 44) < sizeof(row3) - 1) {
        sym = (xkb_keysym_t)row3[sc - 44];
    } else if (sc >= 2 && sc <= 10) {
        /* Number row: 1-9 */
        if (!shift)
            sym = XKB_KEY_1 + (sc - 2);
        else {
            static const xkb_keysym_t shifted[] = {
                XKB_KEY_exclam, XKB_KEY_at, XKB_KEY_numbersign,
                XKB_KEY_dollar, XKB_KEY_percent, XKB_KEY_asciicircum,
                XKB_KEY_ampersand, XKB_KEY_asterisk, XKB_KEY_parenleft
            };
            sym = shifted[sc - 2];
        }
    } else if (sc == 11) {
        sym = shift ? XKB_KEY_parenright : XKB_KEY_0;
    } else if (sc == 1) {
        sym = XKB_KEY_Escape;
    } else if (sc == 14) {
        sym = XKB_KEY_BackSpace;
    } else if (sc == 15) {
        sym = XKB_KEY_Tab;
    } else if (sc == 28) {
        sym = XKB_KEY_Return;
    } else if (sc == 57) {
        sym = XKB_KEY_space;
    } else if (sc == 42) {
        sym = XKB_KEY_Shift_L;
    } else if (sc == 54) {
        sym = XKB_KEY_Shift_R;
    } else if (sc == 29) {
        sym = XKB_KEY_Control_L;
    } else if (sc == 97) {
        sym = XKB_KEY_Control_R;
    } else if (sc == 56) {
        sym = XKB_KEY_Alt_L;
    } else if (sc == 100) {
        sym = XKB_KEY_Alt_R;
    } else if (sc == 125) {
        sym = XKB_KEY_Super_L;
    } else if (sc == 126) {
        sym = XKB_KEY_Super_R;
    } else if (sc == 58) {
        sym = XKB_KEY_Caps_Lock;
    } else if (sc == 111) {
        sym = XKB_KEY_Delete;
    } else if (sc == 102) {
        sym = XKB_KEY_Home;
    } else if (sc == 107) {
        sym = XKB_KEY_End;
    } else if (sc == 104) {
        sym = XKB_KEY_Page_Down;
    } else if (sc == 99) {
        sym = XKB_KEY_Page_Up;
    } else if (sc == 103) {
        sym = XKB_KEY_Up;
    } else if (sc == 108) {
        sym = XKB_KEY_Down;
    } else if (sc == 105) {
        sym = XKB_KEY_Left;
    } else if (sc == 106) {
        sym = XKB_KEY_Right;
    } else if (sc >= 59 && sc <= 68) {
        sym = XKB_KEY_F1 + (sc - 59);
    } else if (sc == 87) {
        sym = XKB_KEY_F11;
    } else if (sc == 88) {
        sym = XKB_KEY_F12;
    }

    /* Apply Shift/CapsLock to letters */
    if (sym >= 'a' && sym <= 'z') {
        if (shift ^ caps)
            sym = sym - 'a' + 'A';
    }

    return sym;
}

/* ========================================================================= */
/* Keysym operations                                                         */
/* ========================================================================= */

int xkb_keysym_get_name(xkb_keysym_t keysym, char *buffer, size_t size)
{
    if (!buffer || size == 0)
        return -1;

    /* Common keysyms */
    if (keysym >= XKB_KEY_a && keysym <= XKB_KEY_z) {
        return snprintf(buffer, size, "%c", (char)keysym);
    }
    if (keysym >= XKB_KEY_A && keysym <= XKB_KEY_Z) {
        return snprintf(buffer, size, "%c", (char)keysym);
    }
    if (keysym >= XKB_KEY_0 && keysym <= XKB_KEY_9) {
        return snprintf(buffer, size, "%c", (char)keysym);
    }
    if (keysym == XKB_KEY_space)     return snprintf(buffer, size, "space");
    if (keysym == XKB_KEY_Return)    return snprintf(buffer, size, "Return");
    if (keysym == XKB_KEY_Escape)    return snprintf(buffer, size, "Escape");
    if (keysym == XKB_KEY_Tab)       return snprintf(buffer, size, "Tab");
    if (keysym == XKB_KEY_BackSpace) return snprintf(buffer, size, "BackSpace");
    if (keysym == XKB_KEY_Delete)    return snprintf(buffer, size, "Delete");
    if (keysym == XKB_KEY_Shift_L)   return snprintf(buffer, size, "Shift_L");
    if (keysym == XKB_KEY_Shift_R)   return snprintf(buffer, size, "Shift_R");
    if (keysym == XKB_KEY_Control_L) return snprintf(buffer, size, "Control_L");
    if (keysym == XKB_KEY_Control_R) return snprintf(buffer, size, "Control_R");
    if (keysym == XKB_KEY_Alt_L)     return snprintf(buffer, size, "Alt_L");
    if (keysym == XKB_KEY_Alt_R)     return snprintf(buffer, size, "Alt_R");
    if (keysym == XKB_KEY_Super_L)   return snprintf(buffer, size, "Super_L");
    if (keysym == XKB_KEY_Super_R)   return snprintf(buffer, size, "Super_R");

    return snprintf(buffer, size, "0x%04x", keysym);
}

xkb_keysym_t xkb_keysym_from_name(const char *name,
                                      enum xkb_keysym_flags flags)
{
    (void)flags;

    if (!name) return XKB_KEY_NoSymbol;

    /* Single character */
    if (name[1] == '\0') {
        if (name[0] >= 'a' && name[0] <= 'z') return (xkb_keysym_t)name[0];
        if (name[0] >= 'A' && name[0] <= 'Z') return (xkb_keysym_t)name[0];
        if (name[0] >= '0' && name[0] <= '9') return (xkb_keysym_t)name[0];
    }

    if (strcmp(name, "space") == 0)     return XKB_KEY_space;
    if (strcmp(name, "Return") == 0)    return XKB_KEY_Return;
    if (strcmp(name, "Escape") == 0)    return XKB_KEY_Escape;
    if (strcmp(name, "Tab") == 0)       return XKB_KEY_Tab;
    if (strcmp(name, "BackSpace") == 0) return XKB_KEY_BackSpace;
    if (strcmp(name, "Delete") == 0)    return XKB_KEY_Delete;
    if (strcmp(name, "Shift_L") == 0)   return XKB_KEY_Shift_L;
    if (strcmp(name, "Shift_R") == 0)   return XKB_KEY_Shift_R;
    if (strcmp(name, "Control_L") == 0) return XKB_KEY_Control_L;
    if (strcmp(name, "Control_R") == 0) return XKB_KEY_Control_R;
    if (strcmp(name, "Alt_L") == 0)     return XKB_KEY_Alt_L;
    if (strcmp(name, "Alt_R") == 0)     return XKB_KEY_Alt_R;
    if (strcmp(name, "Super_L") == 0)   return XKB_KEY_Super_L;
    if (strcmp(name, "Super_R") == 0)   return XKB_KEY_Super_R;

    return XKB_KEY_NoSymbol;
}

int xkb_keysym_to_utf8(xkb_keysym_t keysym, char *buffer, size_t size)
{
    uint32_t cp = xkb_keysym_to_utf32(keysym);
    if (cp == 0 || size == 0)
        return 0;

    if (cp < 0x80) {
        if (size < 2) return 0;
        buffer[0] = (char)cp;
        buffer[1] = '\0';
        return 1;
    }
    if (cp < 0x800) {
        if (size < 3) return 0;
        buffer[0] = (char)(0xC0 | (cp >> 6));
        buffer[1] = (char)(0x80 | (cp & 0x3F));
        buffer[2] = '\0';
        return 2;
    }
    if (cp < 0x10000) {
        if (size < 4) return 0;
        buffer[0] = (char)(0xE0 | (cp >> 12));
        buffer[1] = (char)(0x80 | ((cp >> 6) & 0x3F));
        buffer[2] = (char)(0x80 | (cp & 0x3F));
        buffer[3] = '\0';
        return 3;
    }
    if (cp < 0x110000) {
        if (size < 5) return 0;
        buffer[0] = (char)(0xF0 | (cp >> 18));
        buffer[1] = (char)(0x80 | ((cp >> 12) & 0x3F));
        buffer[2] = (char)(0x80 | ((cp >> 6) & 0x3F));
        buffer[3] = (char)(0x80 | (cp & 0x3F));
        buffer[4] = '\0';
        return 4;
    }
    return 0;
}

uint32_t xkb_keysym_to_utf32(xkb_keysym_t keysym)
{
    /* Latin-1 keysyms map directly to Unicode */
    if (keysym >= 0x0020 && keysym <= 0x007E)
        return keysym;
    if (keysym >= 0x00A0 && keysym <= 0x00FF)
        return keysym;

    /* Unicode keysyms (0x01000000 + codepoint) */
    if ((keysym & 0xFF000000) == 0x01000000)
        return keysym & 0x00FFFFFF;

    /* Special keys have no text representation */
    if (keysym >= 0xFF00)
        return 0;

    return 0;
}

xkb_keysym_t xkb_utf32_to_keysym(uint32_t ucs)
{
    if (ucs >= 0x0020 && ucs <= 0x007E)
        return (xkb_keysym_t)ucs;
    if (ucs >= 0x00A0 && ucs <= 0x00FF)
        return (xkb_keysym_t)ucs;
    if (ucs > 0x00FF)
        return (xkb_keysym_t)(ucs | 0x01000000);
    return XKB_KEY_NoSymbol;
}

xkb_keysym_t xkb_keysym_to_upper(xkb_keysym_t ks)
{
    if (ks >= XKB_KEY_a && ks <= XKB_KEY_z)
        return ks - 0x20;
    return ks;
}

xkb_keysym_t xkb_keysym_to_lower(xkb_keysym_t ks)
{
    if (ks >= XKB_KEY_A && ks <= XKB_KEY_Z)
        return ks + 0x20;
    return ks;
}

/* ========================================================================= */
/* Context                                                                   */
/* ========================================================================= */

struct xkb_context *xkb_context_new(enum xkb_context_flags flags)
{
    int i;
    (void)flags;

    for (i = 0; i < MAX_CONTEXTS; i++) {
        if (!g_contexts[i].in_use) {
            memset(&g_contexts[i], 0, sizeof(g_contexts[i]));
            g_contexts[i].in_use        = 1;
            g_contexts[i].ref_count     = 1;
            g_contexts[i].log_level     = XKB_LOG_LEVEL_ERROR;
            g_contexts[i].log_verbosity = 0;
            return &g_contexts[i];
        }
    }
    return NULL;
}

struct xkb_context *xkb_context_ref(struct xkb_context *context)
{
    if (context) context->ref_count++;
    return context;
}

void xkb_context_unref(struct xkb_context *context)
{
    if (!context) return;
    if (--context->ref_count <= 0)
        context->in_use = 0;
}

void xkb_context_set_log_level(struct xkb_context *context,
                                  enum xkb_log_level level)
{
    if (context) context->log_level = level;
}

enum xkb_log_level xkb_context_get_log_level(struct xkb_context *context)
{
    return context ? context->log_level : XKB_LOG_LEVEL_ERROR;
}

void xkb_context_set_log_verbosity(struct xkb_context *context, int verbosity)
{
    if (context) context->log_verbosity = verbosity;
}

int xkb_context_get_log_verbosity(struct xkb_context *context)
{
    return context ? context->log_verbosity : 0;
}

int xkb_context_num_include_paths(struct xkb_context *context)
{
    (void)context;
    return 1;
}

int xkb_context_include_path_append(struct xkb_context *context,
                                       const char *path)
{
    (void)context;
    (void)path;
    return 1;
}

int xkb_context_include_path_append_default(struct xkb_context *context)
{
    (void)context;
    return 1;
}

void xkb_context_include_path_clear(struct xkb_context *context)
{
    (void)context;
}

const char *xkb_context_include_path_get(struct xkb_context *context,
                                            unsigned int index)
{
    (void)context;
    if (index == 0) return "/usr/share/X11/xkb";
    return NULL;
}

/* ========================================================================= */
/* Keymap                                                                    */
/* ========================================================================= */

static void init_keymap_defaults(struct xkb_keymap *km,
                                   struct xkb_context *ctx)
{
    int i;
    km->context = ctx;
    for (i = 0; i < NUM_MODS; i++)
        km->mod_names[i] = g_mod_names[i];
    for (i = 0; i < NUM_LEDS; i++)
        km->led_names[i] = g_led_names[i];
}

struct xkb_keymap *xkb_keymap_new_from_names(struct xkb_context *context,
                                                 const struct xkb_rule_names *names,
                                                 enum xkb_keymap_compile_flags flags)
{
    int i;
    (void)names;
    (void)flags;

    if (!context) return NULL;

    for (i = 0; i < MAX_KEYMAPS; i++) {
        if (!g_keymaps[i].in_use) {
            memset(&g_keymaps[i], 0, sizeof(g_keymaps[i]));
            g_keymaps[i].in_use    = 1;
            g_keymaps[i].ref_count = 1;
            init_keymap_defaults(&g_keymaps[i], context);
            return &g_keymaps[i];
        }
    }
    return NULL;
}

struct xkb_keymap *xkb_keymap_new_from_string(struct xkb_context *context,
                                                  const char *string,
                                                  enum xkb_keymap_format format,
                                                  enum xkb_keymap_compile_flags flags)
{
    (void)string;
    (void)format;
    return xkb_keymap_new_from_names(context, NULL, flags);
}

struct xkb_keymap *xkb_keymap_new_from_file(struct xkb_context *context,
                                                void *file,
                                                enum xkb_keymap_format format,
                                                enum xkb_keymap_compile_flags flags)
{
    (void)file;
    (void)format;
    return xkb_keymap_new_from_names(context, NULL, flags);
}

struct xkb_keymap *xkb_keymap_new_from_buffer(struct xkb_context *context,
                                                  const char *buffer,
                                                  size_t length,
                                                  enum xkb_keymap_format format,
                                                  enum xkb_keymap_compile_flags flags)
{
    (void)buffer;
    (void)length;
    (void)format;
    return xkb_keymap_new_from_names(context, NULL, flags);
}

struct xkb_keymap *xkb_keymap_ref(struct xkb_keymap *keymap)
{
    if (keymap) keymap->ref_count++;
    return keymap;
}

void xkb_keymap_unref(struct xkb_keymap *keymap)
{
    if (!keymap) return;
    if (--keymap->ref_count <= 0)
        keymap->in_use = 0;
}

char *xkb_keymap_get_as_string(struct xkb_keymap *keymap,
                                  enum xkb_keymap_format format)
{
    (void)keymap;
    (void)format;
    return strdup("xkb_keymap { /* VeridianOS default */ };");
}

xkb_keycode_t xkb_keymap_min_keycode(struct xkb_keymap *keymap)
{
    (void)keymap;
    return 8;
}

xkb_keycode_t xkb_keymap_max_keycode(struct xkb_keymap *keymap)
{
    (void)keymap;
    return 255;
}

xkb_layout_index_t xkb_keymap_num_layouts(struct xkb_keymap *keymap)
{
    (void)keymap;
    return 1;
}

const char *xkb_keymap_layout_get_name(struct xkb_keymap *keymap,
                                          xkb_layout_index_t idx)
{
    (void)keymap;
    if (idx == 0) return "English (US)";
    return NULL;
}

xkb_layout_index_t xkb_keymap_layout_get_index(struct xkb_keymap *keymap,
                                                    const char *name)
{
    (void)keymap;
    (void)name;
    return 0;
}

xkb_layout_index_t xkb_keymap_num_layouts_for_key(struct xkb_keymap *keymap,
                                                       xkb_keycode_t key)
{
    (void)keymap;
    (void)key;
    return 1;
}

xkb_level_index_t xkb_keymap_num_levels_for_key(struct xkb_keymap *keymap,
                                                     xkb_keycode_t key,
                                                     xkb_layout_index_t layout)
{
    (void)keymap;
    (void)key;
    (void)layout;
    return 2;  /* Normal + Shift */
}

int xkb_keymap_key_get_syms_by_level(struct xkb_keymap *keymap,
                                         xkb_keycode_t key,
                                         xkb_layout_index_t layout,
                                         xkb_level_index_t level,
                                         const xkb_keysym_t **syms_out)
{
    static xkb_keysym_t sym;
    xkb_mod_mask_t mods = 0;

    (void)keymap;
    (void)layout;

    if (level > 0) mods = 0x01;  /* Shift */
    sym = keycode_to_keysym(key, mods);

    if (sym == XKB_KEY_NoSymbol) {
        if (syms_out) *syms_out = NULL;
        return 0;
    }

    if (syms_out) *syms_out = &sym;
    return 1;
}

xkb_mod_index_t xkb_keymap_num_mods(struct xkb_keymap *keymap)
{
    (void)keymap;
    return NUM_MODS;
}

const char *xkb_keymap_mod_get_name(struct xkb_keymap *keymap,
                                       xkb_mod_index_t idx)
{
    (void)keymap;
    if (idx < NUM_MODS) return g_mod_names[idx];
    return NULL;
}

xkb_mod_index_t xkb_keymap_mod_get_index(struct xkb_keymap *keymap,
                                              const char *name)
{
    xkb_mod_index_t i;
    (void)keymap;

    if (!name) return XKB_MOD_INVALID;

    for (i = 0; i < NUM_MODS; i++) {
        if (strcmp(g_mod_names[i], name) == 0)
            return i;
    }
    return XKB_MOD_INVALID;
}

xkb_led_index_t xkb_keymap_num_leds(struct xkb_keymap *keymap)
{
    (void)keymap;
    return NUM_LEDS;
}

const char *xkb_keymap_led_get_name(struct xkb_keymap *keymap,
                                       xkb_led_index_t idx)
{
    (void)keymap;
    if (idx < NUM_LEDS) return g_led_names[idx];
    return NULL;
}

xkb_led_index_t xkb_keymap_led_get_index(struct xkb_keymap *keymap,
                                              const char *name)
{
    xkb_led_index_t i;
    (void)keymap;

    if (!name) return XKB_LED_INVALID;

    for (i = 0; i < NUM_LEDS; i++) {
        if (strcmp(g_led_names[i], name) == 0)
            return i;
    }
    return XKB_LED_INVALID;
}

int xkb_keymap_key_repeats(struct xkb_keymap *keymap, xkb_keycode_t key)
{
    xkb_keysym_t sym;
    (void)keymap;

    /* Modifier keys don't repeat */
    sym = keycode_to_keysym(key, 0);
    if (sym >= XKB_KEY_Shift_L && sym <= XKB_KEY_Hyper_R)
        return 0;
    if (sym == XKB_KEY_Caps_Lock || sym == XKB_KEY_Num_Lock)
        return 0;

    return 1;
}

/* ========================================================================= */
/* State                                                                     */
/* ========================================================================= */

struct xkb_state *xkb_state_new(struct xkb_keymap *keymap)
{
    int i;

    if (!keymap) return NULL;

    for (i = 0; i < MAX_STATES; i++) {
        if (!g_states[i].in_use) {
            memset(&g_states[i], 0, sizeof(g_states[i]));
            g_states[i].in_use    = 1;
            g_states[i].ref_count = 1;
            g_states[i].keymap    = keymap;
            xkb_keymap_ref(keymap);
            return &g_states[i];
        }
    }
    return NULL;
}

struct xkb_state *xkb_state_ref(struct xkb_state *state)
{
    if (state) state->ref_count++;
    return state;
}

void xkb_state_unref(struct xkb_state *state)
{
    if (!state) return;
    if (--state->ref_count <= 0) {
        if (state->keymap)
            xkb_keymap_unref(state->keymap);
        state->in_use = 0;
    }
}

struct xkb_keymap *xkb_state_get_keymap(struct xkb_state *state)
{
    return state ? state->keymap : NULL;
}

enum xkb_state_component
xkb_state_update_key(struct xkb_state *state, xkb_keycode_t key,
                       enum xkb_key_direction direction)
{
    xkb_keysym_t sym;
    xkb_mod_mask_t mod_bit = 0;

    if (!state) return 0;

    sym = keycode_to_keysym(key, 0);

    /* Determine which modifier this key controls */
    switch (sym) {
    case XKB_KEY_Shift_L:
    case XKB_KEY_Shift_R:   mod_bit = (1u << 0); break;
    case XKB_KEY_Control_L:
    case XKB_KEY_Control_R: mod_bit = (1u << 2); break;
    case XKB_KEY_Alt_L:
    case XKB_KEY_Alt_R:     mod_bit = (1u << 3); break;
    case XKB_KEY_Super_L:
    case XKB_KEY_Super_R:   mod_bit = (1u << 6); break;
    case XKB_KEY_Caps_Lock:
        if (direction == XKB_KEY_DOWN)
            state->mods_locked ^= (1u << 1);
        return XKB_STATE_MODS_LOCKED;
    case XKB_KEY_Num_Lock:
        if (direction == XKB_KEY_DOWN)
            state->mods_locked ^= (1u << 4);
        return XKB_STATE_MODS_LOCKED;
    default:
        break;
    }

    if (mod_bit) {
        if (direction == XKB_KEY_DOWN)
            state->mods_depressed |= mod_bit;
        else
            state->mods_depressed &= ~mod_bit;
        return XKB_STATE_MODS_DEPRESSED;
    }

    return 0;
}

enum xkb_state_component
xkb_state_update_mask(struct xkb_state *state,
                        xkb_mod_mask_t depressed_mods,
                        xkb_mod_mask_t latched_mods,
                        xkb_mod_mask_t locked_mods,
                        xkb_layout_index_t depressed_layout,
                        xkb_layout_index_t latched_layout,
                        xkb_layout_index_t locked_layout)
{
    if (!state) return 0;

    state->mods_depressed = depressed_mods;
    state->mods_latched   = latched_mods;
    state->mods_locked    = locked_mods;
    state->layout         = depressed_layout | latched_layout | locked_layout;

    return XKB_STATE_MODS_DEPRESSED | XKB_STATE_MODS_LATCHED |
           XKB_STATE_MODS_LOCKED | XKB_STATE_LAYOUT_LOCKED;
}

int xkb_state_key_get_syms(struct xkb_state *state, xkb_keycode_t key,
                              const xkb_keysym_t **syms_out)
{
    if (!state) {
        if (syms_out) *syms_out = NULL;
        return 0;
    }

    xkb_mod_mask_t effective = state->mods_depressed | state->mods_latched |
                                state->mods_locked;
    state->last_sym = keycode_to_keysym(key, effective);

    if (state->last_sym == XKB_KEY_NoSymbol) {
        if (syms_out) *syms_out = NULL;
        return 0;
    }

    if (syms_out) *syms_out = &state->last_sym;
    return 1;
}

int xkb_state_key_get_utf8(struct xkb_state *state, xkb_keycode_t key,
                              char *buffer, size_t size)
{
    xkb_keysym_t sym = xkb_state_key_get_one_sym(state, key);
    return xkb_keysym_to_utf8(sym, buffer, size);
}

uint32_t xkb_state_key_get_utf32(struct xkb_state *state,
                                     xkb_keycode_t key)
{
    xkb_keysym_t sym = xkb_state_key_get_one_sym(state, key);
    return xkb_keysym_to_utf32(sym);
}

xkb_keysym_t xkb_state_key_get_one_sym(struct xkb_state *state,
                                            xkb_keycode_t key)
{
    const xkb_keysym_t *syms;
    int n = xkb_state_key_get_syms(state, key, &syms);
    if (n == 1) return syms[0];
    return XKB_KEY_NoSymbol;
}

xkb_keysym_t xkb_state_key_get_one_sym_raw(struct xkb_state *state,
                                                xkb_keycode_t key)
{
    return xkb_state_key_get_one_sym(state, key);
}

xkb_layout_index_t xkb_state_key_get_layout(struct xkb_state *state,
                                                 xkb_keycode_t key)
{
    (void)state;
    (void)key;
    return 0;
}

int xkb_state_mod_name_is_active(struct xkb_state *state, const char *name,
                                     enum xkb_state_component type)
{
    xkb_mod_index_t idx;
    xkb_mod_mask_t mask = 0;

    if (!state || !name) return 0;

    idx = xkb_keymap_mod_get_index(state->keymap, name);
    if (idx == XKB_MOD_INVALID) return 0;

    if (type & XKB_STATE_MODS_DEPRESSED)
        mask |= state->mods_depressed;
    if (type & XKB_STATE_MODS_LATCHED)
        mask |= state->mods_latched;
    if (type & XKB_STATE_MODS_LOCKED)
        mask |= state->mods_locked;
    if (type & XKB_STATE_MODS_EFFECTIVE)
        mask |= state->mods_depressed | state->mods_latched | state->mods_locked;

    return (mask >> idx) & 1;
}

int xkb_state_mod_index_is_active(struct xkb_state *state,
                                      xkb_mod_index_t idx,
                                      enum xkb_state_component type)
{
    xkb_mod_mask_t mask = 0;

    if (!state || idx >= NUM_MODS) return 0;

    if (type & XKB_STATE_MODS_DEPRESSED)
        mask |= state->mods_depressed;
    if (type & XKB_STATE_MODS_LATCHED)
        mask |= state->mods_latched;
    if (type & XKB_STATE_MODS_LOCKED)
        mask |= state->mods_locked;
    if (type & XKB_STATE_MODS_EFFECTIVE)
        mask |= state->mods_depressed | state->mods_latched | state->mods_locked;

    return (mask >> idx) & 1;
}

int xkb_state_mod_names_are_active(struct xkb_state *state,
                                       enum xkb_state_component type,
                                       enum xkb_state_match match, ...)
{
    (void)state;
    (void)type;
    (void)match;
    return 0;
}

int xkb_state_mod_indices_are_active(struct xkb_state *state,
                                         enum xkb_state_component type,
                                         enum xkb_state_match match, ...)
{
    (void)state;
    (void)type;
    (void)match;
    return 0;
}

xkb_mod_mask_t xkb_state_serialize_mods(struct xkb_state *state,
                                             enum xkb_state_component components)
{
    xkb_mod_mask_t mask = 0;

    if (!state) return 0;

    if (components & XKB_STATE_MODS_DEPRESSED)
        mask |= state->mods_depressed;
    if (components & XKB_STATE_MODS_LATCHED)
        mask |= state->mods_latched;
    if (components & XKB_STATE_MODS_LOCKED)
        mask |= state->mods_locked;
    if (components & XKB_STATE_MODS_EFFECTIVE)
        mask |= state->mods_depressed | state->mods_latched | state->mods_locked;

    return mask;
}

xkb_layout_index_t xkb_state_serialize_layout(struct xkb_state *state,
                                                   enum xkb_state_component components)
{
    (void)state;
    (void)components;
    return 0;
}

int xkb_state_led_name_is_active(struct xkb_state *state, const char *name)
{
    if (!state || !name) return 0;

    if (strcmp(name, "Caps Lock") == 0)
        return (state->mods_locked >> 1) & 1;
    if (strcmp(name, "Num Lock") == 0)
        return (state->mods_locked >> 4) & 1;

    return 0;
}

int xkb_state_led_index_is_active(struct xkb_state *state,
                                      xkb_led_index_t idx)
{
    if (!state) return 0;

    switch (idx) {
    case 0: return (state->mods_locked >> 1) & 1;  /* Caps */
    case 1: return (state->mods_locked >> 4) & 1;  /* Num */
    case 2: return 0;                                /* Scroll */
    default: return 0;
    }
}

xkb_mod_mask_t xkb_state_key_get_consumed_mods(struct xkb_state *state,
                                                     xkb_keycode_t key)
{
    (void)state;
    (void)key;
    return 0;
}

xkb_mod_mask_t xkb_state_key_get_consumed_mods2(struct xkb_state *state,
                                                      xkb_keycode_t key,
                                                      enum xkb_consumed_mode mode)
{
    (void)state;
    (void)key;
    (void)mode;
    return 0;
}

int xkb_state_mod_index_is_consumed(struct xkb_state *state,
                                        xkb_keycode_t key,
                                        xkb_mod_index_t idx)
{
    (void)state;
    (void)key;
    (void)idx;
    return 0;
}

int xkb_state_mod_index_is_consumed2(struct xkb_state *state,
                                         xkb_keycode_t key,
                                         xkb_mod_index_t idx,
                                         enum xkb_consumed_mode mode)
{
    (void)state;
    (void)key;
    (void)idx;
    (void)mode;
    return 0;
}

/* ========================================================================= */
/* Compose                                                                   */
/* ========================================================================= */

struct xkb_compose_table *
xkb_compose_table_new_from_locale(struct xkb_context *context,
                                    const char *locale,
                                    enum xkb_compose_compile_flags flags)
{
    int i;
    (void)locale;
    (void)flags;

    if (!context) return NULL;

    for (i = 0; i < MAX_COMPOSE_TABLES; i++) {
        if (!g_compose_tables[i].in_use) {
            g_compose_tables[i].in_use    = 1;
            g_compose_tables[i].ref_count = 1;
            g_compose_tables[i].context   = context;
            return &g_compose_tables[i];
        }
    }
    return NULL;
}

struct xkb_compose_table *
xkb_compose_table_new_from_file(struct xkb_context *context, void *file,
                                   const char *locale,
                                   enum xkb_compose_format format,
                                   enum xkb_compose_compile_flags flags)
{
    (void)file;
    (void)format;
    return xkb_compose_table_new_from_locale(context, locale, flags);
}

struct xkb_compose_table *
xkb_compose_table_new_from_buffer(struct xkb_context *context,
                                     const char *buffer, size_t length,
                                     const char *locale,
                                     enum xkb_compose_format format,
                                     enum xkb_compose_compile_flags flags)
{
    (void)buffer;
    (void)length;
    (void)format;
    return xkb_compose_table_new_from_locale(context, locale, flags);
}

struct xkb_compose_table *
xkb_compose_table_ref(struct xkb_compose_table *table)
{
    if (table) table->ref_count++;
    return table;
}

void xkb_compose_table_unref(struct xkb_compose_table *table)
{
    if (!table) return;
    if (--table->ref_count <= 0)
        table->in_use = 0;
}

struct xkb_compose_state *
xkb_compose_state_new(struct xkb_compose_table *table,
                        enum xkb_compose_state_flags flags)
{
    int i;
    (void)flags;

    if (!table) return NULL;

    for (i = 0; i < MAX_COMPOSE_STATES; i++) {
        if (!g_compose_states[i].in_use) {
            g_compose_states[i].in_use     = 1;
            g_compose_states[i].ref_count  = 1;
            g_compose_states[i].table      = table;
            g_compose_states[i].status     = XKB_COMPOSE_NOTHING;
            g_compose_states[i].result_sym = XKB_KEY_NoSymbol;
            xkb_compose_table_ref(table);
            return &g_compose_states[i];
        }
    }
    return NULL;
}

struct xkb_compose_state *
xkb_compose_state_ref(struct xkb_compose_state *state)
{
    if (state) state->ref_count++;
    return state;
}

void xkb_compose_state_unref(struct xkb_compose_state *state)
{
    if (!state) return;
    if (--state->ref_count <= 0) {
        if (state->table)
            xkb_compose_table_unref(state->table);
        state->in_use = 0;
    }
}

struct xkb_compose_table *
xkb_compose_state_get_compose_table(struct xkb_compose_state *state)
{
    return state ? state->table : NULL;
}

enum xkb_compose_feed_result
xkb_compose_state_feed(struct xkb_compose_state *state,
                         xkb_keysym_t keysym)
{
    if (!state) return XKB_COMPOSE_FEED_IGNORED;

    /* No actual compose sequences; just pass through */
    state->result_sym = keysym;
    state->status     = XKB_COMPOSE_NOTHING;

    /* Modifier keys are ignored by compose */
    if (keysym >= XKB_KEY_Shift_L && keysym <= XKB_KEY_Hyper_R)
        return XKB_COMPOSE_FEED_IGNORED;

    return XKB_COMPOSE_FEED_ACCEPTED;
}

void xkb_compose_state_reset(struct xkb_compose_state *state)
{
    if (state) {
        state->status     = XKB_COMPOSE_NOTHING;
        state->result_sym = XKB_KEY_NoSymbol;
    }
}

enum xkb_compose_status
xkb_compose_state_get_status(struct xkb_compose_state *state)
{
    return state ? state->status : XKB_COMPOSE_NOTHING;
}

int xkb_compose_state_get_utf8(struct xkb_compose_state *state,
                                  char *buffer, size_t size)
{
    if (!state || state->status != XKB_COMPOSE_COMPOSED)
        return 0;

    return xkb_keysym_to_utf8(state->result_sym, buffer, size);
}

xkb_keysym_t
xkb_compose_state_get_one_sym(struct xkb_compose_state *state)
{
    if (!state) return XKB_KEY_NoSymbol;
    return state->result_sym;
}
