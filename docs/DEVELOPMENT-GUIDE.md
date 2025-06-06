# VeridianOS Development Guide

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

# Install specific nightly version
rustup toolchain install nightly-2025-01-15
rustup default nightly-2025-01-15

# Add required components
rustup component add rust-src llvm-tools-preview rustfmt clippy

# Add target architectures
rustup target add x86_64-unknown-none
rustup target add aarch64-unknown-none
rustup target add riscv64gc-unknown-none-elf
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
cargo install cargo-xbuild
cargo install bootimage
cargo install cargo-watch
cargo install cargo-expand
cargo install cargo-tree
cargo install cargo-audit
cargo install cargo-outdated
cargo install cargo-nextest

# Just (command runner)
cargo install just

# Optional: Performance tools
cargo install flamegraph
cargo install cargo-profiling
```

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
    "rust-analyzer.cargo.target": "x86_64-unknown-none",
    "rust-analyzer.cargo.features": ["test-harness"],
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
veridian-os/
├── kernel/                 # Microkernel implementation
│   ├── src/
│   │   ├── arch/          # Architecture-specific code
│   │   ├── mm/            # Memory management
│   │   ├── sched/         # Scheduler
│   │   ├── ipc/           # Inter-process communication
│   │   ├── cap/           # Capability system
│   │   └── main.rs        # Kernel entry point
│   └── Cargo.toml
├── drivers/               # User-space drivers
│   ├── block/            # Block device drivers
│   ├── net/              # Network drivers
│   ├── gpu/              # Graphics drivers
│   └── common/           # Shared driver code
├── services/             # System services
│   ├── init/             # Init system
│   ├── vfs/              # Virtual file system
│   ├── network/          # Network stack
│   └── display/          # Display server
├── libs/                 # Libraries
│   ├── veridian-abi/     # System call interface
│   ├── veridian-std/     # Standard library
│   └── common/           # Shared utilities
├── userland/             # User applications
├── tools/                # Development tools
├── tests/                # Integration tests
├── docs/                 # Documentation
└── targets/              # Custom target specifications
```

## Development Workflow

### 1. Building the Kernel

```bash
# Build for default target (x86_64)
just build

# Build for specific architecture
just build-x86_64
just build-aarch64
just build-riscv64

# Build everything
just build-all

# Clean build
just clean
just build
```

### 2. Running in QEMU

```bash
# Run with default settings
just run

# Run with debugging enabled
just debug

# Run with specific memory/CPU configuration
just run-custom MEMORY=1024 CPUS=4

# Run and attach GDB
just gdb
```

### 3. Development Commands

```bash
# Watch for changes and rebuild
cargo watch -x build

# Run tests
just test

# Run specific test
cargo test test_name

# Check code formatting
just fmt-check

# Format code
just fmt

# Run clippy lints
just clippy

# Generate documentation
just doc
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

Use `Result<T, Error>` for fallible operations:

```rust
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("out of memory")]
    OutOfMemory,
    
    #[error("invalid address {0:#x}")]
    InvalidAddress(usize),
    
    #[error("alignment error: address {addr:#x} not aligned to {align}")]
    AlignmentError { addr: usize, align: usize },
}

pub fn allocate_page() -> Result<Page, MemoryError> {
    // ...
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
    // Safety: Caller ensures preconditions are met
    std::slice::from_raw_parts(ptr, size).to_vec()
}
```

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

Create integration tests in `tests/`:

```rust
// tests/memory_integration.rs
use veridian_test::*;

#[test]
fn test_cross_process_memory_sharing() {
    let env = TestEnvironment::new();
    // Test implementation
}
```

### Running Tests

```bash
# Run all tests
just test

# Run tests for specific package
cargo test -p veridian-kernel

