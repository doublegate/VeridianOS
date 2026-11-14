<!-- Managed by Master-Claude. Universal rules come from the imported/inlined core.
     Edit only inside the MC-PROJECT block; mc-sync overwrites everything else. -->
<!-- mc-core: 0.2.0 | mode=import | lang=rust -->
# AGENTS.md — VeridianOS

@~/.claude/master-core/AGENTS.base.md
@~/.claude/master-core/lang/rust.md
@~/.claude/master-core/modules/10-commits-and-versioning.md
@~/.claude/master-core/modules/20-testing-and-accuracy.md
@~/.claude/master-core/modules/30-quality-gates.md
@~/.claude/master-core/modules/40-docs-and-adrs.md
@~/.claude/master-core/modules/50-architecture-patterns.md
@~/.claude/master-core/modules/60-security.md
@~/.claude/master-core/modules/70-release-ceremony.md
@~/.claude/master-core/modules/80-phase-sprint-workflow.md
@~/.claude/master-core/modules/90-multi-language-integration.md
@~/.claude/master-core/modules/91-agent-system-architecture.md
@~/.claude/master-core/modules/95-named-pattern-library.md

<<< MC-PROJECT-START >>>
## Project: VeridianOS

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

Next-generation microkernel OS in Rust. Capability-based security, user-space drivers as the design goal (today they run in ring 0, critique C6; see docs/KNOWN-LIMITATIONS.md), multi-arch (x86_64, AArch64, RISC-V). All phases (0-12) complete, v0.27.0 (v0.28 next). See CLAUDE.local.md for current state and `docs/audit/AUDIT-VERIFICATION-2026-10-05.md` for the open audit remediation checklist.

## Essential Commands

### Building

```bash
# Recommended: build script
./build-kernel.sh all dev          # All architectures, dev mode
./build-kernel.sh x86_64 release   # Specific arch, release mode

# Manual
cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc
cargo build --target aarch64-unknown-none-softfloat -p veridian-kernel
cargo build --target riscv64imac-unknown-none-elf -p veridian-kernel
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
    -kernel target/aarch64-unknown-none-softfloat/debug/veridian-kernel \
    -serial stdio -display none
# Add -device ramfb for graphical display | Add -s -S for GDB
```

#### RISC-V 64 (OpenSBI + kernel)

```bash
./build-kernel.sh riscv64 dev
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64imac-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none
# Add -device ramfb for graphical display | Add -s -S for GDB
```

#### QEMU Quick Reference

| Arch | Boot | Firmware | Image | KVM |
|------|------|----------|-------|-----|
| x86_64 | UEFI disk | OVMF.4m.fd `-drive if=pflash` | `target/x86_64-veridian/debug/veridian-uefi.img` | `-enable-kvm` (REQUIRED) |
| AArch64 | Direct `-kernel` | None | `target/aarch64-unknown-none-softfloat/debug/veridian-kernel` | N/A (TCG) |
| RISC-V | `-kernel` + `-bios default` | OpenSBI | `target/riscv64imac-unknown-none-elf/debug/veridian-kernel` | N/A (TCG) |

**Expected**: All 3 archs boot Stage 6 BOOTOK with every test passed. Default build (one CPU): x86_64 43/43, AArch64 64/64, RISC-V 57/57; `-smp 4`: 47/47, 68/68, 61/61 (AArch64 and RISC-V boot with `-device ramfb`); `scripts/dev/gate.sh --boot --rootfs --smp` runs both, plus the BusyBox and runtime suites on all three, on one and four CPUs (AArch64 and RISC-V roots from `scripts/build-musl-rootfs.sh`). Skips are tests needing more than one CPU. x86_64 shows Ring 3 entry.

