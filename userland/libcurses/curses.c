/*
 * VeridianOS libcurses -- curses.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal curses implementation using ANSI escape sequences and termios.
 * Provides enough functionality for nano and basic TUI applications.
 *
 * Architecture:
 *   - Terminal set to raw mode via termios
 *   - Virtual screen buffer per WINDOW
 *   - refresh() diffs virtual vs physical screen, emits ANSI escapes
 *   - getch() reads raw bytes, parses escape sequences for special keys
 *   - Colors via ANSI SGR sequences (8 basic colors)
 *   - Terminal size via TIOCGWINSZ ioctl
 */

#include "curses.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <stdarg.h>

/*
 * Local cfmakeraw implementation.
 * Some sysroot libc.a builds may not include cfmakeraw (BSD extension).
 * This avoids a link dependency on the libc version.
 */
static void _cfmakeraw(struct termios *t)
{
    t->c_iflag &= ~(unsigned int)(IGNBRK | BRKINT | PARMRK | ISTRIP |
                                   INLCR | IGNCR | ICRNL | IXON);
    t->c_oflag &= ~(unsigned int)OPOST;
    t->c_lflag &= ~(unsigned int)(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
    t->c_cflag &= ~(unsigned int)(CSIZE | PARENB);
    t->c_cflag |= CS8;
    t->c_cc[VMIN]  = 1;
    t->c_cc[VTIME] = 0;
}

/* ========================================================================= */
/* Internal state                                                            */
/* ========================================================================= */

WINDOW *stdscr = NULL;
WINDOW *curscr = NULL;
int LINES = 24;
int COLS  = 80;
chtype acs_map[128];

/* Saved terminal state for endwin() restoration */
static struct termios _saved_termios;
static int _termios_saved = 0;
static int _curses_active = 0;
static int _cursor_visible = 1;

/* Echo mode (default: on until noecho() called) */
static int _echo_mode = 1;
/* Raw mode flags */
static int _raw_mode = 0;
static int _cbreak_mode = 0;
/* NL translation */
static int _nl_mode = 1;

/* Color pair table: [pair_number] = { fg, bg } */
static struct { short fg; short bg; } _color_pairs[COLOR_PAIRS];
static int _colors_started = 0;
static int _default_colors = 0;

/* Ungetch buffer */
#define UNGETCH_MAX 32
static int _ungetch_buf[UNGETCH_MAX];
static int _ungetch_count = 0;

/* Write buffer for batching terminal output */
#define OUTBUF_SIZE 8192
static char _outbuf[OUTBUF_SIZE];
static int  _outbuf_pos = 0;

/* ========================================================================= */
/* Low-level terminal I/O                                                    */
/* ========================================================================= */

static void _flush_out(void)
{
    if (_outbuf_pos > 0) {
        write(STDOUT_FILENO, _outbuf, _outbuf_pos);
        _outbuf_pos = 0;
    }
}

static void _put_raw(const char *s, int len)
{
    while (len > 0) {
        int space = OUTBUF_SIZE - _outbuf_pos;
        if (space <= 0) {
            _flush_out();
            space = OUTBUF_SIZE;
        }
        int chunk = len < space ? len : space;
        memcpy(_outbuf + _outbuf_pos, s, chunk);
        _outbuf_pos += chunk;
        s += chunk;
        len -= chunk;
    }
}

static void _put_str(const char *s)
{
    _put_raw(s, strlen(s));
}

/* Simple integer-to-string for escape sequences (avoids sprintf dependency) */
static int _itoa(int val, char *buf)
{
    if (val == 0) {
        buf[0] = '0';
        return 1;
    }
    char tmp[12];
    int i = 0;
    int neg = 0;
    if (val < 0) { neg = 1; val = -val; }
    while (val > 0) {
        tmp[i++] = '0' + (val % 10);
        val /= 10;
    }
    int pos = 0;
    if (neg) buf[pos++] = '-';
    while (i > 0) buf[pos++] = tmp[--i];
    return pos;
}

/* Move cursor to (row, col) -- 1-based for ANSI */
static void _move_cursor(int row, int col)
{
    char buf[24];
    int pos = 0;
    buf[pos++] = '\033';
    buf[pos++] = '[';
    pos += _itoa(row + 1, buf + pos);
    buf[pos++] = ';';
    pos += _itoa(col + 1, buf + pos);
    buf[pos++] = 'H';
    _put_raw(buf, pos);
}

/* Apply SGR (Select Graphic Rendition) for attributes + color pair */
static void _apply_attrs(attr_t attrs)
{
    /* Reset first */
    _put_str("\033[0");

    if (attrs & A_BOLD)      _put_str(";1");
    if (attrs & A_DIM)       _put_str(";2");
    if (attrs & A_UNDERLINE) _put_str(";4");
    if (attrs & A_BLINK)     _put_str(";5");
    if (attrs & A_REVERSE)   _put_str(";7");
    if (attrs & A_INVIS)     _put_str(";8");
    if (attrs & A_STANDOUT)  _put_str(";7");  /* standout = reverse */

    /* Color pair */
    int pair = PAIR_NUMBER(attrs);
    if (pair > 0 && pair < COLOR_PAIRS && _colors_started) {
        short fg = _color_pairs[pair].fg;
        short bg = _color_pairs[pair].bg;
        char num[16];
        int n;
        if (fg >= 0 && fg < 8) {
            _put_str(";3");
            n = _itoa(fg, num);
            _put_raw(num, n);
        } else if (_default_colors && fg == -1) {
            _put_str(";39");
        }
        if (bg >= 0 && bg < 8) {
            _put_str(";4");
            n = _itoa(bg, num);
            _put_raw(num, n);
        } else if (_default_colors && bg == -1) {
            _put_str(";49");
        }
    }

    _put_str("m");
}

/* ========================================================================= */
/* Terminal size detection                                                   */
/* ========================================================================= */

static void _get_terminal_size(void)
{
    struct winsize ws;
    if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) == 0 && ws.ws_row > 0 && ws.ws_col > 0) {
        LINES = ws.ws_row;
        COLS  = ws.ws_col;
    } else {
        /* Fallback: try to detect via cursor position query */
        LINES = 24;
        COLS  = 80;
    }
}

/* ========================================================================= */
/* Window management                                                         */
/* ========================================================================= */

