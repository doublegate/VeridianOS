/*
 * edit -- VeridianOS nano-inspired text editor
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * A purpose-built text editor for VeridianOS providing essential nano-like
 * functionality using the VeridianOS curses library (libcurses.a).
 *
 * Features:
 *   - Full-screen TUI with status bar and key binding help bar
 *   - File open, save, save-as
 *   - Text editing: insert, delete, backspace, newline
 *   - Cursor movement: arrow keys, Home, End, PgUp, PgDn
 *   - Search forward (Ctrl+W)
 *   - Cut line (Ctrl+K) / Paste line (Ctrl+U)
 *   - Help screen (Ctrl+G)
 *   - Prompt to save on exit if modified
 *
 * Key bindings:
 *   Ctrl+X   Exit (prompt to save if modified)
 *   Ctrl+O   Save (prompt for filename if new file)
 *   Ctrl+W   Search forward
 *   Ctrl+K   Cut current line
 *   Ctrl+U   Paste cut line
 *   Ctrl+G   Help screen
 *   Ctrl+A   Beginning of line
 *   Ctrl+E   End of line
 *   Arrows   Move cursor
 *   Home     Beginning of line
 *   End      End of line
 *   PgUp     Page up
 *   PgDn     Page down
 *   Del      Delete character at cursor
 *   Bksp     Delete character before cursor
 */

#include <curses.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

#define INITIAL_CAP    256    /* Initial line buffer capacity */
#define INITIAL_LINESZ 128   /* Initial character capacity per line */
#define TAB_STOP       4     /* Tab display width */
#define VERSION        "1.0.0"
#define PROGRAM_NAME   "edit"

/* Control key macro: Ctrl+A = 1, Ctrl+B = 2, ... */
#define CTRL(c) ((c) & 0x1F)

/* ========================================================================= */
/* Editor state                                                              */
/* ========================================================================= */

/* A single line of text (dynamically sized). */
struct line {
    char *data;    /* NUL-terminated string */
    int   len;     /* Current length (excluding NUL) */
    int   cap;     /* Allocated capacity */
};

/* The editor buffer. */
static struct line *lines  = NULL;   /* Array of lines */
static int num_lines       = 0;      /* Number of lines */
static int cap_lines       = 0;      /* Allocated line slots */

/* Cursor position in the buffer (0-indexed). */
static int cx = 0;   /* Column within the line */
static int cy = 0;   /* Row (line number) in the buffer */

/* Scroll offset: the buffer row displayed at the top of the text area. */
static int row_offset = 0;
/* Horizontal scroll offset. */
static int col_offset = 0;

/* File state. */
static char filename[1024] = {0};
static int  modified        = 0;

/* Cut buffer (for Ctrl+K / Ctrl+U). */
static char *cut_buf = NULL;

/* Status message (displayed briefly). */
static char status_msg[256] = {0};

/* ========================================================================= */
/* Line management helpers                                                   */
/* ========================================================================= */

/*
 * Initialise a line structure with a given string (may be NULL for empty).
 */
static void line_init(struct line *ln, const char *s)
{
    int slen = s ? (int)strlen(s) : 0;
    ln->cap = slen + INITIAL_LINESZ;
    ln->data = (char *)malloc(ln->cap);
    if (!ln->data) {
        ln->cap = 0;
        ln->len = 0;
        return;
    }
    if (s && slen > 0)
        memcpy(ln->data, s, slen);
    ln->data[slen] = '\0';
    ln->len = slen;
}

/*
 * Ensure a line has room for at least `needed` more bytes.
 */
static void line_grow(struct line *ln, int needed)
{
    if (ln->len + needed + 1 <= ln->cap)
        return;
    int newcap = ln->cap * 2;
    if (newcap < ln->len + needed + 1)
        newcap = ln->len + needed + INITIAL_LINESZ;
    char *tmp = (char *)realloc(ln->data, newcap);
    if (!tmp) return;
    ln->data = tmp;
    ln->cap = newcap;
}