#### QEMU 10.2 Pitfalls
- **DO NOT** use `timeout` -- causes "drive exists" errors. Use background+kill: `cmd </dev/null > log 2>&1 &; PID=$!; sleep N; kill $PID`
- **DO NOT** use `-kernel` for x86_64 -- fails with "PVH ELF Note" error
- **DO NOT** use `-bios` instead of `-drive if=pflash` -- different semantics
- **DO NOT** use `-drive` without explicit ID -- conflicts with pflash
- **DO NOT** use `-cdrom` alongside `-drive` on same bus
- **DO NOT** use `cargo run` for x86_64 -- wrong runner
- **ALWAYS** run `pkill -9 -f qemu-system` as its own command (Rule #1), then `sleep 3` as a separate command, before re-running
- **ALWAYS** use `-enable-kvm` for x86_64 (TCG is ~100x slower)

**PS/2 keyboard**: Polling (ports 0x64/0x60). APIC replaces PIC so IRQ-based keyboard doesn't work. Input from both serial and keyboard.

### Testing

```bash
# Format and lint (always run these). x86_64 must use the custom target JSON
# (kernel code model) -- the same target CI builds.
cargo fmt --all
BS="-Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem"
cargo clippy --target targets/x86_64-veridian.json -p veridian-kernel $BS -- -D warnings
cargo clippy --target aarch64-unknown-none-softfloat -p veridian-kernel $BS -- -D warnings
cargo clippy --target riscv64imac-unknown-none-elf -p veridian-kernel $BS -- -D warnings

# Host-target unit tests (4,735 passing). Plain `cargo test` does not work:
# .cargo/config.toml defaults the target to bare metal.
cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu

# In-kernel boot tests: boot each arch in QEMU (commands above) and check for
# "[INIT] Results: N/N passed" (plus "(S skipped)" on AArch64/RISC-V) followed by BOOTOK.
```

### Development Tools

```bash
rustup toolchain install nightly-2025-11-15   # matches rust-toolchain.toml; plain `cargo` fetches it automatically
rustup component add rust-src llvm-tools-preview
cargo install bootimage cargo-xbuild cargo-watch cargo-expand cargo-audit cargo-nextest
```

## Development Patterns

### Build System
- Bare metal targets: `targets/x86_64-veridian.json`, `aarch64-unknown-none-softfloat`, `riscv64imac-unknown-none-elf`. The AArch64 and RISC-V kernels are soft-float (N-178): no kernel code may use the FP/SIMD registers, which belong to user threads; only the FP save/restore code names them (`.arch_extension fp`, `.option arch, +d`)
- x86_64 uses `targets/x86_64-veridian.json` (kernel code model) -- CI must use this, not `x86_64-unknown-none`
- `-Zbuild-std` handled by .cargo/config.toml; Cargo.lock committed
- Feature flags: `alloc` for heap-dependent code, `#[cfg(feature = "alloc")]`

### Rust 2024 GlobalState Pattern (CURRENT)
`GlobalState<T>` from `crate::sync::once_lock`; example code in `docs/agents/project-reference.md`.
120+ static mut eliminated. The justified ones left (early boot, per-CPU) carry SAFETY docs:
`PER_CPU_DATA`, `BOOT_INFO`, `EARLY_SERIAL` (`extern` declarations of boot.S symbols in the
`kmap` modules are not Rust statics). The TSS stacks
became an `UnsafeCell` wrapper (`IstStack`, `arch/x86_64/gdt.rs`) in v0.26.0; new code must not add
`static mut`.

### CI/CD Configuration
- GitHub Actions: job consolidation, cargo caching, RUSTFLAGS="-D warnings", rustsec audit
- Cancel-in-progress to prevent duplicate runs
- AArch64: prefix unused vars with underscore (println! is no-op)
- **cfg gate rule**: Use `#[cfg(all(target_arch = "x86_64", target_os = "none"))]` for bare-metal-only functions (CI host shares `target_arch` but has `target_os = "linux"`)
- `vec!` macro needs `use alloc::vec;` in test modules for host-target coverage
- No floating point in kernel -- integer/fixed-point only

### Architecture-Specific
- **x86_64**: Bootloader crate, UEFI GOP 1280x800, GDT/IDT, CMOS RTC
- **AArch64**: PL011 UART 0x09000000, stack 0x80000. Iterator-based code hangs -- use `safe_iter.rs` / `aarch64_for!` macro. print! must use DirectUartWriter (LLVM loop bug)
- **RISC-V**: OpenSBI, UART 0x10000000. Frame allocator memory start must be AFTER kernel end

## Key Files

| Path | Purpose |
|------|---------|
| `kernel/src/arch/` | Architecture-specific (aarch64/safe_iter.rs, x86_64/rtc.rs) |
| `kernel/src/mm/` | Memory management (hybrid bitmap+buddy, VMM, VAS) |
| `kernel/src/ipc/` | IPC (endpoint queues, registry; capabilities name registered endpoints) |
| `kernel/src/sched/` | Dispatcher (ADR 0006) with EEVDF/RT/deadline policy classes (ADR 0007); one CPU runs tasks until D5 |
| `kernel/src/cap/` | Capability system (inheritance, revocation, cache) |
| `kernel/src/perf/` | Counters, benchmarks, tracepoints (10 events) |
| `kernel/src/test_framework.rs` | No-std test infrastructure |

## Reference Docs (read when relevant; not imported)
- `docs/agents/project-reference.md` -- microkernel design summary, x86_64 memory layout, project directory tree, GlobalState example code, Clippy fix patterns, IPC patterns, performance targets, common tasks (adding a syscall, debugging kernel panics), key technical patterns, documentation links (GitHub Pages, mdBook, design docs, TODO tracking)

<<< MC-PROJECT-END >>>

