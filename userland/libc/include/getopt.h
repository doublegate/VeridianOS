/*
 * VeridianOS libc -- <getopt.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX command-line option parsing.
 */

#ifndef _GETOPT_H
#define _GETOPT_H

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

/** Points to the option-argument for the current option (e.g. -f VALUE). */
extern char *optarg;

/** Index of the next element in argv to be processed (starts at 1). */
extern int optind;

/** If non-zero, getopt prints error messages to stderr (default: 1). */
extern int opterr;

/** Set to the unrecognised option character on error. */
extern int optopt;

/* ========================================================================= */
/* getopt                                                                    */
/* ========================================================================= */

/**
 * Parse command-line options.
 *
 * @param argc       Argument count.
 * @param argv       Argument vector.
 * @param optstring  String of recognised option characters.  A colon after
 *                   a character means it requires an argument.
 * @return The next option character, '?' on error, or -1 when done.
 */
int getopt(int argc, char *const argv[], const char *optstring);

#ifdef __cplusplus
}
#endif

#endif /* _GETOPT_H */
