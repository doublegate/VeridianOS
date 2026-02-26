# Release Management TODO

**Purpose**: Track release planning, milestones, and deployment tasks
**Last Updated**: February 26, 2026
**Current Version**: v0.5.7 (Released February 26, 2026)
**Current Status**: Phases 0-4.5 Complete. Self-hosting Tiers 0-7 COMPLETE. Phase 5 ~75%. 32 releases published (v0.1.0 through v0.5.7).

## 🎯 Release Strategy

### Versioning Scheme
Following Semantic Versioning (SemVer):
- **MAJOR.MINOR.PATCH** (e.g., 1.2.3)
- **MAJOR**: Incompatible API changes
- **MINOR**: Backwards-compatible functionality
- **PATCH**: Backwards-compatible bug fixes

### Release Channels
- **Nightly**: Automated daily builds
- **Beta**: Weekly/bi-weekly test releases
- **Stable**: Production-ready releases
- **LTS**: Long-term support versions

## 📅 Release Roadmap

### Recent Maintenance Updates (June 2025)

#### v0.2.1 Critical Boot Fixes
**Date**: June 17, 2025  
**Status**: Released as v0.2.1  
**Issues Fixed**:
- **ISSUE-0013**: AArch64 iterator/loop compilation bug
  - Fixed with assembly-only workaround bypassing LLVM bug
  - AArch64 now boots to Stage 6 successfully
- **ISSUE-0014**: Context switching not loading initial context
  - Fixed scheduler to properly load task context on start
  - All architectures now have working context switching
- **Boot Testing**: All three architectures verified booting to Stage 6

### Pre-1.0 Releases (Development)

#### v0.2.1 - Boot Fixes ✅ RELEASED!
**Released**: June 17, 2025  
**Phase**: 1 (Maintenance)  
**Achievements**:
- [x] AArch64 assembly-only workaround for LLVM bug ✅
- [x] All architectures boot to Stage 6 successfully ✅
- [x] Zero warnings across all platforms ✅
- [x] Clippy-clean codebase ✅
- [x] Updated documentation for boot status ✅

#### v0.1.0 - Foundation ✅ RELEASED!
**Released**: June 7, 2025  
**Phase**: 0 (Complete)  
**Achievements**:
- [x] Basic boot on x86_64 ✅
- [x] Basic boot on AArch64 ✅
- [x] Basic boot on RISC-V ✅
- [x] Build system complete ✅
- [x] Serial console output ✅
- [x] CI/CD pipeline 100% operational ✅
- [x] GDB debugging infrastructure ✅
- [x] Test framework foundation ✅
- [x] Documentation framework ✅
- [x] Version control hooks ✅

#### v0.2.0 - Core Kernel ✅ RELEASED!
**Released**: June 12, 2025  
**Phase**: 1 (Complete)  
**Achievements**:
- [x] IPC implementation (100% complete) ✅
  - [x] Synchronous message passing ✅
  - [x] Fast path optimization (<1μs achieved!) ✅
  - [x] Zero-copy transfers ✅
  - [x] Asynchronous channels ✅
  - [x] Performance benchmarks ✅
  - [x] Full capability integration ✅
  - [x] Rate limiting and registry ✅
- [x] Memory management (100% complete) ✅
  - [x] Hybrid frame allocator ✅
  - [x] Virtual memory manager ✅
  - [x] Kernel heap with slab allocator ✅
  - [x] Page tables and TLB management ✅
  - [x] User space safety validation ✅
- [x] Process management (100% complete) ✅
  - [x] Full lifecycle implementation ✅
  - [x] Context switching all architectures ✅
  - [x] Synchronization primitives ✅
  - [x] System calls ✅
- [x] Scheduler (100% complete) ✅
  - [x] CFS implementation ✅
  - [x] SMP support ✅
  - [x] Load balancing ✅
  - [x] CPU hotplug ✅
- [x] Capability system (100% complete) ✅
  - [x] Inheritance mechanisms ✅
  - [x] Cascading revocation ✅
  - [x] Per-CPU cache ✅

