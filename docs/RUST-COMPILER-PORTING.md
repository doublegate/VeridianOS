# Porting the Rust Compiler to VeridianOS

**Target Version**: Rust 1.93.1
**Target Triple**: `x86_64-unknown-veridian`
**Last Updated**: February 27, 2026

## Overview

This guide documents the process of cross-compiling a fully functional Rust
compiler (rustc + cargo) that runs natively on VeridianOS. The port targets
x86_64-unknown-veridian as the initial platform, using LLVM 19 as the backend
code generator. Once complete, VeridianOS can compile Rust programs on-device
without relying on a Linux host, closing the self-hosting loop for Rust
alongside the existing GCC 14.2 C toolchain (completed in Tier 7).

The Rust port follows the same cross-compilation strategy proven during the
GCC 14.2 port: build a cross-compiler on Linux, use it to produce static
VeridianOS binaries, embed them in a BlockFS rootfs image, and verify
self-hosting by compiling a non-trivial program (rustc itself, ideally)
on the target.

### Prerequisites

- Linux host with Rust nightly-2025-01-15 and `rust-src` component
- LLVM 19 source tree (matching rustc's bundled LLVM fork)
- VeridianOS cross-sysroot at `/opt/veridian/sysroot/x86_64` with libc headers
  and static libraries (libc.a, libm.a, crt0.o, crti.o, crtn.o)
- GCC 14.2 cross-compiler (`x86_64-veridian-gcc`) for C library compilation
- 2GB+ BlockFS rootfs image capacity
- QEMU 10.2 with KVM acceleration and 4096MB RAM allocation

### Scope

| Component | Included | Notes |
|-----------|----------|-------|
| rustc | Yes | Rust compiler driver |
| LLVM 19 | Yes | Code generation backend (static) |
| cargo | Yes | Package manager and build system |
| std | Yes | Standard library with VeridianOS platform layer |
| rustdoc | Deferred | Documentation generator (Phase 7) |
| clippy | Deferred | Lint tool (Phase 7) |
| rustfmt | Deferred | Code formatter (Phase 7) |

---

## Target Specification

### x86_64-unknown-veridian Target JSON

VeridianOS registers a custom target specification that instructs rustc how
to generate code and link binaries for the platform. This file lives in the
Rust source tree at `compiler/rustc_target/src/spec/targets/` and is also
available standalone for use with `--target` flags.

```json
{
    "llvm-target": "x86_64-unknown-veridian",
    "data-layout": "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
    "arch": "x86_64",
    "target-endian": "little",
    "target-pointer-width": "64",
    "target-c-int-width": "32",
    "os": "veridian",
    "env": "",
    "vendor": "unknown",
    "linker-flavor": "gcc",
    "linker": "x86_64-veridian-gcc",
    "pre-link-args": {
        "gcc": ["-static", "-nostdlib", "-Wl,--gc-sections"]
    },
    "post-link-args": {
        "gcc": ["-lc", "-lm", "-lgcc"]
    },
    "dynamic-linking": false,
    "executables": true,
    "has-rpath": false,
    "position-independent-executables": true,
    "relocation-model": "pic",
    "code-model": "small",
    "tls-model": "local-exec",
    "disable-redzone": false,
    "eliminate-frame-pointer": true,
    "function-sections": true,
    "abi": "SystemV",
    "panic-strategy": "abort",
    "features": "+sse,+sse2,+sse3,+ssse3,+sse4.1,+sse4.2",
    "crt-static-default": true,
    "crt-static-respected": true,
    "crt-static-allows-dylibs": false,
    "max-atomic-width": 64,
    "stack-probes": { "kind": "inline" }
}
```

### Key Design Decisions

| Property | Value | Rationale |
|----------|-------|-----------|
| `os` | `"veridian"` | Distinct OS identifier for conditional compilation via `#[cfg(target_os = "veridian")]` |
| `env` | `""` | No sub-environment distinction (no musl/gnu split) |
| `relocation-model` | `"pic"` | Position-independent code for ASLR compatibility (Phase 3 W^X/KPTI) |
| `panic-strategy` | `"abort"` | No unwinding support in VeridianOS libc; matches kernel convention |
| `dynamic-linking` | `false` | Static linking only; dynamic linker (ld-veridian) exists but is not yet mature enough for std |
| `tls-model` | `"local-exec"` | Thread-local storage via `arch_prctl(ARCH_SET_FS)` syscall; simplest model for static binaries |
| `crt-static-default` | `true` | All VeridianOS binaries are statically linked against libc.a |

### LLVM Triple Registration

The `x86_64-unknown-veridian` triple must be registered in LLVM's Triple.cpp
so that the backend recognizes VeridianOS as a valid operating system. This
was partially completed during Tier 6 (T6-4) of the self-hosting work.

```cpp
// llvm/lib/TargetParser/Triple.cpp
case Triple::VeridianOS:
    return "veridian";
```

The corresponding enum value is added to `llvm/include/llvm/TargetParser/Triple.h`:

```cpp
enum OSType {
    // ...existing entries...
    VeridianOS,
};
```

---

## std::sys::veridian Module Structure

The Rust standard library abstracts platform-specific behavior through the
`std::sys` module tree. Each supported OS provides an implementation of
file I/O, networking, threading, time, and other primitives. For VeridianOS,
this implementation lives at `library/std/src/sys/veridian/`.

### Module Layout

```
library/std/src/sys/veridian/
  mod.rs             -- Module root, feature gates, platform constants
  fs.rs              -- File and directory operations (open, read, write, stat, readdir)
  io.rs              -- I/O primitives (stdin, stdout, stderr, pipe, dup2)
  net.rs             -- TCP/UDP sockets (AF_INET, bind, listen, connect, sendto, recvfrom)
  process.rs         -- Process management (fork, exec, wait, exit, signal)
  thread.rs          -- Thread creation and synchronization (clone, futex, arch_prctl)
  time.rs            -- Clock sources (clock_gettime, gettimeofday, monotonic)
  os.rs              -- Environment variables, getcwd, chdir, hostname, uname
  alloc.rs           -- Global allocator bridge (mmap/munmap or brk/sbrk)
  locks.rs           -- Mutex, RwLock, Condvar backed by futex syscalls
  target_spec.rs     -- Target configuration constants (page size, path separator, etc.)
  fd.rs              -- Raw file descriptor wrapper (OwnedFd, BorrowedFd)
  path.rs            -- Path parsing rules (separator '/', current dir '.', parent '..')
  stdio.rs           -- Standard stream initialization and buffering
  args.rs            -- Command-line argument retrieval from process stack
```

### Syscall Bridge

All platform operations funnel through a thin syscall bridge that translates
Rust function calls into VeridianOS system calls. The bridge uses inline
assembly matching the kernel's syscall ABI:

```rust
// library/std/src/sys/veridian/syscall.rs
#[inline(always)]
pub unsafe fn syscall6(
    nr: usize,
    a0: usize, a1: usize, a2: usize,
    a3: usize, a4: usize, a5: usize,
) -> isize {
    let ret: isize;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr as isize => ret,
        in("rdi") a0,
        in("rsi") a1,
        in("rdx") a2,
        in("r10") a3,
        in("r8")  a4,
        in("r9")  a5,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack, preserves_flags),
    );
    ret
}
```

### Syscall Number Mapping

The std platform layer maps to VeridianOS syscall numbers defined in
`kernel/src/syscall/mod.rs`. Key mappings:

| std Operation | Syscall Number | Kernel Function |
|---------------|----------------|-----------------|
| `open` | 2 | `sys_open` |
| `read` | 0 | `sys_read` |
| `write` | 1 | `sys_write` |
| `close` | 3 | `sys_close` |
| `mmap` | 9 | `sys_mmap` |
| `munmap` | 11 | `sys_munmap` |
| `fork` | 57 | `sys_fork` |
| `execve` | 59 | `sys_execve` |
| `exit` | 60 | `sys_exit` |
| `wait4` | 61 | `sys_wait4` |
| `clone` | 56 | `sys_clone` |
| `futex` | 98 | `sys_futex` |
| `clock_gettime` | 228 | `sys_clock_gettime` |
| `pipe2` | 293 | `sys_pipe2` |
| `dup2` | 33 | `sys_dup2` |

### Conditional Compilation

Crates in the Rust ecosystem use `#[cfg(target_os = "veridian")]` to
conditionally include VeridianOS-specific code paths. The std library itself
uses this extensively:

```rust
#[cfg(target_os = "veridian")]
mod veridian;

#[cfg(target_os = "veridian")]
pub use veridian::*;
```

External crates that depend on OS-specific behavior (libc, nix, mio) will
need VeridianOS support added incrementally. For Phase 6.5, only the core
std library is ported; external crate compatibility is a Phase 7 objective.

---

## LLVM 19 Cross-Compilation

### Build Requirements

| Requirement | Version | Purpose |
|-------------|---------|---------|
| CMake | 3.20+ | LLVM build system |
| Ninja | 1.12+ | Fast parallel builds |
| GCC (host) | 13+ | Bootstrap C/C++ compiler |
| Python | 3.8+ | LLVM test infrastructure |
| x86_64-veridian-gcc | 14.2 | Cross-compiler for VeridianOS target libraries |

### CMake Toolchain File

```cmake
# toolchain/cmake/llvm-veridian-x86_64.cmake

set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(VERIDIAN_SYSROOT "/opt/veridian/sysroot/x86_64")

set(CMAKE_C_COMPILER   x86_64-veridian-gcc)
set(CMAKE_CXX_COMPILER x86_64-veridian-g++)
set(CMAKE_ASM_COMPILER x86_64-veridian-gcc)
set(CMAKE_AR           x86_64-veridian-ar)
set(CMAKE_RANLIB       x86_64-veridian-ranlib)
set(CMAKE_STRIP        x86_64-veridian-strip)

set(CMAKE_SYSROOT ${VERIDIAN_SYSROOT})
set(CMAKE_FIND_ROOT_PATH ${VERIDIAN_SYSROOT})

set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)

set(CMAKE_C_FLAGS_INIT   "-static -fPIC -ffunction-sections -fdata-sections")
set(CMAKE_CXX_FLAGS_INIT "-static -fPIC -ffunction-sections -fdata-sections -fno-exceptions -fno-rtti")
set(CMAKE_EXE_LINKER_FLAGS_INIT "-static -Wl,--gc-sections")
```

### LLVM Build Steps

```bash
#!/bin/bash
# build-llvm-veridian.sh
# Cross-compiles LLVM 19 static libraries for VeridianOS x86_64

set -euo pipefail

LLVM_SRC="${1:-llvm-project-19.1.0}"
BUILD_DIR="build-llvm-veridian"
INSTALL_DIR="/opt/veridian/llvm"
TOOLCHAIN_FILE="toolchain/cmake/llvm-veridian-x86_64.cmake"

mkdir -p "${BUILD_DIR}" && cd "${BUILD_DIR}"

cmake -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="../${TOOLCHAIN_FILE}" \
    -DCMAKE_INSTALL_PREFIX="${INSTALL_DIR}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DLLVM_TARGETS_TO_BUILD="X86" \
    -DLLVM_ENABLE_PROJECTS="" \
    -DLLVM_BUILD_TOOLS=OFF \
    -DLLVM_BUILD_UTILS=OFF \
    -DLLVM_INCLUDE_BENCHMARKS=OFF \
    -DLLVM_INCLUDE_EXAMPLES=OFF \
    -DLLVM_INCLUDE_TESTS=OFF \
    -DLLVM_INCLUDE_DOCS=OFF \
    -DLLVM_ENABLE_ZLIB=OFF \
    -DLLVM_ENABLE_ZSTD=OFF \
    -DLLVM_ENABLE_LIBXML2=OFF \
    -DLLVM_ENABLE_TERMINFO=OFF \
    -DLLVM_ENABLE_THREADS=OFF \
    -DLLVM_BUILD_STATIC=ON \
    -DLLVM_LINK_LLVM_DYLIB=OFF \
    -DLLVM_DEFAULT_TARGET_TRIPLE="x86_64-unknown-veridian" \
    -DLLVM_HOST_TRIPLE="x86_64-unknown-veridian" \
    "../${LLVM_SRC}/llvm"

ninja -j$(nproc)
ninja install
```

Key configuration notes:
- `LLVM_ENABLE_THREADS=OFF`: VeridianOS pthreads are functional but
  single-threaded LLVM avoids concurrency edge cases during initial porting.
- `LLVM_BUILD_STATIC=ON`: All LLVM libraries are linked statically into
  the final rustc binary. No shared library infrastructure required.
- `LLVM_TARGETS_TO_BUILD="X86"`: Only the x86_64 backend is needed for
  self-hosting. AArch64 and RISC-V backends can be added later.
- `LLVM_ENABLE_ZLIB=OFF` and similar: External library dependencies are
  disabled because VeridianOS does not yet have zlib, zstd, or libxml2 ports.

---

## rustc Build Stages

Building rustc for a new platform follows the standard Rust bootstrap
process, with modifications for cross-compilation to VeridianOS.

### config.toml

```toml
# config.toml for cross-compiling rustc to VeridianOS

changelog-seen = 2

[build]
host = ["x86_64-unknown-linux-gnu"]
target = ["x86_64-unknown-veridian", "x86_64-unknown-linux-gnu"]
docs = false
compiler-docs = false
extended = true
tools = ["cargo"]
vendor = true

[install]
prefix = "/usr/local"
sysconfdir = "/etc"

[llvm]
link-shared = false
static-libstdcpp = true
targets = "X86"
experimental-targets = ""

[target.x86_64-unknown-veridian]
cc = "x86_64-veridian-gcc"
cxx = "x86_64-veridian-g++"
ar = "x86_64-veridian-ar"
ranlib = "x86_64-veridian-ranlib"
linker = "x86_64-veridian-gcc"
llvm-config = "/opt/veridian/llvm/bin/llvm-config"
crt-static = true

[rust]
channel = "nightly"
codegen-units = 1
optimize = true
debug = false
debug-assertions = false
incremental = false
default-linker = "x86_64-veridian-gcc"
```

### Stage 0: Cross-Compile from Linux

Stage 0 uses the host Linux Rust compiler to build a cross-compiler that
targets x86_64-unknown-veridian. This stage produces rustc and std libraries
that run on Linux but emit VeridianOS binaries.

```bash
#!/bin/bash
# build-rustc-veridian.sh -- Stage 0 cross-compilation

set -euo pipefail

RUST_SRC="${1:-rust-1.93.1-src}"
BUILD_DIR="build-rustc-veridian"

# Apply VeridianOS platform patches
patch -d "${RUST_SRC}" -p1 < patches/rust-veridian-target.patch
patch -d "${RUST_SRC}" -p1 < patches/rust-veridian-std.patch

# Copy target specification
cp targets/x86_64-unknown-veridian.json \
   "${RUST_SRC}/compiler/rustc_target/src/spec/targets/"

# Copy std::sys::veridian module
cp -r std-veridian/ \
   "${RUST_SRC}/library/std/src/sys/veridian/"

# Copy config.toml
cp config-veridian.toml "${RUST_SRC}/config.toml"

cd "${RUST_SRC}"

# Stage 0: Build cross-compiler (runs on Linux, targets VeridianOS)
python3 x.py build --stage 0 \
    --target x86_64-unknown-veridian \
    library/std

# Build rustc itself for VeridianOS target
python3 x.py build --stage 1 \
    --target x86_64-unknown-veridian \
    compiler/rustc

# Build cargo for VeridianOS target
python3 x.py build --stage 1 \
    --target x86_64-unknown-veridian \
    src/tools/cargo

echo "Stage 0+1 complete. Binaries in build/x86_64-unknown-veridian/"
```

### Stage 1: Self-Hosted Compilation

Stage 1 uses the Stage 0 cross-compiled rustc (running on VeridianOS inside
QEMU) to compile rustc again. This validates that the compiler can function
on the target platform.

```bash
# Run inside VeridianOS (QEMU)
export PATH="/usr/local/bin:$PATH"
export RUST_TARGET="x86_64-unknown-veridian"

# Verify the cross-compiled rustc runs
rustc --version --verbose
# Expected: rustc 1.93.1 (veridian)
# Expected: host: x86_64-unknown-veridian

# Compile a simple test program
cat > /tmp/hello.rs << 'HELLO'
fn main() {
    println!("Hello from VeridianOS rustc!");
}
HELLO

rustc /tmp/hello.rs -o /tmp/hello
/tmp/hello
# Expected: Hello from VeridianOS rustc!
```

### Stage 2: Verification Build

Stage 2 compiles rustc a second time using the Stage 1 compiler. The
resulting binaries are byte-compared against Stage 1 output. Matching
binaries confirm deterministic compilation and prove the compiler is
fully self-hosting.

```bash
# Stage 2 verification (resource-intensive -- single-threaded recommended)
# Run inside QEMU with 4096MB RAM
python3 x.py build --stage 2 \
    --target x86_64-unknown-veridian \
    compiler/rustc

# Compare Stage 1 and Stage 2 artifacts
diff build/stage1/x86_64-unknown-veridian/release/rustc \
     build/stage2/x86_64-unknown-veridian/release/rustc
# Expected: identical (or reproducible differences from path embedding)
```

---

## Cargo Cross-Compilation

### Vendored Dependencies

VeridianOS does not have network access inside QEMU (TCP/IP stack exists
but no host-facing network bridge is configured by default). All crate
dependencies must be vendored into the rootfs before compilation.

```bash
# On the Linux host, vendor all dependencies
cd my-project
cargo vendor vendor/

# Create .cargo/config.toml for vendored deps
cat > .cargo/config.toml << 'EOF'
[source.crates-io]
replace-with = "vendored"

[source.vendored]
directory = "vendor"

[build]
target = "x86_64-unknown-veridian"

[target.x86_64-unknown-veridian]
linker = "x86_64-veridian-gcc"
rustflags = ["-C", "target-feature=+crt-static"]
EOF
```

### Building Crates on VeridianOS

```bash
# Inside VeridianOS QEMU session
cd /home/user/my-project
cargo build --release

# Run the compiled binary
./target/x86_64-unknown-veridian/release/my-project
```

### Limitations of Cargo on VeridianOS

| Limitation | Reason | Workaround |
|------------|--------|------------|
| No `cargo install` from registry | No network access in QEMU | Vendor deps on host, embed in rootfs |
| No parallel compilation | Single-threaded recommended | Use `-j1` or `codegen-units = 1` |
| No incremental compilation | Filesystem performance | Set `incremental = false` in config |
| No proc-macro cross-compilation | proc-macro runs on host during build | Pre-expand macros on Linux host |
| Large binary sizes | Static linking, no LTO across C/Rust | Accept for now; LTO in Phase 7 |

---

## Rootfs Integration

### BlockFS Layout

The Rust toolchain is embedded in a 2GB BlockFS rootfs image alongside the
existing GCC 14.2 toolchain and BusyBox utilities:

```
/
  bin/
    busybox          -- BusyBox 1.36.1 (95 applets)
    ash              -- Symlink to busybox
    vsh              -- VeridianOS native shell
  sbin/
    init             -- PID 1 init binary
  usr/
    local/
      bin/
        rustc        -- Rust compiler (~180MB static)
        cargo        -- Cargo package manager (~60MB static)
        gcc          -- GCC 14.2 (~35MB static)
        as           -- GNU assembler
        ld           -- GNU linker
        ar           -- Archive tool
        make         -- GNU Make 4.4.1
        ninja        -- Ninja 1.12.1
        vpkg         -- VeridianOS package manager
      lib/
        rustlib/
          x86_64-unknown-veridian/
            lib/
              libstd-*.rlib
              libcore-*.rlib
              liballoc-*.rlib
              libcompiler_builtins-*.rlib
              libpanic_abort-*.rlib
        libc.a
        libm.a
        libgcc.a
      include/
        (C headers)
  etc/
    os-release
  tmp/
```

### Build Script

```bash
#!/bin/bash
# build-rust-rootfs.sh
# Builds a 2GB BlockFS rootfs with Rust toolchain

set -euo pipefail

ROOTFS_SIZE=$((2 * 1024 * 1024 * 1024))  # 2GB
ROOTFS_IMG="target/rootfs-rust.img"
MKFS="tools/mkfs-blockfs/target/release/mkfs-blockfs"

# Build mkfs-blockfs if needed
if [ ! -f "${MKFS}" ]; then
    cd tools/mkfs-blockfs && cargo build --release && cd ../..
fi

# Create image
dd if=/dev/zero of="${ROOTFS_IMG}" bs=1M count=2048
"${MKFS}" "${ROOTFS_IMG}"

# Populate with existing rootfs contents
"${MKFS}" --add "${ROOTFS_IMG}" target/rootfs/ /

# Add Rust toolchain
"${MKFS}" --add "${ROOTFS_IMG}" build-rustc-veridian/stage1/bin/rustc /usr/local/bin/rustc
"${MKFS}" --add "${ROOTFS_IMG}" build-rustc-veridian/stage1/bin/cargo /usr/local/bin/cargo
"${MKFS}" --add-dir "${ROOTFS_IMG}" build-rustc-veridian/stage1/lib/rustlib/ /usr/local/lib/rustlib/

echo "Rootfs built: ${ROOTFS_IMG} ($(du -h ${ROOTFS_IMG} | cut -f1))"
```

---

## Self-Hosting Verification

### Automated Verification Script

```bash
#!/bin/bash
# self-host-verify.sh
# Verifies Rust self-hosting inside QEMU

set -euo pipefail

LOG="/tmp/VeridianOS/rust-selfhost.log"
mkdir -p /tmp/VeridianOS

# Kill any existing QEMU
pkill -9 -f qemu-system-x86_64 || true
sleep 3

# Boot with 2GB rootfs
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -drive file=target/rootfs-rust.img,if=none,id=vd0,format=raw \
    -device virtio-blk-pci,drive=vd0 \
    -serial stdio -display none -m 4096M \
    </dev/null > "${LOG}" 2>&1 &
QEMU_PID=$!

sleep 60  # Allow boot + rootfs mount

# Check log for self-hosting markers
if grep -q "RUST_SELFHOST_PASS" "${LOG}"; then
    echo "PASS: Rust self-hosting verified"
else
    echo "FAIL: Self-hosting verification did not complete"
    echo "Last 20 lines of log:"
    tail -20 "${LOG}"
fi

kill "${QEMU_PID}" 2>/dev/null
wait "${QEMU_PID}" 2>/dev/null
```

### Verification Criteria

| Criterion | Test | Expected |
|-----------|------|----------|
| rustc runs | `rustc --version` | `rustc 1.93.1 (veridian)` |
| Hello world compiles | `rustc hello.rs -o hello` | Exit code 0 |
| Hello world executes | `./hello` | `Hello from VeridianOS rustc!` |
| cargo init works | `cargo init /tmp/test-project` | Project scaffolded |
| cargo build works | `cargo build` | Binary produced |
| core compiles | `rustc --edition 2021 --crate-type lib core_test.rs` | Exit code 0 |
| std links | Binary uses std functions (println, File, Vec) | Runs correctly |

### Performance Expectations

Compilation performance on VeridianOS under QEMU with KVM:

| Workload | Estimated Time | Notes |
|----------|----------------|-------|
| Hello world (rustc) | ~5-10 seconds | Single file, no deps |
| Small crate (cargo) | ~30-60 seconds | 5-10 source files |
| Medium crate (cargo) | ~5-15 minutes | 50-100 source files |
| rustc Stage 1 | ~2-4 hours | Full compiler rebuild |
| rustc Stage 2 | ~2-4 hours | Verification build |

These estimates assume single-threaded compilation (`-j1`) on QEMU with KVM.
Without KVM (TCG), multiply by approximately 10-50x.

---

## Known Limitations

| Limitation | Impact | Resolution Timeline |
|------------|--------|---------------------|
| QEMU testing only | Cannot run on real hardware yet | Phase 7 (hardware bringup) |
| No GPU acceleration | LLVM backend is CPU-only | Phase 7 (GPU drivers) |
| Single-thread compilation | Parallel rustc risks thread scheduler edge cases | Phase 7 (threading hardening) |
| No dynamic linking | All binaries statically linked (~180MB for rustc) | Phase 7 (ld-veridian maturity) |
| No network for crates.io | Must vendor all dependencies offline | Phase 7 (network bridge) |
| No incremental compilation | Every build is a full rebuild | Phase 7 (filesystem perf) |
| 2GB rootfs limit | Constrains toolchain + project size | Phase 7 (larger BlockFS) |
| No proc-macro support | Cannot build crates using derive macros natively | Phase 7 (proc-macro host) |
| x86_64 only | AArch64 and RISC-V ports deferred | Phase 7 (multi-arch rustc) |

---

## Build Scripts Summary

| Script | Purpose | Host/Target | Output |
|--------|---------|-------------|--------|
| `build-llvm-veridian.sh` | Cross-compile LLVM 19 static libs | Runs on Linux, targets VeridianOS | `/opt/veridian/llvm/` |
| `build-rustc-veridian.sh` | Cross-compile rustc + cargo + std | Runs on Linux, targets VeridianOS | `build-rustc-veridian/stage1/` |
| `build-rust-rootfs.sh` | Package toolchain into BlockFS image | Runs on Linux | `target/rootfs-rust.img` |
| `self-host-verify.sh` | Verify self-hosting in QEMU | Runs on Linux (QEMU host) | PASS/FAIL |

---

## Troubleshooting

### Common Build Failures

**LLVM CMake cannot find VeridianOS system headers**

Ensure the sysroot is populated and the toolchain file points to it:
```bash
ls /opt/veridian/sysroot/x86_64/include/stdio.h
# Should exist
```

**rustc Stage 0 fails with "unknown target os: veridian"**

Apply the target specification patch before building:
```bash
patch -d rust-1.93.1-src -p1 < patches/rust-veridian-target.patch
```

**Linker errors: undefined reference to `pthread_create`**

VeridianOS libc provides pthread stubs. Ensure libc.a includes pthread symbols:
```bash
x86_64-veridian-nm /opt/veridian/sysroot/x86_64/lib/libc.a | grep pthread_create
# Should show T pthread_create
```

**Out of memory during LLVM compilation inside QEMU**

Increase QEMU RAM to 4096MB. LLVM compilation peak memory can reach 2-3GB:
```bash
qemu-system-x86_64 -enable-kvm ... -m 4096M
```

**Stage 1 rustc binary is too large for rootfs**

Strip debug symbols and use `--release` profile:
```bash
x86_64-veridian-strip build/stage1/bin/rustc
# Reduces from ~500MB to ~180MB
```

---

## References

- [Rust Target Tier Policy](https://doc.rust-lang.org/nightly/rustc/target-tier-policy.html)
- [Adding a New Target to Rust](https://rustc-dev-guide.rust-lang.org/building/new-target.html)
- [LLVM Cross-Compilation Guide](https://llvm.org/docs/HowToCrossCompileLLVM.html)
- [VeridianOS Self-Hosting Status](SELF-HOSTING-STATUS.md)
- [VeridianOS Cross-Compilation Guide](CROSS-COMPILATION.md)
- [VeridianOS Compiler Toolchain Guide](COMPILER-TOOLCHAIN-GUIDE.md)
