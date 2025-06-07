# VeridianOS Native Compiler Toolchain Integration Guide

## Overview

This guide details the strategy for integrating a complete native compiler toolchain into VeridianOS, supporting C, C++, Rust, Go, Python, and Assembly across all target architectures (x86_64, AArch64, RISC-V).

## Toolchain Architecture

### Unified LLVM Backend Strategy

VeridianOS prioritizes LLVM as the primary compiler infrastructure due to:
- Unified backend for multiple languages (C/C++, Rust, Swift, etc.)
- Modern architecture with excellent optimization
- Easier to port than GCC's complex build system
- Native cross-compilation support

```
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Clang     │  │    Rust     │  │   Other     │
│  Frontend   │  │  Frontend   │  │ Frontends   │
└──────┬──────┘  └──────┬──────┘  └──────┬──────┘
       │                 │                 │
       └─────────────────┴─────────────────┘
                         │
                   ┌─────▼─────┐
                   │   LLVM    │
                   │    IR     │
                   └─────┬─────┘
                         │
        ┌────────────────┼────────────────┐
        │                │                │
  ┌─────▼─────┐   ┌─────▼─────┐   ┌─────▼─────┐
  │  x86_64   │   │  AArch64  │   │  RISC-V   │
  │  Backend  │   │  Backend  │   │  Backend  │
  └───────────┘   └───────────┘   └───────────┘
```

## Phase 4 Implementation Plan

### Stage 1: Bootstrap Cross-Toolchain (Months 1-2)

#### 1.1 LLVM/Clang Port

```bash
# Add VeridianOS target to LLVM
llvm/lib/Target/VeridianOS/
├── VeridianOSTargetInfo.cpp
├── VeridianOSTargetMachine.cpp
├── VeridianOSABI.cpp
└── CMakeLists.txt

# Target triple registration
Triple::VeridianOS:
  - x86_64-unknown-veridian
  - aarch64-unknown-veridian
  - riscv64-unknown-veridian
```

Implementation steps:
1. Fork LLVM repository
2. Add VeridianOS as recognized OS in Triple.h
3. Implement target-specific ABI handling
4. Configure default linker scripts
5. Build cross-compiler on Linux host

#### 1.2 GNU Binutils Port

