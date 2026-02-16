# Issues and Bug Tracking TODO

**Purpose**: Central tracking for all bugs, issues, and defects
**Last Updated**: February 15, 2026

## 🐛 Issue Categories

### Severity Levels
- **P0 - Critical**: System crash, data loss, security vulnerability
- **P1 - High**: Major functionality broken, significant performance issue
- **P2 - Medium**: Minor functionality issue, workaround available
- **P3 - Low**: Cosmetic issue, enhancement request

### Issue Types
- **Bug**: Defect in existing functionality
- **Regression**: Previously working feature broken
- **Performance**: Speed or resource usage issue
- **Security**: Security vulnerability or concern
- **Compatibility**: Hardware or software compatibility issue

## 🚨 Critical Issues (P0)

### ISSUE-0012: x86_64 Boot Hang
- **Status**: RESOLVED (Fixed across v0.3.0 through v0.3.5)
- **Component**: Kernel/Boot
- **Reported**: 2025-06-13
- **Resolved**: February 15, 2026 (v0.3.5)
- **Reporter**: Boot Testing
- **Description**: x86_64 kernel hangs very early in boot with no serial output
- **Root Cause**: Multiple issues - bootloader upgrade needed, CSPRNG CPUID check missing (RDRAND on QEMU qemu64 CPU), boot stack too small (64KB insufficient for debug builds with ~20KB CapabilitySpace on stack)
- **Fix**: Bootloader upgrade (v0.3.0), CPUID RDRAND check with timer-jitter fallback + 256KB boot stack (v0.3.5)
- **Verification**: x86_64 now boots to Stage 6 BOOTOK with 27/27 tests and zero warnings

### ISSUE-0013: AArch64 Function Call Hang
- **Status**: RESOLVED ✅
- **Component**: Kernel/Boot
- **Reported**: 2025-06-13
- **Resolved**: 2025-06-17
- **Reporter**: Boot Testing / Debugging Session
- **Assignee**: Claude
- **Description**: AArch64 kernel hangs when any function call is made
- **Impact**: AArch64 platform severely limited - no function calls possible
- **Root Cause**: Improper stack initialization - hardcoded to 0x80000 instead of using linker symbols
- **Fix**: Updated boot.S to use linker-defined __stack_top with proper 16-byte alignment
- **Resolution**: Function calls now work correctly, unified bootstrap used
- **Note**: What appeared to be an LLVM bug was actually incorrect stack setup

### ISSUE-0017: AArch64 Bootstrap Completion
- **Status**: RESOLVED
- **Component**: Kernel/Bootstrap
- **Reported**: 2025-06-17
- **Resolved**: August 2025 (v0.3.0 bootstrap refactoring)
- **Reporter**: Implementation Session
- **Description**: AArch64 bootstrap returns at Stage 6 instead of transitioning to scheduler
- **Root Cause**: Early return in bootstrap.rs for AArch64 at Stage 6
- **Fix**: Bootstrap refactoring simplified bootstrap.rs to ~150 lines with per-arch entry.rs/bootstrap.rs/serial.rs modules. AArch64 now completes full boot sequence.
- **Verification**: AArch64 boots to Stage 6 BOOTOK with 27/27 tests and zero warnings

### ISSUE-0018: RISC-V Frame Allocator Lock Hang
- **Status**: RESOLVED
- **Component**: Kernel/Memory Management
- **Reported**: 2025-06-17
- **Resolved**: February 2026 (v0.3.1 static mut elimination + v0.3.5 memory region fix)
- **Reporter**: Stack Setup Audit
- **Description**: RISC-V kernel restarts when trying to acquire frame allocator lock
- **Root Cause**: Static mut initialization issues causing lock state corruption; also memory start was set to 0x88000000 (END of 128MB RAM) instead of after kernel end
- **Fix**: Static mut elimination (v0.3.1) resolved lock issues. Frame allocator memory start changed to 0x80E00000 (v0.3.5).
- **Verification**: RISC-V boots to Stage 6 BOOTOK with 27/27 tests and zero warnings

### ISSUE-0014: Context Switching Not Connected
- **Status**: RESOLVED ✅
- **Component**: Kernel/Scheduler
- **Reported**: 2025-06-15
- **Resolved**: 2025-06-15
- **Reporter**: Code Analysis
- **Assignee**: Claude
- **Description**: Context switching was implemented but scheduler wasn't using it
- **Impact**: Could not switch between processes/threads
- **Root Cause**: Scheduler's start() function entered idle loop instead of loading context
- **Fix**: Updated `sched/mod.rs` start() to properly load initial task context
- **Files Fixed**: 
  - kernel/src/sched/mod.rs - Added context loading logic
  - kernel/src/arch/{x86_64,aarch64,riscv}/context.rs - Already had implementations
