# No-Std Test Harness Implementation Guide

## Overview

VeridianOS kernel operates in a `no_std` environment, which means it cannot use Rust's standard test framework that depends on `std`. This document outlines the implementation strategy for creating a custom test harness and the phased approach for restoring tests throughout the project.

## Problem Statement

The current test and benchmark compilation errors occur because:
- The `#[test]` and `#[bench]` attributes require the standard library's test crate
- Benchmarks using `test::Bencher` are unavailable in `no_std`
- Integration tests attempt to link against `std` which is not available in kernel space
- The kernel must run tests in a bare-metal environment without OS support

## Solution Architecture

### Custom Test Framework Components

1. **Test Runner** (Already partially implemented in `kernel/src/test_framework.rs`)
   - Custom test collection and execution
   - QEMU exit device integration for result reporting
   - Serial output for test status
   - Panic handler for test failures

2. **Test Macros**
   - Replace `#[test]` with custom attributes
   - Support for async test cases
   - Benchmark timing infrastructure
   - Test isolation and cleanup

3. **Integration Test Harness**
   - Separate test kernel builds
   - Hardware abstraction for test environments
   - Multi-architecture test support

## Implementation Phases

### Phase 0: Foundation (Months 1-3) ✅ COMPLETE
- **Status**: Basic test framework already implemented
- **Location**: `kernel/src/test_framework.rs`
- **Features**:
  - Custom test runner with `#[test_case]` support
  - QEMU exit device integration
  - Serial output for test results
  - Architecture-specific test initialization

### Phase 1: Enhanced Test Framework (Current - Months 4-9)

#### Priority 1: Core Test Infrastructure Enhancement
**Timeline**: Immediate (blocking other testing)
**Tasks**:
1. Extend test framework to support module-level tests
2. Add benchmark infrastructure without `test::Bencher`
3. Create test registry for dynamic test discovery
4. Implement per-test timeout mechanism
5. Add multi-threaded test support for SMP testing

**Implementation Details**:
```rust
// In kernel/src/test_framework.rs
pub trait Benchmark {
    fn run(&self, iterations: u64) -> Duration;
    fn warmup(&self, iterations: u64);
}

#[macro_export]
macro_rules! kernel_bench {
    ($name:ident, $body:expr) => {
        #[test_case]
        fn $name() {
            use crate::arch::time::read_tsc;
            let start = read_tsc();
            for _ in 0..1000 {
                $body
            }
            let end = read_tsc();
            serial_println!("Benchmark {}: {} cycles/iter", 
                stringify!($name), (end - start) / 1000);
        }
    };
}
```

#### Priority 2: Restore IPC Tests
**Timeline**: After core infrastructure (Week 2-3)
**Location**: `kernel/tests/ipc_integration_tests.rs`
**Tests to Restore**:
- Small message passing benchmarks
- Large message benchmarks  
- Async channel throughput tests
- Shared memory region tests
- Capability creation/validation tests
- Fast path IPC benchmarks
- Zero-copy transfer tests
- Rate limiting tests

**Migration Strategy**:
1. Convert `#[test]` to `#[test_case]`
2. Replace `test::Bencher` with custom benchmark macro
3. Fix ProcessId type mismatches (use tuple struct)
4. Add proper test initialization (IPC init)

#### Priority 3: Restore Scheduler Tests
**Timeline**: Week 3-4
**Location**: `kernel/tests/scheduler_tests.rs` (to be created)
**Tests to Implement**:
- Task creation and scheduling
- Priority-based preemption
- Load balancing verification
- IPC blocking/waking
- Context switch benchmarks
- Per-CPU queue management
- Idle task behavior
- Task migration tests

#### Priority 4: Restore Process Management Tests
**Timeline**: Week 4-5
**Location**: `kernel/tests/process_tests.rs` (to be created)
**Tests to Implement**:
- Process lifecycle (fork, exec, exit)
- Thread creation and termination
- Synchronization primitives (mutex, semaphore, etc.)
- Process table operations
- Memory cleanup verification
- CPU affinity tests
- TLS functionality

### Phase 2: Integration Testing (Months 10-15)

#### Advanced Test Infrastructure
**Tasks**:
1. Multi-process test scenarios
2. Stress testing framework
3. Performance regression testing
4. Security vulnerability testing
5. Hardware simulation tests

**New Test Categories**:
- **System Integration Tests**: Full microkernel functionality
- **Driver Tests**: User-space driver testing framework
- **IPC Stress Tests**: High-load message passing
- **Memory Stress Tests**: Allocation/deallocation patterns
- **Capability Security Tests**: Permission enforcement

### Phase 3: Continuous Testing (Months 16-21)

#### Test Automation
**Tasks**:
1. Automated test discovery
2. Parallel test execution
3. Test coverage reporting without `std`
4. Performance tracking dashboard
5. Failure reproduction system

#### Fuzz Testing
**Implementation**:
- System call fuzzing
- IPC message fuzzing
- Capability token fuzzing
- Memory operation fuzzing

