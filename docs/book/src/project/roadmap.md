# Roadmap

## All Phases Complete

VeridianOS has completed all 13 development phases, progressing from bare-metal boot to a fully functional microkernel OS with KDE Plasma 6 desktop cross-compiled from source.

### Phase Completion History

| Phase | Description | Version | Date | Key Deliverables |
|-------|-------------|---------|------|------------------|
| **0** | Foundation & Tooling | v0.1.0 | Jun 2025 | Build system, CI/CD, multi-arch boot, GDB |
| **1** | Microkernel Core | v0.2.0 | Jun 2025 | Memory, IPC (<1us), scheduler, capabilities |
| **2** | User Space Foundation | v0.3.2 | Feb 2026 | VFS, ELF loader, drivers, shell, init |
| **3** | Security Hardening | v0.3.2 | Feb 2026 | Crypto, post-quantum, MAC/RBAC, audit |
| **4** | Package Ecosystem | v0.4.0 | Feb 2026 | Package manager, DPLL resolver, SDK |
| **5** | Performance | v0.16.2 | Mar 2026 | 10/10 traces, benchmarks, per-CPU caches |
| **5.5** | Infrastructure Bridge | v0.5.13 | Feb 2026 | ACPI/APIC stubs, hardware abstraction |
| **6** | Advanced Features & GUI | v0.6.4 | Feb 2026 | Wayland compositor, desktop, TCP/IP |
| **6.5** | Rust Compiler + Shell | v0.7.0 | Feb 2026 | std::sys::veridian, LLVM 19, vsh (49 builtins) |
| **7** | Production Readiness | v0.10.0 | Mar 2026 | GPU, multimedia, hypervisor, containers |
| **7.5** | Follow-On Features | v0.16.0 | Mar 2026 | ext4, TLS 1.3, xHCI, WireGuard, DRM KMS |
| **8** | Next-Generation | v0.16.3 | Mar 2026 | Browser engine, enterprise, cloud-native, Kani |
| **9** | KDE Plasma 6 Porting | v0.22.0 | Mar 2026 | Qt 6 QPA, KF6, KWin, Breeze, XWayland |
| **10** | KDE Remediation | v0.23.0 | Mar 2026 | PipeWire, NetworkManager, BlueZ, power mgmt |
| **11** | KDE Integration | v0.24.0 | Mar 2026 | startgui, session config, auto-fallback |
| **12** | KDE Cross-Compilation | v0.25.0 | Mar 2026 | musl pipeline, static binaries, 180MB rootfs |

### Post-Phase Fix

| Release | Description |
|---------|-------------|
| v0.25.1 | KDE session launch fix: direct ELF binary execution, stripped rootfs |
| v0.26.0 | Audit remediation (P0 + P1): security fixes, clocks and timers on every architecture, per-CPU allocator, lazy BlockFS cache |
| v0.27.0 | Process model and SMP: dispatched tasks on every CPU, copy-on-write fork, AArch64 and RISC-V user mode, Linux system call numbers |

## Version History

85 releases published from v0.1.0 through v0.27.0. See [CHANGELOG.md](https://github.com/doublegate/VeridianOS/blob/main/CHANGELOG.md) for the complete release history.

## Performance Targets

| Metric | Target | Measured |
|--------|--------|----------|
| IPC Latency | <1us | not measured (messages are queued on the endpoint; the IPC redesign is v0.28 work) |
| Context Switch | <10us | not measured |
| Memory Allocation | <1us | 62 ns per-CPU cache, 272 ns global allocator |
| Capability Lookup | O(1), <100 ns | 93 ns (hash table) |
| Concurrent Processes | 1000+ | not measured |

Figures from the in-kernel `perf` command; see the
[Performance Report](https://github.com/doublegate/VeridianOS/blob/main/docs/PERFORMANCE-REPORT.md).

## Future Directions

### v0.28

- The remaining Linux system call groups, tested with LTP and musl libc-test
- A full Linux capability model (capget/capset) and every namespace type (unshare/setns)
- The KDE shared-library pipeline and a media backend; relocatable RISC-V boot

### After v0.28

- Networking and storage (sprint F), hardening (G), real hardware (H)
- Drivers and services moved out of the kernel (critique C6)
- A full KDE Plasma 6 runtime

### Community Goals
- First external contributors
- Upstream KDE cross-compilation patches
- Conference presentations
- Security audit by third party

### Long-term Vision
- Production deployments for security-critical systems
- Hardware vendor partnerships
- Commercial support options
- Active research community

## Technical Targets for v1.0.0

| Feature | Status |
|---------|--------|
| Kernel syscall completeness | Pending |
| KDE Plasma 6 full runtime | Pending (ELF loads, syscalls needed) |
| Real hardware boot | Pending |
| Third-party security audit | Planned |
| Community contributor onboarding | Planned |

The project has achieved all original development goals across 13 phases. The path to v1.0.0 focuses on polishing the kernel-userspace interface to enable the cross-compiled KDE stack to run fully.
