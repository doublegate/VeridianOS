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
# Build for x86_64
cargo build --target ../targets/x86_64-veridian.json -Zbuild-std=core,compiler_builtins,alloc

# Build for all architectures
just build-all
```

## Architecture Support

- **x86_64**: Full support with VGA output, GDT/IDT, paging
- **AArch64**: Full support with PL011 UART, MMU configuration
- **RISC-V**: Full support with SBI interface, UART output

## Key Components

### Memory Management (`mm/`)
- Physical frame allocator (hybrid bitmap/buddy)
- Virtual memory management
- Page table manipulation

### Scheduler (`sched/`)
- Round-robin scheduler (Phase 1)
- Priority-based scheduling (Phase 2)
- Real-time support (Phase 3)

### IPC (`ipc/`)
- Synchronous message passing
- Asynchronous channels
- Zero-copy transfers

### Capability System (`cap/`)
- Unforgeable capability tokens
- O(1) capability lookups
- Hardware security integration

## Testing

```bash
# Run unit tests
cargo test

# Run benchmarks
cargo bench

# Run in QEMU
just run-x86_64
```

## Debugging

```bash
# Debug with GDB
just debug-x86_64

# View assembly
just objdump-x86_64
```

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