- **Verification**: Created test tasks to demonstrate context switching

<!-- Template:
### ISSUE-0001: [Title]
- **Status**: Open/In Progress/Fixed/Verified
- **Component**: Kernel/Driver/Service/Other
- **Reported**: YYYY-MM-DD
- **Reporter**: Name
- **Assignee**: Name
- **Description**: Brief description
- **Impact**: What is affected
- **Workaround**: Temporary solution if available
- **Fix**: PR# or commit hash when fixed
-->

## 🔴 High Priority Issues (P1)

Currently no high priority issues.

## 🟡 Medium Priority Issues (P2)

Currently no medium priority issues.

## 🟢 Low Priority Issues (P3)

Currently no low priority issues.

## ✅ Recently Resolved Issues

### ISSUE-0015: x86_64 Context Switch Infinite Loop
- **Status**: RESOLVED ✅
- **Component**: Kernel/Context Switching
- **Reported**: 2025-06-15
- **Resolved**: 2025-06-15
- **Reporter**: Debugging Session
- **Assignee**: Claude
- **Description**: x86_64 context switching caused infinite loop when switching from scheduler
- **Impact**: Could not execute tasks on x86_64
- **Root Cause**: Using `iretq` (interrupt return) instead of `ret` for kernel-to-kernel context switch
- **Fix**: Changed `load_context` in `arch/x86_64/context.rs` to use `ret` instruction
- **Verification**: Bootstrap_stage4 now executes successfully
- **Details**: `iretq` expects interrupt frame on stack, but kernel-to-kernel switches don't have that

### ISSUE-0016: x86_64 Memory Mapping Errors
- **Status**: RESOLVED ✅
- **Component**: Kernel/Memory Management
- **Reported**: 2025-06-15
- **Resolved**: 2025-06-15
- **Reporter**: Init Process Creation
- **Assignee**: Claude
- **Description**: "Address range already mapped" error when creating init process
- **Impact**: Could not create user processes on x86_64
- **Root Causes**: 
  1. Duplicate kernel space mapping (init() already calls map_kernel_space())
  2. Kernel heap size of 256MB exceeded total memory of 128MB
- **Fixes**: 
  1. Removed duplicate `map_kernel_space()` call in `process/lifecycle.rs`
  2. Reduced heap size from 256MB to 16MB in `mm/vas.rs`
- **Verification**: Init process creation now progresses past memory setup

### ISSUE-0001: CI Build Failures for Custom Targets
- **Status**: Fixed/Verified
- **Component**: Build System/CI
- **Reported**: 2025-06-06
- **Reporter**: CI Pipeline
- **Assignee**: Claude
- **Description**: CI builds failing with "can't find crate for core" error
- **Impact**: All architecture builds failing in CI
- **Root Cause**: Custom targets require -Zbuild-std to build core library from source
- **Fix**: Updated CI workflow to use -Zbuild-std flags (commit: 8790414)

### ISSUE-0002: RISC-V Target Specification Invalid ABI
- **Status**: Fixed/Verified
- **Component**: Build System
- **Reported**: 2025-06-06
- **Reporter**: CI Pipeline
- **Assignee**: Claude
- **Description**: RISC-V builds failing with "invalid RISC-V ABI name" error
- **Impact**: RISC-V architecture builds failing
- **Root Cause**: Missing llvm-abiname field in target specification
- **Fix**: Added llvm-abiname and corrected llvm-target (commit: f49cc2f)

### ISSUE-0003: Security Audit Job Missing Cargo.lock
- **Status**: Fixed/Verified
- **Component**: CI/Security
- **Reported**: 2025-06-06
- **Reporter**: CI Pipeline
- **Assignee**: Claude
- **Description**: cargo-audit failing with "Couldn't load ./Cargo.lock"
- **Impact**: Security audit CI job failing
- **Root Cause**: Cargo.lock was in .gitignore
- **Fix**: Removed Cargo.lock from .gitignore and committed it (commit: 8790414)

### ISSUE-0005: Clippy warnings for unused code
- **Status**: Fixed/Verified ✅
- **Component**: Kernel
- **Reported**: 2025-06-06
- **Reporter**: CI/Clippy
- **Assignee**: Claude
- **Description**: Clippy reported unused import and dead code warnings
- **Impact**: CI would fail with -D warnings flag
- **Workaround**: None needed
- **Root Cause**: Stub implementations for future phases were not marked as allowed dead code
- **Fix**: (commit: 9a263b5)
  - Removed unused `core::fmt::Write` import in serial.rs:18
  - Added `#[allow(dead_code)]` to placeholder functions in:
    - arch/x86_64/mod.rs:31 (idle)
    - cap/mod.rs:3 (init)
    - ipc/mod.rs:3 (init) 
    - mm/mod.rs:3 (init)
    - sched/mod.rs:3 (init)
    - sched/mod.rs:11 (run)
  - Fixed all formatting issues with `cargo fmt`
  - **Result: CI/CD pipeline now 100% passing all checks!** 🎉

