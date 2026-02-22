/*
 * sysinfo -- VeridianOS system information display
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * A purpose-built system info tool for VeridianOS, inspired by fastfetch.
 * Reads /proc files and calls uname() to display formatted system info
 * with an ASCII logo sidebar.
 *
 * Data sources:
 *   /proc/cpuinfo  -- CPU model, vendor, frequency
 *   /proc/meminfo  -- Memory total/free/used
 *   /proc/uptime   -- System uptime in seconds
 *   /proc/version  -- Kernel version string
 *   uname()        -- OS name, release, arch, hostname
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/utsname.h>

/* ========================================================================= */
/* ANSI escape codes                                                         */
/* ========================================================================= */

#define ESC       "\033["
#define BOLD      ESC "1m"
#define RESET     ESC "0m"
#define GREEN     ESC "32m"
#define CYAN      ESC "36m"
#define YELLOW    ESC "33m"
#define BLUE      ESC "34m"
#define MAGENTA   ESC "35m"
#define WHITE     ESC "37m"
#define BGREEN    ESC "1;32m"   /* Bold green */
#define BCYAN     ESC "1;36m"   /* Bold cyan  */

/* Label color for the left-hand key text */
#define LABEL     BGREEN
/* Value color */
#define VALUE     RESET

/* ========================================================================= */
/* ASCII logo (displayed alongside system info)                              */
/* ========================================================================= */

/*
 * The VeridianOS logo: a stylised "V" in green.
 * Each line is exactly 24 characters wide (padded with spaces).
 */
#define LOGO_WIDTH 26
#define LOGO_LINES 10

static const char *logo[LOGO_LINES] = {
    BGREEN "  \\\\    //               " RESET,
    BGREEN "   \\\\  //  ___  _ __     " RESET,
    BGREEN "    \\\\//  / _ \\| '__|    " RESET,
    BGREEN "     \\/  |  __/| |       " RESET,
    BGREEN "     /\\   \\___||_|       " RESET,
    GREEN  "    //\\\\    _     _ _    " RESET,
    GREEN  "   //  \\\\  (_) __| (_)   " RESET,
    GREEN  "  //    \\\\ | |/ _` | |   " RESET,
    GREEN  "         \\\\| | (_| | |   " RESET,
    GREEN  "          \\_\\__,_|_|     " RESET,
};

/* ========================================================================= */
/* /proc helpers                                                             */
/* ========================================================================= */

/*
 * Read the first line from a /proc file into buf.
 * Returns 0 on success, -1 on failure.
 */
static int read_proc_line(const char *path, char *buf, int size)
{
    FILE *f = fopen(path, "r");
    if (!f)
        return -1;
    if (!fgets(buf, size, f)) {
        fclose(f);
        return -1;
    }
    /* Strip trailing newline */
    int len = strlen(buf);
    if (len > 0 && buf[len - 1] == '\n')
        buf[len - 1] = '\0';
    fclose(f);
    return 0;
}

/*
 * Search a /proc file for a line starting with `key`, then extract the
 * value portion (everything after the first ':' and any following
 * whitespace).  Returns 0 on success, -1 if the key is not found.
 */
static int parse_proc_value(const char *path, const char *key,
                            char *buf, int size)
{
    FILE *f = fopen(path, "r");
    if (!f)
        return -1;

    char line[256];
    int keylen = strlen(key);

    while (fgets(line, (int)sizeof(line), f)) {
        if (strncmp(line, key, keylen) == 0) {
            /* Skip past the key and any ": \t" separator */
            char *val = line + keylen;
            while (*val == ':' || *val == ' ' || *val == '\t')
                val++;
            /* Strip trailing newline */
            int len = strlen(val);
            if (len > 0 && val[len - 1] == '\n')
                val[len - 1] = '\0';
            strncpy(buf, val, size - 1);
            buf[size - 1] = '\0';
            fclose(f);
            return 0;
        }
    }

    fclose(f);
    return -1;
}

/* ========================================================================= */
/* Info-line builder                                                         */
/* ========================================================================= */

/*
 * We collect up to MAX_INFO_LINES formatted information strings, then
 * print them side-by-side with the ASCII logo.
 */
#define MAX_INFO_LINES 16
#define INFO_BUF_SIZE  256

static char info_lines[MAX_INFO_LINES][INFO_BUF_SIZE];
static int  info_count = 0;

/*
 * Append a labelled info line:  "Label: value"
 */