Essential tools needed:
- `as` - Assembler (multi-arch)
- `ld` - Linker (or use LLVM's lld)
- `ar` - Archive manager
- `nm` - Symbol lister
- `objdump` - Object file analyzer

### Stage 2: Native Toolchain (Months 2-3)

#### 2.1 Self-Hosting Preparation

```toml
# Package: llvm-toolchain
[package]
name = "llvm-toolchain"
version = "17.0"
targets = ["x86_64", "aarch64", "riscv64"]

[components]
clang = { version = "17.0", features = ["all-targets"] }
lld = { version = "17.0" }
compiler-rt = { version = "17.0" }
libcxx = { version = "17.0" }
```

#### 2.2 Three-Stage Bootstrap Process

**Stage 0: Cross-compilation from Linux**
```bash
# Build LLVM targeting VeridianOS
cmake -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DLLVM_TARGETS_TO_BUILD="X86;AArch64;RISCV" \
  -DLLVM_DEFAULT_TARGET_TRIPLE=x86_64-unknown-veridian \
  -DLLVM_HOST_TRIPLE=x86_64-unknown-veridian \
  -DCMAKE_CROSSCOMPILING=ON \
  -DCMAKE_INSTALL_PREFIX=/usr \
  ../llvm

ninja && ninja install DESTDIR=$VERIDIAN_SYSROOT
```

**Stage 1: Minimal native tools**
- Basic assembler and linker
- Minimal C compiler (no optimization)
- Core runtime libraries

**Stage 2: Full native toolchain**
- Optimizing compiler
- Full standard libraries
- Development tools

### Stage 3: Language-Specific Integration (Months 3-4)

#### 3.1 C/C++ Support

**Standard Library Strategy:**
- Use LLVM's libc++ for C++ standard library
- Port musl libc for C standard library
- Custom VeridianOS-specific headers

```cpp
// veridian-libc/include/veridian/syscall.h
#define SYS_capability_create  100
#define SYS_capability_invoke  101
#define SYS_ipc_send          200
#define SYS_ipc_receive       201

static inline long veridian_syscall(long nr, ...) {
    // Architecture-specific syscall implementation
    #ifdef __x86_64__
        // syscall instruction
    #elif __aarch64__
        // svc instruction
    #elif __riscv
        // ecall instruction
    #endif
}
```

#### 3.2 Rust Integration

**Rust std Implementation:**

```rust
// library/std/src/sys/veridian/mod.rs
pub mod alloc;
pub mod args;
pub mod env;
pub mod fs;
pub mod io;
pub mod net;
pub mod os;
pub mod path;
pub mod pipe;
pub mod process;
pub mod thread;
pub mod time;

// Capability-based file operations
pub mod fs {
    use crate::io::{self, Result};
    use crate::sys::veridian::capability::{Capability, CapabilityType};
    
    pub struct File {
        cap: Capability,
    }
    
    impl File {
        pub fn open(path: &Path, opts: &OpenOptions) -> Result<File> {
            let vfs_cap = capability::get_vfs_capability()?;
            let file_cap = vfs_cap.invoke_open(path, opts)?;
            Ok(File { cap: file_cap })
        }
    }
}
```

**Cargo Configuration:**
```toml
# .cargo/config.toml
[target.x86_64-unknown-veridian]
linker = "veridian-ld"
rustflags = ["-C", "target-feature=+crt-static"]

[target.aarch64-unknown-veridian]
linker = "veridian-ld"

[target.riscv64gc-unknown-veridian]
linker = "veridian-ld"
```

#### 3.3 Go Support

**Two-Phase Approach:**

1. **Phase 4: gccgo** (easier)
   - Part of GCC suite
   - Uses standard C runtime
   - Lower performance but easier porting

2. **Phase 5: Native Go** (optimal)
   - Port official Go runtime
   - Implement VeridianOS-specific runtime/os_veridian.go
   - Custom goroutine scheduler integration

```go
// src/runtime/os_veridian.go
package runtime

import "unsafe"

const (
    _CAPABILITY_CREATE = 100
    _IPC_SEND         = 200
)

//go:nosplit
func osyield() {
    veridian_syscall(_SYS_yield, 0, 0, 0)
}

//go:nosplit
func futexsleep(addr *uint32, val uint32, ns int64) {
    // Map to VeridianOS synchronization primitives
}
```

#### 3.4 Python Integration

**CPython Port Configuration:**

```python
# Modules/Setup.veridian
# VeridianOS-specific module configuration

# Core modules (static)
posix posixmodule.c
errno errnomodule.c
pwd pwdmodule.c

# Disabled modules (not available on VeridianOS)
#spwd spwdmodule.c    # No shadow passwords
#nis nismodule.c      # No NIS/YP

# VeridianOS-specific modules
veridian veridianmodule.c -lcapability
```

**Custom Module for Capabilities:**
```c
// Modules/veridianmodule.c
static PyObject *
veridian_get_capability(PyObject *self, PyObject *args) {
    const char *name;
    if (!PyArg_ParseTuple(args, "s", &name))
        return NULL;
    
    capability_t cap = veridian_capability_get(name);
    return PyLong_FromLong(cap);
}

static PyMethodDef veridian_methods[] = {
    {"get_capability", veridian_get_capability, METH_VARARGS,
     "Get a named capability"},
    {NULL, NULL, 0, NULL}
};
```

#### 3.5 Assembly Support

**Multi-Architecture Assembler:**

```makefile
# Unified assembler wrapper
/usr/bin/as:
  ├── as-x86_64    (x86_64 assembler)
  ├── as-aarch64   (ARM64 assembler)
  └── as-riscv64   (RISC-V assembler)

# Automatic architecture detection
#!/bin/sh
case "$1" in
  *x86_64*) exec as-x86_64 "$@" ;;
  *aarch64*) exec as-aarch64 "$@" ;;
  *riscv*) exec as-riscv64 "$@" ;;
  *) exec as-$(uname -m) "$@" ;;
esac
```

## Multi-Architecture Considerations

### Cross-Compilation Matrix

| Host Arch | Target Arch | Toolchain Package |
|-----------|-------------|-------------------|
| x86_64    | x86_64      | native-toolchain  |
| x86_64    | aarch64     | cross-aarch64     |
| x86_64    | riscv64     | cross-riscv64     |
| aarch64   | x86_64      | cross-x86_64      |
| aarch64   | aarch64     | native-toolchain  |
| aarch64   | riscv64     | cross-riscv64     |
| riscv64   | x86_64      | cross-x86_64      |
| riscv64   | aarch64     | cross-aarch64     |
| riscv64   | riscv64     | native-toolchain  |

### Sysroot Organization

```
/usr/lib/veridian-sysroots/
├── x86_64-veridian/
│   ├── usr/
│   │   ├── include/
│   │   └── lib/
│   └── lib/
├── aarch64-veridian/
└── riscv64-veridian/
```

## Build System Integration

### CMake Toolchain Files

```cmake
# /usr/share/cmake/veridian-toolchain-x86_64.cmake
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)
set(CMAKE_AR llvm-ar)
set(CMAKE_RANLIB llvm-ranlib)

# Multi-arch support
set(CMAKE_C_COMPILER_TARGET x86_64-unknown-veridian)
set(CMAKE_CXX_COMPILER_TARGET x86_64-unknown-veridian)
```

### Autotools Support

```m4
# /usr/share/aclocal/veridian.m4
AC_DEFUN([AC_VERIDIAN_INIT], [
  case $host_os in
    veridian*)
      AC_DEFINE([__VERIDIAN__], [1], [VeridianOS target])
      AC_DEFINE([_VERIDIAN_SOURCE], [1], [Enable VeridianOS extensions])
      
      # No fork() support
      ac_cv_func_fork=no
      ac_cv_func_vfork=no
      
      # Capability-based security
      AC_CHECK_HEADERS([veridian/capability.h])
      AC_CHECK_LIB([capability], [capability_create])
      ;;
  esac
])
```

## Package Management

### Toolchain Packages

```toml
# Base toolchain metapackage
[package]
name = "build-essential"
version = "1.0"
type = "metapackage"

[dependencies]
requires = [
    "clang >= 17.0",
    "lld >= 17.0",
    "make >= 4.3",
    "cmake >= 3.25",
    "ninja >= 1.11",
    "pkg-config >= 0.29"
]

[variants]
full = ["gcc", "gdb", "valgrind", "strace"]
minimal = []
```

### Language-Specific Packages

```toml
# Rust development
[package]
name = "rust-dev"
version = "1.75"

[components]
rustc = { targets = ["native", "wasm32"] }
cargo = { features = ["vendored-openssl"] }
rust-src = { optional = true }
rust-analyzer = { optional = true }

# Python development
[package]
name = "python3-dev"
version = "3.12"

[components]
python3 = { modules = ["ssl", "sqlite3", "ctypes"] }
pip3 = { index-url = "https://pypi.veridian-os.org" }
python3-venv = {}
```

## Testing and Validation

### Compiler Test Suite

```bash
#!/bin/bash
# /usr/share/veridian-toolchain/test-suite.sh

echo "Testing C compiler..."
cat > test.c <<EOF
#include <stdio.h>
int main() {
    printf("Hello from VeridianOS!\n");
    return 0;
}
EOF
clang test.c -o test && ./test || exit 1

echo "Testing C++ compiler..."
cat > test.cpp <<EOF
#include <iostream>
int main() {
    std::cout << "C++ works on VeridianOS!" << std::endl;
    return 0;
}
EOF
clang++ test.cpp -o test++ && ./test++ || exit 1

echo "Testing Rust compiler..."
cat > test.rs <<EOF
fn main() {
    println!("Rust works on VeridianOS!");
}
EOF
rustc test.rs -o test-rust && ./test-rust || exit 1

echo "All compiler tests passed!"
```

### Cross-Architecture Validation

```makefile
# Makefile for multi-arch testing
ARCHS = x86_64 aarch64 riscv64
TARGETS = $(ARCHS:%=test-%)

all: $(TARGETS)

test-%: test.c
	clang --target=$*-unknown-veridian test.c -o $@
	file $@ | grep -q $*
	@echo "✓ Built for $*"

clean:
	rm -f test-*
```

## Optimization and Performance

### Compiler Optimization Levels

```bash
# VeridianOS-specific optimization flags
-O2                    # Standard optimization
-Os                    # Size optimization (for embedded)
-O3 -march=native      # Maximum performance
-Oz                    # Extreme size optimization

# VeridianOS-specific flags
-fcapability-safety    # Enable capability checks
-fno-fork             # Disable fork() usage
-fveridian-ipc        # Optimize for VeridianOS IPC
```

### Link-Time Optimization (LTO)

```cmake
# Enable LTO for release builds
set(CMAKE_INTERPROCEDURAL_OPTIMIZATION_RELEASE ON)
set(CMAKE_C_FLAGS_RELEASE "${CMAKE_C_FLAGS_RELEASE} -flto=thin")
set(CMAKE_CXX_FLAGS_RELEASE "${CMAKE_CXX_FLAGS_RELEASE} -flto=thin")
```

## Development Workflow

### IDE/Editor Integration

```json
// .vscode/c_cpp_properties.json
{
    "configurations": [{
        "name": "VeridianOS",
        "compilerPath": "/usr/bin/clang",
        "compilerArgs": [
            "--target=x86_64-unknown-veridian",
            "-isystem/usr/include/veridian"
        ],
        "intelliSenseMode": "clang-x64",
        "cStandard": "c17",
        "cppStandard": "c++20"
    }]
}
```

### Debugging Support

```gdb
# /usr/share/gdb/veridian-gdb-init
# VeridianOS-specific GDB configuration

# Capability-aware printing
define print-capability
  set $cap = $arg0
  printf "Capability: type=%d, id=%d, rights=%x\n", \
    ($cap >> 48) & 0xFFFF, \
    ($cap >> 16) & 0xFFFFFFFF, \
    $cap & 0xFFFF
end

# IPC message tracing
catch syscall ipc_send ipc_receive
commands
  silent
  printf "IPC: %s cap=%x\n", $_syscall_name, $rdi
  continue
end
```

## Future Enhancements

### Phase 5: Advanced Features

1. **Profile-Guided Optimization (PGO)**
   - Kernel-integrated profiling
   - Automatic PGO builds

2. **Distributed Compilation**
   - VeridianOS-native distcc
   - Capability-secured build farm

3. **Language Server Protocol**
   - Native LSP support
   - IDE integration packages

### Phase 6: Specialized Compilers

1. **GPU Compilers**
   - CUDA/ROCm support
   - OpenCL implementation

2. **Domain-Specific Languages**
   - Shader compilers
   - Query language compilers

## Troubleshooting

### Common Issues

1. **"Cannot find crt0.o"**
   ```bash
   export VERIDIAN_SYSROOT=/usr/local/veridian-sysroot
   export LIBRARY_PATH=$VERIDIAN_SYSROOT/usr/lib
   ```

2. **"Undefined reference to __veridian_syscall"**
   - Ensure linking against VeridianOS libc
   - Check syscall wrapper implementation

3. **Cross-compilation failures**
   - Verify target triple spelling
   - Check sysroot paths
   - Ensure all dependencies are cross-compiled

## References

- LLVM VeridianOS Target: `/docs/dev/llvm-target.md`
- Rust std Porting: `/docs/dev/rust-std-port.md`
- Package Format Spec: `/docs/package-format.md`
- Toolchain Testing: `/tests/toolchain/`