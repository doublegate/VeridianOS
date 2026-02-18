/*
 * VeridianOS User-Space Shell -- /bin/sh
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal POSIX-like shell for VeridianOS user space.
 *
 * Supported features:
 *   - Interactive mode with prompt
 *   - Non-interactive: sh -c "command"
 *   - Simple command execution via fork/exec/wait
 *   - Pipelines: cmd1 | cmd2 | cmd3
 *   - I/O redirection: < > >> 2>
 *   - Background execution: cmd &
 *   - Built-in commands: cd, exit, echo, pwd, export, unset, exec, test/[
 *   - Environment variable expansion: $VAR, ${VAR}
 *   - Quoting: "double" and 'single'
 *   - Comment lines: # ...
 *   - Exit status: $?
 *   - Command separator: ;
 *   - Conditional execution: && and ||
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/wait.h>
#include <sys/stat.h>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

#define MAX_LINE    4096
#define MAX_ARGS    256
#define MAX_PIPES   16

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static int last_exit_status = 0;

/* ========================================================================= */
/* String utilities                                                          */
/* ========================================================================= */

/* Skip leading whitespace, return pointer to first non-space char. */
static char *skip_spaces(char *s)
{
    while (*s == ' ' || *s == '\t')
        s++;
    return s;
}

/* Strip trailing newline/whitespace from a string. */
static void strip_trailing(char *s)
{
    size_t len = strlen(s);
    while (len > 0 && (s[len - 1] == '\n' || s[len - 1] == '\r' ||
                       s[len - 1] == ' '  || s[len - 1] == '\t'))
        s[--len] = '\0';
}

/* ========================================================================= */
/* Variable expansion                                                        */
/* ========================================================================= */

/*
 * Expand $VAR, ${VAR}, $?, and $$ in the input string.
 * Writes the result into out (which must be at least MAX_LINE bytes).
 */
static void expand_variables(const char *in, char *out, size_t outsize)
{
    size_t oi = 0;
    size_t maxout = outsize - 1;

    while (*in && oi < maxout) {
        if (*in == '\'') {
            /* Single-quoted: copy literally until closing quote */
            in++;
            while (*in && *in != '\'' && oi < maxout)
                out[oi++] = *in++;
            if (*in == '\'')
                in++;
            continue;
        }

        if (*in == '$') {
            in++;
            const char *val = NULL;
            char numbuf[16];

            if (*in == '?') {
                /* $? -- last exit status */
                in++;
                int n = last_exit_status;
                int pos = 0;
                if (n < 0) { numbuf[pos++] = '-'; n = -n; }
                if (n == 0) {
                    numbuf[pos++] = '0';
                } else {
                    char tmp[12];
                    int ti = 0;
                    while (n > 0) { tmp[ti++] = '0' + (n % 10); n /= 10; }
                    while (ti > 0) numbuf[pos++] = tmp[--ti];
                }
                numbuf[pos] = '\0';
                val = numbuf;
            } else if (*in == '$') {
                /* $$ -- PID */
                in++;
                pid_t pid = getpid();
                int pos = 0;
                if (pid == 0) {
                    numbuf[pos++] = '0';
                } else {
                    char tmp[12];
                    int ti = 0;
                    while (pid > 0) { tmp[ti++] = '0' + (pid % 10); pid /= 10; }
                    while (ti > 0) numbuf[pos++] = tmp[--ti];
                }
                numbuf[pos] = '\0';
                val = numbuf;
            } else {
                /* $VAR or ${VAR} */
                int braced = 0;
                if (*in == '{') {
                    braced = 1;
                    in++;
                }
                const char *start = in;
                while ((*in >= 'A' && *in <= 'Z') ||
                       (*in >= 'a' && *in <= 'z') ||
                       (*in >= '0' && *in <= '9') ||
                       *in == '_')
                    in++;
                size_t namelen = (size_t)(in - start);
                if (braced && *in == '}')
                    in++;
                if (namelen > 0 && namelen < 256) {
                    char name[256];
                    memcpy(name, start, namelen);
                    name[namelen] = '\0';
                    val = getenv(name);
                }
            }

            if (val) {
                while (*val && oi < maxout)
                    out[oi++] = *val++;
            }
            continue;
        }

        /* Regular character */
        out[oi++] = *in++;
    }
    out[oi] = '\0';
}