static void add_info(const char *label, const char *value)
{
    if (info_count >= MAX_INFO_LINES)
        return;

    /* Build:  <LABEL>Label<RESET>: value */
    char *dst = info_lines[info_count];
    int n = 0;
    const char *lbl = LABEL;
    while (*lbl && n < INFO_BUF_SIZE - 1) dst[n++] = *lbl++;
    while (*label && n < INFO_BUF_SIZE - 1) dst[n++] = *label++;
    const char *rst = VALUE;
    while (*rst && n < INFO_BUF_SIZE - 1) dst[n++] = *rst++;
    const char *sep = ": ";
    while (*sep && n < INFO_BUF_SIZE - 1) dst[n++] = *sep++;
    while (*value && n < INFO_BUF_SIZE - 1) dst[n++] = *value++;
    dst[n] = '\0';

    info_count++;
}

/*
 * Append a separator / title line (no label: value formatting).
 */
static void add_info_raw(const char *text)
{
    if (info_count >= MAX_INFO_LINES)
        return;
    strncpy(info_lines[info_count], text, INFO_BUF_SIZE - 1);
    info_lines[info_count][INFO_BUF_SIZE - 1] = '\0';
    info_count++;
}

/* ========================================================================= */
/* Colour-bar helper                                                         */
/* ========================================================================= */

/*
 * Print a row of 8 coloured blocks (palette display).
 */
static void format_color_bar(char *buf, int size)
{
    /* Standard 8 terminal colours as background blocks */
    static const char *blocks =
        ESC "40m   " ESC "41m   " ESC "42m   " ESC "43m   "
        ESC "44m   " ESC "45m   " ESC "46m   " ESC "47m   " RESET;

    strncpy(buf, blocks, size - 1);
    buf[size - 1] = '\0';
}

/* ========================================================================= */
/* main                                                                      */
/* ========================================================================= */

