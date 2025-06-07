# Issues and Bug Tracking TODO

**Purpose**: Central tracking for all bugs, issues, and defects  
**Last Updated**: 2025-06-07

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

Currently no critical issues.

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

## 📊 Issue Statistics

### Overall Status
- **Total Issues**: 7
- **Open Issues**: 0
- **In Progress**: 0
- **Fixed**: 7
- **Verified**: 7 ✅
- **Closed**: 0

### By Component
| Component | Open | In Progress | Fixed | Total |
|-----------|------|-------------|-------|-------|
| Kernel | 0 | 0 | 3 | 3 |
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