/*
 * Free a line's data.
 */
static void line_free(struct line *ln)
{
    free(ln->data);
    ln->data = NULL;
    ln->len = 0;
    ln->cap = 0;
}

/*
 * Insert a character at position `pos` within a line.
 */
static void line_insert_char(struct line *ln, int pos, char ch)
{
    if (pos < 0) pos = 0;
    if (pos > ln->len) pos = ln->len;
    line_grow(ln, 1);
    memmove(ln->data + pos + 1, ln->data + pos, ln->len - pos + 1);
    ln->data[pos] = ch;
    ln->len++;
}

/*
 * Delete the character at position `pos` within a line.
 */
static void line_delete_char(struct line *ln, int pos)
{
    if (pos < 0 || pos >= ln->len) return;
    memmove(ln->data + pos, ln->data + pos + 1, ln->len - pos);
    ln->len--;
}

/* ========================================================================= */
/* Buffer management                                                         */
/* ========================================================================= */

/*
 * Ensure the lines array has room for at least one more line.
 */
static void buf_grow(void)
{
    if (num_lines < cap_lines) return;
    int newcap = cap_lines == 0 ? INITIAL_CAP : cap_lines * 2;
    struct line *tmp = (struct line *)realloc(lines, newcap * sizeof(struct line));
    if (!tmp) return;
    lines = tmp;
    cap_lines = newcap;
}

/*
 * Insert an empty line at position `at`.
 */
static void buf_insert_line(int at, const char *s)
{
    if (at < 0) at = 0;
    if (at > num_lines) at = num_lines;
    buf_grow();
    /* Shift lines down */
    if (at < num_lines)
        memmove(&lines[at + 1], &lines[at],
                (num_lines - at) * sizeof(struct line));
    line_init(&lines[at], s);
    num_lines++;
}

/*
 * Delete the line at position `at`.
 */
static void buf_delete_line(int at)
{
    if (at < 0 || at >= num_lines) return;
    line_free(&lines[at]);
    if (at < num_lines - 1)
        memmove(&lines[at], &lines[at + 1],
                (num_lines - at - 1) * sizeof(struct line));
    num_lines--;
    if (num_lines == 0) {
        /* Always keep at least one line */
        buf_insert_line(0, "");
    }
}

/*
 * Free all buffer lines.
 */
static void buf_free(void)
{
    for (int i = 0; i < num_lines; i++)
        line_free(&lines[i]);
    free(lines);
    lines = NULL;
    num_lines = 0;
    cap_lines = 0;
}

/* ========================================================================= */
/* File I/O                                                                  */
/* ========================================================================= */

/*
 * Load a file into the buffer.  Returns 0 on success, -1 on failure.
 */
static int load_file(const char *path)
{
    FILE *fp = fopen(path, "r");
    if (!fp) return -1;

    buf_free();

    char buf[4096];
    while (fgets(buf, (int)sizeof(buf), fp)) {
        /* Strip trailing newline */
        int len = (int)strlen(buf);
        while (len > 0 && (buf[len - 1] == '\n' || buf[len - 1] == '\r'))
            buf[--len] = '\0';
        buf_insert_line(num_lines, buf);
    }
    fclose(fp);

    if (num_lines == 0)
        buf_insert_line(0, "");

    return 0;
}

/*
 * Save the buffer to a file.  Returns 0 on success, -1 on failure.
 */
static int save_file(const char *path)
{
    FILE *fp = fopen(path, "w");
    if (!fp) return -1;

    for (int i = 0; i < num_lines; i++) {
        fputs(lines[i].data, fp);
        fputc('\n', fp);
    }
    fclose(fp);
    modified = 0;
    return 0;
}

/* ========================================================================= */
/* Screen geometry helpers                                                   */
/* ========================================================================= */