static WINDOW *_alloc_window(int nlines, int ncols, int begin_y, int begin_x)
{
    WINDOW *win = (WINDOW *)calloc(1, sizeof(WINDOW));
    if (!win) return NULL;

    win->_maxy = nlines;
    win->_maxx = ncols;
    win->_begy = begin_y;
    win->_begx = begin_x;
    win->_cury = 0;
    win->_curx = 0;
    win->_attrs = A_NORMAL;
    win->_bkgd = ' ';
    win->_scroll = 0;
    win->_keypad = 0;
    win->_nodelay = 0;
    win->_clearok = 1; /* Force full draw on first refresh */

    int size = nlines * ncols;
    if (size > 0) {
        win->_line = (chtype *)calloc(size, sizeof(chtype));
        win->_prev = (chtype *)calloc(size, sizeof(chtype));
        if (!win->_line || !win->_prev) {
            free(win->_line);
            free(win->_prev);
            free(win);
            return NULL;
        }
        /* Fill with spaces */
        for (int i = 0; i < size; i++) {
            win->_line[i] = ' ' | A_NORMAL;
            win->_prev[i] = 0; /* Force initial diff */
        }
    }

    return win;
}

static void _free_window(WINDOW *win)
{
    if (!win) return;
    free(win->_line);
    free(win->_prev);
    free(win);
}

/* Scroll the window content up by one line */
static void _scroll_up(WINDOW *win)
{
    if (!win || !win->_line) return;
    int cols = win->_maxx;
    int lines = win->_maxy;
    /* Move lines 1..n-1 up to 0..n-2 */
    if (lines > 1) {
        memmove(win->_line, win->_line + cols, (lines - 1) * cols * sizeof(chtype));
    }
    /* Clear last line */
    chtype fill = ' ' | (win->_bkgd & A_ATTRIBUTES);
    for (int i = 0; i < cols; i++) {
        win->_line[(lines - 1) * cols + i] = fill;
    }
}

/* ========================================================================= */
/* ACS initialization                                                        */
/* ========================================================================= */

static void _init_acs(void)
{
    /* Default: use ASCII approximations for line drawing */
    for (int i = 0; i < 128; i++)
        acs_map[i] = (chtype)i;

    /* Unicode/UTF-8 line drawing would be better, but for VT100 compat: */
    acs_map['l'] = '+';  /* upper-left corner */
    acs_map['m'] = '+';  /* lower-left corner */
    acs_map['k'] = '+';  /* upper-right corner */
    acs_map['j'] = '+';  /* lower-right corner */
    acs_map['t'] = '+';  /* left tee */
    acs_map['u'] = '+';  /* right tee */
    acs_map['v'] = '+';  /* bottom tee */
    acs_map['w'] = '+';  /* top tee */
    acs_map['q'] = '-';  /* horizontal line */
    acs_map['x'] = '|';  /* vertical line */
    acs_map['n'] = '+';  /* plus/crossover */
}

/* ========================================================================= */
/* Initialization / termination                                              */
/* ========================================================================= */

WINDOW *initscr(void)
{
    if (_curses_active) return stdscr;

    /* Save current terminal state */
    if (tcgetattr(STDIN_FILENO, &_saved_termios) == 0)
        _termios_saved = 1;

    /* Get terminal size */
    _get_terminal_size();

    /* Create stdscr */
    stdscr = _alloc_window(LINES, COLS, 0, 0);
    if (!stdscr) return NULL;

    /* Create curscr (physical screen representation) */
    curscr = _alloc_window(LINES, COLS, 0, 0);
    if (!curscr) {
        _free_window(stdscr);
        stdscr = NULL;
        return NULL;
    }

    /* Initialize ACS map */
    _init_acs();

    /* Set raw mode by default (most TUI apps want this) */
    struct termios raw_term;
    if (_termios_saved) {
        raw_term = _saved_termios;
    } else {
        memset(&raw_term, 0, sizeof(raw_term));
    }
    _cfmakeraw(&raw_term);
    /* Keep output processing for \n -> \r\n */
    raw_term.c_oflag |= OPOST | ONLCR;
    raw_term.c_cc[VMIN]  = 1;
    raw_term.c_cc[VTIME] = 0;
    tcsetattr(STDIN_FILENO, TCSANOW, &raw_term);

    /* Switch to alternate screen buffer and clear */
    _put_str("\033[?1049h");  /* Enter alternate screen */
    _put_str("\033[2J");       /* Clear screen */
    _put_str("\033[H");        /* Home cursor */
    _flush_out();

    _curses_active = 1;
    _echo_mode = 0;  /* ncurses default: echo off after initscr */

    return stdscr;
}

int endwin(void)
{
    if (!_curses_active) return ERR;

    /* Show cursor */
    _put_str("\033[?25h");
    /* Reset attributes */
    _put_str("\033[0m");
    /* Leave alternate screen */
    _put_str("\033[?1049l");
    _flush_out();

    /* Restore terminal */
    if (_termios_saved)
        tcsetattr(STDIN_FILENO, TCSANOW, &_saved_termios);

    _curses_active = 0;

    return OK;
}

int isendwin(void)
{
    return !_curses_active;
}

/* ========================================================================= */
/* Input mode                                                                */
/* ========================================================================= */

static void _update_termios(void)
{
    if (!_curses_active) return;
    struct termios t;
    tcgetattr(STDIN_FILENO, &t);

    if (_raw_mode) {
        _cfmakeraw(&t);
        t.c_oflag |= OPOST | ONLCR;
    } else if (_cbreak_mode) {
        t.c_lflag &= ~(ICANON);
        t.c_lflag |= ISIG;
        t.c_cc[VMIN]  = 1;
        t.c_cc[VTIME] = 0;
    } else {
        /* Cooked mode */
        t.c_lflag |= (ICANON | ISIG);
    }

    if (_echo_mode)
        t.c_lflag |= ECHO;
    else
        t.c_lflag &= ~ECHO;

    if (_nl_mode)
        t.c_iflag |= ICRNL;
    else
        t.c_iflag &= ~ICRNL;

    tcsetattr(STDIN_FILENO, TCSANOW, &t);
}

int raw(void)
{
    _raw_mode = 1;
    _cbreak_mode = 0;
    _update_termios();
    return OK;
}

int noraw(void)
{
    _raw_mode = 0;
    _update_termios();
    return OK;
}

int cbreak(void)
{
    _cbreak_mode = 1;
    _raw_mode = 0;
    _update_termios();
    return OK;
}

int nocbreak(void)
{
    _cbreak_mode = 0;
    _update_termios();
    return OK;
}

int echo(void)
{
    _echo_mode = 1;
    _update_termios();
    return OK;
}

int noecho(void)
{
    _echo_mode = 0;
    _update_termios();
    return OK;
}

int nl(void)
{
    _nl_mode = 1;
    _update_termios();
    return OK;
}

int nonl(void)
{
    _nl_mode = 0;
    _update_termios();
    return OK;
}

int keypad(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_keypad = bf;
    return OK;
}

int nodelay(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_nodelay = bf;
    return OK;
}

int notimeout(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_notimeout = bf;
    return OK;
}

int meta(WINDOW *win, int bf)
{
    (void)win;
    (void)bf;
    return OK;
}

void timeout(int delay)
{
    wtimeout(stdscr, delay);
}

