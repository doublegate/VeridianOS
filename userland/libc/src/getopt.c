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

    /* glibc compatibility: setting optind=0 resets getopt state.
     * BusyBox relies on this to re-initialize between applet calls. */
    if (optind == 0) {
        optind = 1;
        __optpos = 0;
    }

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

/* ========================================================================= */
/* getopt_long                                                               */
/* ========================================================================= */

int getopt_long(int argc, char *const argv[], const char *optstring,
                const struct option *longopts, int *longindex)
{
    optarg = NULL;

    /* glibc compatibility: setting optind=0 resets getopt state.
     * BusyBox relies on this to re-initialize between applet calls.
     * Must be checked here (not only in getopt()) because early-exit
     * paths below bypass the getopt() delegation at the bottom. */
    if (optind == 0) {
        optind = 1;
        __optpos = 0;
    }

    if (optind >= argc)
        return -1;

    const char *arg = argv[optind];

    if (!arg || arg[0] != '-' || arg[1] == '\0')
        return -1;

    /* "--" terminates option scanning. */
    if (arg[1] == '-' && arg[2] == '\0') {
        optind++;
        return -1;
    }

    /* Long option: starts with "--" */
    if (arg[1] == '-' && arg[2] != '\0') {
        const char *name = arg + 2;

        /* Find '=' separator if present: --name=value */
        const char *eq = NULL;
        size_t name_len = 0;
        for (const char *p = name; *p; p++) {
            if (*p == '=') {
                eq = p;
                name_len = (size_t)(p - name);
                break;
            }
        }
        if (!eq)
            name_len = strlen(name);

        /* Search longopts for a match. */
        int match_idx = -1;
        for (int i = 0; longopts[i].name; i++) {
            if (strlen(longopts[i].name) == name_len &&
                memcmp(longopts[i].name, name, name_len) == 0) {
                match_idx = i;
                break;
            }
        }

        if (match_idx < 0) {
            if (opterr)
                fprintf(stderr, "%s: unrecognized option '--%.*s'\n",
                        argv[0], (int)name_len, name);
            optind++;
            return '?';
        }

        if (longindex)
            *longindex = match_idx;

        const struct option *opt = &longopts[match_idx];

        /* Handle argument. */
        if (opt->has_arg == no_argument) {
            if (eq) {
                if (opterr)
                    fprintf(stderr, "%s: option '--%s' doesn't allow an argument\n",
                            argv[0], opt->name);
                optind++;
                return '?';
            }
        } else if (opt->has_arg == required_argument) {
            if (eq) {
                optarg = (char *)(eq + 1);
            } else {
                optind++;
                if (optind >= argc) {
                    if (opterr)
                        fprintf(stderr, "%s: option '--%s' requires an argument\n",
                                argv[0], opt->name);
                    return '?';
                }
                optarg = (char *)argv[optind];
            }
        } else {
            /* optional_argument: only accept --name=value form */
            if (eq)
                optarg = (char *)(eq + 1);
        }

        optind++;

        if (opt->flag) {
            *opt->flag = opt->val;
            return 0;
        }
        return opt->val;
    }

    /* Short option: delegate to getopt(). */
    return getopt(argc, argv, optstring);
}
