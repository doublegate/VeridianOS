/* ls.c -- List directory contents
 *
 * VeridianOS coreutil.  Validates opendir/readdir/closedir, stat, qsort,
 * printf formatting, gmtime.
 *
 * Usage: ls [-la1] [DIR...]
 *   -l  Long format (permissions, nlink, size, date, name).
 *   -a  Show hidden entries (names starting with '.').
 *   -1  One entry per line (default).
 *
 * Self-test: ls /usr/src/ -> lists known files + LS_PASS
 *
 * Syscalls exercised: open, getdents64, stat, write, close + qsort
 */

#include <dirent.h>
#include <getopt.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define MAX_ENTRIES 256
#define PATH_BUF    512

struct entry {
    char name[NAME_MAX + 1];
    struct stat st;
    int has_stat;
};

static struct entry entries[MAX_ENTRIES];

static int cmp_name(const void *a, const void *b)
{
    const struct entry *ea = (const struct entry *)a;
    const struct entry *eb = (const struct entry *)b;
    return strcmp(ea->name, eb->name);
}

static void format_permissions(mode_t mode, char *buf)
{
    buf[0] = S_ISDIR(mode) ? 'd' :
             S_ISLNK(mode) ? 'l' :
             S_ISCHR(mode) ? 'c' :
             S_ISBLK(mode) ? 'b' :
             S_ISFIFO(mode) ? 'p' :
             S_ISSOCK(mode) ? 's' : '-';
    buf[1] = (mode & S_IRUSR) ? 'r' : '-';
    buf[2] = (mode & S_IWUSR) ? 'w' : '-';
    buf[3] = (mode & S_IXUSR) ? 'x' : '-';
    buf[4] = (mode & S_IRGRP) ? 'r' : '-';
    buf[5] = (mode & S_IWGRP) ? 'w' : '-';
    buf[6] = (mode & S_IXGRP) ? 'x' : '-';
    buf[7] = (mode & S_IROTH) ? 'r' : '-';
    buf[8] = (mode & S_IWOTH) ? 'w' : '-';
    buf[9] = (mode & S_IXOTH) ? 'x' : '-';
    buf[10] = '\0';
}

static void format_time(time_t mtime, char *buf, int bufsize)
{
    static const char *months[] = {
        "Jan", "Feb", "Mar", "Apr", "May", "Jun",
        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
    };
    struct tm *tm = gmtime(&mtime);
    if (tm && tm->tm_mon >= 0 && tm->tm_mon < 12) {
        snprintf(buf, bufsize, "%s %2d %02d:%02d",
                 months[tm->tm_mon], tm->tm_mday,
                 tm->tm_hour, tm->tm_min);
    } else {
        snprintf(buf, bufsize, "Jan  1 00:00");
    }
}

static int list_dir(const char *path, int show_all, int long_fmt)
{
    DIR *dir = opendir(path);
    if (!dir) {
        perror(path);
        return 1;
    }

    int count = 0;
    struct dirent *de;
    while ((de = readdir(dir)) != NULL && count < MAX_ENTRIES) {
        if (!show_all && de->d_name[0] == '.')
            continue;

        strncpy(entries[count].name, de->d_name, NAME_MAX);
        entries[count].name[NAME_MAX] = '\0';

        if (long_fmt) {
            char fullpath[PATH_BUF];
            int plen = strlen(path);
            /* Build full path: path + '/' + name */
            if (plen > 0 && path[plen - 1] == '/') {
                snprintf(fullpath, PATH_BUF, "%s%s", path, de->d_name);
            } else {
                snprintf(fullpath, PATH_BUF, "%s/%s", path, de->d_name);
            }
            if (stat(fullpath, &entries[count].st) == 0) {
                entries[count].has_stat = 1;
            } else {
                entries[count].has_stat = 0;
            }
        } else {
            entries[count].has_stat = 0;
        }

        count++;
    }
    closedir(dir);

    qsort(entries, count, sizeof(struct entry), cmp_name);

    for (int i = 0; i < count; i++) {
        if (long_fmt && entries[i].has_stat) {
            char perms[12];
            char timebuf[32];
            format_permissions(entries[i].st.st_mode, perms);
            format_time(entries[i].st.st_mtime, timebuf, sizeof(timebuf));
            printf("%s %2ld %8ld %s %s\n",
                   perms,
                   (long)entries[i].st.st_nlink,
                   (long)entries[i].st.st_size,
                   timebuf,
                   entries[i].name);
        } else {
            printf("%s\n", entries[i].name);
        }
    }

    return 0;
}

int main(int argc, char *argv[])
{
    int show_all = 0, long_fmt = 0;
    int opt;

    while ((opt = getopt(argc, argv, "la1")) != -1) {
        switch (opt) {
        case 'l': long_fmt = 1; break;
        case 'a': show_all = 1; break;
        case '1': break; /* already default */
        default:
            write(2, "Usage: ls [-la1] [DIR...]\n", 26);
            return 1;
        }
    }

    int ret = 0;
    int nargs = argc - optind;

    if (nargs == 0) {
        ret = list_dir(".", show_all, long_fmt);
    } else if (nargs == 1) {
        ret = list_dir(argv[optind], show_all, long_fmt);
    } else {
        for (int i = optind; i < argc; i++) {
            if (i > optind)
                printf("\n");
            printf("%s:\n", argv[i]);
            if (list_dir(argv[i], show_all, long_fmt) != 0)
                ret = 1;
        }
    }

    return ret;
}