void wtimeout(WINDOW *win, int delay)
{
    if (!win) return;
    if (delay < 0) {
        win->_nodelay = 0;
    } else if (delay == 0) {
        win->_nodelay = 1;
    } else {
        /* Positive timeout: set VTIME (in tenths of a second) */
        win->_nodelay = 0;
        struct termios t;
        if (tcgetattr(STDIN_FILENO, &t) == 0) {
            t.c_cc[VMIN]  = 0;
            t.c_cc[VTIME] = (delay + 99) / 100;  /* Convert ms to tenths */
            if (t.c_cc[VTIME] == 0) t.c_cc[VTIME] = 1;
            tcsetattr(STDIN_FILENO, TCSANOW, &t);
        }
    }
}

int halfdelay(int tenths)
{
    (void)tenths;
    return OK;
}

int typeahead(int fd)
{
    (void)fd;
    return OK;
}

int intrflush(WINDOW *win, int bf)
{
    (void)win;
    (void)bf;
    return OK;
}

/* ========================================================================= */
/* Cursor movement                                                           */
/* ========================================================================= */

int move(int y, int x)
{
    return wmove(stdscr, y, x);
}

int wmove(WINDOW *win, int y, int x)
{
    if (!win) return ERR;
    if (y < 0 || y >= win->_maxy || x < 0 || x >= win->_maxx)
        return ERR;
    win->_cury = y;
    win->_curx = x;
    return OK;
}

/* ========================================================================= */
/* Character output                                                          */
/* ========================================================================= */

int waddch(WINDOW *win, const chtype ch)
{
    if (!win || !win->_line) return ERR;

    chtype c = ch;
    /* Merge window attributes if char has no attributes */
    if ((c & A_ATTRIBUTES) == 0)
        c |= win->_attrs;

    char printable = (char)(c & A_CHARTEXT);

    /* Handle special characters */
    if (printable == '\n') {
        /* Clear to end of line, then move to next line */
        wclrtoeol(win);
        if (win->_cury < win->_maxy - 1) {
            win->_cury++;
            win->_curx = 0;
        } else if (win->_scroll) {
            _scroll_up(win);
            win->_curx = 0;
        }
        return OK;
    }
    if (printable == '\r') {
        win->_curx = 0;
        return OK;
    }
    if (printable == '\t') {
        /* Expand tab to spaces */
        int next_tab = ((win->_curx / 8) + 1) * 8;
        while (win->_curx < next_tab && win->_curx < win->_maxx) {
            waddch(win, ' ' | (c & A_ATTRIBUTES));
        }
        return OK;
    }
    if (printable == '\b') {
        if (win->_curx > 0) win->_curx--;
        return OK;
    }

    /* Write character to virtual buffer */
    if (win->_cury >= 0 && win->_cury < win->_maxy &&
        win->_curx >= 0 && win->_curx < win->_maxx) {
        win->_line[win->_cury * win->_maxx + win->_curx] = c;
    }

    /* Advance cursor */
    win->_curx++;
    if (win->_curx >= win->_maxx) {
        win->_curx = 0;
        if (win->_cury < win->_maxy - 1) {
            win->_cury++;
        } else if (win->_scroll) {
            _scroll_up(win);
        } else {
            win->_curx = win->_maxx - 1;
        }
    }

    return OK;
}

int addch(const chtype ch) { return waddch(stdscr, ch); }

int mvaddch(int y, int x, const chtype ch)
{
    if (move(y, x) == ERR) return ERR;
    return addch(ch);
}

int mvwaddch(WINDOW *win, int y, int x, const chtype ch)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return waddch(win, ch);
}

int echochar(const chtype ch) { return wechochar(stdscr, ch); }

int wechochar(WINDOW *win, const chtype ch)
{
    if (waddch(win, ch) == ERR) return ERR;
    return wrefresh(win);
}

/* ========================================================================= */
/* String output                                                             */
/* ========================================================================= */

int waddnstr(WINDOW *win, const char *str, int n)
{
    if (!win || !str) return ERR;
    int i = 0;
    while (*str && (n < 0 || i < n)) {
        waddch(win, (chtype)(unsigned char)*str);
        str++;
        i++;
    }
    return OK;
}

int waddstr(WINDOW *win, const char *str) { return waddnstr(win, str, -1); }
int addstr(const char *str) { return waddstr(stdscr, str); }
int addnstr(const char *str, int n) { return waddnstr(stdscr, str, n); }

int mvaddstr(int y, int x, const char *str)
{
    if (move(y, x) == ERR) return ERR;
    return addstr(str);
}

int mvaddnstr(int y, int x, const char *str, int n)
{
    if (move(y, x) == ERR) return ERR;
    return addnstr(str, n);
}

int mvwaddstr(WINDOW *win, int y, int x, const char *str)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return waddstr(win, str);
}

int mvwaddnstr(WINDOW *win, int y, int x, const char *str, int n)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return waddnstr(win, str, n);
}

/* ========================================================================= */
/* Formatted output                                                          */
/* ========================================================================= */

int vwprintw(WINDOW *win, const char *fmt, va_list varglist)
{
    char buf[1024];
    int len = vsnprintf(buf, sizeof(buf), fmt, varglist);
    if (len < 0) return ERR;
    return waddnstr(win, buf, len < (int)sizeof(buf) ? len : (int)sizeof(buf) - 1);
}

int vw_printw(WINDOW *win, const char *fmt, va_list varglist)
{
    return vwprintw(win, fmt, varglist);
}

int wprintw(WINDOW *win, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vwprintw(win, fmt, ap);
    va_end(ap);
    return ret;
}

int printw(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int ret = vwprintw(stdscr, fmt, ap);
    va_end(ap);
    return ret;
}

int mvprintw(int y, int x, const char *fmt, ...)
{
    if (move(y, x) == ERR) return ERR;
    va_list ap;
    va_start(ap, fmt);
    int ret = vwprintw(stdscr, fmt, ap);
    va_end(ap);
    return ret;
}

int mvwprintw(WINDOW *win, int y, int x, const char *fmt, ...)
{
    if (wmove(win, y, x) == ERR) return ERR;
    va_list ap;
    va_start(ap, fmt);
    int ret = vwprintw(win, fmt, ap);
    va_end(ap);
    return ret;
}

/* ========================================================================= */
/* Input (getch with escape sequence parsing)                                */
/* ========================================================================= */