/* Number of rows available for text (LINES minus status bar minus help bar). */
static int text_rows(void)
{
    return LINES - 2;
}

/* ========================================================================= */
/* Prompt (mini-buffer input at status bar line)                              */
/* ========================================================================= */

/*
 * Display a prompt at the status bar and read a string.
 * Returns the input string (into caller's buffer), or NULL if cancelled
 * (Ctrl+C / Escape).
 */
static char *prompt(const char *msg, char *buf, int bufsz)
{
    int len = 0;
    buf[0] = '\0';

    for (;;) {
        /* Draw the prompt on the status bar line */
        int row = LINES - 2;
        move(row, 0);
        attron(A_REVERSE);
        printw("%-*s", COLS, "");
        move(row, 0);
        printw("%s%s", msg, buf);
        attroff(A_REVERSE);
        /* Position cursor after the input */
        move(row, (int)strlen(msg) + len);
        refresh();

        int ch = getch();
        if (ch == 27 || ch == CTRL('C')) {
            /* Cancel */
            return NULL;
        } else if (ch == '\n' || ch == '\r' || ch == KEY_ENTER) {
            return buf;
        } else if (ch == KEY_BACKSPACE || ch == 127 || ch == 8) {
            if (len > 0) {
                len--;
                buf[len] = '\0';
            }
        } else if (ch >= 0x20 && ch < 0x7F && len < bufsz - 1) {
            buf[len++] = (char)ch;
            buf[len] = '\0';
        }
    }
}

/* ========================================================================= */
/* Drawing                                                                   */
/* ========================================================================= */

/*
 * Convert a buffer column to a screen column, expanding tabs.
 */
static int col_to_screen(const struct line *ln, int col)
{
    int sx = 0;
    for (int i = 0; i < col && i < ln->len; i++) {
        if (ln->data[i] == '\t')
            sx += TAB_STOP - (sx % TAB_STOP);
        else
            sx++;
    }
    return sx;
}

/*
 * Draw the full screen: text area + status bar + help bar.
 */
static void draw_screen(void)
{
    int trows = text_rows();
    int tcols = COLS;

    /* Compute screen column of cursor for horizontal scrolling */
    int screen_cx = 0;
    if (cy >= 0 && cy < num_lines)
        screen_cx = col_to_screen(&lines[cy], cx);

    /* Adjust col_offset so cursor is visible */
    if (screen_cx < col_offset)
        col_offset = screen_cx;
    if (screen_cx >= col_offset + tcols)
        col_offset = screen_cx - tcols + 1;

    /* Draw text lines */
    for (int y = 0; y < trows; y++) {
        int file_row = y + row_offset;
        move(y, 0);
        clrtoeol();
        if (file_row < num_lines) {
            struct line *ln = &lines[file_row];
            /* Render the line with tab expansion, applying col_offset */
            int sx = 0;  /* screen x */
            for (int i = 0; i < ln->len; i++) {
                char ch = ln->data[i];
                if (ch == '\t') {
                    int spaces = TAB_STOP - (sx % TAB_STOP);
                    for (int s = 0; s < spaces; s++) {
                        if (sx >= col_offset && sx < col_offset + tcols)
                            mvaddch(y, sx - col_offset, ' ');
                        sx++;
                    }
                } else {
                    if (sx >= col_offset && sx < col_offset + tcols)
                        mvaddch(y, sx - col_offset, (chtype)(unsigned char)ch);
                    sx++;
                }
            }
        } else {
            /* Empty line marker */
            mvaddch(y, 0, '~');
        }
    }

    /* ---- Status bar (second to last line) ---- */
    attron(A_REVERSE);
    move(LINES - 2, 0);
    {
        char left[512];
        char right[128];

        /* Left side: filename + modified indicator */
        const char *fn = filename[0] ? filename : "[New File]";
        int fnlen = (int)strlen(fn);
        if (fnlen > 40) {
            /* Truncate long filenames */
            snprintf(left, sizeof(left), "[ ...%s ]%s",
                     fn + fnlen - 36, modified ? "  Modified" : "");
        } else {
            snprintf(left, sizeof(left), "[ %s ]%s",
                     fn, modified ? "  Modified" : "");
        }

        /* Right side: line/col position */
        snprintf(right, sizeof(right), "Line %d/%d, Col %d  ",
                 cy + 1, num_lines, cx + 1);

        /* Print status bar */
        int left_len = (int)strlen(left);
        int right_len = (int)strlen(right);
        int padding = COLS - left_len - right_len;
        if (padding < 1) padding = 1;

        printw("%s", left);
        for (int i = 0; i < padding; i++)
            addch(' ');
        printw("%s", right);
    }
    attroff(A_REVERSE);

    /* ---- Status message (if set, overwrite status bar briefly) ---- */
    if (status_msg[0]) {
        move(LINES - 2, 0);
        attron(A_REVERSE);
        printw("%-*s", COLS, status_msg);
        attroff(A_REVERSE);
        status_msg[0] = '\0';
    }

    /* ---- Help bar (last line) ---- */
    attron(A_BOLD);
    mvprintw(LINES - 1, 0, "%-*s", COLS, "");
    mvprintw(LINES - 1, 0,
        "^X Exit  ^O Save  ^W Search  ^K Cut  ^U Paste  ^G Help");
    attroff(A_BOLD);

    /* Position the cursor */
    {
        int scr_y = cy - row_offset;
        int scr_x = screen_cx - col_offset;
        if (scr_y >= 0 && scr_y < trows && scr_x >= 0 && scr_x < tcols)
            move(scr_y, scr_x);
    }

    refresh();
}