#### v0.3.x - User Space Foundation -- RELEASED (v0.3.0 through v0.3.9)
**Released**: February 14-15, 2026
**Phase**: 2-3 COMPLETE
**Achievements**:
- [x] VFS: RamFS, DevFS, ProcFS, BlockFS with ext2-style directories
- [x] ELF loader, driver framework, init system, shell
- [x] Full crypto suite (ChaCha20, Ed25519, X25519, ML-DSA, ML-KEM)
- [x] MAC policy, RBAC/MLS, audit system, memory protection (ASLR, DEP/NX, W^X)
- [x] Ring 3 user-space entry, SYSCALL/SYSRET

#### v0.4.x - Package Ecosystem + Interactive Shell + Self-Hosting -- RELEASED (v0.4.0 through v0.4.9)
**Released**: February 15-18, 2026
**Phase**: 4 + 4.5 COMPLETE, Self-hosting Tiers 0-5 COMPLETE
**Achievements**:
- [x] Package manager, DPLL dependency resolver, ports system, SDK
- [x] Interactive shell (vsh): 18 sprints, 24+ builtins, pipes, redirection, job control, scripting
- [x] Framebuffer console: UEFI GOP (x86_64), ramfb (AArch64/RISC-V), PS/2 keyboard
- [x] Self-hosting: complete libc (17 files, 6,547 LOC), GCC cross-compiler, virtio-blk, TAR rootfs
- [x] User-space exec: /bin/minimal runs, CR3 switching removed (~2000 cycles/syscall saved)

#### v0.5.7 - Phase 5 Sprint 2: Performance Optimization -- RELEASED
**Released**: February 26, 2026
**Phase**: Phase 5 (Performance Optimization) ~75%
**Achievements**:
- [x] Per-CPU page frame cache (PerCpuPageCache, 64-frame, batch refill/drain)
- [x] IPC fast path completion (per-task ipc_regs, direct register transfer)
- [x] TLB optimization (TlbFlushBatch, lazy TLB, tlb_generation counter)
- [x] Priority inheritance protocol (PiMutex in process/sync.rs)
- [x] Benchmarking suite (7 micro-benchmarks, perf shell builtin)
- [x] Software tracepoints (10 event types, per-CPU ring buffers, trace shell builtin)
- [x] Documentation sync (MASTER_TODO, PHASE5_TODO, RELEASE_TODO updated)

#### v0.5.6 - Phase 5 Sprint 1 -- RELEASED
**Released**: February 25, 2026
**Phase**: Phase 5 (Performance Optimization) ~30%
**Achievements**:
- [x] Scheduler context switch wiring (all 3 architectures)
- [x] IPC blocking/wake with fast path framework
- [x] TSS RSP0 management for per-task kernel stacks
- [x] User-space /sbin/init (PID 1 in Ring 3)
- [x] Native binary execution (NATIVE_ECHO_PASS)
- [x] Dead code audit (136 to <100 annotations)
- [x] All 56 TODO(phase5) markers resolved

#### v0.5.5 - POSIX Partial munmap + Native BusyBox -- RELEASED
**Released**: February 25, 2026
**Achievements**:
- [x] POSIX-compliant partial munmap (5-case: exact, front/back trim, hole punch, sub-range)
- [x] Consolidated brk() heap mapping (O(1) per extension)
- [x] Native BusyBox 208/208 sources compiled + linked
- [x] 12 missing libc stubs

#### v0.5.4 - Critical Memory Leak Fixes -- RELEASED
**Released**: February 25, 2026
**Achievements**:
- [x] GP fault wrmsr register constraint fix (release-only)
- [x] Page table subtree leak fix during exec (~75MB over 630 execs)
- [x] Thread stack lifecycle frame leak fix (~197MB over 630 processes)

#### v0.5.3 - BusyBox Compatibility -- RELEASED
**Released**: February 24, 2026
**Achievements**:
- [x] BusyBox ash shell compatibility (B-10)
- [x] Process lifecycle hardening for 213+ sequential execs (B-11)
- [x] ARG_MAX enforcement (128KB POSIX limit, B-13)
- [x] strftime + popen/pclose (B-18)