/* Read a single byte with optional timeout. Returns -1 on no input. */
static int _read_byte(int timeout_ms)
{
    if (timeout_ms >= 0) {
        struct termios t, oldt;
        tcgetattr(STDIN_FILENO, &oldt);
        t = oldt;
        t.c_cc[VMIN]  = 0;
        t.c_cc[VTIME] = (timeout_ms > 0) ? ((timeout_ms + 99) / 100) : 1;
        if (t.c_cc[VTIME] == 0) t.c_cc[VTIME] = 1;
        tcsetattr(STDIN_FILENO, TCSANOW, &t);

        unsigned char c;
        int n = read(STDIN_FILENO, &c, 1);

        tcsetattr(STDIN_FILENO, TCSANOW, &oldt);
        return (n == 1) ? (int)c : -1;
    } else {
        /* Blocking read */
        unsigned char c;
        int n = read(STDIN_FILENO, &c, 1);
        return (n == 1) ? (int)c : -1;
    }
}

int wgetch(WINDOW *win)
{
    if (!win) return ERR;

    /* Flush output before reading input */
    wrefresh(win);
    _flush_out();

    /* Check ungetch buffer first */
    if (_ungetch_count > 0) {
        return _ungetch_buf[--_ungetch_count];
    }

    int ch;
    if (win->_nodelay) {
        ch = _read_byte(0);
        if (ch < 0) return ERR;
    } else {
        ch = _read_byte(-1);
        if (ch < 0) return ERR;
    }

    /* Parse escape sequences when keypad mode is enabled */
    if (ch == 27 && win->_keypad) {
        int ch2 = _read_byte(50);  /* 50ms timeout for escape sequences */
        if (ch2 < 0) return 27;    /* Bare ESC key */

        if (ch2 == '[') {
            /* CSI sequence: ESC [ ... */
            int ch3 = _read_byte(50);
            if (ch3 < 0) return 27;

            /* Check for numeric parameters */
            if (ch3 >= '0' && ch3 <= '9') {
                /* Numeric parameter: ESC [ <num> <final> */
                int num = ch3 - '0';
                int ch4 = _read_byte(50);
                if (ch4 < 0) return 27;

                /* Multi-digit parameter */
                while (ch4 >= '0' && ch4 <= '9') {
                    num = num * 10 + (ch4 - '0');
                    ch4 = _read_byte(50);
                    if (ch4 < 0) return 27;
                }

                /* Check for modifier: ESC [ num ; mod final */
                if (ch4 == ';') {
                    /* Read modifier and final byte */
                    int _mod = _read_byte(50);
                    if (_mod < 0) return 27;
                    /* Skip multi-digit modifier */
                    while (_mod >= '0' && _mod <= '9') {
                        _mod = _read_byte(50);
                        if (_mod < 0) return 27;
                    }
                    ch4 = _mod; /* final byte */
                }

                if (ch4 == '~') {
                    switch (num) {
                    case 1:  return KEY_HOME;
                    case 2:  return KEY_IC;
                    case 3:  return KEY_DC;
                    case 4:  return KEY_END;
                    case 5:  return KEY_PPAGE;
                    case 6:  return KEY_NPAGE;
                    case 7:  return KEY_HOME;
                    case 8:  return KEY_END;
                    case 11: return KEY_F(1);
                    case 12: return KEY_F(2);
                    case 13: return KEY_F(3);
                    case 14: return KEY_F(4);
                    case 15: return KEY_F(5);
                    case 17: return KEY_F(6);
                    case 18: return KEY_F(7);
                    case 19: return KEY_F(8);
                    case 20: return KEY_F(9);
                    case 21: return KEY_F(10);
                    case 23: return KEY_F(11);
                    case 24: return KEY_F(12);
                    default: return ERR;
                    }
                }
                /* xterm-style: ESC [ 1 ; mod <letter> */
                switch (ch4) {
                case 'A': return KEY_UP;
                case 'B': return KEY_DOWN;
                case 'C': return KEY_RIGHT;
                case 'D': return KEY_LEFT;
                case 'H': return KEY_HOME;
                case 'F': return KEY_END;
                default:  return ERR;
                }
            }

            /* Simple letter sequences: ESC [ <letter> */
            switch (ch3) {
            case 'A': return KEY_UP;
            case 'B': return KEY_DOWN;
            case 'C': return KEY_RIGHT;
            case 'D': return KEY_LEFT;
            case 'H': return KEY_HOME;
            case 'F': return KEY_END;
            case 'Z': return KEY_BTAB;  /* Shift-Tab */
            default:  return ERR;
            }
        } else if (ch2 == 'O') {
            /* SS3 sequence: ESC O <letter> */
            int ch3 = _read_byte(50);
            if (ch3 < 0) return 27;
            switch (ch3) {
            case 'A': return KEY_UP;
            case 'B': return KEY_DOWN;
            case 'C': return KEY_RIGHT;
            case 'D': return KEY_LEFT;
            case 'H': return KEY_HOME;
            case 'F': return KEY_END;
            case 'P': return KEY_F(1);
            case 'Q': return KEY_F(2);
            case 'R': return KEY_F(3);
            case 'S': return KEY_F(4);
            default:  return ERR;
            }
        } else {
            /* ESC + normal char: treat as Alt+key */
            /* For now, return the character with high bit set */
            return ch2 | 0x80;
        }
    }

    /* Map common byte values */
    if (ch == 127 || ch == 8) return KEY_BACKSPACE;

    return ch;
}

int getch(void) { return wgetch(stdscr); }

int mvgetch(int y, int x)
{
    if (move(y, x) == ERR) return ERR;
    return getch();
}

int mvwgetch(WINDOW *win, int y, int x)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return wgetch(win);
}

int ungetch(int ch)
{
    if (_ungetch_count >= UNGETCH_MAX) return ERR;
    _ungetch_buf[_ungetch_count++] = ch;
    return OK;
}

int has_key(int ch)
{
    (void)ch;
    return TRUE;
}

/* ========================================================================= */
/* String input                                                              */
/* ========================================================================= */

int wgetnstr(WINDOW *win, char *str, int n)
{
    if (!win || !str || n <= 0) return ERR;
    int i = 0;
    int ch;
    while (i < n - 1) {
        ch = wgetch(win);
        if (ch == ERR) break;
        if (ch == '\n' || ch == '\r') break;
        if (ch == KEY_BACKSPACE && i > 0) { i--; continue; }
        if (ch >= 0x20 && ch < 0x7F) {
            str[i++] = (char)ch;
            if (_echo_mode) waddch(win, (chtype)ch);
        }
    }
    str[i] = '\0';
    return OK;
}

int wgetstr(WINDOW *win, char *str)
{
    return wgetnstr(win, str, 1024);
}

/* ========================================================================= */
/* Screen refresh -- the heart of curses                                     */
/* ========================================================================= */

