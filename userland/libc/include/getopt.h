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

/* ========================================================================= */
/* getopt_long                                                               */
/* ========================================================================= */

/** Argument requirement constants for struct option. */
#define no_argument        0
#define required_argument  1
#define optional_argument  2

/**
 * Long option descriptor for getopt_long().
 */
struct option {
    /** Long option name (without leading "--"). */
    const char *name;
    /** no_argument (0), required_argument (1), or optional_argument (2). */
    int         has_arg;
    /** If non-NULL, set *flag to val and return 0.  If NULL, return val. */
    int        *flag;
    /** Value to return (or store in *flag). */
    int         val;
};

/**
 * Parse long and short command-line options.
 *
 * Processes "--name", "--name=value", and short "-x" options.
 *
 * @param argc       Argument count.
 * @param argv       Argument vector.
 * @param optstring  Short-option characters (as for getopt()).
 * @param longopts   NULL-terminated array of struct option.
 * @param longindex  If non-NULL, set to the index of the matched long option.
 * @return Option character (short), val/0 (long), '?' on error, -1 when done.
 */
int getopt_long(int argc, char *const argv[], const char *optstring,
                const struct option *longopts, int *longindex);

#ifdef __cplusplus
}
#endif

#endif /* _GETOPT_H */