#### v0.5.2 - BusyBox Build Infrastructure -- RELEASED
**Released**: February 24, 2026
**Achievements**:
- [x] EPIPE/BrokenPipe support, float printf, POSIX regex (1291 lines BRE/ERE)
- [x] 384MB kernel heap, sbrk hardening, 30+ libc headers
- [x] CI target fix (kernel code model)

#### v0.5.1 - Coreutils + Pipe Fix -- RELEASED
**Released**: February 23, 2026
**Achievements**:
- [x] 6 coreutils (echo/cat/wc/ls/sort/pipeline_test)
- [x] pipe() fd ABI fix (usize -> i32)
- [x] Tri-arch clippy clean

#### v0.5.0 - Self-Hosting Complete + User-Space Foundation -- RELEASED
**Released**: February 21, 2026
**Phase**: Self-hosting Tiers 6-7 COMPLETE + User-Space Foundation
**Achievements**:
- [x] Merge `test-codex` Tier 6 work (T6-0 through T6-5) + QEMU validation
- [x] Native GCC on VeridianOS (T7-3) -- static GCC 14.2 via Canadian cross-compilation
- [x] make/ninja cross-compiled (T7-4) -- GNU Make 4.4.1 + Ninja 1.12.1
- [x] vpkg user-space package manager (T7-5)
- [x] Rust user-space targets + std port (T7-1, T7-2)
- [x] ELF loader dynamic binary fix, fork verification
- [x] Console blocking read, fd 0/1/2 auto-open, user-space shell bootstrap
- [x] dead_code audit, TODO(future) recategorization to phase5/phase6

#### v0.6.0 - Rust Self-Hosting + Advanced Features
**Target Date**: Q3 2026
**Phase**: Self-hosting Tier 7 + Phase 6
**Goals**:
- [ ] Rust user-space target JSON (T7-1)
- [ ] Rust std port (T7-2)
- [ ] vpkg user-space migration (T7-5)
- [ ] Wayland compositor
- [ ] Basic desktop environment

### v1.0.0 - First Stable Release
**Target Date**: Q3 2027  
**Criteria**:
- [ ] All phase goals complete
- [ ] Security audit passed
- [ ] Performance targets met
- [ ] Documentation complete
- [ ] Ecosystem established

### Post-1.0 Releases

#### v1.1.0
**Target**: Q4 2027  
**Focus**: Stability and polish

#### v1.2.0
**Target**: Q1 2028  
**Focus**: Enhanced features

#### v2.0.0
**Target**: 2028  
**Focus**: Next-generation features

## 📋 Release Process

### Pre-Release Checklist

#### Code Quality
- [ ] All tests passing
- [ ] Code coverage > 80%
- [ ] No critical bugs
- [ ] Performance benchmarks pass
- [ ] Security scan clean

#### Documentation
- [ ] Release notes written
- [ ] API docs updated
- [ ] Migration guide (if needed)
- [ ] Known issues documented
- [ ] Installation guide updated

#### Testing
- [ ] Full regression suite
- [ ] Platform testing
- [ ] Upgrade testing
- [ ] Performance testing
- [ ] Security testing

#### Infrastructure
- [ ] Build artifacts ready
- [ ] Repository updated
- [ ] Mirror sync
- [ ] Download servers ready
- [ ] Backup plans

### Release Steps

1. **Code Freeze**
   - [ ] Announce freeze date
   - [ ] Branch creation
   - [ ] Stop feature additions
   - [ ] Focus on bug fixes

2. **Release Candidate**
   - [ ] Tag RC version
   - [ ] Build all targets
   - [ ] Publish to beta channel
   - [ ] Community testing

3. **Final Release**
   - [ ] Final version tag
   - [ ] Build release artifacts
   - [ ] Sign artifacts
   - [ ] Upload to servers