/* ========================================================================= */
/* Tokenizer                                                                 */
/* ========================================================================= */

/*
 * Tokenize a command string into an argv-style array.
 * Handles double and single quoting.  Sets argc.
 * Returns the number of arguments parsed.
 * The input string is modified in place (quotes removed, NUL terminators).
 */
static int tokenize(char *line, char **argv, int max_args)
{
    int argc = 0;
    char *p = line;

    while (*p && argc < max_args - 1) {
        /* Skip whitespace */
        while (*p == ' ' || *p == '\t')
            p++;
        if (*p == '\0' || *p == '#')
            break;

        char *token_start;
        char *wp;  /* write pointer for unquoting */

        token_start = p;
        wp = p;

        while (*p && *p != ' ' && *p != '\t') {
            if (*p == '"') {
                /* Double quote: copy until closing " */
                p++;
                while (*p && *p != '"')
                    *wp++ = *p++;
                if (*p == '"')
                    p++;
            } else if (*p == '\'') {
                /* Single quote: copy literally until closing ' */
                p++;
                while (*p && *p != '\'')
                    *wp++ = *p++;
                if (*p == '\'')
                    p++;
            } else if (*p == '\\' && p[1]) {
                /* Escape: copy next char literally */
                p++;
                *wp++ = *p++;
            } else {
                *wp++ = *p++;
            }
        }

        if (*p) {
            *p = '\0';
            p++;
        }
        *wp = '\0';

        argv[argc++] = token_start;
    }

    argv[argc] = NULL;
    return argc;
}

/* ========================================================================= */
/* I/O redirection                                                           */
/* ========================================================================= */

/* Parsed redirection for a single command */
struct redir {
    const char *in_file;   /* < file */
    const char *out_file;  /* > file */
    int         out_append; /* >> file */
    const char *err_file;  /* 2> file */
};

/*
 * Scan argv for redirection operators and remove them from the argument
 * list.  Fills in the redir struct.
 */
static void parse_redirections(char **argv, int *argc, struct redir *r)
{
    r->in_file = NULL;
    r->out_file = NULL;
    r->out_append = 0;
    r->err_file = NULL;

    int out = 0;
    for (int i = 0; i < *argc; i++) {
        if (strcmp(argv[i], "<") == 0 && i + 1 < *argc) {
            r->in_file = argv[++i];
        } else if (strcmp(argv[i], ">>") == 0 && i + 1 < *argc) {
            r->out_file = argv[++i];
            r->out_append = 1;
        } else if (strcmp(argv[i], ">") == 0 && i + 1 < *argc) {
            r->out_file = argv[++i];
            r->out_append = 0;
        } else if (strcmp(argv[i], "2>") == 0 && i + 1 < *argc) {
            r->err_file = argv[++i];
        } else {
            argv[out++] = argv[i];
        }
    }
    argv[out] = NULL;
    *argc = out;
}

/* Apply redirections in the child process (after fork, before exec). */
static void apply_redirections(const struct redir *r)
{
    if (r->in_file) {
        int fd = open(r->in_file, O_RDONLY, 0);
        if (fd < 0) {
            fprintf(stderr, "sh: %s: %s\n", r->in_file, strerror(errno));
            _exit(1);
        }
        dup2(fd, 0);
        close(fd);
    }
    if (r->out_file) {
        int flags = O_WRONLY | O_CREAT;
        flags |= r->out_append ? O_APPEND : O_TRUNC;
        int fd = open(r->out_file, flags, 0666);
        if (fd < 0) {
            fprintf(stderr, "sh: %s: %s\n", r->out_file, strerror(errno));
            _exit(1);
        }
        dup2(fd, 1);
        close(fd);
    }
    if (r->err_file) {
        int fd = open(r->err_file, O_WRONLY | O_CREAT | O_TRUNC, 0666);
        if (fd < 0) {
            _exit(1);
        }
        dup2(fd, 2);
        close(fd);
    }
}

