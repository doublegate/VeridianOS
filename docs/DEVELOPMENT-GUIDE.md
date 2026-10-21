# VeridianOS Development Guide

**Last Updated**: October 20, 2026
**Latest Release**: v0.27.0 - process model, SMP and user mode on all three architectures

## Getting Started

This guide covers everything you need to know to start developing VeridianOS, from setting up your environment to submitting your first contribution.

## Development Environment Setup

### Prerequisites

- **Operating System**: Linux (Ubuntu 22.04+, Fedora 38+, or similar)
- **RAM**: Minimum 8GB, 16GB recommended
- **Disk Space**: At least 20GB free
- **CPU**: x86_64 processor with virtualization support

### Required Software

#### 1. Install Rust Toolchain

```bash
# Install rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# The nightly pinned in rust-toolchain.toml, with its components
rustup toolchain install "$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)" \
    --component rust-src,llvm-tools-preview,rustfmt,clippy

# No `rustup target add` is needed: the kernel builds core/alloc itself (-Zbuild-std)
```

#### 2. Install System Dependencies

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install -y \
    build-essential \
    qemu-system-x86 \
    qemu-system-arm \
    qemu-system-misc \
    ovmf \
    gdb-multiarch \
    nasm \
    mtools \
    xorriso \
    clang \
    lld \
    cmake \
    ninja-build \
    python3 \
    python3-pip
```

**Fedora:**
```bash
sudo dnf install -y \
    @development-tools \
    qemu-system-x86 \
    qemu-system-aarch64 \
    qemu-system-riscv \
    edk2-ovmf \
    gdb \
    nasm \
    mtools \
    xorriso \
    clang \
    lld \
    cmake \
    ninja-build \
    python3 \
    python3-pip
```

#### 3. Install Development Tools

```bash
# Cargo extensions
cargo install --locked cargo-audit
cargo install --locked cargo-llvm-cov      # coverage, as CI measures it
cargo install --locked cargo-outdated      # `just outdated`
cargo install mdbook --version 0.4.37 --locked

# Just (command runner)
cargo install just
```

`./scripts/install-deps.sh` installs the toolchain and these tools in one step. The x86_64 UEFI
disk image is built by `tools/bootimage-builder`, which `build-kernel.sh` runs; no `bootimage` or
`cargo-xbuild` is needed.

### IDE Setup

#### Visual Studio Code

1. Install VS Code
2. Install extensions:
   - rust-analyzer
   - CodeLLDB (for debugging)
   - Better TOML
   - crates
   - Error Lens

**`.vscode/settings.json`:**
```json
{
    "rust-analyzer.cargo.target": "x86_64-unknown-linux-gnu",
    "rust-analyzer.cargo.features": ["alloc"],
    "rust-analyzer.checkOnSave.allTargets": false,
    "rust-analyzer.checkOnSave.command": "clippy",
    "editor.formatOnSave": true,
    "files.watcherExclude": {
        "**/target/**": true
    }
}
```

#### IntelliJ IDEA / CLion

1. Install Rust plugin
2. Configure custom target in project settings
3. Set up run configurations for QEMU

## Project Structure

```
VeridianOS/
├── kernel/                # The kernel crate
│   └── src/
│       ├── arch/          # x86_64, aarch64, riscv64
│       ├── mm/            # Memory management
│       ├── sched/         # Scheduler and dispatcher
│       ├── ipc/           # Inter-process communication
│       ├── cap/           # Capability system
│       ├── process/       # Processes, threads, signals
│       ├── syscall/       # System calls (Linux numbers, ADR 0009)
│       ├── fs/            # VFS, BlockFS, tmpfs, procfs
│       ├── drivers/       # Device drivers (in the kernel today; critique C6)
│       ├── net/           # Network stack (in the kernel today)
│       ├── services/      # Shell and system services (in the kernel today)
│       └── desktop/       # Built-in desktop environment
├── userland/              # libc, programs, tests, KDE/Qt ports
├── abi/                   # The system call table (abi/syscalls.map)
├── tools/                 # Host tools (mkfs-blockfs, bootimage-builder, cross scripts)
├── scripts/               # Build, test, debug and gate scripts
├── targets/               # Custom target specifications
├── verification/          # Kani proofs and TLA+ specifications
├── ports/                 # Ports system recipes
├── docs/                  # Documentation and the mdBook
└── to-dos/                # Planning and tracking
```

## Development Workflow

### 1. Building the Kernel

**Recommended Method - Build Script:**
```bash
# Build all architectures (development mode)
./build-kernel.sh all dev

# Build specific architecture
./build-kernel.sh x86_64 dev     # Uses kernel code model for x86_64
./build-kernel.sh aarch64 release
./build-kernel.sh riscv64 dev
```

**Alternative - Using Just:**
```bash
# Build every architecture (dev)
just build

# Build for specific architecture
just build-x86_64
just build-aarch64
just build-riscv64

