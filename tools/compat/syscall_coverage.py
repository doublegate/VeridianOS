#!/usr/bin/env python3
"""Linux syscall coverage report for VeridianOS.

Compares the x86_64 syscall table of a Linux release with what the
VeridianOS Linux-ABI layer (kernel/src/syscall/linux_compat.rs and the
special cases in kernel/src/syscall/mod.rs) handles, and writes a Markdown
table with one row per syscall.

Usage:
    curl -fsSL -o syscall_64.tbl \\
      https://raw.githubusercontent.com/torvalds/linux/v7.2/arch/x86/entry/syscalls/syscall_64.tbl
    tools/compat/syscall_coverage.py syscall_64.tbl v7.2 > docs/compat/LINUX-SYSCALL-COVERAGE.md

Status values:
    implemented  routed to a kernel implementation (may still be partial)
    stub         answered without doing the work (fake success or ENOSYS)
    missing      returns ENOSYS; Linux implements it
    enosys       Linux itself has no implementation (ENOSYS there too)
"""

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# Calls dispatched ahead of the translation table in syscall/mod.rs.
SPECIAL_CASED = {"ppoll", "faccessat", "faccessat2"}

# Grouping used by the compatibility plan (docs/compat and the roadmap).
GROUPS = [
    ("process", r"^(v?fork|clone3?|execve(at)?|exit(_group)?|wait4|waitid|kill|t?g?kill|"
                r"get(p|pp|t)id|[gs]et(p?gid|pgrp|sid|res?[ug]id|e?[ug]id|re[ug]id|fs[ug]id|groups)|"
                r"setsid|setpgid|getpgid|getsid|cap[gs]et|times|personality|prctl|arch_prctl|"
                r"set_tid_address|[gs]et_robust_list|unshare|setns|pidfd_\w+|kcmp|ptrace|"
                r"process_vm_\w+|getcpu|rseq\w*|[gs]etpriority|[gs]etrlimit|prlimit64|getrusage)$"),
    ("signals", r"^(rt_sig\w+|sigaltstack|pause|alarm|[gs]etitimer|restart_syscall|signalfd4?|"
                r"rt_tgsigqueueinfo)$"),
    ("time", r"^(time|gettimeofday|settimeofday|clock_\w+|nanosleep|adjtimex|timer_\w+|"
             r"timerfd_\w+|utimes?|utimensat|futimesat)$"),
    ("files", r"^(read|write|open|openat2?|close|close_range|creat|lseek|pread64|pwrite64|readv|"
              r"writev|preadv2?|pwritev2?|n?f?stat|lstat|newfstatat|statx|access|faccessat2?|"
              r"[fl]?ch(mod|own)|fchmodat2?|fchownat|truncate|ftruncate|fallocate|fsync|fdatasync|"
              r"sync|syncfs|sync_file_range|readahead|fadvise64|rename(at2?)?|(sym)?link(at)?|"
              r"unlink(at)?|mkdir(at)?|rmdir|mknod(at)?|readlink(at)?|getcwd|f?chdir|chroot|"
              r"getdents(64)?|umask|dup[23]?|fcntl|flock|ioctl|pipe2?|sendfile|splice|tee|"
              r"vmsplice|copy_file_range|memfd_\w+|name_to_handle_at|open_by_handle_at|"
              r"\w*xattr\w*|file_[gs]etattr|inotify_\w+|fanotify_\w+|statfs|fstatfs|ustat|sysfs|"
              r"uselib|cachestat)$"),
    ("memory", r"^(brk|mmap|munmap|mremap|mprotect|madvise|msync|mincore|m(un)?lock\w*|"
               r"remap_file_pages|pkey_\w+|mseal|membarrier|process_madvise|process_mrelease|"
               r"userfaultfd|map_shadow_stack|mbind|[gs]et_mempolicy\w*|migrate_pages|move_pages)$"),
    ("ipc", r"^(shm\w+|sem\w+|msg\w+|mq_\w+|futex\w*)$"),
    ("events", r"^(poll|ppoll|select|pselect6|epoll_\w+|eventfd2?|io_\w+|io_uring_\w+)$"),
    ("network", r"^(socket|socketpair|bind|listen|accept4?|connect|getsockname|getpeername|"
                r"sendto|recvfrom|sendm?msg|recvm?msg|shutdown|[gs]etsockopt)$"),
    ("scheduling", r"^(sched_\w+|ioprio_\w+)$"),
    ("mount", r"^(mount|umount2|pivot_root|open_tree\w*|move_mount|fs(open|config|mount|pick)|"
              r"mount_setattr|statmount|listmount|listns|quotactl(_fd)?)$"),
    ("system", r"^(uname|sysinfo|syslog|reboot|sethostname|setdomainname|acct|swapo(n|ff)|"
               r"kexec_\w+|(init|finit|delete|create|query)_module|get_kernel_syms|iopl|ioperm|"
               r"modify_ldt|vhangup|_sysctl|perf_event_open|bpf|seccomp|landlock_\w+|lsm_\w+|"
               r"(add|request)_key|keyctl|getrandom|u(ret)?probe)$"),
]


def group_of(name: str) -> str:
    for group, pattern in GROUPS:
        if re.match(pattern, name):
            return group
    return "other"


def veridian_status() -> tuple[set[int], set[int]]:
    src = (REPO / "kernel/src/syscall/linux_compat.rs").read_text()
    consts = {m[1]: int(m[2]) for m in re.finditer(r"const LINUX_([A-Z0-9_]+): usize = (\d+);", src)}

    def body(fn: str) -> str:
        start = src.index(fn)
        return src[start:src.index("\n}\n", start)]

    routed = {consts[n] for n in re.findall(r"LINUX_([A-Z0-9_]+)", body("fn translate_linux_syscall"))
              if n in consts}
    stubbed = {consts[n] for n in re.findall(r"LINUX_([A-Z0-9_]+)", body("fn handle_linux_stub"))
               if n in consts}
    routed |= {consts[n.upper()] for n in SPECIAL_CASED if n.upper() in consts}
    return routed - stubbed, stubbed


def main() -> None:
    table_path, release = sys.argv[1], sys.argv[2]
    rows = []
    for line in Path(table_path).read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split()
        if fields[1] not in ("common", "64"):
            continue
        rows.append((int(fields[0]), fields[2], len(fields) >= 4))

    implemented, stubbed = veridian_status()
    counts = {"implemented": 0, "stub": 0, "missing": 0, "enosys": 0}
    out = []
    for number, name, has_entry in rows:
        if number in implemented:
            status = "implemented"
        elif number in stubbed:
            status = "stub"
        elif not has_entry:
            status = "enosys"
        else:
            status = "missing"
        counts[status] += 1
        out.append(f"| {number} | `{name}` | {group_of(name)} | {status} |")

    print(f"# Linux syscall coverage (x86_64, Linux {release})\n")
    print("Generated by `tools/compat/syscall_coverage.py`; do not edit by hand.\n")
    print(f"Linux {release} defines {len(rows)} native x86_64 syscalls. VeridianOS: "
          f"{counts['implemented']} implemented (some partially), {counts['stub']} stubbed, "
          f"{counts['missing']} missing; {counts['enosys']} have no implementation in Linux "
          "either and correctly return ENOSYS.\n")
    print("`implemented` means routed to a kernel implementation, which may still be partial; "
          "behavioural conformance is measured by LTP and the Open POSIX Test Suite, not by this "
          "table.\n")
    print("| # | Syscall | Group | Status |")
    print("|---|---|---|---|")
    print("\n".join(out))


if __name__ == "__main__":
    main()