int main(void)
{
    struct utsname uts;
    char buf[256];
    char buf2[256];

    /* ---- Collect system information ------------------------------------ */

    /* Title line: user@hostname */
    if (uname(&uts) == 0) {
        /* root@hostname */
        char title[256];
        int n = 0;
        const char *user_lbl = BCYAN "root" RESET "@" BCYAN;
        while (*user_lbl && n < 250) title[n++] = *user_lbl++;
        const char *hn = uts.nodename;
        while (*hn && n < 250) title[n++] = *hn++;
        const char *r = RESET;
        while (*r && n < 250) title[n++] = *r++;
        title[n] = '\0';
        add_info_raw(title);

        /* Separator line */
        add_info_raw("--------------------");

        /* OS */
        buf[0] = '\0';
        strncpy(buf, uts.sysname, sizeof(buf) - 1);
        buf[sizeof(buf) - 1] = '\0';
        /* Append release */
        {
            int len = strlen(buf);
            buf[len] = ' ';
            strncpy(buf + len + 1, uts.release, sizeof(buf) - len - 2);
            buf[sizeof(buf) - 1] = '\0';
            /* Append machine */
            len = strlen(buf);
            buf[len] = ' ';
            strncpy(buf + len + 1, uts.machine, sizeof(buf) - len - 2);
            buf[sizeof(buf) - 1] = '\0';
        }
        add_info("OS", buf);

        /* Host */
        add_info("Host", uts.nodename);

        /* Kernel */
        buf[0] = '\0';
        strncpy(buf, uts.release, sizeof(buf) - 1);
        buf[sizeof(buf) - 1] = '\0';
        add_info("Kernel", buf);
    }

    /* Uptime */
    if (read_proc_line("/proc/uptime", buf, sizeof(buf)) == 0) {
        /* Format: "seconds.frac idle_seconds" -- parse just the integer part */
        long seconds = atol(buf);
        long days  = seconds / 86400;
        long hours = (seconds % 86400) / 3600;
        long mins  = (seconds % 3600) / 60;
        long secs  = seconds % 60;

        buf2[0] = '\0';
        int pos = 0;
        if (days > 0) {
            /* manual int-to-string since snprintf may not exist */
            char tmp[32];
            int ti = 0;
            long v = days;
            if (v == 0) tmp[ti++] = '0';
            else while (v > 0) { tmp[ti++] = '0' + (v % 10); v /= 10; }
            while (ti > 0 && pos < 200) buf2[pos++] = tmp[--ti];
            const char *sfx = " day(s), ";
            while (*sfx && pos < 200) buf2[pos++] = *sfx++;
        }
        {
            /* HH:MM:SS or MM:SS or Ns */
            if (hours > 0) {
                if (hours >= 10) buf2[pos++] = '0' + (hours / 10);
                buf2[pos++] = '0' + (hours % 10);
                buf2[pos++] = 'h';
                buf2[pos++] = ' ';
            }
            if (mins > 0 || hours > 0) {
                if (mins >= 10) buf2[pos++] = '0' + (mins / 10);
                buf2[pos++] = '0' + (mins % 10);
                buf2[pos++] = 'm';
                buf2[pos++] = ' ';
            }
            if (secs >= 10) buf2[pos++] = '0' + (secs / 10);
            buf2[pos++] = '0' + (secs % 10);
            buf2[pos++] = 's';
        }
        buf2[pos] = '\0';
        add_info("Uptime", buf2);
    }

    /* Shell */
    add_info("Shell", "/bin/sh");

    /* CPU */
    if (parse_proc_value("/proc/cpuinfo", "model name", buf, sizeof(buf)) == 0) {
        add_info("CPU", buf);
    }

    /* CPU frequency */
    if (parse_proc_value("/proc/cpuinfo", "cpu MHz", buf, sizeof(buf)) == 0) {
        /* Append " MHz" */
        int len = strlen(buf);
        if (len + 5 < (int)sizeof(buf)) {
            buf[len] = ' ';
            buf[len + 1] = 'M';
            buf[len + 2] = 'H';
            buf[len + 3] = 'z';
            buf[len + 4] = '\0';
        }
        add_info("CPU Freq", buf);
    }

    /* Memory */
    {
        long total_kb = 0, free_kb = 0;
        if (parse_proc_value("/proc/meminfo", "MemTotal", buf, sizeof(buf)) == 0)
            total_kb = atol(buf);
        if (parse_proc_value("/proc/meminfo", "MemFree", buf, sizeof(buf)) == 0)
            free_kb = atol(buf);

        if (total_kb > 0) {
            long used_kb = total_kb > free_kb ? total_kb - free_kb : 0;
            long total_mb = total_kb / 1024;
            long used_mb  = used_kb  / 1024;

            /* Build "USED MiB / TOTAL MiB (XX%)" */
            int pos = 0;
            char tmp[32];
            int ti;
            long v;

            /* used_mb */
            v = used_mb;
            ti = 0;
            if (v == 0) tmp[ti++] = '0';
            else while (v > 0) { tmp[ti++] = '0' + (v % 10); v /= 10; }
            while (ti > 0 && pos < 200) buf2[pos++] = tmp[--ti];

            const char *s1 = " MiB / ";
            while (*s1 && pos < 200) buf2[pos++] = *s1++;

            /* total_mb */
            v = total_mb;
            ti = 0;
            if (v == 0) tmp[ti++] = '0';
            else while (v > 0) { tmp[ti++] = '0' + (v % 10); v /= 10; }
            while (ti > 0 && pos < 200) buf2[pos++] = tmp[--ti];

            const char *s2 = " MiB";
            while (*s2 && pos < 200) buf2[pos++] = *s2++;

            /* percentage */
            if (total_kb > 0) {
                long pct = (used_kb * 100) / total_kb;
                buf2[pos++] = ' ';
                buf2[pos++] = '(';
                v = pct;
                ti = 0;
                if (v == 0) tmp[ti++] = '0';
                else while (v > 0) { tmp[ti++] = '0' + (v % 10); v /= 10; }
                while (ti > 0 && pos < 200) buf2[pos++] = tmp[--ti];
                buf2[pos++] = '%';
                buf2[pos++] = ')';
            }
            buf2[pos] = '\0';
            add_info("Memory", buf2);
        }
    }

    /* Kernel version from /proc/version */
    if (read_proc_line("/proc/version", buf, sizeof(buf)) == 0) {
        add_info("Version", buf);
    }

    /* Blank line + colour bar */
    add_info_raw("");
    {
        char cbar[512];
        format_color_bar(cbar, sizeof(cbar));
        add_info_raw(cbar);
    }

    /* ---- Render: logo on the left, info on the right ------------------- */

    int max_lines = LOGO_LINES > info_count ? LOGO_LINES : info_count;
    printf("\n");

    for (int i = 0; i < max_lines; i++) {
        /* Logo column */
        if (i < LOGO_LINES) {
            printf("%s", logo[i]);
        } else {
            /* Pad with spaces to logo width */
            for (int j = 0; j < LOGO_WIDTH; j++)
                printf(" ");
        }

        /* Info column */
        printf("  ");
        if (i < info_count) {
            printf("%s", info_lines[i]);
        }

        printf("\n");
    }

    printf("\n");
    return 0;
}
