#!/usr/bin/env bash
# Phase 6.5 Wave 0 Prerequisite Verification Tests
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Verifies that all Wave 0 prerequisite sprints (P-1 through P-11) are
# implemented and compile correctly before proceeding to Wave 1+.
#
# Usage:
#   ./tests/phase6.5/test_prerequisites.sh
#
# Tests are grouped by sprint. Each test verifies:
#   1. Required source files exist
#   2. Key functions/types are defined
#   3. Kernel compiles clean on all 3 architectures
#   4. Clippy passes with -D warnings

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "$PROJECT_ROOT"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS=0
FAIL=0
SKIP=0

pass() { printf "${GREEN}PASS${NC}  %s\n" "$*"; PASS=$((PASS + 1)); }
fail() { printf "${RED}FAIL${NC}  %s\n" "$*"; FAIL=$((FAIL + 1)); }
skip() { printf "${YELLOW}SKIP${NC}  %s\n" "$*"; SKIP=$((SKIP + 1)); }

check_file() {
    if [[ -f "$1" ]]; then
        pass "File exists: $1"
    else
        fail "File missing: $1"
    fi
}

check_grep() {
    local file="$1"
    local pattern="$2"
    local desc="$3"
    if grep -q "$pattern" "$file" 2>/dev/null; then
        pass "$desc"
    else
        fail "$desc (pattern '$pattern' not found in $file)"
    fi
}

echo "====================================="
echo "Phase 6.5 Wave 0 Prerequisite Tests"
echo "====================================="
echo ""

# ---------------------------------------------------------------------------
# P-1: Memory Scaling
# ---------------------------------------------------------------------------
echo "--- P-1: Memory Scaling ---"
check_grep "kernel/src/syscall/memory.rs" "sys_getrlimit\|getrlimit" "getrlimit syscall defined"
check_grep "kernel/src/syscall/memory.rs" "sys_setrlimit\|setrlimit" "setrlimit syscall defined"
check_grep "kernel/src/syscall/mod.rs" "GetRlimit\|Getrlimit" "getrlimit wired in syscall dispatch"
echo ""

# ---------------------------------------------------------------------------
# P-2: Dynamic Linker
# ---------------------------------------------------------------------------
echo "--- P-2: Dynamic Linker ---"
check_file "userland/ld-veridian/ld-veridian.c"
check_grep "userland/ld-veridian/ld-veridian.c" "dlopen" "dlopen implementation present"
check_grep "userland/ld-veridian/ld-veridian.c" "dlsym" "dlsym implementation present"
check_grep "userland/ld-veridian/ld-veridian.c" "_start" "_start entry point present"
check_grep "userland/ld-veridian/ld-veridian.c" "AT_PHDR\|AT_ENTRY" "Auxiliary vector parsing present"
echo ""

# ---------------------------------------------------------------------------
# P-3: libc Networking
# ---------------------------------------------------------------------------
echo "--- P-3: libc Networking ---"
check_grep "userland/libc/include/veridian/syscall.h" "SYS_SOCKET" "Socket syscall numbers defined"
check_grep "userland/libc/src/posix_stubs2.c" "socket\|connect\|bind" "Socket functions wired to syscalls"
echo ""

# ---------------------------------------------------------------------------
# P-4: libc Threading
# ---------------------------------------------------------------------------
echo "--- P-4: libc Threading ---"
check_grep "userland/libc/include/pthread.h" "pthread_rwlock_t" "pthread_rwlock_t type defined"
check_grep "userland/libc/include/pthread.h" "pthread_key_t" "pthread_key_t type defined"
check_grep "userland/libc/include/pthread.h" "pthread_barrier_t" "pthread_barrier_t type defined"
check_grep "userland/libc/src/pthread.c" "pthread_rwlock_rdlock\|pthread_rwlock_wrlock" "rwlock implementation present"
check_grep "userland/libc/src/pthread.c" "pthread_key_create" "TLS key implementation present"
echo ""

# ---------------------------------------------------------------------------
# P-5: libc stdio/stdlib
# ---------------------------------------------------------------------------
echo "--- P-5: libc stdio/stdlib ---"
check_grep "userland/libc/src/stdio.c" "asprintf\|vasprintf" "asprintf/vasprintf implemented"
check_grep "userland/libc/src/stdlib.c" "MMAP_THRESHOLD\|mmap" "mmap-backed large allocations"
echo ""

# ---------------------------------------------------------------------------
# P-6: libc POSIX Gaps
# ---------------------------------------------------------------------------
echo "--- P-6: libc POSIX Gaps ---"
check_grep "userland/libc/src/posix_stubs.c" "getpwnam\|getpwuid" "passwd file parsing functions"
check_grep "userland/libc/src/posix_stubs3.c" "getgrnam\|getgrgid" "group file parsing functions"
echo ""

# ---------------------------------------------------------------------------
# P-7: Kernel Signal Infrastructure
# ---------------------------------------------------------------------------
echo "--- P-7: Kernel Signal Infrastructure ---"
check_grep "kernel/src/syscall/filesystem.rs" "send_signal_to_pgid\|kill.*pgid" "kill(-pgid) process group signaling"
check_grep "kernel/src/process/exit.rs" "SIGCHLD\|sigchld" "SIGCHLD delivery on process exit"
check_grep "kernel/src/syscall/signal.rs" "block_process\|Blocked" "sigsuspend scheduler blocking"
echo ""