/* ========================================================================= */
/* Scroll adjustment                                                         */
/* ========================================================================= */

/*
 * Ensure the cursor is within the visible text area and adjust
 * row_offset as needed.
 */
static void scroll_to_cursor(void)
{
    int trows = text_rows();
    if (cy < row_offset)
        row_offset = cy;
    if (cy >= row_offset + trows)
        row_offset = cy - trows + 1;
    if (row_offset < 0)
        row_offset = 0;
}

/* ========================================================================= */
/* Cursor movement                                                           */
/* ========================================================================= */

static void move_left(void)
{
    if (cx > 0) {
        cx--;
    } else if (cy > 0) {
        /* Wrap to end of previous line */
        cy--;
        cx = lines[cy].len;
    }
}

static void move_right(void)
{
    if (cy < num_lines) {
        if (cx < lines[cy].len) {
            cx++;
        } else if (cy < num_lines - 1) {
            /* Wrap to start of next line */
            cy++;
            cx = 0;
        }
    }
}

static void move_up(void)
{
    if (cy > 0) {
        cy--;
        if (cx > lines[cy].len)
            cx = lines[cy].len;
    }
}

static void move_down(void)
{
    if (cy < num_lines - 1) {
        cy++;
        if (cx > lines[cy].len)
            cx = lines[cy].len;
    }
}

static void move_home(void)
{
    cx = 0;
}

static void move_end(void)
{
    if (cy < num_lines)
        cx = lines[cy].len;
}

static void move_page_up(void)
{
    int trows = text_rows();
    cy -= trows;
    if (cy < 0) cy = 0;
    row_offset -= trows;
    if (row_offset < 0) row_offset = 0;
    if (cx > lines[cy].len) cx = lines[cy].len;
}

static void move_page_down(void)
{
    int trows = text_rows();
    cy += trows;
    if (cy >= num_lines) cy = num_lines - 1;
    if (cy < 0) cy = 0;
    row_offset += trows;
    if (row_offset > num_lines - trows)
        row_offset = num_lines - trows;
    if (row_offset < 0) row_offset = 0;
    if (cx > lines[cy].len) cx = lines[cy].len;
}

