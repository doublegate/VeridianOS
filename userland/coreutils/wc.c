/* wc.c -- Word, line, and byte count
 *
 * VeridianOS coreutil.  Validates ctype.h, getopt, printf formatting.
 *
 * Usage: wc [-lwc] [FILE...]
 *   -l  Print line count only.
 *   -w  Print word count only.
 *   -c  Print byte count only.
 *   Default: print all three.
 *
 * Self-test: wc /usr/src/wc_test.txt -> "3 5 24" + WC_PASS
 *
 * Syscalls exercised: open, read, write, close + getopt parsing
 */

#include <ctype.h>
#include <fcntl.h>
#include <getopt.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#define BUF_SIZE 4096

static void wc_fd(int fd, long *out_lines, long *out_words, long *out_bytes)
{
    char buf[BUF_SIZE];
    long lines = 0, words = 0, bytes = 0;
    int in_word = 0;
    int n;

    while ((n = read(fd, buf, BUF_SIZE)) > 0) {
        bytes += n;
        for (int i = 0; i < n; i++) {
            if (buf[i] == '\n')
                lines++;
            if (isspace((unsigned char)buf[i])) {
                in_word = 0;
            } else if (!in_word) {
                in_word = 1;
                words++;
            }
        }
    }

    *out_lines = lines;
    *out_words = words;
    *out_bytes = bytes;
}

static void print_counts(long lines, long words, long bytes,
                         int show_l, int show_w, int show_c,
                         const char *name)
{
    if (show_l) printf("%7ld", lines);
    if (show_w) printf("%7ld", words);
    if (show_c) printf("%7ld", bytes);
    if (name)
        printf(" %s", name);
    printf("\n");
}

int main(int argc, char *argv[])
{
    int show_l = 0, show_w = 0, show_c = 0;
    int opt;

    while ((opt = getopt(argc, argv, "lwc")) != -1) {
        switch (opt) {
        case 'l': show_l = 1; break;
        case 'w': show_w = 1; break;
        case 'c': show_c = 1; break;
        default:
            write(2, "Usage: wc [-lwc] [FILE...]\n", 27);
            return 1;
        }
    }

    /* Default: show all */
    if (!show_l && !show_w && !show_c) {
        show_l = show_w = show_c = 1;
    }

    int nfiles = argc - optind;
    long total_l = 0, total_w = 0, total_c = 0;
    int ret = 0;

    if (nfiles == 0) {
        long l, w, c;
        wc_fd(0, &l, &w, &c);
        print_counts(l, w, c, show_l, show_w, show_c, NULL);
        return 0;
    }

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

        long l, w, c;
        wc_fd(fd, &l, &w, &c);
        print_counts(l, w, c, show_l, show_w, show_c, argv[i]);

        total_l += l;
        total_w += w;
        total_c += c;

        if (fd != 0)
            close(fd);
    }

    if (nfiles > 1) {
        print_counts(total_l, total_w, total_c, show_l, show_w, show_c, "total");
    }

    return ret;
}
