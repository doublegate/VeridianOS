/*
 * VeridianOS libcurses -- <ncurses.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Compatibility header -- just includes curses.h.
 * Many programs (including nano) include <ncurses.h>.
 */

#ifndef _NCURSES_H
#define _NCURSES_H

#include <curses.h>

/* ncurses version info (nano checks these) */
#ifndef NCURSES_VERSION_MAJOR
#define NCURSES_VERSION_MAJOR 6
#endif
#ifndef NCURSES_VERSION_MINOR
#define NCURSES_VERSION_MINOR 4
#endif
#ifndef NCURSES_VERSION_PATCH
#define NCURSES_VERSION_PATCH 20230520
#endif
#ifndef NCURSES_VERSION
#define NCURSES_VERSION       "6.4"
#endif
#ifndef NCURSES_MOUSE_VERSION
#define NCURSES_MOUSE_VERSION 2
#endif

#endif /* _NCURSES_H */