/* ========================================================================= */
/* Editing operations                                                        */
/* ========================================================================= */

/*
 * Insert a character at the current cursor position.
 */
static void insert_char(char ch)
{
    if (cy >= num_lines) return;
    line_insert_char(&lines[cy], cx, ch);
    cx++;
    modified = 1;
}

/*
 * Handle Enter key: split the current line at the cursor.
 */
static void insert_newline(void)
{
    if (cy >= num_lines) return;
    struct line *ln = &lines[cy];
    /* Text after cursor goes to new line */
    const char *tail = ln->data + cx;
    buf_insert_line(cy + 1, tail);
    /* Truncate current line at cursor */
    ln->data[cx] = '\0';
    ln->len = cx;
    /* Move cursor to start of new line */
    cy++;
    cx = 0;
    modified = 1;
}

/*
 * Delete the character at the cursor position (Delete key).
 */
static void delete_char(void)
{
    if (cy >= num_lines) return;
    struct line *ln = &lines[cy];
    if (cx < ln->len) {
        line_delete_char(ln, cx);
        modified = 1;
    } else if (cy < num_lines - 1) {
        /* Join next line to end of current line */
        struct line *next = &lines[cy + 1];
        line_grow(ln, next->len);
        memcpy(ln->data + ln->len, next->data, next->len + 1);
        ln->len += next->len;
        buf_delete_line(cy + 1);
        modified = 1;
    }
}

/*
 * Backspace: delete character before cursor.
 */
static void backspace(void)
{
    if (cx > 0) {
        cx--;
        delete_char();
    } else if (cy > 0) {
        /* Join current line to end of previous line */
        int prev_len = lines[cy - 1].len;
        struct line *prev = &lines[cy - 1];
        struct line *cur  = &lines[cy];
        line_grow(prev, cur->len);
        memcpy(prev->data + prev->len, cur->data, cur->len + 1);
        prev->len += cur->len;
        buf_delete_line(cy);
        cy--;
        cx = prev_len;
        modified = 1;
    }
}

/*
 * Insert a tab character.
 */
static void insert_tab(void)
{
    insert_char('\t');
}

/* ========================================================================= */
/* Cut / Paste                                                               */
/* ========================================================================= */

/*
 * Cut the current line (Ctrl+K).
 */
static void cut_line(void)
{
    if (cy >= num_lines) return;

    /* Store cut line */
    free(cut_buf);
    cut_buf = strdup(lines[cy].data);

    buf_delete_line(cy);
    if (cy >= num_lines && cy > 0)
        cy = num_lines - 1;
    if (cx > lines[cy].len)
        cx = lines[cy].len;
    modified = 1;

    snprintf(status_msg, sizeof(status_msg), "Cut 1 line");
}

/*
 * Paste the cut buffer (Ctrl+U).
 */
static void paste_line(void)
{
    if (!cut_buf) {
        snprintf(status_msg, sizeof(status_msg), "Cut buffer is empty");
        return;
    }

    buf_insert_line(cy, cut_buf);
    cy++;
    if (cx > lines[cy].len)
        cx = lines[cy].len;
    modified = 1;

    snprintf(status_msg, sizeof(status_msg), "Pasted 1 line");
}

/* ========================================================================= */
/* Search                                                                    */
/* ========================================================================= */

/*
 * Search forward for a string (Ctrl+W).
 */