int wrefresh(WINDOW *win)
{
    if (!win || !win->_line) return ERR;

    /* If this is not stdscr, composite onto stdscr first */
    if (win != stdscr && stdscr) {
        wnoutrefresh(win);
        return doupdate();
    }

    int rows = win->_maxy;
    int cols = win->_maxx;
    attr_t cur_attrs = A_NORMAL;
    int last_row = -1, last_col = -1;

    if (win->_clearok) {
        /* Full screen clear + redraw */
        _put_str("\033[2J");
        _put_str("\033[0m");
        cur_attrs = A_NORMAL;
        /* Mark all previous as dirty */
        if (win->_prev) {
            memset(win->_prev, 0xFF, rows * cols * sizeof(chtype));
        }
        win->_clearok = 0;
    }

    for (int y = 0; y < rows; y++) {
        for (int x = 0; x < cols; x++) {
            int idx = y * cols + x;
            chtype newch = win->_line[idx];

            /* Skip if unchanged */
            if (win->_prev && win->_prev[idx] == newch)
                continue;

            /* Move cursor if needed */
            if (y != last_row || x != last_col) {
                _move_cursor(y + win->_begy, x + win->_begx);
            }

            /* Update attributes if changed */
            attr_t new_attrs = newch & A_ATTRIBUTES;
            if (new_attrs != cur_attrs) {
                _apply_attrs(new_attrs);
                cur_attrs = new_attrs;
            }

            /* Output the character */
            char c = (char)(newch & A_CHARTEXT);
            if (c < 0x20 || c == 0x7F) c = ' ';  /* Non-printable -> space */
            _put_raw(&c, 1);

            last_row = y;
            last_col = x + 1;

            /* Update prev buffer */
            if (win->_prev)
                win->_prev[idx] = newch;
        }
    }

    /* Reset attributes */
    if (cur_attrs != A_NORMAL) {
        _put_str("\033[0m");
    }

    /* Position cursor at window cursor position */
    if (!win->_leaveok) {
        _move_cursor(win->_cury + win->_begy, win->_curx + win->_begx);
    }

    /* Show/hide cursor */
    if (_cursor_visible)
        _put_str("\033[?25h");
    else
        _put_str("\033[?25l");

    _flush_out();
    return OK;
}

int refresh(void) { return wrefresh(stdscr); }

int wnoutrefresh(WINDOW *win)
{
    if (!win || !stdscr || !win->_line || !stdscr->_line) return ERR;
    if (win == stdscr) return OK;

    /* Copy window contents to stdscr at the appropriate position */
    for (int y = 0; y < win->_maxy; y++) {
        int sy = y + win->_begy;
        if (sy < 0 || sy >= stdscr->_maxy) continue;
        for (int x = 0; x < win->_maxx; x++) {
            int sx = x + win->_begx;
            if (sx < 0 || sx >= stdscr->_maxx) continue;
            stdscr->_line[sy * stdscr->_maxx + sx] = win->_line[y * win->_maxx + x];
        }
    }
    return OK;
}

int doupdate(void)
{
    return wrefresh(stdscr);
}

int redrawwin(WINDOW *win)
{
    if (!win) return ERR;
    win->_clearok = 1;
    return wrefresh(win);
}

int wredrawln(WINDOW *win, int beg_included, int num_lines)
{
    (void)beg_included;
    (void)num_lines;
    return redrawwin(win);
}

/* ========================================================================= */
/* Clearing                                                                  */
/* ========================================================================= */

int werase(WINDOW *win)
{
    if (!win || !win->_line) return ERR;
    chtype fill = ' ' | (win->_bkgd & A_ATTRIBUTES);
    int size = win->_maxy * win->_maxx;
    for (int i = 0; i < size; i++)
        win->_line[i] = fill;
    win->_cury = 0;
    win->_curx = 0;
    return OK;
}

int erase(void) { return werase(stdscr); }

int wclear(WINDOW *win)
{
    if (!win) return ERR;
    werase(win);
    win->_clearok = 1;
    return OK;
}

int clear(void) { return wclear(stdscr); }

int wclrtoeol(WINDOW *win)
{
    if (!win || !win->_line) return ERR;
    chtype fill = ' ' | (win->_bkgd & A_ATTRIBUTES);
    int y = win->_cury;
    for (int x = win->_curx; x < win->_maxx; x++)
        win->_line[y * win->_maxx + x] = fill;
    return OK;
}

int clrtoeol(void) { return wclrtoeol(stdscr); }

int wclrtobot(WINDOW *win)
{
    if (!win || !win->_line) return ERR;
    /* Clear from cursor to end of current line */
    wclrtoeol(win);
    /* Clear all subsequent lines */
    chtype fill = ' ' | (win->_bkgd & A_ATTRIBUTES);
    for (int y = win->_cury + 1; y < win->_maxy; y++)
        for (int x = 0; x < win->_maxx; x++)
            win->_line[y * win->_maxx + x] = fill;
    return OK;
}

int clrtobot(void) { return wclrtobot(stdscr); }

/* ========================================================================= */
/* Attribute control                                                         */
/* ========================================================================= */

int wattrset(WINDOW *win, int attrs)
{
    if (!win) return ERR;
    win->_attrs = (attr_t)attrs;
    return OK;
}

int wattron(WINDOW *win, int attrs)
{
    if (!win) return ERR;
    win->_attrs |= (attr_t)attrs;
    return OK;
}

int wattroff(WINDOW *win, int attrs)
{
    if (!win) return ERR;
    win->_attrs &= ~(attr_t)attrs;
    return OK;
}

int attrset(int attrs)  { return wattrset(stdscr, attrs); }
int attron(int attrs)   { return wattron(stdscr, attrs); }
int attroff(int attrs)  { return wattroff(stdscr, attrs); }

int attr_on(attr_t attrs, void *opts)
{
    (void)opts;
    return wattron(stdscr, (int)attrs);
}

int attr_off(attr_t attrs, void *opts)
{
    (void)opts;
    return wattroff(stdscr, (int)attrs);
}

int attr_set(attr_t attrs, short pair, void *opts)
{
    (void)opts;
    return wattrset(stdscr, (int)(attrs | COLOR_PAIR(pair)));
}

int wattr_on(WINDOW *win, attr_t attrs, void *opts)
{
    (void)opts;
    return wattron(win, (int)attrs);
}

int wattr_off(WINDOW *win, attr_t attrs, void *opts)
{
    (void)opts;
    return wattroff(win, (int)attrs);
}

int wattr_set(WINDOW *win, attr_t attrs, short pair, void *opts)
{
    (void)opts;
    return wattrset(win, (int)(attrs | COLOR_PAIR(pair)));
}

int attr_get(attr_t *attrs, short *pair, void *opts)
{
    return wattr_get(stdscr, attrs, pair, opts);
}

int wattr_get(WINDOW *win, attr_t *attrs, short *pair, void *opts)
{
    (void)opts;
    if (!win) return ERR;
    if (attrs) *attrs = win->_attrs;
    if (pair)  *pair  = (short)PAIR_NUMBER(win->_attrs);
    return OK;
}