# Release builds of everything
just build-release
```

There is no `clean` recipe: `target/` also holds the cross-built sysroot and rootfs trees, which
`cargo clean` would delete.

**Note**: The x86_64 build requires special handling with kernel code model to avoid relocation errors.

### 2. Running in QEMU

```bash
# x86_64 in a QEMU window (builds first)
just run

# x86_64 with the persistent BlockFS root
just run-blockfs

# AArch64 or RISC-V on the serial console
just run-aarch64
just run-riscv64

# Debug with GDB (builds, starts QEMU paused, attaches GDB)
just debug            # x86_64; also debug-aarch64, debug-riscv64
```

For other memory sizes or CPU counts, use the QEMU command lines in the README's Quick Start
with `-m` and `-smp` changed.

### 3. Development Commands

```bash
# Run the host unit tests
just test

# Run specific host test
cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu test_name

# Check code formatting
just fmt-check

# Format code
just fmt

# Run clippy on every target
just clippy

# Build the mdBook guide
just doc

# The full pre-commit gate (fmt, clippy, tests, builds, QEMU boots, runtime suites)
just gate
```

## Code Style Guidelines

### Rust Style

We follow the standard Rust style guide with some additions:

#### Naming Conventions

- **Types**: `PascalCase`
- **Functions/Methods**: `snake_case`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Modules**: `snake_case`

#### Documentation

Every public item must have documentation:

```rust
/// Frame allocator using a hybrid buddy/bitmap approach.
/// 
/// This allocator combines the benefits of buddy allocation for
/// large contiguous allocations with bitmap allocation for
/// single-page allocations.
/// 
/// # Example
/// 
/// ```
/// let mut allocator = FrameAllocator::new();
/// let frame = allocator.allocate()?;
/// allocator.deallocate(frame);
/// ```
pub struct FrameAllocator {
    // ...
}
```

#### Error Handling

Use `Result<T, KernelError>` for fallible operations. The kernel is `no_std` and has no
`thiserror`; errors are plain enums (`kernel/src/error.rs`), and `#[must_use]` on the result
catches ignored errors:

```rust
pub fn allocate_page() -> Result<FrameNumber, KernelError> {
    FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .map_err(|_| KernelError::OutOfMemory { requested: 4096, available: 0 })
}
```

#### Unsafe Code

Minimize unsafe code and document safety requirements:

```rust
/// Dereferences a raw pointer.
/// 
/// # Safety
/// 
/// - `ptr` must be valid for reads of `size` bytes
/// - `ptr` must be properly aligned
/// - The memory must not be mutated while this function executes
unsafe fn read_raw(ptr: *const u8, size: usize) -> Vec<u8> {
    // SAFETY: the caller guarantees `ptr` is valid and aligned for `size`
    // bytes and not mutated meanwhile (the contract above).
    unsafe { core::slice::from_raw_parts(ptr, size) }.to_vec()
}
```

Every `unsafe` block needs a `// SAFETY:` comment (uppercase); clippy's
`undocumented_unsafe_blocks` enforces it.

### Git Workflow

#### Branch Naming

- `feature/description` - New features
- `fix/issue-number-description` - Bug fixes
- `refactor/description` - Code refactoring
- `docs/description` - Documentation updates
- `test/description` - Test additions/improvements

#### Commit Messages

Follow conventional commits:

```
type(scope): subject

body

footer
```

Types:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation
- `refactor`: Code refactoring
- `test`: Test changes
- `perf`: Performance improvement
- `chore`: Maintenance tasks

Example:
```
feat(mm): implement huge page support

Add support for 2MB and 1GB huge pages in the memory manager.
This improves TLB efficiency for large allocations.

Closes #123
```

## Testing Guidelines

### Unit Tests

Place unit tests in the same file as the code:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_allocate_single_frame() {
        let mut allocator = FrameAllocator::new_test();
        let frame = allocator.allocate().unwrap();
        assert_eq!(frame.size(), PAGE_SIZE);
    }
}
```

### Integration Tests

Behaviour across subsystems is tested inside the running kernel and in user space rather than
with a `tests/` crate:

- In-kernel boot tests (`kernel/src/test_framework.rs` and the boot test list in
  `kernel/src/bootstrap.rs`) run at every boot; `scripts/boot-test.sh <arch>` checks them.
- User-space runtime suites in `userland/tests/` (`audit_runtime_test.c`, `musl_runtime_test.c`,
  `busybox_test.sh`) run on the BlockFS root; `scripts/run-rootfs-tests.sh` runs them.

## Debugging

### Kernel Debugging with GDB

The debug scripts build the kernel, start QEMU paused with a GDB server and attach GDB with the
right symbols and boot breakpoints; see [GDB-DEBUGGING.md](GDB-DEBUGGING.md).

```bash
just debug-x86_64      # or debug-aarch64, debug-riscv64
```

GDB commands:

```gdb
# Set breakpoint (on x86_64 use hbreak until the kernel is loaded)
(gdb) break veridian_kernel::bootstrap::kernel_init
(gdb) hbreak veridian_kernel::bootstrap::kernel_init   # x86_64
(gdb) break kernel/src/mm/frame_allocator.rs:42