/* ========================================================================= */
/* PATH search                                                               */
/* ========================================================================= */

/*
 * Search PATH for a command.  If found, write the full path into buf
 * and return buf.  Otherwise return NULL.
 */
static char *find_in_path(const char *cmd, char *buf, size_t bufsz)
{
    /* If cmd contains '/', use it directly */
    if (strchr(cmd, '/')) {
        if (strlen(cmd) < bufsz) {
            strcpy(buf, cmd);
            return buf;
        }
        return NULL;
    }

    const char *path = getenv("PATH");
    if (!path)
        path = "/bin:/usr/bin";

    while (*path) {
        const char *end = path;
        while (*end && *end != ':')
            end++;

        size_t dirlen = (size_t)(end - path);
        if (dirlen == 0) {
            /* Empty component means current directory */
            dirlen = 1;
            path = ".";
        }

        size_t cmdlen = strlen(cmd);
        if (dirlen + 1 + cmdlen + 1 <= bufsz) {
            memcpy(buf, path, dirlen);
            buf[dirlen] = '/';
            memcpy(buf + dirlen + 1, cmd, cmdlen);
            buf[dirlen + 1 + cmdlen] = '\0';

            struct stat st;
            if (stat(buf, &st) == 0)
                return buf;
        }

        path = end;
        if (*path == ':')
            path++;
    }

    return NULL;
}

/* ========================================================================= */
/* Built-in commands                                                         */
/* ========================================================================= */

/*
 * Try to execute a built-in command.
 * Returns 1 if it was a builtin (and sets last_exit_status),
 * returns 0 if not a builtin (caller should fork/exec).
 */