int standout(void)  { return wattron(stdscr, A_STANDOUT); }
int standend(void)  { return wattrset(stdscr, A_NORMAL); }
int wstandout(WINDOW *win) { return wattron(win, A_STANDOUT); }
int wstandend(WINDOW *win) { return wattrset(win, A_NORMAL); }

/* ========================================================================= */
/* Color support                                                             */
/* ========================================================================= */

int has_colors(void)
{
    /* Assume terminal supports ANSI colors (VT100+) */
    return TRUE;
}

int can_change_color(void)
{
    return FALSE;  /* We don't support redefining color values */
}

int start_color(void)
{
    _colors_started = 1;
    /* Initialize default color pairs */
    for (int i = 0; i < COLOR_PAIRS; i++) {
        _color_pairs[i].fg = COLOR_WHITE;
        _color_pairs[i].bg = COLOR_BLACK;
    }
    /* Pair 0 is special: default terminal colors */
    _color_pairs[0].fg = -1;
    _color_pairs[0].bg = -1;
    return OK;
}

int init_pair(short pair, short f, short b)
{
    if (pair < 0 || pair >= COLOR_PAIRS) return ERR;
    _color_pairs[pair].fg = f;
    _color_pairs[pair].bg = b;
    return OK;
}

int init_color(short color, short r, short g, short b)
{
    (void)color; (void)r; (void)g; (void)b;
    return ERR;  /* Not supported */
}

int pair_content(short pair, short *f, short *b)
{
    if (pair < 0 || pair >= COLOR_PAIRS) return ERR;
    if (f) *f = _color_pairs[pair].fg;
    if (b) *b = _color_pairs[pair].bg;
    return OK;
}

int color_content(short color, short *r, short *g, short *b)
{
    (void)color;
    if (r) *r = 0;
    if (g) *g = 0;
    if (b) *b = 0;
    return OK;
}

int use_default_colors(void)
{
    _default_colors = 1;
    return OK;
}

int assume_default_colors(int fg, int bg)
{
    _color_pairs[0].fg = (short)fg;
    _color_pairs[0].bg = (short)bg;
    _default_colors = 1;
    return OK;
}

/* ========================================================================= */
/* Window management                                                         */
/* ========================================================================= */

WINDOW *newwin(int nlines, int ncols, int begin_y, int begin_x)
{
    if (nlines <= 0) nlines = LINES - begin_y;
    if (ncols  <= 0) ncols  = COLS  - begin_x;
    return _alloc_window(nlines, ncols, begin_y, begin_x);
}

int delwin(WINDOW *win)
{
    if (!win || win == stdscr || win == curscr) return ERR;
    _free_window(win);
    return OK;
}

WINDOW *subwin(WINDOW *orig, int nlines, int ncols, int begin_y, int begin_x)
{
    if (!orig) return NULL;
    WINDOW *win = newwin(nlines, ncols, begin_y, begin_x);
    if (win) win->_parent = orig;
    return win;
}

WINDOW *derwin(WINDOW *orig, int nlines, int ncols, int begin_y, int begin_x)
{
    if (!orig) return NULL;
    return subwin(orig, nlines, ncols, orig->_begy + begin_y, orig->_begx + begin_x);
}

WINDOW *dupwin(WINDOW *win)
{
    if (!win) return NULL;
    WINDOW *dup = _alloc_window(win->_maxy, win->_maxx, win->_begy, win->_begx);
    if (!dup) return NULL;
    dup->_cury = win->_cury;
    dup->_curx = win->_curx;
    dup->_attrs = win->_attrs;
    dup->_bkgd = win->_bkgd;
    dup->_scroll = win->_scroll;
    dup->_keypad = win->_keypad;
    dup->_nodelay = win->_nodelay;
    int size = win->_maxy * win->_maxx;
    if (size > 0 && win->_line && dup->_line)
        memcpy(dup->_line, win->_line, size * sizeof(chtype));
    return dup;
}

int mvwin(WINDOW *win, int y, int x)
{
    if (!win) return ERR;
    win->_begy = y;
    win->_begx = x;
    return OK;
}

int mvderwin(WINDOW *win, int y, int x)
{
    return mvwin(win, y, x);
}

void wsyncup(WINDOW *win) { (void)win; }
void wsyncdown(WINDOW *win) { (void)win; }
int syncok(WINDOW *win, int bf) { (void)win; (void)bf; return OK; }

/* ========================================================================= */
/* Scrolling                                                                 */
/* ========================================================================= */

int scrollok(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_scroll = bf;
    return OK;
}

int scroll(WINDOW *win)
{
    return wscrl(win, 1);
}

int wscrl(WINDOW *win, int n)
{
    if (!win || !win->_scroll) return ERR;
    if (n > 0) {
        for (int i = 0; i < n; i++)
            _scroll_up(win);
    }
    /* Scroll down not implemented (rarely needed) */
    return OK;
}

int idlok(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_idlok = bf;
    return OK;
}

int wsetscrreg(WINDOW *win, int top, int bot)
{
    (void)win; (void)top; (void)bot;
    return OK;
}

int setscrreg(int top, int bot) { return wsetscrreg(stdscr, top, bot); }

/* ========================================================================= */
/* Insert / Delete                                                           */
/* ========================================================================= */

int wdelch(WINDOW *win)
{
    if (!win || !win->_line) return ERR;
    int y = win->_cury;
    int x = win->_curx;
    int cols = win->_maxx;
    chtype fill = ' ' | (win->_bkgd & A_ATTRIBUTES);
    /* Shift characters left */
    for (int i = x; i < cols - 1; i++)
        win->_line[y * cols + i] = win->_line[y * cols + i + 1];
    win->_line[y * cols + cols - 1] = fill;
    return OK;
}

int delch(void) { return wdelch(stdscr); }

int mvdelch(int y, int x)
{
    if (move(y, x) == ERR) return ERR;
    return delch();
}

int mvwdelch(WINDOW *win, int y, int x)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return wdelch(win);
}

int winsch(WINDOW *win, chtype ch)
{
    if (!win || !win->_line) return ERR;
    int y = win->_cury;
    int x = win->_curx;
    int cols = win->_maxx;
    /* Shift characters right */
    for (int i = cols - 1; i > x; i--)
        win->_line[y * cols + i] = win->_line[y * cols + i - 1];
    chtype c = ch;
    if ((c & A_ATTRIBUTES) == 0) c |= win->_attrs;
    win->_line[y * cols + x] = c;
    return OK;
}

int insch(chtype ch) { return winsch(stdscr, ch); }

int mvinsch(int y, int x, chtype ch)
{
    if (move(y, x) == ERR) return ERR;
    return insch(ch);
}

int mvwinsch(WINDOW *win, int y, int x, chtype ch)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return winsch(win, ch);
}