# Run with verbose output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run tests with coverage
cargo tarpaulin
```

## Debugging

### Kernel Debugging with GDB

1. Build with debug symbols:
```bash
just build-debug
```

2. Start QEMU with GDB server:
```bash
just gdb-server
```

3. In another terminal, connect GDB:
```bash
just gdb-connect
```

4. GDB commands:
```gdb
# Set breakpoint
(gdb) break kernel_main
(gdb) break kernel/src/mm/physical.rs:42

# Continue execution
(gdb) continue

# Print variable
(gdb) print allocator

# Examine memory
(gdb) x/10x 0xffff800000000000

# Backtrace
(gdb) bt
```

### Logging

Use the logging framework:

```rust
use log::{debug, info, warn, error};

pub fn init_memory() {
    info!("Initializing memory subsystem");
    
    debug!("Available memory: {} MB", available_mb);
    
    if available_mb < 64 {
        warn!("Low memory detected");
    }
    
    if let Err(e) = setup_page_tables() {
        error!("Failed to setup page tables: {}", e);
    }
}
```

Configure log level:
```bash
RUST_LOG=debug just run
```

### Performance Profiling

#### Using Flamegraph

```bash
# Generate flamegraph
cargo flamegraph --bin kernel

# Profile specific scenario
just profile-scenario boot
```

#### Using perf

```bash
# Record performance data
perf record -g target/release/kernel

# Generate report
perf report
```

## Common Tasks

### Adding a New System Call

1. Define the system call number in `kernel/src/syscall/numbers.rs`:
```rust
pub const SYS_NEW_CALL: usize = 100;
```

2. Add handler in `kernel/src/syscall/mod.rs`:
```rust
match nr {
    SYS_NEW_CALL => handle_new_call(arg1, arg2),
    // ...
}
```

3. Implement handler:
```rust
fn handle_new_call(arg1: usize, arg2: usize) -> Result<usize, SyscallError> {
    // Implementation
}
```

4. Add user-space wrapper in `libs/veridian-abi/src/syscalls.rs`:
```rust
pub fn new_call(arg1: usize, arg2: usize) -> Result<usize, Error> {
    unsafe { syscall2(SYS_NEW_CALL, arg1, arg2) }
}
```

### Adding a New Driver

1. Create driver crate:
```bash
cargo new --lib drivers/mydriver
```

2. Implement driver trait:
```rust
use veridian_driver::Driver;

pub struct MyDriver {
    // Driver state
}

impl Driver for MyDriver {
    fn init(&mut self, device: DeviceInfo) -> Result<(), Error> {
        // Initialize hardware
    }
    
    fn handle_interrupt(&mut self, vector: u8) {
        // Handle device interrupt
    }
}
```

3. Add to workspace in root `Cargo.toml`:
```toml
[workspace]
members = [
    # ...
    "drivers/mydriver",
]
```

### Memory Debugging

Enable memory debugging features:

```toml
[features]
memory-debug = ["heap-tracking", "allocation-stats"]
```

Use memory debugging tools:

```rust
#[cfg(feature = "memory-debug")]
fn debug_memory_state() {
    let stats = ALLOCATOR.stats();
    debug!("Allocated: {} bytes", stats.allocated);
    debug!("Free: {} bytes", stats.free);
    debug!("Fragmentation: {:.2}%", stats.fragmentation * 100.0);
}
```

## Contributing

### Before Submitting

1. **Run tests**: `just test`
2. **Check formatting**: `just fmt-check`
3. **Run clippy**: `just clippy`
4. **Update documentation**: `just doc`
5. **Add tests for new features**
6. **Update CHANGELOG.md**

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

- Discord: [VeridianOS Discord](#)
- Forums: [VeridianOS Forums](#)
- IRC: #veridian-os on irc.libera.chat
- Mailing List: dev@veridian-os.org

### Troubleshooting

See [TROUBLESHOOTING.md](TROUBLESHOOTING.md) for common issues and solutions.

## License

VeridianOS is dual-licensed under MIT and Apache 2.0. See LICENSE-MIT and LICENSE-APACHE for details.