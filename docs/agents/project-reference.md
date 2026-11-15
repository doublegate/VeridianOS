# Agent Project Reference — VeridianOS

Reference material moved out of the root `AGENTS.md`, and brought up to date on the way, (also `CLAUDE.md` / `GEMINI.md`) to keep that file within agent rule-file size limits. Read the section you need.

## Architecture

### Microkernel Design
- **Core**: Memory management, scheduling, IPC, hardware abstraction
- **User-space drivers (design goal, C6)**: Capability-controlled MMIO, interrupt forwarding, IOMMU DMA. Today drivers run in ring 0 inside the kernel (docs/KNOWN-LIMITATIONS.md)
- **Zero-copy IPC**: Shared memory mapping; messages queued on endpoints (no register fast path until the v0.28 IPC redesign; IPC latency not measured)
- **Security**: 64-bit capability tokens, post-quantum (ML-DSA-65 verification per FIPS 204; experimental non-standard Kyber, N-57)

### Memory Layout (x86_64)
```
User:   0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF (128 TB)
Kernel: 0xFFFF_8000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF (128 TB)
  Physical-memory map: at the offset the bootloader supplies (mm::PHYS_MEM_OFFSET)
  Heap: physical frames taken at boot, used through that map (mm/heap.rs)
  MMIO: through the same map (mm::vas::map_mmio maps what it lacks, e.g. high BARs)
  Stacks: 0xFFFF_E000_0000_0000 (guarded kernel stacks, mm/kstack.rs)
```

### Project Structure
```
kernel/src/{arch/, mm/, sched/, cap/, ipc/, syscall/, process/, perf/, desktop/, browser/}
kernel/src/drivers/   # Device drivers (in the kernel today; user-space drivers are planned, critique C6)
kernel/src/services/  # System services: shell, VFS glue, CRI/CNI/CSI (also in the kernel)
userland/         # User applications and libraries
  libc/           # C library shims
  qt6/            # Qt 6 QPA plugin and shims
  kf6/            # KDE Frameworks 6 backends
  kwin/           # KWin compositor backend
  plasma/         # Plasma Desktop components
  integration/    # KDE integration tests
tools/            # Build tools and utilities
  cross/          # KDE cross-compilation scripts (15 build scripts)
debug/            # Debug logs and scripts (gitignored)
verification/     # Kani proofs + TLA+ specs
```

## Development Patterns

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

### Clippy Fix Patterns

| Pattern | Fix |
|---------|-----|
| new_without_default | Add Default impl |
| manual_flatten | Use iter().flatten() |
| Unused vars on non-x86_64 | `#[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]` |
| Empty loop | Replace `loop {}` with `panic!("message")` |

### IPC Patterns
SmallMessage (four u64 data words, 32 bytes) by value, queued on the endpoint (receivers sleep on its wait queue; register fast path removed in v0.27) | BufferedMessage copied through a kernel buffer, up to 16 KiB (MAX_BUFFERED_PAYLOAD) | LargeMessage zero-copy SharedRegion | 64-bit capability tokens with generation counter | Global O(1) registry | Token-bucket IPC rate limiter written but not yet called (N-90) | no NUMA support yet | ProcessId = u64

### Performance Targets (goals; see docs/PERFORMANCE-REPORT.md for what is measured)
IPC <1us (not measured) | Context switch <10us (not measured) | Memory alloc <1us (met) | Capability lookup O(1) | 1000+ processes

## Common Tasks

### Adding a System Call
1. Register the call in `abi/syscalls.map`, then run `python3 -I tools/compat/gen_syscalls.py` (which writes `kernel/src/syscall/numbers.rs`, `userland/libc/include/veridian/sysno.h` and `userland/abi/syscall_numbers.rs`; never edit those by hand)
2. Define capability requirements in `kernel/src/cap/`
3. Add handler in `kernel/src/syscall/`
4. Create user-space wrapper in `userland/libc/src/syscall.c`
5. Run `tools/compat/syscall_coverage.py` once the handler is in place (it reads the dispatcher, so run earlier it would count a stub as implemented)
6. Add tests (host unit tests, and a runtime test in `userland/tests/` for the user ABI)

### Debugging Kernel Panics
- QEMU `-s -S` for GDB (server :1234, start paused)
- `gdb-multiarch` for cross-arch; scripts in `scripts/gdb/`
- `scripts/debug-x86_64.sh`, `scripts/debug-aarch64.sh`, `scripts/debug-riscv64.sh`: build the dev kernel, start QEMU paused with a GDB server and attach GDB with `scripts/gdb/<arch>.gdb`
- Docs: `docs/GDB-DEBUGGING.md`

### Key Technical Patterns
| Pattern | Details |
|---------|---------|
| R_X86_64_32S relocation | Kernel must be in top 2GB |
| PIC initialization | Must mask interrupts during init |
| Kernel heap | Physical frames taken from the frame allocator at boot, used through the physical-memory map and sized from usable RAM; never a static array (N-278) |
| OnceLock soundness | set() error path must extract value before dropping Box |
| #[must_use] on errors | Catches ignored KernelError at compile time |
| Memory barriers | `arch/barriers.rs`: memory_fence(), data_sync_barrier() |

## Documentation
- **GitHub Pages**: <https://doublegate.github.io/VeridianOS/>
- **mdBook**: `docs/book/src/` | Build: `mdbook build` in `docs/book/`
- **Design docs**: `docs/design/{MEMORY-ALLOCATOR,IPC,SCHEDULER,CAPABILITY-SYSTEM}-DESIGN.md`
- **TODO tracking**: `to-dos/{MASTER,TESTING,ISSUES,RELEASE}_TODO.md` (completed phase TODOs in `to-dos/archive/`)
