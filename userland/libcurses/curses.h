/*
 * VeridianOS libcurses -- <curses.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal curses implementation for VeridianOS.
 * Provides core functionality via ANSI escape sequences and termios raw mode.
 * Designed to support nano and other basic TUI applications.
 *
 * This is NOT a full ncurses replacement -- only commonly used functions
 * are implemented. The terminal is assumed to be VT100/xterm compatible.
 */

#ifndef _CURSES_H
#define _CURSES_H

#include <stdarg.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

typedef unsigned int chtype;
typedef chtype attr_t;

#ifndef TRUE
#define TRUE  1
#endif
#ifndef FALSE
#define FALSE 0
#endif
#ifndef ERR
#define ERR (-1)
#endif
#ifndef OK
#define OK  0
#endif

#if !defined(__cplusplus) && !defined(__bool_true_false_are_defined)
typedef int bool;
#endif

/* ========================================================================= */
/* WINDOW structure                                                          */
/* ========================================================================= */

/*
 * Internal window structure.  The virtual screen buffer stores characters
 * and attributes for diff-based refresh().
 */
typedef struct _win_st {
    int _cury, _curx;       /* Current cursor position */
    int _maxy, _maxx;       /* Window dimensions (max row/col, 0-indexed) */
    int _begy, _begx;       /* Window origin on screen */
    int _flags;             /* Window state flags */
    attr_t _attrs;          /* Current default attributes */
    chtype _bkgd;           /* Background char + attributes */
    int _scroll;            /* Scrolling enabled */
    int _idlok;             /* Use insert/delete line */
    int _idcok;             /* Use insert/delete char */
    int _immed;             /* Immediate refresh */
    int _sync;              /* Sync with parent */
    int _keypad;            /* Keypad mode enabled */
    int _nodelay;           /* Non-blocking getch */
    int _notimeout;         /* No escape sequence timeout */
    int _leaveok;           /* Leave cursor after update */
    int _clearok;           /* Force clear on next refresh */
    chtype *_line;          /* Virtual screen buffer [_maxy * _maxx] */
    chtype *_prev;          /* Previous screen for diff [_maxy * _maxx] */
    struct _win_st *_parent; /* Parent window (for subwindows) */
} WINDOW;

/* ========================================================================= */
/* Standard windows and globals                                              */
/* ========================================================================= */

extern WINDOW *stdscr;
extern WINDOW *curscr;
extern int LINES;
extern int COLS;

/* ========================================================================= */
/* Attribute constants                                                       */
/* ========================================================================= */

#define A_NORMAL     0x00000000U
#define A_STANDOUT   0x00010000U
#define A_UNDERLINE  0x00020000U
#define A_REVERSE    0x00040000U
#define A_BLINK      0x00080000U
#define A_DIM        0x00100000U
#define A_BOLD       0x00200000U
#define A_INVIS      0x00800000U
#define A_PROTECT    0x01000000U
#define A_ALTCHARSET 0x02000000U
#define A_CHARTEXT   0x000000FFU
#define A_ATTRIBUTES 0xFFFFFF00U
#define A_COLOR      0x0000FF00U

/* Color pair encoding: bits 8-15 hold the pair number */
#define COLOR_PAIR(n) (((n) & 0xFF) << 8)
#define PAIR_NUMBER(a) (((a) >> 8) & 0xFF)

/* ========================================================================= */
/* Color constants                                                           */
/* ========================================================================= */

#define COLOR_BLACK   0
#define COLOR_RED     1
#define COLOR_GREEN   2
#define COLOR_YELLOW  3
#define COLOR_BLUE    4
#define COLOR_MAGENTA 5
#define COLOR_CYAN    6
#define COLOR_WHITE   7

#define COLORS 8
#define COLOR_PAIRS 64

/* ========================================================================= */
/* Key constants                                                             */
/* ========================================================================= */