static void search(void)
{
    char query[256];
    char *result = prompt("Search: ", query, sizeof(query));
    if (!result || query[0] == '\0') {
        snprintf(status_msg, sizeof(status_msg), "Search cancelled");
        return;
    }

    /* Search forward from current position */
    for (int i = cy; i < num_lines; i++) {
        int start = (i == cy) ? cx + 1 : 0;
        if (start > lines[i].len) continue;
        char *found = strstr(lines[i].data + start, query);
        if (found) {
            cy = i;
            cx = (int)(found - lines[i].data);
            scroll_to_cursor();
            snprintf(status_msg, sizeof(status_msg), "Found at line %d", cy + 1);
            return;
        }
    }

    /* Wrap around: search from beginning */
    for (int i = 0; i <= cy; i++) {
        int end = (i == cy) ? cx : lines[i].len;
        char *found = strstr(lines[i].data, query);
        if (found) {
            int pos = (int)(found - lines[i].data);
            if (i < cy || pos <= end) {
                cy = i;
                cx = pos;
                scroll_to_cursor();
                snprintf(status_msg, sizeof(status_msg),
                         "Found at line %d (wrapped)", cy + 1);
                return;
            }
        }
    }

    snprintf(status_msg, sizeof(status_msg), "\"%s\" not found", query);
}

/* ========================================================================= */
/* Save handler                                                              */
/* ========================================================================= */

/*
 * Save the file (Ctrl+O).  Prompts for filename if none set.
 * Returns 0 on success, -1 on cancel/failure.
 */
static int do_save(void)
{
    if (filename[0] == '\0') {
        /* Prompt for filename */
        char fn_buf[1024];
        char *result = prompt("File Name to Write: ", fn_buf, sizeof(fn_buf));
        if (!result || fn_buf[0] == '\0') {
            snprintf(status_msg, sizeof(status_msg), "Save cancelled");
            return -1;
        }
        strncpy(filename, fn_buf, sizeof(filename) - 1);
        filename[sizeof(filename) - 1] = '\0';
    }

    if (save_file(filename) == 0) {
        snprintf(status_msg, sizeof(status_msg),
                 "Wrote %d lines to %s", num_lines, filename);
        return 0;
    } else {
        snprintf(status_msg, sizeof(status_msg),
                 "Error writing %s", filename);
        return -1;
    }
}

/* ========================================================================= */
/* Help screen                                                               */
/* ========================================================================= */

/*
 * Display a full-screen help overlay (Ctrl+G).
 */
static void show_help(void)
{
    clear();

    int row = 1;
    attron(A_BOLD);
    mvprintw(row++, 2, "%s %s -- Help", PROGRAM_NAME, VERSION);
    attroff(A_BOLD);
    row++;

    mvprintw(row++, 2, "A nano-inspired text editor for VeridianOS.");
    row++;

    attron(A_BOLD);
    mvprintw(row++, 2, "Key Bindings:");
    attroff(A_BOLD);
    row++;

    mvprintw(row++, 4, "Ctrl+X       Exit (prompt to save if modified)");
    mvprintw(row++, 4, "Ctrl+O       Save file (prompt for name if new)");
    mvprintw(row++, 4, "Ctrl+W       Search forward");
    mvprintw(row++, 4, "Ctrl+K       Cut current line");
    mvprintw(row++, 4, "Ctrl+U       Paste cut line");
    mvprintw(row++, 4, "Ctrl+G       This help screen");
    row++;
    mvprintw(row++, 4, "Ctrl+A       Move to beginning of line");
    mvprintw(row++, 4, "Ctrl+E       Move to end of line");
    mvprintw(row++, 4, "Arrow keys   Move cursor");
    mvprintw(row++, 4, "Home / End   Beginning / end of line");
    mvprintw(row++, 4, "PgUp / PgDn  Page up / down");
    mvprintw(row++, 4, "Delete       Delete character at cursor");
    mvprintw(row++, 4, "Backspace    Delete character before cursor");
    mvprintw(row++, 4, "Tab          Insert tab character");
    row++;

    attron(A_REVERSE);
    mvprintw(LINES - 1, 0, "%-*s", COLS,
             "  Press any key to return to editing...");
    attroff(A_REVERSE);

    refresh();
    getch();

    /* Force full redraw */
    clear();
}

/* ========================================================================= */
/* Exit handler                                                              */
/* ========================================================================= */

