# VeridianOS Cross-Compilation Toolchain Files

This directory contains build system integration files for cross-compiling
C/C++ software to run on VeridianOS from a Linux host.

## Overview

VeridianOS supports three target architectures. These toolchain files configure
CMake and Meson to use the appropriate cross-compiler, sysroot, and
architecture-specific flags for each target.

| Architecture | Triplet | ISA Baseline | ABI |
|---|---|---|---|
| x86_64 | `x86_64-veridian` | x86-64 (SSE2) | SysV AMD64 |
| AArch64 | `aarch64-veridian` | ARMv8-A (NEON) | AAPCS64 |
| RISC-V 64 | `riscv64-veridian` | RV64GC (IMAFDC) | LP64D |

## Prerequisites

### Cross-Compiler Toolchain

A GCC or Clang cross-compiler targeting VeridianOS must be installed. The
default expected location is `/opt/veridian/toolchain/`, with binaries at:

```
/opt/veridian/toolchain/bin/<triplet>-gcc
/opt/veridian/toolchain/bin/<triplet>-g++
/opt/veridian/toolchain/bin/<triplet>-ar
/opt/veridian/toolchain/bin/<triplet>-ld
...
```

Where `<triplet>` is one of `x86_64-veridian`, `aarch64-veridian`, or
`riscv64-veridian`.

To build the cross-compiler from source, see `docs/building-toolchain.md`
(when available) or use the VeridianOS ports system to bootstrap a
cross-compilation environment.

### Sysroot

A populated VeridianOS sysroot is required, containing the target system's
headers and libraries. The default expected locations are:

```
/opt/veridian/sysroot/x86_64/
/opt/veridian/sysroot/aarch64/
/opt/veridian/sysroot/riscv64/
```

Each sysroot should have the standard layout:

```
sysroot/
  usr/
    include/        # C/C++ headers
    lib/            # Libraries (.a, .so)
      pkgconfig/    # pkg-config .pc files
    share/
      pkgconfig/    # Architecture-independent .pc files
```

### Environment Variables

Both CMake and Meson toolchain files respect these environment variables to
override default paths:

| Variable | Default | Description |
|---|---|---|
| `VERIDIAN_TOOLCHAIN_PREFIX` | `/opt/veridian/toolchain` | Root of the cross-compiler installation |
| `VERIDIAN_SYSROOT` | `/opt/veridian/sysroot/<arch>` | Path to the target sysroot |

For Meson, you may also need to edit the `.ini` files directly if your
toolchain is installed in a non-standard location, as Meson cross files
require literal paths in the `[binaries]` section.

## Usage

### CMake

```bash
# x86_64
cmake -B build-x86_64 \
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-x86_64.cmake \
    -DCMAKE_BUILD_TYPE=Release
cmake --build build-x86_64

# AArch64
cmake -B build-aarch64 \
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-aarch64.cmake \
    -DCMAKE_BUILD_TYPE=Release
cmake --build build-aarch64

# RISC-V 64
cmake -B build-riscv64 \
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-riscv64.cmake \
    -DCMAKE_BUILD_TYPE=Release
cmake --build build-riscv64
```

With a custom sysroot location:

```bash
VERIDIAN_SYSROOT=/home/user/veridian-sysroot/x86_64 \
    cmake -B build \
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-x86_64.cmake
```

### Meson

```bash
# x86_64
meson setup build-x86_64 --cross-file toolchain/meson/veridian-x86_64.ini
meson compile -C build-x86_64

# AArch64
meson setup build-aarch64 --cross-file toolchain/meson/veridian-aarch64.ini
meson compile -C build-aarch64

# RISC-V 64
meson setup build-riscv64 --cross-file toolchain/meson/veridian-riscv64.ini
meson compile -C build-riscv64
```

Meson cross files can be combined with native files and additional overrides:

```bash
meson setup build \
    --cross-file toolchain/meson/veridian-x86_64.ini \
    --cross-file my-project-overrides.ini \
    -Dsome_option=value
```

## Architecture Details

### x86_64

- **ISA**: x86-64 baseline (equivalent to `-march=x86-64`)
- **Floating point**: SSE2 (guaranteed by x86-64 baseline)
- **Code model**: Default (small). Kernel code uses large/kernel model, but
  userspace applications use the standard small code model.
- **Flags**: `-march=x86-64 -mtune=generic`

### AArch64

- **ISA**: ARMv8-A baseline
- **Floating point**: Hardware FP and NEON SIMD (enabled by default on AArch64)
- **Code model**: Default (small)
- **Flags**: `-march=armv8-a -mtune=generic`

### RISC-V 64

- **ISA**: RV64GC (RV64I + M, A, F, D, C extensions)
  - M: Integer multiplication/division
  - A: Atomic instructions
  - F: Single-precision floating point
  - D: Double-precision floating point
  - C: Compressed instructions (16-bit)
- **ABI**: LP64D (64-bit long/pointer, hardware double-precision float passing)
- **Flags**: `-march=rv64gc -mabi=lp64d -mtune=generic`

## Troubleshooting

### "No such file or directory" for compiler

The cross-compiler is not installed or not at the expected path. Either install
it to `/opt/veridian/toolchain/` or set `VERIDIAN_TOOLCHAIN_PREFIX` to point
to your installation.

### "cannot find crt1.o" or missing startup files

The sysroot is not populated with the C runtime. Ensure `libc`, `crt1.o`,
`crti.o`, and `crtn.o` are present in `${VERIDIAN_SYSROOT}/usr/lib/`.

### CMake reports "System is unknown to cmake"

CMake does not have a built-in platform module for VeridianOS. The toolchain
file sets `CMAKE_TRY_COMPILE_TARGET_TYPE` to `STATIC_LIBRARY` to work around
this. For full integration, a `Platform/VeridianOS.cmake` module can be added
to CMake's module path.

### Meson reports "exe_wrapper not found"

The `exe_wrapper` entries in the Meson cross files use `qemu-<arch>-static`
for running target binaries during the build (e.g., for code generators).
Install the appropriate QEMU user-mode static binary, or remove the
`exe_wrapper` line if your build does not require running target binaries on
the host.

### pkg-config finds host libraries instead of target

Ensure `PKG_CONFIG_SYSROOT_DIR` and `PKG_CONFIG_LIBDIR` are set correctly.
The CMake toolchain files set these automatically. For Meson, they are
configured in the `[properties]` section of the cross file.

## File Listing

```
toolchain/
  cmake/
    veridian-x86_64.cmake       # CMake toolchain - x86_64
    veridian-aarch64.cmake      # CMake toolchain - AArch64
    veridian-riscv64.cmake      # CMake toolchain - RISC-V 64
  meson/
    veridian-x86_64.ini         # Meson cross file - x86_64
    veridian-aarch64.ini        # Meson cross file - AArch64
    veridian-riscv64.ini        # Meson cross file - RISC-V 64
  README.md                     # This file
```