4. **Announcement**
   - [ ] Website update
   - [ ] Blog post
   - [ ] Social media
   - [ ] Mailing lists
   - [ ] Press release (major versions)

5. **Post-Release**
   - [ ] Monitor feedback
   - [ ] Track downloads
   - [ ] Handle issues
   - [ ] Plan patches

## 🎯 Next Release Planning

### v0.5.0 - Self-Hosting Tier 6 + Performance
**Target**: Q2 2026
**Current Status**: Tier 6 coded on `test-codex`, needs merge + QEMU validation
**Key Deliverables**:
- Merge `test-codex` branch (T6-0 through T6-5)
- QEMU-validate all Tier 6 items
- Native GCC on VeridianOS (T7-3)
- Performance optimization sprint

## 🔧 Release Artifacts

### Binary Releases
- [ ] Kernel images (all architectures)
- [ ] Installer ISO images
- [ ] VM images (QEMU, VirtualBox)
- [ ] Cloud images (AWS, Azure, GCP)
- [ ] Container images

### Source Releases
- [ ] Source tarball
- [ ] Git tag
- [ ] Signed checksums
- [ ] Release signatures

### Documentation
- [ ] Release notes
- [ ] Installation guide
- [ ] Upgrade guide
- [ ] API documentation
- [ ] Man pages

## 📊 Release Metrics

### Quality Metrics
- Bug count by severity
- Test pass rate
- Code coverage
- Performance benchmarks
- Security issues

### Adoption Metrics
- Download count
- Active installations
- Community growth
- Contributor count
- Package ecosystem size

## 🐛 Release Issues

### Known Issues
Track issues specific to releases.

### Blocking Issues
Issues that must be fixed before release.

## 📝 Release Notes Template

```markdown
# VeridianOS vX.Y.Z Release Notes

**Release Date**: YYYY-MM-DD  
**Type**: Major/Minor/Patch

## Highlights
- Key feature 1
- Key feature 2
- Key improvement

## New Features
### Category
- Feature description

## Improvements
### Performance
- Improvement description

### Security
- Security enhancement

## Bug Fixes
- Fixed issue #XXX: Description
- Fixed issue #YYY: Description

## Breaking Changes
- Change description
- Migration instructions

## Deprecations
- Deprecated feature
- Replacement recommendation

## Known Issues
- Issue description
- Workaround if available

## Contributors
Thanks to all contributors!
[List of contributors]

## Upgrade Instructions
[Upgrade steps]

## Download
[Download links]
```

## 🔒 Security Releases

### Security Release Process
1. Security report received
2. Verify and assess impact
3. Develop fix in private
4. Coordinate disclosure
5. Release with announcement

### Embargo Period
- Critical: 7-14 days
- High: 30 days
- Medium: 60 days
- Low: 90 days

## 📅 Release Calendar

### 2025 (Completed)
- Q2: v0.1.0 (Foundation), v0.2.0 (Core Kernel), v0.2.1 (Boot Fixes)
- Q4: v0.2.5, v0.3.0 (Architecture cleanup)

### 2026 (Completed + In Progress)
- Q1: v0.3.1 through v0.5.7 (User Space, Security, Packages, Shell, Self-hosting, Phase 5)
- Q2: v0.5.7+ (Phase 5 Performance + Phase 6 GUI) -- PLANNED
- Q3: v0.6.0 (Rust self-hosting + Advanced features) -- PLANNED
- Q4: v0.7.0 (Full self-hosting loop) -- PLANNED

### 2027
- Q1: v0.8.0 (Performance + Polish)
- Q2: v0.9.0 (GUI)
- Q3: v1.0.0 (Stable)
- Q4: v1.1.0 (LTS candidate)

## 🔗 Release Resources

### Tools
- Release automation scripts
- Signing keys
- Build infrastructure
- Distribution network

### Documentation
- [Release Process](../docs/RELEASE-PROCESS.md)
- [Versioning Policy](../docs/VERSIONING.md)
- [Security Policy](../SECURITY.md)

---

**Note**: This document tracks all release planning and execution. Update after each release and during planning sessions.