static int try_builtin(int argc, char **argv)
{
    if (argc == 0)
        return 0;

    const char *cmd = argv[0];

    /* cd [dir] */
    if (strcmp(cmd, "cd") == 0) {
        const char *dir = argv[1];
        if (!dir)
            dir = getenv("HOME");
        if (!dir)
            dir = "/";
        if (chdir(dir) != 0) {
            fprintf(stderr, "cd: %s: %s\n", dir, strerror(errno));
            last_exit_status = 1;
        } else {
            last_exit_status = 0;
        }
        return 1;
    }

    /* exit [code] */
    if (strcmp(cmd, "exit") == 0) {
        int code = 0;
        if (argv[1])
            code = atoi(argv[1]);
        exit(code);
        return 1;  /* unreachable */
    }

    /* echo [args...] */
    if (strcmp(cmd, "echo") == 0) {
        for (int i = 1; i < argc; i++) {
            if (i > 1)
                write(1, " ", 1);
            write(1, argv[i], strlen(argv[i]));
        }
        write(1, "\n", 1);
        last_exit_status = 0;
        return 1;
    }

    /* pwd */
    if (strcmp(cmd, "pwd") == 0) {
        char cwd[1024];
        if (getcwd(cwd, sizeof(cwd))) {
            write(1, cwd, strlen(cwd));
            write(1, "\n", 1);
            last_exit_status = 0;
        } else {
            fprintf(stderr, "pwd: %s\n", strerror(errno));
            last_exit_status = 1;
        }
        return 1;
    }

    /* export VAR=value */
    if (strcmp(cmd, "export") == 0) {
        if (argv[1]) {
            char *eq = strchr(argv[1], '=');
            if (eq) {
                *eq = '\0';
                setenv(argv[1], eq + 1, 1);
                *eq = '=';
            }
        }
        last_exit_status = 0;
        return 1;
    }

    /* unset VAR */
    if (strcmp(cmd, "unset") == 0) {
        if (argv[1])
            unsetenv(argv[1]);
        last_exit_status = 0;
        return 1;
    }

    /* exec cmd [args] -- replace shell process */
    if (strcmp(cmd, "exec") == 0) {
        if (argc < 2) {
            last_exit_status = 0;
            return 1;
        }
        char pathbuf[1024];
        char *fullpath = find_in_path(argv[1], pathbuf, sizeof(pathbuf));
        if (fullpath) {
            execve(fullpath, &argv[1], environ);
            fprintf(stderr, "exec: %s: %s\n", argv[1], strerror(errno));
        } else {
            fprintf(stderr, "exec: %s: not found\n", argv[1]);
        }
        last_exit_status = 127;
        return 1;
    }

    /* true / false */
    if (strcmp(cmd, "true") == 0)  { last_exit_status = 0; return 1; }
    if (strcmp(cmd, "false") == 0) { last_exit_status = 1; return 1; }

    /* test / [ -- minimal implementation */
    if (strcmp(cmd, "test") == 0 || strcmp(cmd, "[") == 0) {
        /* Minimal test: -n STR, -z STR, STR = STR, -f FILE, -d FILE */
        last_exit_status = 1;  /* default: false */
        int ac = argc;
        /* If invoked as "[", strip trailing "]" */
        if (cmd[0] == '[' && ac > 1 && strcmp(argv[ac - 1], "]") == 0)
            ac--;

        if (ac == 2) {
            /* test STRING -- true if non-empty */
            last_exit_status = (argv[1][0] != '\0') ? 0 : 1;
        } else if (ac == 3) {
            if (strcmp(argv[1], "-n") == 0)
                last_exit_status = (argv[2][0] != '\0') ? 0 : 1;
            else if (strcmp(argv[1], "-z") == 0)
                last_exit_status = (argv[2][0] == '\0') ? 0 : 1;
            else if (strcmp(argv[1], "-f") == 0) {
                struct stat st;
                last_exit_status = (stat(argv[2], &st) == 0) ? 0 : 1;
            } else if (strcmp(argv[1], "-d") == 0) {
                struct stat st;
                last_exit_status = (stat(argv[2], &st) == 0) ? 0 : 1;
            } else if (strcmp(argv[1], "!") == 0) {
                last_exit_status = (argv[2][0] != '\0') ? 1 : 0;
            }
        } else if (ac == 4) {
            if (strcmp(argv[2], "=") == 0)
                last_exit_status = (strcmp(argv[1], argv[3]) == 0) ? 0 : 1;
            else if (strcmp(argv[2], "!=") == 0)
                last_exit_status = (strcmp(argv[1], argv[3]) != 0) ? 0 : 1;
        }
        return 1;
    }

    return 0;
}

/* ========================================================================= */
/* Command execution                                                         */
/* ========================================================================= */

/*
 * Execute a single simple command (no pipes).
 * Handles builtins, fork/exec, background, and redirections.
 * Returns the exit status.
 */
static int exec_simple(char **argv, int argc, int background)
{
    struct redir redir;
    parse_redirections(argv, &argc, &redir);

    if (argc == 0)
        return 0;

    /* Try builtins first (no fork needed) */
    if (!background && !redir.in_file && !redir.out_file && !redir.err_file) {
        if (try_builtin(argc, argv))
            return last_exit_status;
    }

    pid_t pid = fork();
    if (pid < 0) {
        fprintf(stderr, "sh: fork: %s\n", strerror(errno));
        return 1;
    }

    if (pid == 0) {
        /* Child */
        apply_redirections(&redir);

        /* Builtins with redirections need to run in child */
        if (try_builtin(argc, argv))
            _exit(last_exit_status);

        /* External command */
        char pathbuf[1024];
        char *fullpath = find_in_path(argv[0], pathbuf, sizeof(pathbuf));
        if (fullpath) {
            execve(fullpath, argv, environ);
            fprintf(stderr, "sh: %s: %s\n", argv[0], strerror(errno));
        } else {
            fprintf(stderr, "sh: %s: command not found\n", argv[0]);
        }
        _exit(127);
    }

    /* Parent */
    if (background) {
        /* Don't wait */
        return 0;
    }

    int status;
    waitpid(pid, &status, 0);
    if (WIFEXITED(status))
        return WEXITSTATUS(status);
    if (WIFSIGNALED(status))
        return 128 + WTERMSIG(status);
    return 1;
}