### ISSUE-0006: AArch64 Boot Sequence Not Reaching Rust Code
- **Status**: Fixed/Verified ✅
- **Component**: Kernel/Boot
- **Reported**: 2025-06-06
- **Reporter**: QEMU Testing
- **Assignee**: Claude
- **Description**: AArch64 kernel builds successfully but cannot branch from assembly to Rust code
- **Impact**: AArch64 architecture cannot boot to kernel main
- **Symptoms**: 
  - Assembly boot code executes
  - Cannot branch to _start_rust or any Rust functions
  - Iterator-based code causes hangs on bare metal
- **Root Cause**: Complex Rust code (iterators, formatting) causes issues on bare metal AArch64
- **Fix**: (2025-06-07)
  - Simplified boot sequence to use direct memory writes only
  - Removed all iterator usage in AArch64 boot path
  - Created working-simple/ directory for known-good implementations
  - Files renamed to match x86_64/riscv64 pattern
  - **Result: AArch64 now boots successfully to kernel_main!** 🎉

### ISSUE-0007: GDB break-boot command "No symbol" errors
- **Type**: Tool/Debugging
- **Severity**: P2
- **Status**: FIXED
- **Reporter**: parobek
- **Assignee**: claude
- **Created**: 2025-06-07
- **Components**: GDB scripts, debugging infrastructure
- **Description**: GDB architecture-specific scripts failed with "No symbol 'aarch64' in current context" errors
- **Root Cause**: Unquoted string arguments in GDB break-boot commands were interpreted as symbols
- **Fix**: (2025-06-07)
  - Added quotes around architecture strings in all GDB scripts
  - Changed `break-boot aarch64` to `break-boot "aarch64"`
  - Applied same fix to x86_64 and riscv64 scripts
  - **Result: All architectures now work with GDB debugging!**

### ISSUE-0008: x86_64 R_X86_64_32S Relocation Errors
- **Type**: Kernel/Build
- **Severity**: P1
- **Status**: FIXED
- **Reporter**: Build System
- **Assignee**: claude
- **Created**: 2025-12-06
- **Components**: Kernel/x86_64
- **Description**: Kernel build failed with R_X86_64_32S relocation errors when linked at high addresses
- **Root Cause**: Default x86_64 code model cannot handle kernel addresses above 2GB
- **Fix**: (2025-12-06, commit f15dfbf)
  - Created custom x86_64 target JSON with kernel code model
  - Updated build script to use custom target
  - Added kernel code model for high address linking
  - **Result: x86_64 kernel builds and links successfully at 0xFFFFFFFF80100000!**

### ISSUE-0009: Kernel Boot Double Fault  
- **Type**: Kernel/Runtime
- **Severity**: P0
- **Status**: FIXED
- **Reporter**: QEMU Testing
- **Assignee**: claude
- **Created**: 2025-12-06
- **Components**: Kernel/Boot
- **Description**: Kernel crashes with double fault immediately after boot
- **Root Cause**: Stack initialization and early boot sequence issues
- **Fix**: (2025-12-06, commit f15dfbf)
  - Fixed stack alignment and initialization
  - Corrected boot sequence for proper memory setup
  - Added proper BSS clearing
  - **Result: Kernel boots successfully without crashes!**

### ISSUE-0010: Heap Initialization Failure
- **Type**: Kernel/Memory
- **Severity**: P1
- **Status**: FIXED
- **Reporter**: Kernel Panic
- **Assignee**: claude
- **Created**: 2025-12-06
- **Components**: Kernel/Memory Management
- **Description**: Kernel panics during heap initialization
- **Root Cause**: Frame allocator not properly initialized before heap setup
- **Fix**: (2025-12-06, commit f15dfbf)
  - Fixed initialization order in kernel main
  - Ensured frame allocator setup before heap
  - Added proper memory region detection
  - **Result: Heap initializes correctly with proper memory management!**

### ISSUE-0011: Memory Allocator Mutex Deadlock
- **Type**: Kernel/Memory
- **Severity**: P1
- **Status**: FIXED
- **Reporter**: RISC-V Boot Testing
- **Assignee**: claude
- **Created**: 2025-12-06
- **Components**: Kernel/Memory Management
- **Description**: RISC-V kernel hangs during memory allocator initialization
- **Root Cause**: Mutex deadlock when stats tracking tries to allocate during init
- **Fix**: (2025-12-06)
  - Skip stats updates during initialization phase
  - Added architecture-specific memory maps for init_default()
  - Deferred stats tracking until after initialization complete
  - **Result: RISC-V now boots successfully through all subsystems!**

