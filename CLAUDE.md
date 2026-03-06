# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Rule #1: NEVER chain pkill with other commands

**CRITICAL**: `pkill` MUST always be run as its own separate, standalone Bash command. NEVER combine with `&&`, `;`, `||`. When `pkill` finds no matching process it returns exit code 1, which prevents chained commands from executing.

```bash
# CORRECT: Two separate Bash calls
pkill -9 -f qemu-system    # Call 1 (may return exit code 1, that's OK)
sleep 2

qemu-system-x86_64 ...     # Call 2 (separate invocation)

# WRONG: Chained together
pkill -9 -f qemu-system; sleep 2; qemu-system-x86_64 ...
```

## VeridianOS Overview

Next-generation microkernel OS in Rust. Capability-based security, user-space drivers, multi-arch (x86_64, AArch64, RISC-V). All phases (0-8) complete, v0.16.3. See CLAUDE.local.md for current state.

## Essential Commands

### Building

```bash
# Recommended: build script
./build-kernel.sh all dev          # All architectures, dev mode
./build-kernel.sh x86_64 release   # Specific arch, release mode

# Manual
cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc
cargo build --target aarch64-unknown-none -p veridian-kernel
cargo build --target riscv64gc-unknown-none-elf -p veridian-kernel
```

**Notes**: x86_64 requires custom target JSON with kernel code model (R_X86_64_32S relocation fix). Kernel linked at 0xFFFFFFFF80100000.

### Running in QEMU (VERIFIED -- QEMU 10.2)

#### x86_64 (UEFI boot -- requires OVMF + disk image)

x86_64 uses UEFI boot via bootloader 0.11+. **CANNOT** use `-kernel` flag directly.

```bash
# Build (creates UEFI disk image automatically)
./build-kernel.sh x86_64 dev

# Run (serial only, ALWAYS use -enable-kvm)
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -serial stdio -display none -m 256M

# With framebuffer (remove -display none for 1280x800 BGR UEFI GOP)
# With GDB: add -s -S (server :1234, start paused)
# With debug exit: add -device isa-debug-exit,iobase=0xf4,iosize=0x04
# With BlockFS rootfs (needs 2048M RAM):
#   add -drive file=target/rootfs-blockfs.img,if=none,id=vd0,format=raw -device virtio-blk-pci,drive=vd0
```

#### AArch64 (direct kernel boot)

```bash
./build-kernel.sh aarch64 dev
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio -display none
# Add -device ramfb for graphical display | Add -s -S for GDB
```

#### RISC-V 64 (OpenSBI + kernel)

```bash
./build-kernel.sh riscv64 dev
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none
# Add -device ramfb for graphical display | Add -s -S for GDB
```

#### QEMU Quick Reference

| Arch | Boot | Firmware | Image | KVM |
|------|------|----------|-------|-----|
| x86_64 | UEFI disk | OVMF.4m.fd `-drive if=pflash` | `target/x86_64-veridian/debug/veridian-uefi.img` | `-enable-kvm` (REQUIRED) |
| AArch64 | Direct `-kernel` | None | `target/aarch64-unknown-none/debug/veridian-kernel` | N/A (TCG) |
| RISC-V | `-kernel` + `-bios default` | OpenSBI | `target/riscv64gc-unknown-none-elf/debug/veridian-kernel` | N/A (TCG) |

**Expected**: All 3 archs boot Stage 6 BOOTOK, 29/29 tests. x86_64 shows Ring 3 entry.

#### QEMU 10.2 Pitfalls
- **DO NOT** use `timeout` -- causes "drive exists" errors. Use background+kill: `cmd </dev/null > log 2>&1 &; PID=$!; sleep N; kill $PID`
- **DO NOT** use `-kernel` for x86_64 -- fails with "PVH ELF Note" error
- **DO NOT** use `-bios` instead of `-drive if=pflash` -- different semantics
- **DO NOT** use `-drive` without explicit ID -- conflicts with pflash
- **DO NOT** use `-cdrom` alongside `-drive` on same bus
- **DO NOT** use `cargo run` for x86_64 -- wrong runner
- **ALWAYS** `pkill -9 -f qemu-system; sleep 3` before re-running
- **ALWAYS** use `-enable-kvm` for x86_64 (TCG is ~100x slower)

