# VeridianOS Rust std Platform Layer

This crate provides the platform-specific implementation layer that bridges
Rust user-space code to VeridianOS kernel syscalls. It is the equivalent of
`std::sys::unix` but for VeridianOS.

## Architecture

```
User Rust Code
      |
      v
  Rust std (core, alloc)
      |
      v
  veridian-std  <-- THIS CRATE
      |
      v
  VeridianOS kernel (via syscall instruction)
```

## Modules

| Module      | Description                                        | Syscalls Used                  |
|-------------|----------------------------------------------------|--------------------------------|
| `fs`        | File I/O (open, read, write, close, stat, etc.)    | 50-66, 150-157                 |
| `io`        | stdin/stdout/stderr via fd 0/1/2                   | 52, 53 (read/write)            |
| `process`   | Process lifecycle (exit, fork, exec, wait, getpid)  | 11-16, 110-113                 |
| `thread`    | Thread creation (clone) and futex sync              | 41, 43, 46, 201-202           |
| `time`      | Clock and sleep operations                          | 100, 160-163                   |
| `alloc`     | Memory allocation via mmap/munmap/brk               | 20-23                          |
| `os`        | Environment variables, identity, kernel info        | 80, 170-173                    |
| `net`       | Network operations (stub -- not yet in kernel)      | N/A                            |

## Building

Requires Rust nightly with `rust-src` component:

```bash
rustup component add rust-src --toolchain nightly

# Build for x86_64
./build.sh x86_64 dev

# Build for AArch64
./build.sh aarch64 dev

# Build for RISC-V 64
./build.sh riscv64 dev
```

## Usage

This is a `no_std` crate. Add it as a dependency and use the syscall wrappers:

```rust
#![no_std]
#![no_main]

use veridian_std::sys::veridian::{fs, io, process};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Write to stdout
    let _ = io::println("Hello from VeridianOS!");

    // Open a file
    let path = b"/etc/hostname\0";
    if let Ok(fd) = fs::open(path.as_ptr(), fs::O_RDONLY, 0) {
        let mut buf = [0u8; 256];
        if let Ok(n) = fs::read(fd, buf.as_mut_ptr(), buf.len()) {
            let _ = fs::write(io::STDOUT_FD, buf.as_ptr(), n);
        }
        let _ = fs::close(fd);
    }

    // Exit
    process::exit(0);
}
```

## Syscall Convention

All syscall numbers match `kernel/src/syscall/mod.rs` and
`toolchain/sysroot/include/veridian/syscall.h`.

Architecture-specific calling conventions:
- **x86_64**: `syscall` instruction, nr in `rax`, args in `rdi/rsi/rdx/r10/r8/r9`
- **aarch64**: `svc #0`, nr in `x8`, args in `x0-x5`
- **riscv64**: `ecall`, nr in `a7`, args in `a0-a5`
