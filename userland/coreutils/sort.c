/* sort.c -- Sort lines of text
 *
 * VeridianOS coreutil.  Validates heap allocation, qsort with function
 * pointers, string processing, realloc.
 *
 * Usage: sort [-rnu] [FILE...]
 *   -r  Reverse sort order.
 *   -n  Numeric sort.
 *   -u  Output only unique lines.
 *
 * Self-test: sort /usr/src/sort_test.txt -> "apple\nbanana\ncherry\n"
 *            + SORT_PASS
 *
 * Syscalls exercised: open, read, write, close + malloc/realloc/strdup + qsort
 */

#include <fcntl.h>
#include <getopt.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define BUF_SIZE      4096
#define INITIAL_CAP   64

static char **lines;
static int line_count;
static int line_cap;

static int add_line(const char *s, int len)
{
    if (line_count >= line_cap) {
        int new_cap = line_cap * 2;
        char **new_lines = (char **)realloc(lines, new_cap * sizeof(char *));
        if (!new_lines) {
            write(2, "sort: out of memory\n", 20);
            return -1;
        }
        lines = new_lines;
        line_cap = new_cap;
    }

    /* Allocate and copy the line (strip trailing newline) */
    char *dup = (char *)malloc(len + 1);
    if (!dup) {
        write(2, "sort: out of memory\n", 20);
        return -1;
    }
    memcpy(dup, s, len);
    dup[len] = '\0';
    lines[line_count++] = dup;
    return 0;
}

static int read_lines(int fd)
{
    char buf[BUF_SIZE];
    char linebuf[BUF_SIZE];
    int linepos = 0;
    int n;

    while ((n = read(fd, buf, BUF_SIZE)) > 0) {
        for (int i = 0; i < n; i++) {
            if (buf[i] == '\n') {
                if (add_line(linebuf, linepos) < 0)
                    return -1;
                linepos = 0;
            } else {
                if (linepos < BUF_SIZE - 1)
                    linebuf[linepos++] = buf[i];
            }
        }
    }

    /* Handle final line without trailing newline */
    if (linepos > 0) {
        if (add_line(linebuf, linepos) < 0)
            return -1;
    }

    return 0;
}

static int cmp_str(const void *a, const void *b)
{
    const char *sa = *(const char *const *)a;
    const char *sb = *(const char *const *)b;
    return strcmp(sa, sb);
}

static int cmp_str_rev(const void *a, const void *b)
{
    return -cmp_str(a, b);
}

static int cmp_num(const void *a, const void *b)
{
    const char *sa = *(const char *const *)a;
    const char *sb = *(const char *const *)b;
    long na = atol(sa);
    long nb = atol(sb);
    if (na < nb) return -1;
    if (na > nb) return 1;
    return 0;
}

static int cmp_num_rev(const void *a, const void *b)
{
    return -cmp_num(a, b);
}

int main(int argc, char *argv[])
{
    int reverse = 0, numeric = 0, unique = 0;
    int opt;

    while ((opt = getopt(argc, argv, "rnu")) != -1) {
        switch (opt) {
        case 'r': reverse = 1; break;
        case 'n': numeric = 1; break;
        case 'u': unique = 1; break;
        default:
            write(2, "Usage: sort [-rnu] [FILE...]\n", 29);
            return 1;
        }
    }

    /* Allocate initial line array */
    line_cap = INITIAL_CAP;
    lines = (char **)malloc(line_cap * sizeof(char *));
    if (!lines) {
        write(2, "sort: out of memory\n", 20);
        return 1;
    }
    line_count = 0;

    /* Read input */
    int ret = 0;
    if (optind >= argc) {
        /* No file args: read stdin */
        if (read_lines(0) < 0)
            ret = 1;
    } else {
        for (int i = optind; i < argc; i++) {
            int fd;
            if (strcmp(argv[i], "-") == 0) {
                fd = 0;
            } else {
                fd = open(argv[i], O_RDONLY);
                if (fd < 0) {
                    perror(argv[i]);
                    ret = 1;
                    continue;
                }
            }
            if (read_lines(fd) < 0)
                ret = 1;
            if (fd != 0)
                close(fd);
        }
    }

    if (ret != 0 || line_count == 0)
        return ret;

    /* Sort */
    int (*cmpfn)(const void *, const void *);
    if (numeric)
        cmpfn = reverse ? cmp_num_rev : cmp_num;
    else
        cmpfn = reverse ? cmp_str_rev : cmp_str;

    qsort(lines, line_count, sizeof(char *), cmpfn);

    /* Output */
    for (int i = 0; i < line_count; i++) {
        if (unique && i > 0 && strcmp(lines[i], lines[i - 1]) == 0)
            continue;
        printf("%s\n", lines[i]);
    }

    return 0;
}