# Continue execution
(gdb) continue

# Backtrace
(gdb) bt

# Examine memory
(gdb) x/10x $sp
```

### Logging

The kernel has no log framework: it writes to the serial console with `println!`, which the
QEMU command lines send to the terminal (`-serial stdio`). Serial tracing of every page fault and
`open()` is a build feature:

```bash
FEATURES=trace ./build-kernel.sh x86_64 dev
```

### Performance Profiling

The kernel measures itself: the shell's `perf` command runs the in-kernel micro-benchmarks
(system call, frame allocation, capability lookup, scheduler and others) and prints timings from
the calibrated clock source. Use a release kernel for figures (`./build-kernel.sh x86_64
release`); dev kernels overstate anything that clears or copies memory (N-281). Method and
results: [PERFORMANCE-REPORT.md](PERFORMANCE-REPORT.md).

## Common Tasks

### Adding a New System Call

1. Add the call to `abi/syscalls.map`: the Linux system call it implements (its number comes from
   the vendored Linux tables), or `private N` for a VeridianOS call with no Linux equivalent.
2. Regenerate the numbers: `python3 tools/compat/gen_syscalls.py`. This writes
   `kernel/src/syscall/numbers.rs`, `userland/libc/include/veridian/sysno.h` and
   `userland/abi/syscall_numbers.rs`; never edit them by hand (`gen_syscalls.py --check` fails CI
   when they are stale).
3. Define the capability requirements in `kernel/src/cap/` if the call needs any.
4. Add the handler in `kernel/src/syscall/` and dispatch the new `Syscall` variant to it.
5. Add the user-space wrapper in `userland/libc/src/syscall.c` if the native libc needs one.
6. Add tests: host unit tests for the handler's logic and, where it matters, a runtime test in
   `userland/tests/`.

### Adding a New Driver

Drivers live in the kernel today (`kernel/src/drivers/`; moving them to user space is critique C6):

1. Add a module under `kernel/src/drivers/` and declare it in `kernel/src/drivers/mod.rs`.
2. Implement the `Driver` trait (`kernel/src/services/driver_framework.rs`).
3. Register and initialize it during boot (see how existing drivers such as `e1000` or
   `virtio_net` are wired in).
4. Gate architecture-specific code with `#[cfg(target_arch = ...)]`, and bare-metal-only code with
   `#[cfg(all(target_arch = "x86_64", target_os = "none"))]` so host tests still build.

### Memory Debugging

`/proc/meminfo` and the shell's `perf` command show the frame allocator's and heap's state. The
`frame_alloc_1`/`frame_alloc_global` benchmarks and the host unit tests in `kernel/src/mm/` cover
the allocators.

## Contributing

### Before Submitting

1. **Run the full gate**: `just gate` (`scripts/dev/gate.sh --boot --rootfs`: formatting, clippy
   on every target, host tests, builds, QEMU boots and the runtime suites); it must pass on the
   tree you submit
2. **Add tests for new features**
3. **Update the documentation** with the change (`just doc` builds the mdBook)
4. **Update CHANGELOG.md**

### Pull Request Process

1. Fork the repository
2. Create feature branch
3. Make changes
4. Push to your fork
5. Create pull request
6. Wait for review
7. Address feedback
8. Merge after approval

### Code Review Checklist

- [ ] Code follows style guidelines
- [ ] Tests pass
- [ ] New tests added for new features
- [ ] Documentation updated
- [ ] No new compiler warnings
- [ ] Performance impact considered
- [ ] Security implications reviewed
- [ ] Breaking changes documented

## Resources

### Documentation

- [Rust Book](https://doc.rust-lang.org/book/)
- [Rust Embedded Book](https://docs.rust-embedded.org/book/)
- [OS Dev Wiki](https://wiki.osdev.org/)
- [Intel SDM](https://software.intel.com/content/www/us/en/develop/articles/intel-sdm.html)
- [ARM Architecture Reference](https://developer.arm.com/documentation/)
- [RISC-V Specifications](https://riscv.org/technical/specifications/)

### Community

- Discord: [VeridianOS Discord](https://discord.gg/WGcgrnuVHt)
- IRC: #veridian-os on irc.libera.chat
- GitHub Issues: <https://github.com/doublegate/VeridianOS/issues>

### Troubleshooting

See [TROUBLESHOOTING.md](TROUBLESHOOTING.md) for common issues and solutions.

## License

VeridianOS is dual-licensed under MIT and Apache 2.0. See LICENSE-MIT and LICENSE-APACHE for details.