/*
 * Execute a pipeline: cmd1 | cmd2 | ... | cmdN
 * Each segment is a NUL-terminated string in the segments array.
 */
static int exec_pipeline(char **segments, int nseg)
{
    if (nseg == 1) {
        /* No pipe -- single command */
        char *argv[MAX_ARGS];
        int argc = tokenize(segments[0], argv, MAX_ARGS);
        if (argc == 0)
            return 0;

        /* Check for background */
        int bg = 0;
        if (argc > 0 && strcmp(argv[argc - 1], "&") == 0) {
            bg = 1;
            argv[--argc] = NULL;
        }

        return exec_simple(argv, argc, bg);
    }

    /* Multiple segments: create pipes */
    int pipefd[2];
    int prev_read = -1;
    pid_t pids[MAX_PIPES];

    for (int i = 0; i < nseg; i++) {
        /* Create pipe for all but last segment */
        if (i < nseg - 1) {
            if (pipe(pipefd) < 0) {
                fprintf(stderr, "sh: pipe: %s\n", strerror(errno));
                return 1;
            }
        }

        pid_t pid = fork();
        if (pid < 0) {
            fprintf(stderr, "sh: fork: %s\n", strerror(errno));
            return 1;
        }

        if (pid == 0) {
            /* Child */
            if (prev_read >= 0) {
                dup2(prev_read, 0);
                close(prev_read);
            }
            if (i < nseg - 1) {
                close(pipefd[0]);
                dup2(pipefd[1], 1);
                close(pipefd[1]);
            }

            char *argv[MAX_ARGS];
            int argc = tokenize(segments[i], argv, MAX_ARGS);

            struct redir redir;
            parse_redirections(argv, &argc, &redir);
            apply_redirections(&redir);

            if (argc == 0)
                _exit(0);

            /* Try builtins */
            if (try_builtin(argc, argv))
                _exit(last_exit_status);

            /* External */
            char pathbuf[1024];
            char *fullpath = find_in_path(argv[0], pathbuf, sizeof(pathbuf));
            if (fullpath) {
                execve(fullpath, argv, environ);
                fprintf(stderr, "sh: %s: %s\n", argv[0], strerror(errno));
            } else {
                fprintf(stderr, "sh: %s: command not found\n", argv[0]);
            }
            _exit(127);
        }

        pids[i] = pid;

        /* Close pipe ends in parent */
        if (prev_read >= 0)
            close(prev_read);
        if (i < nseg - 1) {
            close(pipefd[1]);
            prev_read = pipefd[0];
        }
    }

    /* Wait for all children */
    int status = 0;
    for (int i = 0; i < nseg; i++) {
        int s;
        waitpid(pids[i], &s, 0);
        if (i == nseg - 1) {
            /* Last command's exit status is the pipeline status */
            if (WIFEXITED(s))
                status = WEXITSTATUS(s);
            else if (WIFSIGNALED(s))
                status = 128 + WTERMSIG(s);
        }
    }

    return status;
}

/* ========================================================================= */
/* Line execution                                                            */
/* ========================================================================= */

/*
 * Execute a single line which may contain pipes, ;, &&, ||.
 */