int winsdelln(WINDOW *win, int n)
{
    if (!win || !win->_line) return ERR;
    int cols = win->_maxx;
    int rows = win->_maxy;
    int y = win->_cury;
    chtype fill = ' ' | (win->_bkgd & A_ATTRIBUTES);

    if (n > 0) {
        /* Insert n lines: shift down */
        for (int r = rows - 1; r >= y + n; r--)
            memcpy(win->_line + r * cols, win->_line + (r - n) * cols, cols * sizeof(chtype));
        for (int r = y; r < y + n && r < rows; r++)
            for (int c = 0; c < cols; c++)
                win->_line[r * cols + c] = fill;
    } else if (n < 0) {
        /* Delete |n| lines: shift up */
        int d = -n;
        for (int r = y; r < rows - d; r++)
            memcpy(win->_line + r * cols, win->_line + (r + d) * cols, cols * sizeof(chtype));
        for (int r = rows - d; r < rows; r++)
            for (int c = 0; c < cols; c++)
                win->_line[r * cols + c] = fill;
    }
    return OK;
}

int insdelln(int n) { return winsdelln(stdscr, n); }
int insertln(void) { return winsdelln(stdscr, 1); }
int winsertln(WINDOW *win) { return winsdelln(win, 1); }
int deleteln(void) { return winsdelln(stdscr, -1); }
int wdeleteln(WINDOW *win) { return winsdelln(win, -1); }

/* ========================================================================= */
/* Border / box                                                              */
/* ========================================================================= */

int wborder(WINDOW *win, chtype ls, chtype rs, chtype ts, chtype bs,
            chtype tl, chtype tr, chtype bl, chtype br)
{
    if (!win) return ERR;
    if (ls == 0) ls = ACS_VLINE;
    if (rs == 0) rs = ACS_VLINE;
    if (ts == 0) ts = ACS_HLINE;
    if (bs == 0) bs = ACS_HLINE;
    if (tl == 0) tl = ACS_ULCORNER;
    if (tr == 0) tr = ACS_URCORNER;
    if (bl == 0) bl = ACS_LLCORNER;
    if (br == 0) br = ACS_LRCORNER;

    int maxy = win->_maxy - 1;
    int maxx = win->_maxx - 1;

    /* Corners */
    mvwaddch(win, 0, 0, tl);
    mvwaddch(win, 0, maxx, tr);
    mvwaddch(win, maxy, 0, bl);
    mvwaddch(win, maxy, maxx, br);

    /* Top and bottom */
    for (int x = 1; x < maxx; x++) {
        mvwaddch(win, 0, x, ts);
        mvwaddch(win, maxy, x, bs);
    }

    /* Left and right */
    for (int y = 1; y < maxy; y++) {
        mvwaddch(win, y, 0, ls);
        mvwaddch(win, y, maxx, rs);
    }

    return OK;
}

int border(chtype ls, chtype rs, chtype ts, chtype bs,
           chtype tl, chtype tr, chtype bl, chtype br)
{
    return wborder(stdscr, ls, rs, ts, bs, tl, tr, bl, br);
}

int box(WINDOW *win, chtype verch, chtype horch)
{
    return wborder(win, verch, verch, horch, horch, 0, 0, 0, 0);
}

int whline(WINDOW *win, chtype ch, int n)
{
    if (!win) return ERR;
    if (ch == 0) ch = ACS_HLINE;
    int y = win->_cury;
    int x = win->_curx;
    for (int i = 0; i < n && x + i < win->_maxx; i++)
        mvwaddch(win, y, x + i, ch);
    wmove(win, y, x);
    return OK;
}

int hline(chtype ch, int n) { return whline(stdscr, ch, n); }

int wvline(WINDOW *win, chtype ch, int n)
{
    if (!win) return ERR;
    if (ch == 0) ch = ACS_VLINE;
    int y = win->_cury;
    int x = win->_curx;
    for (int i = 0; i < n && y + i < win->_maxy; i++)
        mvwaddch(win, y + i, x, ch);
    wmove(win, y, x);
    return OK;
}

int vline(chtype ch, int n) { return wvline(stdscr, ch, n); }

int mvhline(int y, int x, chtype ch, int n)
{
    if (move(y, x) == ERR) return ERR;
    return hline(ch, n);
}

int mvwhline(WINDOW *win, int y, int x, chtype ch, int n)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return whline(win, ch, n);
}

int mvvline(int y, int x, chtype ch, int n)
{
    if (move(y, x) == ERR) return ERR;
    return vline(ch, n);
}

int mvwvline(WINDOW *win, int y, int x, chtype ch, int n)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return wvline(win, ch, n);
}

/* ========================================================================= */
/* Window properties                                                         */
/* ========================================================================= */

int clearok(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_clearok = bf;
    return OK;
}

int leaveok(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_leaveok = bf;
    return OK;
}

int immedok(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_immed = bf;
    return OK;
}

int idcok(WINDOW *win, int bf)
{
    if (!win) return ERR;
    win->_idcok = bf;
    return OK;
}

void wbkgdset(WINDOW *win, chtype ch)
{
    if (!win) return;
    win->_bkgd = ch;
}

void bkgdset(chtype ch) { wbkgdset(stdscr, ch); }

int wbkgd(WINDOW *win, chtype ch)
{
    if (!win || !win->_line) return ERR;
    chtype old_bkgd = win->_bkgd;
    win->_bkgd = ch;

    attr_t new_attr = ch & A_ATTRIBUTES;
    char new_char = (char)(ch & A_CHARTEXT);
    if (new_char == 0) new_char = ' ';

    int size = win->_maxy * win->_maxx;
    for (int i = 0; i < size; i++) {
        chtype cell = win->_line[i];
        /* Replace old background char with new */
        if ((cell & A_CHARTEXT) == (old_bkgd & A_CHARTEXT) ||
            (cell & A_CHARTEXT) == ' ') {
            cell = (cell & ~A_CHARTEXT) | (chtype)(unsigned char)new_char;
        }
        /* Update attributes: remove old, add new */
        cell = (cell & ~(old_bkgd & A_ATTRIBUTES)) | new_attr;
        win->_line[i] = cell;
    }
    return OK;
}

int bkgd(chtype ch) { return wbkgd(stdscr, ch); }

chtype getbkgd(WINDOW *win)
{
    return win ? win->_bkgd : (chtype)ERR;
}

/* ========================================================================= */
/* Query functions                                                           */
/* ========================================================================= */

int getcury(WINDOW *win) { return win ? win->_cury : ERR; }
int getcurx(WINDOW *win) { return win ? win->_curx : ERR; }
int getbegy(WINDOW *win) { return win ? win->_begy : ERR; }
int getbegx(WINDOW *win) { return win ? win->_begx : ERR; }
int getmaxy(WINDOW *win) { return win ? win->_maxy : ERR; }
int getmaxx(WINDOW *win) { return win ? win->_maxx : ERR; }
int getpary(WINDOW *win) { return (win && win->_parent) ? win->_begy - win->_parent->_begy : -1; }
int getparx(WINDOW *win) { return (win && win->_parent) ? win->_begx - win->_parent->_begx : -1; }