## 📊 Issue Statistics

### Overall Status
- **Total Issues**: 11
- **Open Issues**: 0
- **In Progress**: 0
- **Fixed**: 11
- **Verified**: 11 ✅
- **Closed**: 0

### Current Architecture Boot Status
- **x86_64**: Boots through all subsystems, hangs at process init (expected)
- **RISC-V**: Boots through all subsystems, hangs at process init (mutex fix applied)
- **AArch64**: Boot issue - kernel_main not reached from _start_rust

**Note**: Memory management implementation completed with no outstanding issues!

### By Component
| Component | Open | In Progress | Fixed | Total |
|-----------|------|-------------|-------|-------|
| Kernel | 0 | 0 | 10 | 10 |
| Drivers | 0 | 0 | 0 | 0 |
| Services | 0 | 0 | 0 | 0 |
| Libraries | 0 | 0 | 0 | 0 |
| Tools | 0 | 0 | 1 | 1 |
| Documentation | 0 | 0 | 0 | 0 |
| Build System | 0 | 0 | 2 | 2 |
| CI/Security | 0 | 0 | 1 | 1 |

### By Type
| Type | Count | Percentage |
|------|-------|------------|
| Bug | 0 | 0% |
| Regression | 0 | 0% |
| Performance | 0 | 0% |
| Security | 0 | 0% |
| Compatibility | 0 | 0% |

## 🔄 Regressions

### Regression Tracking
Track features that have broken after previously working.

Currently no regressions.

<!-- Template:
### REG-0001: [Feature] regression in [version]
- **Working Version**: Last known good version
- **Broken Version**: First broken version
- **Commit Range**: Hash range where regression introduced
- **Status**: Identified/Bisecting/Fixed
- **Root Cause**: What caused the regression
-->

## 🔒 Security Issues

### Security Vulnerability Tracking
Security issues are tracked separately with restricted access.

- **Public Issues**: 0
- **Embargoed Issues**: 0
- **CVEs Assigned**: None

For security issues, see [SECURITY.md](../SECURITY.md)

## 🎯 Issue Resolution Goals

### SLA Targets
| Severity | Response Time | Resolution Target |
|----------|---------------|-------------------|
| P0 | 1 hour | 24 hours |
| P1 | 4 hours | 1 week |
| P2 | 2 days | 1 month |
| P3 | 1 week | Best effort |

### Current Performance
- **Average Response Time**: N/A
- **Average Resolution Time**: N/A
- **SLA Compliance**: N/A

## 🛠️ Issue Management Process

### Issue Lifecycle
1. **Reported** - Issue identified and logged
2. **Triaged** - Severity and component assigned
3. **Assigned** - Developer assigned to fix
4. **In Progress** - Active development
5. **Fixed** - Code changes complete
6. **In Review** - Code review and testing
7. **Verified** - Fix confirmed working
8. **Closed** - Issue resolved

### Triage Process
- Daily triage for P0/P1 issues
- Weekly triage for P2/P3 issues
- Component owners review their queues
- SLA tracking and escalation

## 📝 Issue Templates

### Bug Report Template
```markdown
**Summary**: One-line description

**Component**: Affected component
**Version**: Version where issue found
**Platform**: Hardware/OS details

**Steps to Reproduce**:
1. Step one
2. Step two
3. Step three

**Expected Result**: What should happen
**Actual Result**: What actually happens

**Additional Information**:
- Logs
- Screenshots
- System configuration
```

### Performance Issue Template
```markdown
**Summary**: Performance problem description

**Component**: Affected component
**Metrics**: Specific measurements

**Test Case**: How to reproduce
**Expected Performance**: Target metrics
**Actual Performance**: Current metrics

**Profile Data**: Attach profiling results
**Analysis**: Initial investigation findings
```

## 🔍 Common Issues and Solutions

### Build Issues
Document common build problems and solutions.

### Runtime Issues
Document common runtime problems and solutions.

### Configuration Issues
Document common configuration problems and solutions.

## 📅 Issue Review Schedule

### Daily
- P0 issue review
- New issue triage
- Blocker assessment

### Weekly
- All issue review
- Trend analysis
- Process improvements

### Monthly
- Metrics review
- SLA compliance
- Root cause analysis

## 🔗 External References

### Issue Tracking
- GitHub Issues: [Link when available]
- Security Issues: security@veridian-os.org

### Related Documents
- [Testing TODO](TESTING_TODO.md)
- [QA TODO](QA_TODO.md)
- [Known Issues](../docs/KNOWN-ISSUES.md)

---

**Note**: This document tracks issues discovered during development. For feature requests, see [ENHANCEMENTS_TODO.md](ENHANCEMENTS_TODO.md)