/*
 * Handle exit (Ctrl+X).  Prompts to save if modified.
 * Returns 1 if the editor should quit, 0 to stay.
 */
static int do_quit(void)
{
    if (!modified)
        return 1;

    /* Prompt to save */
    char yn[8];
    char *result = prompt("Save modified buffer? (y/n/c): ", yn, sizeof(yn));
    if (!result || yn[0] == 'c' || yn[0] == 'C') {
        snprintf(status_msg, sizeof(status_msg), "Exit cancelled");
        return 0;
    }
    if (yn[0] == 'y' || yn[0] == 'Y') {
        if (do_save() == 0)
            return 1;
        /* Save failed; stay in editor */
        return 0;
    }
    /* 'n' or anything else: discard changes */
    return 1;
}

/* ========================================================================= */
/* Key dispatch                                                              */
/* ========================================================================= */

/*
 * Handle a single keypress.  Returns 1 if the editor should quit.
 */
static int handle_key(int ch)
{
    switch (ch) {
    /* ---- Control keys ---- */
    case CTRL('X'):
        return do_quit();

    case CTRL('O'):
        do_save();
        break;

    case CTRL('W'):
        search();
        break;

    case CTRL('K'):
        cut_line();
        break;

    case CTRL('U'):
        paste_line();
        break;

    case CTRL('G'):
        show_help();
        break;

    case CTRL('A'):
        move_home();
        break;

    case CTRL('E'):
        move_end();
        break;

    /* ---- Navigation keys ---- */
    case KEY_UP:
        move_up();
        break;

    case KEY_DOWN:
        move_down();
        break;

    case KEY_LEFT:
        move_left();
        break;

    case KEY_RIGHT:
        move_right();
        break;

    case KEY_HOME:
        move_home();
        break;

    case KEY_END:
        move_end();
        break;

    case KEY_PPAGE:
        move_page_up();
        break;

    case KEY_NPAGE:
        move_page_down();
        break;

    /* ---- Editing keys ---- */
    case KEY_BACKSPACE:
    case 127:
    case 8:
        backspace();
        break;

    case KEY_DC:
        delete_char();
        break;

    case '\n':
    case '\r':
    case KEY_ENTER:
        insert_newline();
        break;

    case '\t':
        insert_tab();
        break;

    case ERR:
        /* No input (non-blocking mode or error) */
        break;

    default:
        /* Insert printable characters */
        if (ch >= 0x20 && ch < 0x7F) {
            insert_char((char)ch);
        }
        break;
    }

    return 0;
}

/* ========================================================================= */
/* Main                                                                      */
/* ========================================================================= */

int main(int argc, char *argv[])
{
    /* Parse arguments: edit [filename] */
    if (argc >= 2) {
        strncpy(filename, argv[1], sizeof(filename) - 1);
        filename[sizeof(filename) - 1] = '\0';
    }

    /* Initialize the buffer */
    if (filename[0]) {
        if (load_file(filename) != 0) {
            /* New file: start with one empty line */
            buf_insert_line(0, "");
            snprintf(status_msg, sizeof(status_msg),
                     "[New File] %s", filename);
        } else {
            snprintf(status_msg, sizeof(status_msg),
                     "Read %d lines from %s", num_lines, filename);
        }
    } else {
        buf_insert_line(0, "");
        snprintf(status_msg, sizeof(status_msg),
                 "%s %s  -- ^G for help", PROGRAM_NAME, VERSION);
    }

    /* Initialize curses */
    initscr();
    raw();
    noecho();
    keypad(stdscr, TRUE);
    curs_set(1);

    /* Main loop */
    for (;;) {
        scroll_to_cursor();
        draw_screen();

        int ch = getch();
        if (handle_key(ch))
            break;
    }

    /* Cleanup */
    endwin();
    buf_free();
    free(cut_buf);

    return 0;
}