/* Function keys start above 0x100 to avoid collision with normal chars */
#define KEY_MIN       0x101
#define KEY_DOWN      0x102
#define KEY_UP        0x103
#define KEY_LEFT      0x104
#define KEY_RIGHT     0x105
#define KEY_HOME      0x106
#define KEY_BACKSPACE 0x107
#define KEY_F0        0x108
#define KEY_F(n)      (KEY_F0 + (n))
#define KEY_DL        0x148
#define KEY_IL        0x149
#define KEY_DC        0x14A
#define KEY_IC        0x14B
#define KEY_EIC       0x14C
#define KEY_CLEAR     0x14D
#define KEY_EOS       0x14E
#define KEY_EOL       0x14F
#define KEY_SF        0x150
#define KEY_SR        0x151
#define KEY_NPAGE     0x152
#define KEY_PPAGE     0x153
#define KEY_STAB      0x154
#define KEY_CTAB      0x155
#define KEY_CATAB     0x156
#define KEY_ENTER     0x157
#define KEY_PRINT     0x15A
#define KEY_LL        0x15B
#define KEY_A1        0x15C
#define KEY_A3        0x15D
#define KEY_B2        0x15E
#define KEY_C1        0x15F
#define KEY_C3        0x160
#define KEY_BTAB      0x161
#define KEY_BEG       0x162
#define KEY_END       0x166
#define KEY_SUSPEND   0x17A
#define KEY_UNDO      0x17C
#define KEY_MOUSE     0x199
#define KEY_RESIZE    0x19A
#define KEY_MAX       0x1FF

/* ========================================================================= */
/* ACS (Alternate Character Set) line-drawing characters                     */
/* ========================================================================= */

extern chtype acs_map[128];

#define ACS_ULCORNER acs_map['l']
#define ACS_LLCORNER acs_map['m']
#define ACS_URCORNER acs_map['k']
#define ACS_LRCORNER acs_map['j']
#define ACS_LTEE     acs_map['t']
#define ACS_RTEE     acs_map['u']
#define ACS_BTEE     acs_map['v']
#define ACS_TTEE     acs_map['w']
#define ACS_HLINE    acs_map['q']
#define ACS_VLINE    acs_map['x']
#define ACS_PLUS     acs_map['n']

/* ========================================================================= */
/* Initialization and termination                                            */
/* ========================================================================= */

WINDOW *initscr(void);
int endwin(void);
int isendwin(void);

/* ========================================================================= */
/* Input mode control                                                        */
/* ========================================================================= */

int raw(void);
int noraw(void);
int cbreak(void);
int nocbreak(void);
int echo(void);
int noecho(void);
int nl(void);
int nonl(void);
int keypad(WINDOW *win, int bf);
int nodelay(WINDOW *win, int bf);
int notimeout(WINDOW *win, int bf);
int meta(WINDOW *win, int bf);
void timeout(int delay);
void wtimeout(WINDOW *win, int delay);
int halfdelay(int tenths);
int typeahead(int fd);
int intrflush(WINDOW *win, int bf);

/* ========================================================================= */
/* Output / cursor                                                           */
/* ========================================================================= */

int move(int y, int x);
int wmove(WINDOW *win, int y, int x);

int addch(const chtype ch);
int waddch(WINDOW *win, const chtype ch);
int mvaddch(int y, int x, const chtype ch);
int mvwaddch(WINDOW *win, int y, int x, const chtype ch);
int echochar(const chtype ch);
int wechochar(WINDOW *win, const chtype ch);

int addstr(const char *str);
int addnstr(const char *str, int n);
int waddstr(WINDOW *win, const char *str);
int waddnstr(WINDOW *win, const char *str, int n);
int mvaddstr(int y, int x, const char *str);
int mvaddnstr(int y, int x, const char *str, int n);
int mvwaddstr(WINDOW *win, int y, int x, const char *str);
int mvwaddnstr(WINDOW *win, int y, int x, const char *str, int n);

int printw(const char *fmt, ...);
int wprintw(WINDOW *win, const char *fmt, ...);
int mvprintw(int y, int x, const char *fmt, ...);
int mvwprintw(WINDOW *win, int y, int x, const char *fmt, ...);
int vwprintw(WINDOW *win, const char *fmt, va_list varglist);
int vw_printw(WINDOW *win, const char *fmt, va_list varglist);

/* ========================================================================= */
/* Input                                                                     */
/* ========================================================================= */

