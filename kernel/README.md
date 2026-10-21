# VeridianOS Kernel

This directory contains the core microkernel implementation for VeridianOS.

## Structure

```
kernel/
├── src/
│   ├── arch/          # Architecture-specific code
│   │   ├── x86_64/    # x86_64 implementation
│   │   ├── aarch64/   # AArch64 implementation
│   │   └── riscv64/   # RISC-V 64-bit implementation
│   ├── mm/            # Memory management
│   ├── sched/         # Scheduler
│   ├── ipc/           # Inter-process communication
│   ├── cap/           # Capability system
│   └── main.rs        # Kernel entry point
├── tests/             # Integration tests
├── benches/           # Performance benchmarks
└── Cargo.toml         # Kernel crate configuration
```

## Building

The kernel requires a custom target specification and must be built with the nightly Rust compiler:

```bash
# From the repository root: build every architecture (x86_64 also gets its UEFI disk image)
./build-kernel.sh all dev

# Or one architecture by hand
BS="-Zbuild-std=core,compiler_builtins,alloc"   # as build-kernel.sh passes it
cargo build --target targets/x86_64-veridian.json -p veridian-kernel $BS
cargo build --target aarch64-unknown-none-softfloat -p veridian-kernel $BS
cargo build --target riscv64imac-unknown-none-elf -p veridian-kernel $BS
```

## Architecture Support

All three boot under QEMU on one and four CPUs and run user programs; none has run on real
hardware yet ([`docs/KNOWN-LIMITATIONS.md`](../docs/KNOWN-LIMITATIONS.md)).

- **x86_64**: UEFI boot, GDT/IDT, local APIC, Ring 3 user programs with the native libc
- **AArch64**: PL011 UART, MMU and caches on with the kernel in the upper half, EL0 programs with musl
- **RISC-V**: OpenSBI, Sv48 or Sv39 paging, U-mode programs with musl

## Key Components

### Memory Management (`mm/`)
- Physical frame allocator (hybrid bitmap/buddy)
- Virtual memory management
- Page table manipulation

### Scheduler (`sched/`)
- Every thread a dispatched task on its own guarded kernel stack (ADR 0006)
- EEVDF fair class with SCHED_FIFO/RR and SCHED_DEADLINE above it (ADR 0007)
- Per-CPU run queues with wake placement and load balancing (ADR 0012)

### IPC (`ipc/`)
- Endpoints with queued messages: small messages by value, larger ones through kernel buffers or
  shared regions
- Capability-checked; user programs reach endpoint send and receive through private system call
  numbers 1024-1031 (ADR 0009)
- The IPC redesign (reply capabilities, per-endpoint wait queues) is v0.28 work

### Capability System (`cap/`)
- Unforgeable 64-bit capability tokens with generation counters
- A per-process hash table with O(1) average lookup
- Hierarchical delegation and cascading revocation (not yet along every grant path, N-89)

## Testing

```bash
# Run unit tests (from the repository root)
cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu

# Build, then boot an architecture in QEMU and check the in-kernel tests
./build-kernel.sh all dev
scripts/boot-test.sh x86_64      # or aarch64, riscv64
```

Benchmarks run inside the kernel: the shell's `perf` command (method and results in
[`docs/PERFORMANCE-REPORT.md`](../docs/PERFORMANCE-REPORT.md)). The QEMU command lines for each
architecture are in the top-level [README](../README.md#quick-start).

## Debugging

Add `-s -S` to a QEMU command line to wait for GDB on port 1234, then attach with the scripts in
`scripts/gdb/` (`gdb -x scripts/gdb/x86_64.gdb`; `gdb-multiarch` for AArch64 and RISC-V). Full
guide: [`docs/GDB-DEBUGGING.md`](../docs/GDB-DEBUGGING.md).

## Contributing

See the main [Contributing Guide](../CONTRIBUTING.md) for details on:
- Code style requirements
- Testing requirements
- Review process

## Safety

This is kernel code - most functions are `unsafe`. Key safety requirements:

1. No heap allocation in interrupt handlers
2. All assembly must be documented
3. Page table modifications require proper TLB flushing
4. Capability checks before resource access

## Documentation

Generate documentation with:

```bash
cargo doc --no-deps --open
```

For architecture guides, see the [developer book](../docs/book/).