**PS/2 keyboard**: Polling (ports 0x64/0x60). APIC replaces PIC so IRQ-based keyboard doesn't work. Input from both serial and keyboard.

### Testing

```bash
# Format and lint (always run these)
cargo fmt --all
cargo clippy --target x86_64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target aarch64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target riscv64gc-unknown-none-elf -p veridian-kernel -- -D warnings

# Host-target tests (3,993 passing)
cargo test

# NOTE: Automated kernel tests blocked by Rust toolchain lang items limitation
# Manual kernel testing via QEMU commands above
```

### Development Tools

```bash
rustup toolchain install nightly-2025-01-15
rustup component add rust-src llvm-tools-preview
cargo install bootimage cargo-xbuild cargo-watch cargo-expand cargo-audit cargo-nextest
```

## Architecture

### Microkernel Design
- **Core**: Memory management, scheduling, IPC, hardware abstraction
- **User-space drivers**: Capability-controlled MMIO, interrupt forwarding, IOMMU DMA
- **Zero-copy IPC**: Shared memory mapping, <1us fast path
- **Security**: 64-bit capability tokens, post-quantum ready (ML-KEM, ML-DSA)

### Memory Layout (x86_64)
```
User:   0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF (128 TB)
Kernel: 0xFFFF_8000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF (128 TB)
  Physical mapping: 0xFFFF_8000_0000_0000 | Heap: 0xFFFF_C000_0000_0000
  Stacks: 0xFFFF_E000_0000_0000 | MMIO: 0xFFFF_F000_0000_0000
```

### Project Structure
```
kernel/src/{arch/, mm/, sched/, cap/, ipc/, syscall/, process/, perf/, desktop/, browser/}
drivers/          # User-space driver processes
services/         # System services (VFS, network, CRI/CNI/CSI)
userland/         # User applications and libraries
tools/            # Build tools and utilities
debug/            # Debug logs and scripts (gitignored)
verification/     # Kani proofs + TLA+ specs
```

## Development Patterns

### Build System
- Standard bare metal targets (x86_64-unknown-none, aarch64-unknown-none, riscv64gc-unknown-none-elf)
- x86_64 uses `targets/x86_64-veridian.json` (kernel code model) -- CI must use this, not `x86_64-unknown-none`
- `-Zbuild-std` handled by .cargo/config.toml; Cargo.lock committed
- Feature flags: `alloc` for heap-dependent code, `#[cfg(feature = "alloc")]`

### Rust 2024 GlobalState Pattern (CURRENT)
```rust
use crate::sync::once_lock::GlobalState;
static MANAGER: GlobalState<Manager> = GlobalState::new();

pub fn init() -> Result<(), Error> {
    MANAGER.init(Manager::new()).map_err(|_| Error::AlreadyInitialized)?;
    Ok(())
}
pub fn with_manager<R, F: FnOnce(&Manager) -> R>(f: F) -> Option<R> { MANAGER.with(f) }
// For mutation: GlobalState<RwLock<Manager>> with .with(|lock| { let mut m = lock.write(); f(&mut m) })
```
120+ static mut eliminated. 7 justified remain (early boot, per-CPU, heap) with SAFETY docs:
`PER_CPU_DATA`, `READY_QUEUE_STATIC`, `HEAP_MEMORY`, `BOOT_INFO`, `EARLY_SERIAL`, `KERNEL_STACK`/`STACK`