static void execute_line(char *line)
{
    char expanded[MAX_LINE];
    expand_variables(line, expanded, MAX_LINE);

    char *p = expanded;

    while (*p) {
        p = skip_spaces(p);
        if (*p == '\0' || *p == '#')
            break;

        /* Find the end of this command group (delimited by ;, &&, ||) */
        char *segments[MAX_PIPES];
        int nseg = 0;

        /* Split on pipes first, but respect quotes */
        char *cmd_start = p;
        char *cmd_end = NULL;
        int cond = 0;  /* 0=none, 1=&&, 2=|| */

        /* Find the extent of this pipeline (up to ;, &&, || or end) */
        int in_sq = 0, in_dq = 0;
        char *scan = p;
        while (*scan) {
            if (*scan == '\'' && !in_dq) {
                in_sq = !in_sq;
            } else if (*scan == '"' && !in_sq) {
                in_dq = !in_dq;
            } else if (!in_sq && !in_dq) {
                if (*scan == ';') {
                    cmd_end = scan;
                    cond = 0;
                    break;
                }
                if (*scan == '&' && scan[1] == '&') {
                    cmd_end = scan;
                    cond = 1;
                    break;
                }
                if (*scan == '|' && scan[1] == '|') {
                    cmd_end = scan;
                    cond = 2;
                    break;
                }
            }
            scan++;
        }

        if (!cmd_end) {
            cmd_end = scan;  /* End of string */
            cond = 0;
        }

        /* NUL-terminate this segment */
        char saved = *cmd_end;
        *cmd_end = '\0';

        /* Split the pipeline on '|' (but not '||') */
        char *pp = cmd_start;
        segments[nseg++] = pp;

        in_sq = 0;
        in_dq = 0;
        while (*pp) {
            if (*pp == '\'' && !in_dq) {
                in_sq = !in_sq;
            } else if (*pp == '"' && !in_sq) {
                in_dq = !in_dq;
            } else if (!in_sq && !in_dq && *pp == '|' && pp[1] != '|') {
                *pp = '\0';
                pp++;
                if (nseg < MAX_PIPES)
                    segments[nseg++] = pp;
            } else {
                pp++;
            }
        }

        /* Execute the pipeline */
        last_exit_status = exec_pipeline(segments, nseg);

        /* Advance past the delimiter */
        *cmd_end = saved;
        if (saved == ';') {
            p = cmd_end + 1;
        } else if (cond == 1) {
            /* && -- skip next command if we failed */
            p = cmd_end + 2;
            if (last_exit_status != 0) {
                /* Skip to next ; or end */
                in_sq = 0;
                in_dq = 0;
                while (*p) {
                    if (*p == '\'' && !in_dq) in_sq = !in_sq;
                    else if (*p == '"' && !in_sq) in_dq = !in_dq;
                    else if (!in_sq && !in_dq && *p == ';') { p++; break; }
                    p++;
                }
            }
        } else if (cond == 2) {
            /* || -- skip next command if we succeeded */
            p = cmd_end + 2;
            if (last_exit_status == 0) {
                in_sq = 0;
                in_dq = 0;
                while (*p) {
                    if (*p == '\'' && !in_dq) in_sq = !in_sq;
                    else if (*p == '"' && !in_sq) in_dq = !in_dq;
                    else if (!in_sq && !in_dq && *p == ';') { p++; break; }
                    p++;
                }
            }
        } else {
            p = cmd_end;
        }
    }
}

/* ========================================================================= */
/* Main                                                                      */
/* ========================================================================= */

int main(int argc, char *argv[])
{
    /* sh -c "command" -- execute command and exit */
    if (argc >= 3 && strcmp(argv[1], "-c") == 0) {
        execute_line(argv[2]);
        return last_exit_status;
    }

    /* sh script -- execute file */
    if (argc >= 2 && argv[1][0] != '-') {
        FILE *fp = fopen(argv[1], "r");
        if (!fp) {
            fprintf(stderr, "sh: %s: %s\n", argv[1], strerror(errno));
            return 127;
        }
        char line[MAX_LINE];
        while (fgets(line, sizeof(line), fp)) {
            strip_trailing(line);
            if (line[0] == '\0' || line[0] == '#')
                continue;
            execute_line(line);
        }
        fclose(fp);
        return last_exit_status;
    }

    /* Interactive mode */
    int interactive = isatty(0);

    char line[MAX_LINE];
    for (;;) {
        if (interactive) {
            const char *prompt = "$ ";
            write(1, prompt, 2);
        }

        if (!fgets(line, sizeof(line), stdin))
            break;

        strip_trailing(line);
        if (line[0] == '\0' || line[0] == '#')
            continue;

        execute_line(line);
    }

    if (interactive)
        write(1, "\n", 1);

    return last_exit_status;
}