# ---------------------------------------------------------------------------
# P-8: Terminal/PTY Infrastructure
# ---------------------------------------------------------------------------
echo "--- P-8: Terminal/PTY Infrastructure ---"
check_file "kernel/src/syscall/pty.rs"
check_grep "kernel/src/syscall/pty.rs" "sys_openpty" "openpty syscall implemented"
check_grep "kernel/src/syscall/pty.rs" "sys_ptsname" "ptsname syscall implemented"
check_grep "kernel/src/syscall/pty.rs" "handle_pty_ioctl" "PTY ioctl handler implemented"
check_grep "kernel/src/fs/pty.rs" "PtyMasterNode\|PtySlaveNode" "PTY VfsNode wrappers defined"
echo ""

# ---------------------------------------------------------------------------
# P-9: Filesystem Completeness
# ---------------------------------------------------------------------------
echo "--- P-9: Filesystem Completeness ---"
check_grep "kernel/src/fs/mod.rs" "can_read\|can_write" "Permission check methods on Permissions"
check_grep "kernel/src/fs/mod.rs" "resolve_path_no_follow\|symlink_depth" "Symlink resolution with loop detection"
check_grep "kernel/src/fs/ramfs.rs" "new_symlink\|readlink" "RamFS symlink support"
check_grep "kernel/src/syscall/filesystem.rs" "sys_readlink\|Readlink" "readlink syscall implemented"
check_grep "kernel/src/syscall/mod.rs" "Symlink\|Readlink\|Fchmod\|Fchown" "Filesystem syscalls wired in dispatch"
echo ""

# ---------------------------------------------------------------------------
# P-10: epoll I/O Multiplexing
# ---------------------------------------------------------------------------
echo "--- P-10: epoll I/O Multiplexing ---"
check_file "kernel/src/net/epoll.rs"
check_grep "kernel/src/net/epoll.rs" "epoll_create\|EpollInstance" "epoll_create implementation"
check_grep "kernel/src/net/epoll.rs" "epoll_ctl" "epoll_ctl implementation"
check_grep "kernel/src/net/epoll.rs" "epoll_wait" "epoll_wait implementation"
check_grep "kernel/src/net/mod.rs" "pub mod epoll" "epoll module registered"
check_grep "kernel/src/syscall/mod.rs" "EpollCreate\|epoll_create" "epoll syscalls wired in dispatch"
echo ""

# ---------------------------------------------------------------------------
# P-11: CMake Cross-Compilation
# ---------------------------------------------------------------------------
echo "--- P-11: CMake Cross-Compilation ---"
check_file "scripts/cmake/veridian-x86_64-toolchain.cmake"
check_file "scripts/build-cmake-veridian.sh"
check_grep "scripts/cmake/veridian-x86_64-toolchain.cmake" "CMAKE_SYSTEM_NAME.*VeridianOS" "VeridianOS system name defined"
check_grep "scripts/cmake/veridian-x86_64-toolchain.cmake" "CMAKE_C_COMPILER" "Cross-compiler paths set"
check_grep "scripts/build-cmake-veridian.sh" "llvm\|LLVM" "LLVM build mode supported"
echo ""

# ---------------------------------------------------------------------------
# Build Verification
# ---------------------------------------------------------------------------
echo "--- Build Verification ---"
echo "Building all 3 architectures..."

BUILD_OK=1
for arch in x86_64 aarch64 riscv64; do
    if ./build-kernel.sh "$arch" dev > /dev/null 2>&1; then
        pass "Kernel build: $arch"
    else
        fail "Kernel build: $arch"
        BUILD_OK=0
    fi
done

if [[ $BUILD_OK -eq 1 ]]; then
    echo ""
    echo "Running clippy on all 3 architectures..."
    for target_spec in "targets/x86_64-veridian.json" "aarch64-unknown-none" "riscv64gc-unknown-none-elf"; do
        arch_name="${target_spec%%/*}"
        [[ "$arch_name" == "targets" ]] && arch_name="x86_64"
        if cargo clippy --target "$target_spec" -p veridian-kernel \
            -Zbuild-std=core,compiler_builtins,alloc -- -D warnings > /dev/null 2>&1; then
            pass "Clippy clean: $arch_name"
        else
            fail "Clippy clean: $arch_name"
        fi
    done
fi

echo ""

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo "====================================="
TOTAL=$((PASS + FAIL + SKIP))
printf "Results: ${GREEN}%d passed${NC}, ${RED}%d failed${NC}, ${YELLOW}%d skipped${NC} / %d total\n" \
    "$PASS" "$FAIL" "$SKIP" "$TOTAL"
echo "====================================="

if [[ $FAIL -gt 0 ]]; then
    echo ""
    echo "Wave 0 prerequisites NOT met. Fix failures before proceeding to Wave 1."
    exit 1
else
    echo ""
    echo "All Wave 0 prerequisites verified. Ready for Wave 1 (Rust std platform)."
    exit 0
fi