### CI/CD Configuration
- GitHub Actions: job consolidation, cargo caching, RUSTFLAGS="-D warnings", rustsec audit
- Cancel-in-progress to prevent duplicate runs
- AArch64: prefix unused vars with underscore (println! is no-op)
- **cfg gate rule**: Use `#[cfg(all(target_arch = "x86_64", target_os = "none"))]` for bare-metal-only functions (CI host shares `target_arch` but has `target_os = "linux"`)
- `vec!` macro needs `use alloc::vec;` in test modules for host-target coverage
- No floating point in kernel -- integer/fixed-point only

### Clippy Fix Patterns

| Pattern | Fix |
|---------|-----|
| new_without_default | Add Default impl |
| manual_flatten | Use iter().flatten() |
| Unused vars on non-x86_64 | `#[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]` |
| Empty loop | Replace `loop {}` with `panic!("message")` |

### IPC Patterns
SmallMessage (<=64 bytes) fast path register-based <1us | LargeMessage zero-copy SharedRegion | 64-bit capability tokens with generation counter | Global O(1) registry | Token bucket rate limiting | NUMA-aware | ProcessId = u64

### Architecture-Specific
- **x86_64**: Bootloader crate, UEFI GOP 1280x800, GDT/IDT, CMOS RTC
- **AArch64**: PL011 UART 0x09000000, stack 0x80000. Iterator-based code hangs -- use `safe_iter.rs` / `aarch64_for!` macro. print! must use DirectUartWriter (LLVM loop bug)
- **RISC-V**: OpenSBI, UART 0x10000000. Frame allocator memory start must be AFTER kernel end

### Performance Targets (All Achieved)
IPC <1us | Context switch <10us | Memory alloc <1us | Capability lookup O(1) | 1000+ processes | Kernel ~15K LOC

## Common Tasks

### Adding a System Call
1. Define capability requirements in `kernel/src/cap/`
2. Add handler in `kernel/src/syscall/`
3. Create user-space wrapper in `userland/libs/libveridian/`
4. Add tests

### Debugging Kernel Panics
- QEMU `-s -S` for GDB (server :1234, start paused)
- `gdb-multiarch` for cross-arch; scripts in `scripts/gdb/`
- `debug/kernel-debug.sh x86_64 60` | `debug/gdb-kernel.sh`
- Docs: `docs/GDB-DEBUGGING.md`

### Key Technical Patterns
| Pattern | Details |
|---------|---------|
| R_X86_64_32S relocation | Kernel must be in top 2GB |
| PIC initialization | Must mask interrupts during init |
| Static heap | Use static arrays, not arbitrary addresses |
| OnceLock soundness | set() error path must extract value before dropping Box |
| #[must_use] on errors | Catches ignored KernelError at compile time |
| Memory barriers | `arch/barriers.rs`: memory_fence(), data_sync_barrier() |

## Key Files

| Path | Purpose |
|------|---------|
| `kernel/src/arch/` | Architecture-specific (aarch64/safe_iter.rs, x86_64/rtc.rs) |
| `kernel/src/mm/` | Memory management (hybrid bitmap+buddy, VMM, VAS) |
| `kernel/src/ipc/` | IPC (fast path <1us, registry) |
| `kernel/src/sched/` | Scheduler (CFS, SMP, NUMA, work-stealing) |
| `kernel/src/cap/` | Capability system (inheritance, revocation, cache) |
| `kernel/src/perf/` | Counters, benchmarks, tracepoints (10 events) |
| `kernel/src/process/sync.rs` | Mutex, Semaphore, PiMutex |
| `kernel/src/test_framework.rs` | No-std test infrastructure |

## Documentation
- **GitHub Pages**: <https://doublegate.github.io/VeridianOS/>
- **mdBook**: `docs/book/src/` | Build: `mdbook build` in `docs/book/`
- **Design docs**: `docs/design/{MEMORY-ALLOCATOR,IPC,SCHEDULER,CAPABILITY-SYSTEM}-DESIGN.md`
- **TODO tracking**: `to-dos/{MASTER,PHASE[0-8],TESTING,ISSUES,RELEASE}_TODO.md`
