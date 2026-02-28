# vsh -- VeridianOS Shell User Guide

**Version**: 1.0.0
**Compatibility Target**: Bash 5.3 feature parity
**Last Updated**: February 27, 2026

## Overview

vsh is the native interactive shell for VeridianOS, written entirely in Rust
as a `no_std` Ring 3 user-space binary. It provides near-complete Bash 5.3
feature parity while running without the Rust standard library, linking only
against VeridianOS's custom libc via raw syscall interfaces.

vsh replaces the earlier in-kernel shell (24 builtins, limited scripting)
with a full-featured user-space implementation capable of interactive use,
script execution, job control, and programmable completion. It is the
default login shell for all VeridianOS user sessions.

### Design Goals

| Goal | Approach |
|------|----------|
| Bash compatibility | POSIX + Bash extensions; scripts written for Bash should work unmodified |
| No runtime dependencies | `no_std` Rust binary, statically linked, no heap allocator besides alloc |
| Minimal binary size | Target under 2MB stripped; no LLVM codegen bloat |
| Fast startup | Fork+exec from init in under 5ms; no RC file parsing on non-interactive start |
| Interactive quality | GNU readline-equivalent line editing, history, and completion |

### Quick Start

```bash
# vsh is the default shell -- it starts automatically after boot
root@veridian:/# echo $SHELL
/bin/vsh

# Run a script
root@veridian:/# vsh script.sh

# Run a command string
root@veridian:/# vsh -c 'echo "hello from vsh"'
```

---

## Architecture

### Source Layout

vsh consists of 40 source files organized into 10 modules under
`userland/vsh/src/`:

```
userland/vsh/
  Cargo.toml
  src/
    main.rs              -- Entry point, argument parsing, session init
    lib.rs               -- Crate root, module declarations
    lexer/
      mod.rs             -- Token definitions, lexer state machine
      token.rs           -- Token types (Word, Op, Redirect, Reserved)
      heredoc.rs         -- Here-document (<<, <<-) processing
      quoting.rs         -- Quote state tracking (single, double, escape)
    parser/
      mod.rs             -- Recursive descent parser entry
      ast.rs             -- AST node definitions (Command, Pipeline, List)
      compound.rs        -- if/while/for/case/select parsing
      redirect.rs        -- Redirection parsing and validation
      arith.rs           -- Arithmetic expression parser ($(( )))
      cond.rs            -- Conditional expression parser ([[ ]])
    exec/
      mod.rs             -- Command execution dispatcher
      simple.rs          -- Simple command fork+exec
      pipeline.rs        -- Pipeline construction (|, |&)
      subshell.rs        -- Subshell execution (( ))
      coprocess.rs       -- Coproc management
      redir.rs           -- File descriptor manipulation (dup2, open, close)
    expand/
      mod.rs             -- Expansion pipeline orchestrator
      brace.rs           -- Brace expansion ({a,b,c}, {1..10})
      tilde.rs           -- Tilde expansion (~, ~user, ~+, ~-)
      param.rs           -- Parameter expansion (${var}, ${var:-default}, etc.)
      arith.rs           -- Arithmetic expansion ($((expr)))
      command.rs         -- Command substitution ($(cmd), `cmd`)
      glob.rs            -- Pathname expansion (*, ?, [...], **)
      split.rs           -- Word splitting (IFS)
      quote.rs           -- Quote removal (final expansion stage)
    var/
      mod.rs             -- Variable storage and lookup
      env.rs             -- Environment variable import/export
      special.rs         -- Special variables ($?, $!, $$, $0, $#, $@, $*)
      array.rs           -- Indexed array operations
      assoc.rs           -- Associative array operations
      attr.rs            -- Variable attributes (readonly, integer, export, etc.)
    builtin/
      mod.rs             -- Builtin dispatch table
      core.rs            -- cd, echo, exit, export, unset, set
      test.rs            -- test/[, [[
      io.rs              -- read, printf
      declare.rs         -- declare, local, readonly, alias, unalias
      type_cmd.rs        -- type, hash, command
      job.rs             -- jobs, fg, bg, wait, kill, disown
      trap.rs            -- trap, signal handling
      misc.rs            -- let, shopt, enable, source/., eval
    readline/
      mod.rs             -- Line editor main loop
      keybind.rs         -- Key binding table (Emacs mode)
      history.rs         -- History storage, search, expansion (!!, !n, !string)
      complete.rs        -- Programmable completion engine
      kill_ring.rs       -- Kill ring (Ctrl-K/Ctrl-Y)
      display.rs         -- Terminal display and cursor management
    jobs/
      mod.rs             -- Job table and process group management
      process.rs         -- Process tracking (PID, status, state)
      signal.rs          -- Signal forwarding and disposition
    prompt/
      mod.rs             -- PS1/PS2/PS4 expansion
      escape.rs          -- Prompt escape sequences (\u, \h, \w, \$, etc.)
    config/
      mod.rs             -- Shell options and configuration
      set_opts.rs        -- set -o options
      shopt_opts.rs      -- shopt options
```

