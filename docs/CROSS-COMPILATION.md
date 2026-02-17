# Cross-Compilation Guide for VeridianOS

This guide explains how to cross-compile C, C++, and Rust programs targeting
VeridianOS from a Linux host. VeridianOS supports three architectures: x86_64,
AArch64, and RISC-V 64-bit.

## Prerequisites

### Host System Requirements

- Linux host (any recent distribution)
- CMake 3.16+ or Meson 0.60+ (for C/C++ projects)
- Rust nightly toolchain with `rust-src` component (for Rust projects)
- GNU make or Ninja build system

### Cross-Compiler Toolchain

VeridianOS provides GCC and LLVM/Clang cross-compiler ports. The toolchain is
expected at `/opt/veridian/toolchain` by default. You can override this by
setting the `VERIDIAN_TOOLCHAIN_PREFIX` environment variable.

The official target triples are:

| Architecture | Target Triple                  |
|--------------|--------------------------------|
| x86_64       | `x86_64-veridian`      |
| AArch64      | `aarch64-veridian`     |
| RISC-V 64    | `riscv64gc-veridian`   |

The cross-compiler binaries follow the `<triple>-<tool>` naming convention.
For example, the x86_64 C compiler is `x86_64-veridian-gcc`.

### VeridianOS Sysroot

The sysroot contains headers and libraries for the target. Default locations:

| Architecture | Default Sysroot Path                    |
|--------------|-----------------------------------------|
| x86_64       | `/opt/veridian/sysroot/x86_64`          |
| AArch64      | `/opt/veridian/sysroot/aarch64`         |
| RISC-V 64    | `/opt/veridian/sysroot/riscv64`         |

Override with `VERIDIAN_SYSROOT` environment variable.

**Note:** The sysroot is not yet populated with a libc. A musl or newlib port
is required before user-space C programs can link dynamically. Currently, all
user-space code must be statically linked or compiled as freestanding (no libc
dependency).

## Using CMake Toolchain Files

VeridianOS ships pre-written CMake toolchain files in `toolchain/cmake/`:

- `toolchain/cmake/veridian-x86_64.cmake`
- `toolchain/cmake/veridian-aarch64.cmake`
- `toolchain/cmake/veridian-riscv64.cmake`

### Basic Usage

```bash
# Configure for x86_64
cmake -B build \
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-x86_64.cmake \
    .

# Build
cmake --build build

# Configure for AArch64
cmake -B build-aarch64 \
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-aarch64.cmake \
    .
```

### What the Toolchain Files Set

Each CMake toolchain file configures:

- `CMAKE_SYSTEM_NAME` to `VeridianOS`
- `CMAKE_SYSTEM_PROCESSOR` to the target architecture
- All compiler and binutils paths (`CC`, `CXX`, `AR`, `LD`, etc.)
- `CMAKE_SYSROOT` to the appropriate sysroot directory
- `CMAKE_FIND_ROOT_PATH_MODE_*` to search the sysroot for libraries/headers
  and the host for build programs
- Architecture-specific compiler flags (e.g., `-march=x86-64` or `-march=rv64gc`)
- `CMAKE_TRY_COMPILE_TARGET_TYPE` set to `STATIC_LIBRARY` to avoid link
  failures during compiler detection (no libc yet)
- `pkg-config` paths pointing into the sysroot

### Custom Toolchain Paths

```bash
# Override both paths via environment
export VERIDIAN_TOOLCHAIN_PREFIX=/path/to/toolchain
export VERIDIAN_SYSROOT=/path/to/sysroot

cmake -B build \
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-x86_64.cmake \
    .
```

## Using Meson Cross Files

Meson cross-compilation files are in `toolchain/meson/`:

- `toolchain/meson/veridian-x86_64.ini`
- `toolchain/meson/veridian-aarch64.ini`
- `toolchain/meson/veridian-riscv64.ini`

### Basic Usage

```bash
# Configure for x86_64
meson setup builddir --cross-file toolchain/meson/veridian-x86_64.ini

# Build
meson compile -C builddir

# Configure for AArch64
meson setup builddir-aarch64 --cross-file toolchain/meson/veridian-aarch64.ini
```

### Cross File Details

Each Meson cross file defines:

- `[binaries]` section: paths to `gcc`, `g++`, `ar`, `ld`, `strip`, `objcopy`,
  `objdump`, `ranlib`, `nm`, and `pkgconfig`
- `[host_machine]` section: `system = 'veridian'`, with correct `cpu_family`,
  `cpu`, and `endian`
- `[properties]` section: `sys_root`, `pkg_config_libdir`, and
  `needs_exe_wrapper = true`
- `[built-in options]` section: sysroot flags and architecture-specific
  compiler arguments

The AArch64 and RISC-V cross files include `exe_wrapper` entries pointing to
`qemu-aarch64-static` and `qemu-riscv64-static` respectively, enabling test
execution on x86_64 hosts via user-mode QEMU.

## Using Rust with Custom Target Specs

### Kernel Development

The kernel uses custom JSON target specifications in `targets/`:

- `targets/x86_64-veridian.json` -- kernel code model, no red zone, static
  relocation
- `targets/aarch64-veridian.json` -- ARMv8-A with NEON, custom linker script
- `targets/riscv64gc-veridian.json` -- RV64GC with medium code model, custom
  linker script

Build with:

```bash
cargo build --target targets/x86_64-veridian.json \
    -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc
```

### User-Space Rust Programs

For user-space Rust programs targeting VeridianOS, use the standard bare-metal
targets until a proper `veridian` target is upstreamed to rustc:

```bash
# x86_64 user-space (freestanding)
cargo build --target x86_64-unknown-none

# AArch64 user-space (freestanding)
cargo build --target aarch64-unknown-none

# RISC-V 64 user-space (freestanding)
cargo build --target riscv64gc-unknown-none-elf
```

User-space Rust programs should use `#![no_std]` and `#![no_main]`, and invoke
VeridianOS syscalls directly through inline assembly or the `libveridian`
wrapper (when available).

## Building a "Hello World" C Program

Since VeridianOS does not yet have a libc, C programs must be freestanding and
use raw syscalls. Here is a minimal example:

### hello.c

```c
/* hello.c -- Freestanding "Hello World" for VeridianOS x86_64 */

/* VeridianOS syscall numbers (from kernel/src/syscall/mod.rs) */
#define SYS_WRITE  53   /* FileWrite */
#define SYS_EXIT   11   /* ProcessExit */

/* Stdout file descriptor */
#define STDOUT_FD  1

static long veridian_syscall3(long num, long a1, long a2, long a3) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(num), "D"(a1), "S"(a2), "d"(a3)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long veridian_syscall1(long num, long a1) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(num), "D"(a1)
        : "rcx", "r11", "memory"
    );
    return ret;
}

void _start(void) {
    const char msg[] = "Hello from VeridianOS!\n";
    veridian_syscall3(SYS_WRITE, STDOUT_FD, (long)msg, sizeof(msg) - 1);
    veridian_syscall1(SYS_EXIT, 0);
    __builtin_unreachable();
}
```

### Compile and Link

```bash
# Compile (freestanding, no stdlib)
x86_64-veridian-gcc -ffreestanding -nostdlib -nostartfiles \
    -o hello hello.c

# Or with the host GCC if cross-compiler is not yet built:
gcc -target x86_64-unknown-none -ffreestanding -nostdlib -nostartfiles \
    -o hello hello.c
```

## Linking with CRT (C Runtime)

VeridianOS does not yet ship CRT startup files (`crt0.S`, `crti.S`, `crtn.S`).
These will be provided as part of the libc port (musl or newlib). Until then:

- Use `-nostartfiles` to suppress CRT linking
- Provide your own `_start` entry point
- Handle stack setup and argument passing manually
- Call `SYS_EXIT` (syscall 11) explicitly to terminate

Once a libc is available in the sysroot, the standard CRT files will handle:

| File      | Purpose                                              |
|-----------|------------------------------------------------------|
| `crt0.S`  | Program entry point, sets up argc/argv, calls main() |
| `crti.S`  | Function prologue for `.init` and `.fini` sections   |
| `crtn.S`  | Function epilogue for `.init` and `.fini` sections   |

## Architecture Matrix

| Feature                  | x86_64                          | AArch64                        | RISC-V 64                           |
|--------------------------|---------------------------------|--------------------------------|--------------------------------------|
| Target triple            | `x86_64-veridian`       | `aarch64-veridian`     | `riscv64gc-veridian`         |
| CMake toolchain          | `veridian-x86_64.cmake`         | `veridian-aarch64.cmake`       | `veridian-riscv64.cmake`             |
| Meson cross file         | `veridian-x86_64.ini`           | `veridian-aarch64.ini`         | `veridian-riscv64.ini`               |
| Architecture flags       | `-march=x86-64 -mtune=generic`  | `-march=armv8-a -mtune=generic`| `-march=rv64gc -mabi=lp64d`          |
| Syscall instruction      | `syscall`                       | `svc #0`                       | `ecall`                              |
| Kernel target JSON       | `x86_64-veridian.json`          | `aarch64-veridian.json`        | `riscv64gc-veridian.json`            |
| Code model (kernel)      | `kernel`                        | N/A (static)                   | `medium`                             |
| Endianness               | Little                          | Little                         | Little                               |
| Pointer width            | 64                              | 64                             | 64                                   |
| Max atomic width         | 64                              | 128                            | 64                                   |
| QEMU user-mode emulator  | N/A (native)                    | `qemu-aarch64-static`          | `qemu-riscv64-static`                |

## Common Issues and Troubleshooting

### "cannot find -lc"

VeridianOS does not yet have a libc. Compile with `-nostdlib -ffreestanding`
and avoid including standard headers that require libc backing.

### R_X86_64_32S relocation errors

The x86_64 kernel is linked in the top 2GB of virtual memory
(0xFFFFFFFF80100000). If you see relocation errors, ensure your code uses the
`kernel` code model (`-mcmodel=kernel` for GCC, or the `code-model: kernel`
field in the Rust target JSON).

User-space programs are loaded in the lower 128TB and should use the default
small code model.

### "undefined reference to _start"

Freestanding programs must define their own `_start` symbol. Use
`-nostartfiles` to prevent the linker from looking for CRT files.

### CMake "compiler test failed"

The toolchain files set `CMAKE_TRY_COMPILE_TARGET_TYPE` to `STATIC_LIBRARY`
to avoid link-time failures during CMake's compiler detection. If you
encounter issues, verify that the cross-compiler binary exists at the expected
path.

### Meson "exe_wrapper not found"

For AArch64 and RISC-V builds, install QEMU user-mode static binaries:

```bash
# Debian/Ubuntu
sudo apt install qemu-user-static

# Fedora
sudo dnf install qemu-user-static

# Arch Linux
sudo pacman -S qemu-user-static
```

### pkg-config finds host libraries

Ensure `PKG_CONFIG_SYSROOT_DIR` and `PKG_CONFIG_LIBDIR` point into the
VeridianOS sysroot. The CMake and Meson files set these automatically, but
manual builds may need explicit configuration.