chtype winch(WINDOW *win)
{
    if (!win || !win->_line) return ERR;
    return win->_line[win->_cury * win->_maxx + win->_curx];
}

chtype inch(void) { return winch(stdscr); }

chtype mvinch(int y, int x)
{
    if (move(y, x) == ERR) return ERR;
    return inch();
}

chtype mvwinch(WINDOW *win, int y, int x)
{
    if (wmove(win, y, x) == ERR) return ERR;
    return winch(win);
}

/* ========================================================================= */
/* Cursor visibility                                                         */
/* ========================================================================= */

int curs_set(int visibility)
{
    int old = _cursor_visible;
    _cursor_visible = visibility;
    if (visibility == 0)
        _put_str("\033[?25l");  /* Hide cursor */
    else
        _put_str("\033[?25h");  /* Show cursor */
    _flush_out();
    return old;
}

/* ========================================================================= */
/* Beep / flash                                                              */
/* ========================================================================= */

int beep(void)
{
    _put_str("\007");  /* BEL character */
    _flush_out();
    return OK;
}

int flash(void)
{
    /* Visual bell: reverse video briefly */
    _put_str("\033[?5h");  /* Reverse video on */
    _flush_out();
    napms(100);
    _put_str("\033[?5l");  /* Reverse video off */
    _flush_out();
    return OK;
}

/* ========================================================================= */
/* Miscellaneous                                                             */
/* ========================================================================= */

char *keyname(int c)
{
    static char buf[16];
    if (c == KEY_UP)        return "KEY_UP";
    if (c == KEY_DOWN)      return "KEY_DOWN";
    if (c == KEY_LEFT)      return "KEY_LEFT";
    if (c == KEY_RIGHT)     return "KEY_RIGHT";
    if (c == KEY_HOME)      return "KEY_HOME";
    if (c == KEY_END)       return "KEY_END";
    if (c == KEY_DC)        return "KEY_DC";
    if (c == KEY_IC)        return "KEY_IC";
    if (c == KEY_NPAGE)     return "KEY_NPAGE";
    if (c == KEY_PPAGE)     return "KEY_PPAGE";
    if (c == KEY_BACKSPACE) return "KEY_BACKSPACE";
    if (c == KEY_ENTER)     return "KEY_ENTER";
    if (c >= KEY_F(0) && c <= KEY_F(12)) {
        buf[0] = 'K'; buf[1] = 'E'; buf[2] = 'Y'; buf[3] = '_';
        buf[4] = 'F'; buf[5] = '(';
        int n = c - KEY_F(0);
        int pos = 6;
        if (n >= 10) buf[pos++] = '1';
        buf[pos++] = '0' + (n % 10);
        buf[pos++] = ')';
        buf[pos] = '\0';
        return buf;
    }
    if (c >= 0x20 && c < 0x7F) {
        buf[0] = (char)c;
        buf[1] = '\0';
        return buf;
    }
    if (c < 0x20) {
        buf[0] = '^';
        buf[1] = (char)(c + '@');
        buf[2] = '\0';
        return buf;
    }
    snprintf(buf, sizeof(buf), "0x%x", c);
    return buf;
}

int napms(int ms)
{
    if (ms <= 0) return OK;
    /* Simple busy-wait sleep using usleep if available, otherwise spin */
    /* VeridianOS may not have usleep/nanosleep yet, so use a stub */
    usleep(ms * 1000);
    return OK;
}

int def_prog_mode(void)  { return OK; }
int def_shell_mode(void) { return OK; }
int reset_prog_mode(void)  { return OK; }
int reset_shell_mode(void) { return OK; }
int resetty(void) { return OK; }
int savetty(void) { return OK; }

int baudrate(void) { return 38400; }

char erasechar(void)
{
    return _termios_saved ? (char)_saved_termios.c_cc[VERASE] : 127;
}

char killchar(void)
{
    return _termios_saved ? (char)_saved_termios.c_cc[VKILL] : 21; /* ^U */
}

int has_ic(void) { return TRUE; }
int has_il(void) { return TRUE; }

char *longname(void)
{
    return "VeridianOS ANSI terminal";
}

char *termname(void)
{
    return "vt100";
}

int flushinp(void)
{
    tcflush(STDIN_FILENO, TCIFLUSH);
    _ungetch_count = 0;
    return OK;
}

/* ========================================================================= */
/* SCREEN (minimal stubs for nano compatibility)                             */
/* ========================================================================= */

/* Opaque SCREEN type */
struct _screen {
    int dummy;
};

static struct _screen _default_screen;

SCREEN *newterm(const char *type, void *outfd, void *infd)
{
    (void)type;
    (void)outfd;
    (void)infd;
    initscr();
    return &_default_screen;
}

SCREEN *set_term(SCREEN *new_scr)
{
    (void)new_scr;
    return &_default_screen;
}

void delscreen(SCREEN *sp)
{
    (void)sp;
}

/* ========================================================================= */
/* Resize support                                                            */
/* ========================================================================= */

int resizeterm(int lines, int columns)
{
    LINES = lines;
    COLS = columns;
    if (stdscr) return wresize(stdscr, lines, columns);
    return OK;
}

int wresize(WINDOW *win, int lines, int columns)
{
    if (!win) return ERR;
    int new_size = lines * columns;
    chtype *new_line = (chtype *)calloc(new_size, sizeof(chtype));
    chtype *new_prev = (chtype *)calloc(new_size, sizeof(chtype));
    if (!new_line || !new_prev) {
        free(new_line);
        free(new_prev);
        return ERR;
    }

    /* Initialize with background */
    chtype fill = ' ' | (win->_bkgd & A_ATTRIBUTES);
    for (int i = 0; i < new_size; i++) {
        new_line[i] = fill;
        new_prev[i] = 0;
    }

    /* Copy old content */
    if (win->_line) {
        int copy_rows = win->_maxy < lines ? win->_maxy : lines;
        int copy_cols = win->_maxx < columns ? win->_maxx : columns;
        for (int y = 0; y < copy_rows; y++)
            for (int x = 0; x < copy_cols; x++)
                new_line[y * columns + x] = win->_line[y * win->_maxx + x];
        free(win->_line);
    }
    free(win->_prev);

    win->_line = new_line;
    win->_prev = new_prev;
    win->_maxy = lines;
    win->_maxx = columns;

    /* Clamp cursor */
    if (win->_cury >= lines) win->_cury = lines - 1;
    if (win->_curx >= columns) win->_curx = columns - 1;
    if (win->_cury < 0) win->_cury = 0;
    if (win->_curx < 0) win->_curx = 0;

    win->_clearok = 1;
    return OK;
}