### Phase 5: Performance Testing (Months 28-33)

#### Benchmark Suite
**Categories**:
1. **Microbenchmarks**:
   - IPC latency (target: <1μs)
   - Context switch time (target: <10μs)
   - Memory allocation (target: <1μs)
   - Capability lookup (target: O(1))

2. **Macrobenchmarks**:
   - System throughput
   - Concurrent process scaling
   - NUMA performance
   - Cache efficiency

3. **Real-world Benchmarks**:
   - Database workloads
   - Web server simulation
   - Compile benchmarks
   - Scientific computing

## Tests Requiring Restoration

Based on `DEFERRED-IMPLEMENTATION-ITEMS.md`, the following test categories need restoration:

### High Priority (Phase 1)
1. **Capability Unit Tests** (Lines 406-415)
   - Location: `kernel/src/cap/*.rs`
   - All capability module tests removed due to `no_std`
   - Requires custom test framework integration
   - Security-critical tests for forgery prevention

2. **IPC Integration Tests** (Lines 567-575)
   - Test IPC with process integration
   - Scheduler blocking/waking tests
   - Memory management integration
   - Capability validation tests

3. **Capability Security Tests** (Lines 577-586)
   - Forgery prevention tests
   - Unauthorized access tests
   - Privilege escalation tests
   - Revocation race condition tests

### Medium Priority (Phase 2)
1. **Capability Performance Tests** (Lines 589-599)
   - Lookup latency benchmarks
   - Cache hit rate tests
   - Revocation performance
   - Concurrent access scaling

2. **Capability Stress Tests** (Lines 601-610)
   - Maximum capabilities per process
   - Rapid creation/deletion cycles
   - Deep delegation chains
   - Revocation storms

### Test Framework Requirements

#### Architecture Support
- x86_64: QEMU exit device, serial output
- AArch64: QEMU semihosting, UART output
- RISC-V: OpenSBI console, HTIF interface

#### Test Organization
```
kernel/
├── src/
│   ├── test_framework.rs      # Core test infrastructure
│   └── tests/                  # Unit tests (cfg(test))
├── tests/                      # Integration tests
│   ├── common/
│   │   ├── mod.rs             # Shared test utilities
│   │   └── macros.rs          # Test macros
│   ├── ipc_tests.rs
│   ├── scheduler_tests.rs
│   ├── process_tests.rs
│   ├── capability_tests.rs
│   └── memory_tests.rs
└── benches/                    # Benchmarks
    ├── ipc_benchmarks.rs
    ├── scheduler_benchmarks.rs
    └── memory_benchmarks.rs
```

## Migration Guide

### Converting Standard Tests

#### From `#[test]` to `#[test_case]`
```rust
// Before (std test)
#[test]
fn test_something() {
    assert_eq!(2 + 2, 4);
}

// After (no_std test)  
#[test_case]
fn test_something() {
    assert_eq!(2 + 2, 4);
}
```

#### From `test::Bencher` to Custom Benchmark
```rust
// Before (std bench)
#[bench]
fn bench_ipc(b: &mut Bencher) {
    b.iter(|| {
        send_message();
    });
}

// After (no_std bench)
kernel_bench!(bench_ipc, {
    send_message();
});
```

### Test Initialization

Each test module should include:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_framework::*;
    
    fn setup() {
        // Module-specific initialization
        crate::ipc::init();
        crate::sched::init();
    }
    
    #[test_case]
    fn test_feature() {
        setup();
        // Test implementation
    }
}
```

## Success Criteria

### Phase 1 Completion
- [ ] All unit tests migrated to custom framework
- [ ] Benchmarks running without `std`
- [ ] CI passing all architecture tests
- [ ] Test coverage >80% for critical paths

### Phase 2 Completion  
- [ ] Integration tests fully operational
- [ ] Stress tests identifying edge cases
- [ ] Performance benchmarks baselined
- [ ] Security tests passing

### Phase 5 Completion
- [ ] Automated performance regression detection
- [ ] Comprehensive benchmark suite
- [ ] Test results dashboard
- [ ] Reproducible test environments

## Technical Considerations

### Memory Safety
- Tests must not corrupt kernel state
- Each test should run in isolation
- Failed tests must not prevent other tests

### Performance Impact
- Test code should be excluded from release builds
- Benchmark overhead must be measured
- Tests should not affect kernel timing

### Multi-Architecture
- Tests must work on all three architectures
- Architecture-specific tests clearly marked
- Common test code abstracted properly

## Conclusion

Implementing a custom test harness for VeridianOS is critical for ensuring kernel reliability and performance. This phased approach allows incremental progress while maintaining development velocity. The investment in test infrastructure during Phase 1 will pay dividends throughout the project lifecycle.

## References
- `kernel/src/test_framework.rs` - Existing test framework
- `DEFERRED-IMPLEMENTATION-ITEMS.md` - Tests requiring restoration
- `docs/design/TESTING-STRATEGY.md` - Overall testing approach