### Build Configuration

```toml
# userland/vsh/Cargo.toml
[package]
name = "vsh"
version = "1.0.0"
edition = "2021"

[dependencies]
# No external dependencies -- pure no_std Rust

[profile.release]
panic = "abort"
opt-level = "s"         # Optimize for size
lto = true
codegen-units = 1
strip = "symbols"
```

### Building vsh

```bash
# From the VeridianOS repository root
cd userland/vsh
cargo build --target x86_64-unknown-none \
    -Zbuild-std=core,compiler_builtins,alloc \
    --release

# Output: target/x86_64-unknown-none/release/vsh (~1.6MB stripped)
```

For development builds targeting the host (useful for parser testing):

```bash
# Host build for testing (requires std stubs)
cargo build
cargo test
```

---

## Features

### Quoting

vsh supports all Bash quoting mechanisms:

| Syntax | Name | Behavior |
|--------|------|----------|
| `'text'` | Single quotes | Literal; no expansion of any kind |
| `"text"` | Double quotes | Allows `$var`, `$(cmd)`, `$((arith))`, `\` escaping |
| `$'text'` | ANSI-C quoting | Interprets `\n`, `\t`, `\x41`, `\u0041`, `\077` |
| `\c` | Backslash | Escapes the next character (literal) |
| `$"text"` | Locale translation | Treated as double quotes (no locale support) |

### Expansion

Expansions are performed in the following order, matching Bash:

1. **Brace expansion**: `{a,b,c}` produces `a b c`; `{1..5}` produces `1 2 3 4 5`; `{01..10..2}` produces `01 03 05 07 09`
2. **Tilde expansion**: `~` expands to `$HOME`; `~user` expands to user's home directory; `~+` is `$PWD`; `~-` is `$OLDPWD`
3. **Parameter expansion**: All standard forms (see table below)
4. **Arithmetic expansion**: `$((expression))` evaluates integer arithmetic
5. **Command substitution**: `$(command)` and `` `command` `` capture stdout
6. **Word splitting**: Results split on `$IFS` (default: space, tab, newline)
7. **Pathname expansion**: `*`, `?`, `[...]` glob patterns; `**` for recursive glob (with `shopt -s globstar`)
8. **Quote removal**: Remaining quote characters removed from final result

#### Parameter Expansion Forms

| Form | Description |
|------|-------------|
| `${var}` | Value of var |
| `${var:-word}` | Use default if var is unset or null |
| `${var:=word}` | Assign default if var is unset or null |
| `${var:?word}` | Error if var is unset or null |
| `${var:+word}` | Use alternate if var is set and non-null |
| `${var:offset}` | Substring from offset |
| `${var:offset:length}` | Substring from offset with length |
| `${#var}` | Length of value |
| `${var#pattern}` | Remove shortest prefix match |
| `${var##pattern}` | Remove longest prefix match |
| `${var%pattern}` | Remove shortest suffix match |
| `${var%%pattern}` | Remove longest suffix match |
| `${var/pattern/string}` | Replace first match |
| `${var//pattern/string}` | Replace all matches |
| `${var/#pattern/string}` | Replace prefix match |
| `${var/%pattern/string}` | Replace suffix match |
| `${var^pattern}` | Uppercase first matching character |
| `${var^^pattern}` | Uppercase all matching characters |
| `${var,pattern}` | Lowercase first matching character |
| `${var,,pattern}` | Lowercase all matching characters |
| `${var@U}` | Uppercase entire value |
| `${var@L}` | Lowercase entire value |
| `${var@Q}` | Quote value for re-input |
| `${!prefix*}` | Names of variables with prefix |
| `${!array[@]}` | Indices of array |

### Redirections

| Syntax | Description |
|--------|-------------|
| `> file` | Redirect stdout to file (truncate) |
| `>> file` | Redirect stdout to file (append) |
| `< file` | Redirect stdin from file |
| `<< delimiter` | Here-document (expand variables) |
| `<<- delimiter` | Here-document (strip leading tabs) |
| `<<< word` | Here-string |
| `<> file` | Open file for reading and writing |
| `>&n` | Duplicate stdout to file descriptor n |
| `<&n` | Duplicate stdin from file descriptor n |
| `>|file` | Redirect stdout, overriding `noclobber` |
| `&> file` | Redirect both stdout and stderr to file |
| `&>> file` | Append both stdout and stderr to file |
| `n>&m` | Duplicate file descriptor m to n |
| `n<&m` | Duplicate file descriptor m to n |
| `n<&-` | Close file descriptor n |
| `n>&-` | Close file descriptor n |

### Pipelines and Lists

| Syntax | Description |
|--------|-------------|
| `cmd1 \| cmd2` | Pipeline: stdout of cmd1 to stdin of cmd2 |
| `cmd1 \|& cmd2` | Pipeline: stdout and stderr of cmd1 to stdin of cmd2 |
| `cmd1 && cmd2` | AND list: run cmd2 only if cmd1 succeeds |
| `cmd1 \|\| cmd2` | OR list: run cmd2 only if cmd1 fails |
| `cmd1 ; cmd2` | Sequential execution |
| `cmd &` | Background execution |

### Compound Commands

#### Conditional

```bash
if condition; then
    commands
elif condition; then
    commands
else
    commands
fi
```

#### Loops

```bash
# Standard for loop
for var in word1 word2 word3; do
    commands
done

# C-style for loop
for ((init; condition; step)); do
    commands
done

# While loop
while condition; do
    commands
done

# Until loop
until condition; do
    commands
done

# Select menu
select var in option1 option2 option3; do
    commands
done
```

#### Pattern Matching

```bash
case word in
    pattern1)
        commands
        ;;          # Break
    pattern2)
        commands
        ;;&         # Fall-through and test next
    pattern3)
        commands
        ;&          # Fall-through unconditionally
    *)
        default commands
        ;;
esac
```

#### Grouping

```bash
# Subshell (new process, isolated environment)
( commands )

# Group (current shell, shared environment)
{ commands; }

# Arithmetic evaluation
(( expression ))

# Conditional expression
[[ expression ]]
```

### Builtin Commands

vsh provides 24 builtin commands:

| Builtin | Category | Description |
|---------|----------|-------------|
| `cd` | Core | Change working directory |
| `echo` | Core | Print arguments |
| `exit` | Core | Exit the shell |
| `export` | Core | Set environment variable |
| `unset` | Core | Remove variable or function |
| `set` | Core | Set shell options and positional parameters |
| `shopt` | Config | Toggle shell behavior options |
| `test` / `[` | Conditional | Evaluate conditional expression |
| `read` | I/O | Read a line from stdin |
| `printf` | I/O | Formatted output |
| `declare` | Variables | Declare variable with attributes |
| `local` | Variables | Declare function-local variable |
| `readonly` | Variables | Mark variable as read-only |
| `alias` | Variables | Define command alias |
| `unalias` | Variables | Remove alias |
| `type` | Info | Describe a command |
| `hash` | Info | Manage command hash table |
| `history` | Readline | Display or manipulate command history |
| `jobs` | Job control | List active jobs |
| `fg` | Job control | Move job to foreground |
| `bg` | Job control | Resume job in background |
| `wait` | Job control | Wait for job completion |
| `kill` | Job control | Send signal to process |
| `trap` | Signals | Set signal handlers |
| `let` | Arithmetic | Evaluate arithmetic expression |

### Job Control

vsh implements full POSIX job control:

- Each pipeline runs in its own process group (setpgid)
- The foreground process group receives terminal signals (SIGINT, SIGQUIT)
- Ctrl-Z sends SIGTSTP to the foreground process group
- `jobs` lists all active jobs with status
- `fg %n` brings job n to the foreground
- `bg %n` resumes job n in the background
- `wait %n` waits for job n to complete (or `wait` for all)
- `disown %n` removes job n from the job table (continues running)

Job status indicators:

| Indicator | Meaning |
|-----------|---------|
| `Running` | Process is executing |
| `Stopped` | Process received SIGTSTP |
| `Done` | Process exited with status 0 |
| `Exit N` | Process exited with status N |
| `Killed` | Process was killed by a signal |

### Readline / Line Editing

vsh provides GNU readline-compatible line editing in Emacs mode:

#### Navigation

| Key | Action |
|-----|--------|
| Ctrl-A | Move to beginning of line |
| Ctrl-E | Move to end of line |
| Ctrl-F / Right | Move forward one character |
| Ctrl-B / Left | Move backward one character |
| Alt-F | Move forward one word |
| Alt-B | Move backward one word |

#### Editing

| Key | Action |
|-----|--------|
| Ctrl-D | Delete character under cursor (or EOF on empty line) |
| Backspace | Delete character before cursor |
| Ctrl-K | Kill from cursor to end of line |
| Ctrl-U | Kill from beginning of line to cursor |
| Alt-D | Kill from cursor to end of word |
| Ctrl-W | Kill word before cursor |
| Ctrl-Y | Yank (paste) last killed text |
| Alt-Y | Rotate kill ring and yank |
| Ctrl-T | Transpose characters |
| Alt-U | Uppercase word |
| Alt-L | Lowercase word |
| Alt-C | Capitalize word |

#### History

| Key | Action |
|-----|--------|
| Ctrl-P / Up | Previous history entry |
| Ctrl-N / Down | Next history entry |
| Ctrl-R | Reverse incremental search |
| Ctrl-S | Forward incremental search |
| Alt-< | Beginning of history |
| Alt-> | End of history |
| `!!` | Previous command |
| `!n` | Command number n |
| `!string` | Most recent command starting with string |
| `!?string` | Most recent command containing string |

#### Completion

| Key | Action |
|-----|--------|
| Tab | Complete word (command, file, or variable) |
| Tab Tab | Display all completions |
| Alt-? | Display completions without modifying line |
| Alt-* | Insert all completions |

### Variables

#### Indexed Arrays

```bash
# Declaration
declare -a my_array
my_array=(one two three)
my_array[5]=six

# Access
echo ${my_array[0]}         # one
echo ${my_array[@]}          # all elements
echo ${#my_array[@]}         # number of elements
echo ${!my_array[@]}         # all indices

# Slicing
echo ${my_array[@]:1:2}     # two three

# Append
my_array+=(four five)
```

#### Associative Arrays

```bash
# Declaration (required)
declare -A my_map

# Assignment
my_map[key1]=value1
my_map[key2]=value2
my_map=([key1]=value1 [key2]=value2)

# Access
echo ${my_map[key1]}
echo ${my_map[@]}            # all values
echo ${!my_map[@]}           # all keys
echo ${#my_map[@]}           # number of entries
```

#### Special Variables

| Variable | Meaning |
|----------|---------|
| `$?` | Exit status of last command |
| `$!` | PID of last background command |
| `$$` | PID of current shell |
| `$0` | Name of shell or script |
| `$#` | Number of positional parameters |
| `$@` | All positional parameters (individually quoted) |
| `$*` | All positional parameters (as single word) |
| `$-` | Current option flags |
| `$_` | Last argument of previous command |
| `$LINENO` | Current line number in script |
| `$RANDOM` | Random integer 0-32767 |
| `$SECONDS` | Seconds since shell start |
| `$BASHPID` | PID of current bash process (subshell-aware) |
| `$BASH_VERSION` | vsh version string |

#### Variable Attributes

| Flag | Meaning |
|------|---------|
| `-r` | Readonly |
| `-i` | Integer (arithmetic evaluation on assignment) |
| `-x` | Export to environment |
| `-l` | Lowercase on assignment |
| `-u` | Uppercase on assignment |
| `-a` | Indexed array |
| `-A` | Associative array |
| `-n` | Nameref (reference to another variable) |

### Prompt Customization

PS1 supports the following escape sequences:

| Escape | Expansion |
|--------|-----------|
| `\u` | Username |
| `\h` | Hostname (short) |
| `\H` | Hostname (full) |
| `\w` | Working directory (~ for home) |
| `\W` | Basename of working directory |
| `\$` | `#` if root, `$` otherwise |
| `\d` | Date (e.g., "Tue May 26") |
| `\t` | Time in 24-hour HH:MM:SS format |
| `\T` | Time in 12-hour HH:MM:SS format |
| `\@` | Time in 12-hour AM/PM format |
| `\A` | Time in 24-hour HH:MM format |
| `\n` | Newline |
| `\r` | Carriage return |
| `\s` | Shell name ("vsh") |
| `\v` | Shell version |
| `\V` | Shell version + patch level |
| `\j` | Number of active jobs |
| `\l` | Terminal device basename |
| `\!` | History number |
| `\#` | Command number |
| `\e` | Escape character (ASCII 033) |
| `\[` | Begin non-printing sequence |
| `\]` | End non-printing sequence |
| `\\` | Literal backslash |

Default prompt: `\u@\h:\w\$ ` (produces `root@veridian:/# `)

### Shell Options

#### set -o Options (15)

| Option | Default | Description |
|--------|---------|-------------|
| `errexit` (`-e`) | off | Exit on command failure |
| `nounset` (`-u`) | off | Error on unset variable reference |
| `pipefail` | off | Pipeline returns rightmost non-zero exit status |
| `noclobber` (`-C`) | off | Prevent `>` from overwriting existing files |
| `noglob` (`-f`) | off | Disable pathname expansion |
| `noexec` (`-n`) | off | Read commands but do not execute |
| `verbose` (`-v`) | off | Print input lines as they are read |
| `xtrace` (`-x`) | off | Print commands before execution |
| `allexport` (`-a`) | off | Export all new variables automatically |
| `emacs` | on | Emacs-style line editing |
| `vi` | off | Vi-style line editing |
| `hashall` (`-h`) | on | Hash command paths on first lookup |
| `monitor` (`-m`) | on (interactive) | Enable job control |
| `ignoreeof` | off | Do not exit on Ctrl-D |
| `posix` | off | Strict POSIX compliance mode |

#### shopt Options (20)

| Option | Default | Description |
|--------|---------|-------------|
| `autocd` | off | Directory name as command = cd |
| `cdspell` | off | Correct minor cd typos |
| `checkhash` | off | Check hash table before exec |
| `checkjobs` | off | Warn about running jobs on exit |
| `cmdhist` | on | Multi-line commands in one history entry |
| `dotglob` | off | Include hidden files in glob |
| `expand_aliases` | on (interactive) | Expand aliases |
| `extglob` | on | Extended glob patterns `?(pat)`, `*(pat)`, `+(pat)`, `@(pat)`, `!(pat)` |
| `failglob` | off | Error if glob matches nothing |
| `globstar` | off | `**` recursive glob |
| `histappend` | off | Append to history file |
| `histreedit` | off | Re-edit failed history substitution |
| `histverify` | off | Show history expansion before executing |
| `hostcomplete` | on | Hostname completion after @ |
| `huponexit` | off | Send SIGHUP to jobs on exit |
| `lastpipe` | off | Run last pipeline command in current shell |
| `lithist` | off | Preserve newlines in history |
| `nocaseglob` | off | Case-insensitive globbing |
| `nocasematch` | off | Case-insensitive `case`/`[[` matching |
| `nullglob` | off | Non-matching globs expand to nothing |

---

## Differences from Bash

vsh targets near-complete Bash 5.3 compatibility. The following differences
exist, primarily due to VeridianOS platform constraints:

| Feature | Bash | vsh | Notes |
|---------|------|-----|-------|
| Locale support | Full | None | No locale infrastructure in VeridianOS libc |
| /dev/tcp, /dev/udp | Yes | No | Network pseudo-devices not implemented |
| loadable builtins | `enable -f` | No | No shared library support |
| POSIX regex in `=~` | Full PCRE | BRE/ERE | Uses VeridianOS regcomp/regexec (1291-line implementation) |
| Vi editing mode | Full | Partial | Basic vi keybindings; no ex-mode |
| Coproc | Full | Basic | Single coproc only (no named coprocs) |
| Process substitution | `<(cmd)` | No | Requires /dev/fd or named pipes |
| Programmable completion | `complete -F` | Basic | File and command completion; no function-based completion |
| History file | `~/.bash_history` | `~/.vsh_history` | Different default path |
| Startup files | `.bashrc`, `.profile` | `.vshrc` | Single RC file |
| BASH_REMATCH | Array | Array | Populated by `[[ =~ ]]` but BRE/ERE only |
| mapfile/readarray | Yes | No | Can use `while read` loop instead |
| compgen/compopt | Yes | No | Completion generation builtins not yet implemented |

### Compatibility Mode

Running vsh with `--posix` or `set -o posix` enables strict POSIX sh
compliance mode, which disables Bash extensions (brace expansion, `[[`,
`(())`, extended globs, arrays) and follows POSIX word splitting and field
splitting rules exactly.

---

## Scripting Examples

### Basic Script

```bash
#!/bin/vsh
# System information script

echo "VeridianOS System Report"
echo "========================"
echo "Hostname: $(hostname)"
echo "Kernel:   $(uname -r)"
echo "Uptime:   $(cat /proc/uptime 2>/dev/null || echo 'N/A')"
echo "Shell:    $SHELL (vsh $BASH_VERSION)"
```

### Arrays and Loops

```bash
#!/bin/vsh
# Package status checker

declare -a packages=(kernel libc vsh busybox gcc)
declare -A status

for pkg in "${packages[@]}"; do
    if vpkg info "$pkg" &>/dev/null; then
        status[$pkg]="installed"
    else
        status[$pkg]="missing"
    fi
done

printf "%-15s %s\n" "Package" "Status"
printf "%-15s %s\n" "-------" "------"
for pkg in "${packages[@]}"; do
    printf "%-15s %s\n" "$pkg" "${status[$pkg]}"
done
```

### Error Handling

```bash
#!/bin/vsh
set -euo pipefail

cleanup() {
    echo "Cleaning up temporary files..."
    rm -rf "$TMPDIR"
}
trap cleanup EXIT

TMPDIR=$(mktemp -d)
echo "Working in $TMPDIR"

# Commands here -- script exits on any failure
# cleanup runs automatically via trap
```

### Job Control

```bash
# Start background jobs
find / -name "*.log" > /tmp/logfiles.txt &
wc -l /usr/local/lib/rustlib/x86_64-unknown-veridian/lib/*.rlib &

# List jobs
jobs
# [1]+  Running    find / -name "*.log" > /tmp/logfiles.txt &
# [2]-  Running    wc -l /usr/local/lib/rustlib/.../lib/*.rlib &

# Wait for all
wait
echo "All jobs complete"
```

---

## Configuration

### Startup Files

| File | When Sourced | Purpose |
|------|-------------|---------|
| `/etc/vsh/vshrc` | All interactive shells | System-wide configuration |
| `~/.vshrc` | Interactive login shells | Per-user configuration |
| `~/.vsh_history` | Interactive shells | Command history |

### Example .vshrc

```bash
# ~/.vshrc -- vsh configuration

# Prompt
PS1='\[\e[32m\]\u@\h\[\e[0m\]:\[\e[34m\]\w\[\e[0m\]\$ '

# History
HISTSIZE=10000
HISTFILESIZE=20000
shopt -s histappend cmdhist

# Shell behavior
shopt -s autocd cdspell globstar extglob
set -o pipefail

# Aliases
alias ll='ls -la'
alias la='ls -A'
alias grep='grep --color=auto'
alias ..='cd ..'
alias ...='cd ../..'

# Functions
mkcd() {
    mkdir -p "$1" && cd "$1"
}

# Environment
export EDITOR=edit
export PATH="/usr/local/bin:$PATH"
```

---

## Internals

### Memory Management

vsh uses Rust's `alloc` crate for dynamic memory allocation. The global
allocator bridges to VeridianOS's `brk`/`sbrk` syscalls via the libc. Key
data structures and their memory characteristics:

| Structure | Typical Size | Allocation Pattern |
|-----------|-------------|-------------------|
| AST nodes | 64-256 bytes each | Per-command, freed after execution |
| Variable table | 4-32 KB | Persistent, grows with variable count |
| History buffer | 8-64 KB | Ring buffer, fixed maximum |
| Line buffer | 4 KB | Reused per prompt |
| Job table | 1-4 KB | Persistent, bounded by MAX_JOBS (64) |
| Completion candidates | 1-16 KB | Temporary, freed after Tab press |

Peak memory usage for an interactive session is typically under 2MB. Script
execution memory scales with AST depth and variable count.

### Signal Handling

vsh installs signal handlers for the following signals:

| Signal | Interactive | Script | Action |
|--------|------------|--------|--------|
| SIGINT | Cancel current line | Exit (unless trapped) | Default |
| SIGTSTP | Suspend foreground job | Ignored | Job control |
| SIGCHLD | Update job status | Update job status | Always |
| SIGWINCH | Update terminal size | Ignored | Readline |
| SIGHUP | Exit, notify jobs | Exit | Default |
| SIGTERM | Exit | Exit | Default |
| SIGPIPE | Ignored | Exit | Default |

### Exit Status Conventions

| Status | Meaning |
|--------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Misuse of shell builtin or syntax error |
| 126 | Command found but not executable |
| 127 | Command not found |
| 128+N | Killed by signal N |

---

## References

- [GNU Bash Manual](https://www.gnu.org/software/bash/manual/)
- [POSIX Shell Command Language](https://pubs.opengroup.org/onlinepubs/9699919799/utilities/V3_chap02.html)
- [VeridianOS Self-Hosting Status](SELF-HOSTING-STATUS.md)
- [VeridianOS Syscall API](API-REFERENCE.md)