int getch(void);
int wgetch(WINDOW *win);
int mvgetch(int y, int x);
int mvwgetch(WINDOW *win, int y, int x);
int ungetch(int ch);
int has_key(int ch);

/* ========================================================================= */
/* Screen refresh                                                            */
/* ========================================================================= */

int refresh(void);
int wrefresh(WINDOW *win);
int wnoutrefresh(WINDOW *win);
int doupdate(void);
int redrawwin(WINDOW *win);
int wredrawln(WINDOW *win, int beg_included, int num_lines);

/* ========================================================================= */
/* Clearing                                                                  */
/* ========================================================================= */

int erase(void);
int werase(WINDOW *win);
int clear(void);
int wclear(WINDOW *win);
int clrtobot(void);
int wclrtobot(WINDOW *win);
int clrtoeol(void);
int wclrtoeol(WINDOW *win);

/* ========================================================================= */
/* Attribute control                                                         */
/* ========================================================================= */

int attron(int attrs);
int attroff(int attrs);
int attrset(int attrs);
int wattron(WINDOW *win, int attrs);
int wattroff(WINDOW *win, int attrs);
int wattrset(WINDOW *win, int attrs);
int attr_on(attr_t attrs, void *opts);
int attr_off(attr_t attrs, void *opts);
int attr_set(attr_t attrs, short pair, void *opts);
int wattr_on(WINDOW *win, attr_t attrs, void *opts);
int wattr_off(WINDOW *win, attr_t attrs, void *opts);
int wattr_set(WINDOW *win, attr_t attrs, short pair, void *opts);
int attr_get(attr_t *attrs, short *pair, void *opts);
int wattr_get(WINDOW *win, attr_t *attrs, short *pair, void *opts);
int standout(void);
int standend(void);
int wstandout(WINDOW *win);
int wstandend(WINDOW *win);

/* ========================================================================= */
/* Color                                                                     */
/* ========================================================================= */

int has_colors(void);
int can_change_color(void);
int start_color(void);
int init_pair(short pair, short f, short b);
int init_color(short color, short r, short g, short b);
int pair_content(short pair, short *f, short *b);
int color_content(short color, short *r, short *g, short *b);
int use_default_colors(void);
int assume_default_colors(int fg, int bg);

/* ========================================================================= */
/* Windows                                                                   */
/* ========================================================================= */

WINDOW *newwin(int nlines, int ncols, int begin_y, int begin_x);
int delwin(WINDOW *win);
WINDOW *subwin(WINDOW *orig, int nlines, int ncols, int begin_y, int begin_x);
WINDOW *derwin(WINDOW *orig, int nlines, int ncols, int begin_y, int begin_x);
WINDOW *dupwin(WINDOW *win);
int mvwin(WINDOW *win, int y, int x);
int mvderwin(WINDOW *win, int y, int x);
void wsyncup(WINDOW *win);
void wsyncdown(WINDOW *win);
int syncok(WINDOW *win, int bf);

/* ========================================================================= */
/* Scrolling                                                                 */
/* ========================================================================= */

int scrollok(WINDOW *win, int bf);
int scroll(WINDOW *win);
int wscrl(WINDOW *win, int n);
int idlok(WINDOW *win, int bf);
int wsetscrreg(WINDOW *win, int top, int bot);
int setscrreg(int top, int bot);

/* ========================================================================= */
/* Insert / Delete                                                           */
/* ========================================================================= */

int delch(void);
int wdelch(WINDOW *win);
int mvdelch(int y, int x);
int mvwdelch(WINDOW *win, int y, int x);
int insch(chtype ch);
int winsch(WINDOW *win, chtype ch);
int mvinsch(int y, int x, chtype ch);
int mvwinsch(WINDOW *win, int y, int x, chtype ch);
int insdelln(int n);
int winsdelln(WINDOW *win, int n);
int insertln(void);
int winsertln(WINDOW *win);
int deleteln(void);
int wdeleteln(WINDOW *win);

/* ========================================================================= */
/* Line drawing / borders                                                    */
/* ========================================================================= */

