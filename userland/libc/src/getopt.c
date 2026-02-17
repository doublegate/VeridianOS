/*
 * VeridianOS libc -- getopt.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX getopt() implementation for command-line option parsing.
 */

#include <getopt.h>
#include <string.h>
#include <stdio.h>

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

char *optarg = NULL;
int   optind = 1;
int   opterr = 1;
int   optopt = 0;

/*
 * Internal state: position within the current argv element when multiple
 * short options are bundled (e.g. "-abc").
 */
static int __optpos = 0;

/* ========================================================================= */
/* getopt                                                                    */
/* ========================================================================= */

int getopt(int argc, char *const argv[], const char *optstring)
{
    optarg = NULL;

    if (optind >= argc)
        return -1;

    const char *arg = argv[optind];

    /* Not an option, or bare "-", or "--" end-of-options marker. */
    if (!arg || arg[0] != '-' || arg[1] == '\0') {
        return -1;
    }

    if (arg[1] == '-' && arg[2] == '\0') {
        /* "--" terminates option scanning. */
        optind++;
        return -1;
    }

    /* Position within the current option cluster. */
    if (__optpos == 0)
        __optpos = 1;  /* Skip the leading '-'. */

    char c = arg[__optpos];
    __optpos++;

    /*
     * A leading ':' in optstring means: suppress error messages and
     * return ':' (instead of '?') when a required argument is missing.
     */
    int colon_mode = (optstring[0] == ':');
    const char *osp = colon_mode ? optstring + 1 : optstring;

    /* Look up the character in optstring. */
    const char *match = NULL;
    for (const char *p = osp; *p; p++) {
        if (*p == c) {
            match = p;
            break;
        }
    }

    if (!match) {
        /* Unknown option. */
        optopt = (unsigned char)c;
        if (arg[__optpos] == '\0') {
            optind++;
            __optpos = 0;
        }
        if (opterr && !colon_mode)
            fprintf(stderr, "%s: unknown option '-%c'\n", argv[0], c);
        return '?';
    }

    /* Check if this option requires an argument. */
    if (match[1] == ':') {
        /* Option requires an argument. */
        if (arg[__optpos] != '\0') {
            /* Argument is the rest of this argv element: -fVALUE */
            optarg = (char *)&arg[__optpos];
        } else {
            /* Argument is the next argv element: -f VALUE */
            optind++;
            if (optind >= argc) {
                optopt = (unsigned char)c;
                __optpos = 0;
                if (colon_mode)
                    return ':';
                if (opterr)
                    fprintf(stderr, "%s: option '-%c' requires an argument\n",
                            argv[0], c);
                return '?';
            }
            optarg = (char *)argv[optind];
        }
        optind++;
        __optpos = 0;
    } else {
        /* No argument required. */
        if (arg[__optpos] == '\0') {
            optind++;
            __optpos = 0;
        }
    }

    return (unsigned char)c;
}
