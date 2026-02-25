# VeridianOS Project Context

VeridianOS is a research microkernel operating system written in Rust, strictly prioritizing **correctness, isolation, and explicit architectural invariants**. It demonstrates how capability-based security, strong isolation boundaries, and disciplined `unsafe` code usage can create a resilient system.

## 1. Current Status & Roadmap (v0.5.0)

-   **Phase 4 (Package Ecosystem)**: **COMPLETE**. Includes package manager, SDK, and repository infra.
-   **Phase 4.5 (Interactive Shell - vsh)**: **COMPLETE**. Tri-architecture shell prompt (`root@veridian:/#`) is operational.
-   **Phase 5 (Performance Optimization)**: **IN PROGRESS (~10%)**. Focus is on kernel memory management, scheduler tuning, and IPC optimization.
-   **Phase 6 (Advanced Features)**: **PLANNED (~5%)**. GUI, advanced drivers, virtualization.

**Latest Release (v0.5.0 - Feb 21, 2026):**
-   **Self-hosting Toolchain Complete**: Tiers 0-7 all complete -- Rust targets, std port, native GCC 14.2, make/ninja, vpkg.
-   **User-Space Foundation**: Fixed exec return path (iretq), fork verification, fd 0/1/2 auto-open, console blocking read, shell bootstrap.
-   **Code Quality**: dead_code audit with justification comments, TODO(future) recategorized to phase5/phase6.

## 2. Architectural Invariants (Non-Negotiable)

Adherence to `docs/invariants.md` is mandatory.
1.  **Authority Is Explicit**: No component performs an action without an explicit capability. No ambient authority.
2.  **Isolation Boundaries**: Enforced by design (kernel vs. user-space), not convention.
3.  **Memory Ownership**: Every region has a clear owner; transfer is explicit and kernel-mediated.
4.  **TCB Is Minimal**: Only code that *must* be trusted stays in the kernel. Drivers and services live in user-space.

## 3. Unsafe Code Policy

**Strict Adherence Required (`docs/unsafe-policy.md`):**
-   **Exceptional**: Never use `unsafe` for convenience or premature optimization.
-   **Localized**: Keep unsafe blocks minimal.
-   **Documented**: Every `unsafe` block **MUST** have a `// SAFETY:` comment explaining:
    1.  Which invariant it upholds.
    2.  Why safe Rust is insufficient.
    3.  The specific preconditions being satisfied.

## 4. Development Workflow

### Build & Run
-   **Standard Build**: `./build-kernel.sh all dev` (Builds x86_64, AArch64, RISC-V).
-   **Justfile Shortcuts**:
    -   `just build` (Default x86_64 release)
    -   `just run` (Run x86_64 in QEMU)
-   **QEMU (v10.2+)**:
    -   **x86_64**: Requires UEFI (`OVMF.fd`) and disk image. **ALWAYS use `-enable-kvm`**.
    -   **AArch64**: `qemu-system-aarch64 -M virt -cpu cortex-a72 -kernel ...`
    -   **RISC-V**: `qemu-system-riscv64 -M virt -m 256M -bios default -kernel ...`
    -   **⚠️ Pitfall**: NEVER use `timeout` to wrap QEMU; it causes drive conflicts in v10.2.

### Testing
-   **Tri-Arch Boot**: All 3 architectures must pass 29/29 boot tests (including `fbcon_initialized`).
-   **Linting**: `just fmt-check` and `just clippy` **MUST** pass before submission. Zero warnings allowed.

## 5. Coding Standards & Patterns

### Error Handling
-   **Kernel**: Use specific `KernelError` enums. No string-based errors or `Err("...")`.
-   **Userland**: Use `thiserror` for library errors.
-   **General**: No `unwrap()` or `expect()` in production code. Handle all `Result`s.

### Global State
-   **Avoid `static mut`**. Use the **GlobalState** pattern (wrapped in `OnceLock` or `RwLock`) for Rust 2024 compatibility.
-   **Remaining `static mut`**: Only 7 justified instances remain (early boot, per-CPU, heap).

### Implementation Patterns

#### Performance Optimization (Phase 5)
-   **Syscalls**: CR3 switching is removed; syscalls run with user CR3 to avoid TLB flushes.
-   **Graphics**: Use `fbcon` back-buffer and pixel ring buffers to minimize MMIO writes.
-   **Memory**: Implement per-CPU page lists and NUMA-aware allocations.

#### Hardware Abstraction
-   **AArch64**: Use `DirectUartWriter` (assembly-based) for output to bypass LLVM loop-compilation bugs.
-   **Interrupts**: Use the unified IRQ framework and `PlatformTimer` trait for cross-arch parity.

## 6. Directory Structure

-   `kernel/`: Core microkernel (TCB).
    -   `src/arch/`: Architecture-specific code. Note `safe_iter.rs` for AArch64.
    -   `src/mm/`: Hybrid bitmap+buddy allocator, NUMA-aware paging.
    -   `src/ipc/`: Zero-copy IPC (<1μs latency).
    -   `src/cap/`: Two-level O(1) capability lookup.
-   `drivers/`: User-space drivers (isolated processes).
-   `services/`: System services (VFS, Init, Network).
-   `libs/`: `veridian-abi` (Syscalls), `veridian-std` (Libc).
-   `userland/`: `vsh` (Interactive Shell), `minimal` (Test binary).