int border(chtype ls, chtype rs, chtype ts, chtype bs,
           chtype tl, chtype tr, chtype bl, chtype br);
int wborder(WINDOW *win, chtype ls, chtype rs, chtype ts, chtype bs,
            chtype tl, chtype tr, chtype bl, chtype br);
int box(WINDOW *win, chtype verch, chtype horch);
int hline(chtype ch, int n);
int whline(WINDOW *win, chtype ch, int n);
int vline(chtype ch, int n);
int wvline(WINDOW *win, chtype ch, int n);
int mvhline(int y, int x, chtype ch, int n);
int mvwhline(WINDOW *win, int y, int x, chtype ch, int n);
int mvvline(int y, int x, chtype ch, int n);
int mvwvline(WINDOW *win, int y, int x, chtype ch, int n);

/* ========================================================================= */
/* Misc window properties                                                    */
/* ========================================================================= */

int clearok(WINDOW *win, int bf);
int leaveok(WINDOW *win, int bf);
int immedok(WINDOW *win, int bf);
int idcok(WINDOW *win, int bf);
void bkgdset(chtype ch);
void wbkgdset(WINDOW *win, chtype ch);
int bkgd(chtype ch);
int wbkgd(WINDOW *win, chtype ch);
chtype getbkgd(WINDOW *win);

/* ========================================================================= */
/* Querying                                                                  */
/* ========================================================================= */

int getcury(WINDOW *win);
int getcurx(WINDOW *win);
int getbegy(WINDOW *win);
int getbegx(WINDOW *win);
int getmaxy(WINDOW *win);
int getmaxx(WINDOW *win);
int getpary(WINDOW *win);
int getparx(WINDOW *win);
chtype inch(void);
chtype winch(WINDOW *win);
chtype mvinch(int y, int x);
chtype mvwinch(WINDOW *win, int y, int x);

/* ========================================================================= */
/* Cursor visibility                                                         */
/* ========================================================================= */

int curs_set(int visibility);

/* ========================================================================= */
/* Beep / flash                                                              */
/* ========================================================================= */

int beep(void);
int flash(void);

/* ========================================================================= */
/* Miscellaneous                                                             */
/* ========================================================================= */

char *keyname(int c);
int napms(int ms);
int def_prog_mode(void);
int def_shell_mode(void);
int reset_prog_mode(void);
int reset_shell_mode(void);
int resetty(void);
int savetty(void);
int baudrate(void);
char erasechar(void);
char killchar(void);
int has_ic(void);
int has_il(void);
char *longname(void);
char *termname(void);
int flushinp(void);
int ungetch(int ch);

/* SCREEN type (opaque) -- minimal support for nano */
typedef struct _screen SCREEN;
SCREEN *newterm(const char *type, void *outfd, void *infd);
SCREEN *set_term(SCREEN *new_scr);
void delscreen(SCREEN *sp);

/* Resize support */
int resizeterm(int lines, int columns);
int wresize(WINDOW *win, int lines, int columns);

/* ========================================================================= */
/* Convenience macros                                                        */
/* ========================================================================= */

#define getyx(win, y, x)    ((y) = (win)->_cury, (x) = (win)->_curx)
#define getbegyx(win, y, x) ((y) = (win)->_begy, (x) = (win)->_begx)
#define getmaxyx(win, y, x) ((y) = (win)->_maxy, (x) = (win)->_maxx)
#define getparyx(win, y, x) ((y) = (win)->_parent ? (win)->_begy - (win)->_parent->_begy : -1, \
                              (x) = (win)->_parent ? (win)->_begx - (win)->_parent->_begx : -1)

/* Macros that delegate to w* functions on stdscr */
#define getnstr(s, n)       wgetnstr(stdscr, s, n)
#define getstr(s)           wgetstr(stdscr, s)
#define mvgetnstr(y,x,s,n)  (wmove(stdscr,y,x) == ERR ? ERR : wgetnstr(stdscr,s,n))

/* Input string functions (needed by some apps) */
int wgetnstr(WINDOW *win, char *str, int n);
int wgetstr(WINDOW *win, char *str);

#ifdef __cplusplus
}
#endif

#endif /* _CURSES_H */
