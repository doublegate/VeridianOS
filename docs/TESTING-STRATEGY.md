# VeridianOS Testing Strategy

## Overview

This document outlines the comprehensive testing strategy for VeridianOS, covering all levels from unit tests to system-wide validation. Our testing approach ensures reliability, security, and performance across all components.

## Testing Philosophy

1. **Test Early, Test Often**: Write tests alongside code
2. **Comprehensive Coverage**: Aim for >90% code coverage
3. **Automated Testing**: All tests run in CI/CD pipeline
4. **Performance Testing**: Prevent regressions
5. **Security Testing**: Continuous vulnerability assessment

## Testing Levels

### 1. Unit Testing

Unit tests validate individual functions and modules in isolation.

#### Kernel Unit Tests

```rust
// kernel/src/mm/physical/tests.rs
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    
    #[test]
    fn test_frame_allocator_basic() {
        let mut allocator = FrameAllocator::new_test();
        
        // Allocate frame
        let frame1 = allocator.allocate_frame().expect("allocation failed");
        assert!(frame1.is_aligned(PAGE_SIZE));
        
        // Ensure different frames
        let frame2 = allocator.allocate_frame().expect("allocation failed");
        assert_ne!(frame1, frame2);
        
        // Free and reallocate
        allocator.deallocate_frame(frame1);
        let frame3 = allocator.allocate_frame().expect("allocation failed");
        assert_eq!(frame1, frame3); // Should reuse freed frame
    }
    
    proptest! {
        #[test]
        fn test_allocation_stress(ops in prop::collection::vec(
            prop_oneof![
                Just(AllocOp::Allocate),
                Just(AllocOp::Deallocate),
            ],
            0..1000
        )) {
            let mut allocator = FrameAllocator::new_test();
            let mut allocated = Vec::new();
            
            for op in ops {
                match op {
                    AllocOp::Allocate => {
                        if let Some(frame) = allocator.allocate_frame() {
                            assert!(!allocated.contains(&frame));
                            allocated.push(frame);
                        }
                    }
                    AllocOp::Deallocate => {
                        if let Some(frame) = allocated.pop() {
                            allocator.deallocate_frame(frame);
                        }
                    }
                }
            }
            
            // Verify no double allocation
            let unique: HashSet<_> = allocated.iter().collect();
            assert_eq!(unique.len(), allocated.len());
        }
    }
}
```

#### Driver Unit Tests

```rust
// drivers/nvme/src/tests.rs
#[cfg(test)]
mod tests {
    use super::*;
    use mockall::*;
    
    #[automock]
    trait NvmeController {
        fn submit_command(&mut self, cmd: NvmeCommand) -> Result<(), Error>;
        fn get_completion(&mut self) -> Option<NvmeCompletion>;
    }
    
    #[test]
    fn test_nvme_read_command() {
        let mut mock = MockNvmeController::new();
        
        mock.expect_submit_command()
            .withf(|cmd| {
                cmd.opcode == NVME_OP_READ &&
                cmd.nsid == 1 &&
                cmd.cdw10 == 0x1000 // LBA
            })
            .times(1)
            .returning(|_| Ok(()));
            
        mock.expect_get_completion()
            .times(1)
            .returning(|| Some(NvmeCompletion {
                status: 0,
                cid: 1,
                result: 0,
            }));
            
        let mut driver = NvmeDriver::new(Box::new(mock));
        let result = driver.read(0x1000, 1, &mut [0u8; 512]);
        assert!(result.is_ok());
    }
}
```

### 2. Integration Testing

Integration tests verify interactions between components.

#### IPC Integration Tests

```rust
// tests/ipc_integration.rs
use veridian_test::*;

#[test]
fn test_ipc_message_passing() {
    let env = TestEnvironment::new();
    
    // Create two processes
    let proc1 = env.create_process("sender");
    let proc2 = env.create_process("receiver");
    
    // Create endpoint
    let endpoint = env.create_endpoint(proc2);
    
    // Grant capability to sender
    env.grant_capability(proc1, endpoint, Permissions::SEND);
    
    // Send message
    let msg = Message {
        data: b"Hello, IPC!".to_vec(),
        caps: vec![],
    };
    
    proc1.send(endpoint, msg.clone()).expect("send failed");
    
    // Receive message
    let received = proc2.receive(endpoint).expect("receive failed");
    assert_eq!(received.data, msg.data);
}

#[test]
fn test_shared_memory_concurrent_access() {
    let env = TestEnvironment::new();
    
    // Create shared memory region
    let shmem = env.create_shared_memory(PAGE_SIZE);
    
    // Map into multiple processes
    let procs: Vec<_> = (0..4)
        .map(|i| {
            let proc = env.create_process(&format!("proc{}", i));
            env.map_shared_memory(proc, shmem, Permissions::READ_WRITE);
            proc
        })
        .collect();
    
    // Concurrent writes
    let handles: Vec<_> = procs
        .into_iter()
        .enumerate()
        .map(|(i, proc)| {
            env.spawn(move || {
                let data = &mut proc.shared_memory(shmem);
                data[i] = i as u8;
            })
        })
        .collect();
    
    // Wait for completion
    for handle in handles {
        handle.join().expect("thread failed");
    }
    
    // Verify all writes succeeded
    let data = env.read_shared_memory(shmem);
    for i in 0..4 {
        assert_eq!(data[i], i as u8);
    }
}
```

#### Network Stack Integration

```rust
// tests/network_integration.rs
#[tokio::test]
async fn test_tcp_connection() {
    let mut net = NetworkTestHarness::new();
    
    // Start server
    let server = net.create_tcp_server("127.0.0.1:8080").await;
    
    // Connect client
    let client = net.create_tcp_client().await;
    client.connect("127.0.0.1:8080").await.expect("connect failed");
    
    // Accept connection
    let conn = server.accept().await.expect("accept failed");
    
    // Send data
    let data = b"Hello, Network!";
    client.send(data).await.expect("send failed");
    
    // Receive data
    let mut buf = vec![0u8; 1024];
    let n = conn.recv(&mut buf).await.expect("recv failed");
    assert_eq!(&buf[..n], data);
}

#[test]
fn test_packet_routing() {
    let mut router = TestRouter::new();
    
    // Configure routes
    router.add_route("10.0.0.0/24", Interface::Eth0);
    router.add_route("10.0.1.0/24", Interface::Eth1);
    router.add_route("0.0.0.0/0", Interface::Eth0); // Default
    
    // Test routing decisions
    assert_eq!(
        router.route_packet("10.0.0.5"),
        Some(Interface::Eth0)
    );
    assert_eq!(
        router.route_packet("10.0.1.10"),
        Some(Interface::Eth1)
    );
    assert_eq!(
        router.route_packet("8.8.8.8"),
        Some(Interface::Eth0) // Default route
    );
}
```

### 3. System Testing

System tests validate the entire OS in realistic scenarios.

#### Boot Testing

```rust
// tests/system/boot.rs
use veridian_test::qemu::*;

#[test]
fn test_basic_boot() {
    let vm = QemuVM::new()
        .memory(512)
        .cpus(2)
        .kernel("target/x86_64-veridian/release/kernel")
        .timeout(Duration::from_secs(30))
        .build();
        
    let output = vm.run_until("Init process started");
    assert!(output.contains("VeridianOS"));
    assert!(output.contains("Memory: "));
    assert!(output.contains("CPUs: 2"));
}

#[test]
fn test_multicore_boot() {
    for cpu_count in [1, 2, 4, 8, 16] {
        let vm = QemuVM::new()
            .cpus(cpu_count)
            .build();
            
        let output = vm.run_until("All CPUs online");
        assert!(output.contains(&format!("Brought up {} CPUs", cpu_count)));
    }
}
```

#### Stress Testing

```rust
// tests/system/stress.rs
#[test]
fn test_process_stress() {
    let vm = QemuVM::new().memory(2048).build();
    
    vm.run_command("stress --processes 1000 --timeout 60");
    
    // Verify system remains responsive
    let start = Instant::now();
    let output = vm.run_command("echo responsive");
    let latency = start.elapsed();
    
    assert!(output.contains("responsive"));
    assert!(latency < Duration::from_secs(1));
}

#[test]
fn test_memory_pressure() {
    let vm = QemuVM::new().memory(1024).build();
    
    // Allocate 90% of memory
    vm.run_command("stress --vm 1 --vm-bytes 900M --timeout 30");
    
    // Verify OOM killer works correctly
    let output = vm.run_command("dmesg | grep -i oom");
    assert!(output.contains("Out of memory"));
    
    // System should recover
    let output = vm.run_command("free -m");
    assert!(output.contains("available"));
}
```

### 4. Performance Testing

Performance tests ensure the system meets latency and throughput requirements.

#### Benchmark Suite

```rust
// benches/kernel_benchmarks.rs
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_syscall_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("syscall");
    
    for payload_size in [0, 64, 256, 1024, 4096] {
        group.bench_with_input(
            BenchmarkId::new("getpid", payload_size),
            &payload_size,
            |b, _| {
                b.iter(|| {
                    unsafe { syscall(SYS_GETPID) };
                });
            },
        );
        
        group.bench_with_input(
            BenchmarkId::new("ipc_send", payload_size),
            &payload_size,
            |b, &size| {
                let data = vec![0u8; size];
                b.iter(|| {
                    syscall_ipc_send(endpoint, &data);
                });
            },
        );
    }
    
    group.finish();
}

fn bench_memory_allocation(c: &mut Criterion) {
    c.bench_function("page_alloc", |b| {
        b.iter(|| {
            let page = allocate_page();
            deallocate_page(page);
        });
    });
    
    c.bench_function("huge_page_alloc", |b| {
        b.iter(|| {
            let page = allocate_huge_page();
            deallocate_huge_page(page);
        });
    });
}

criterion_group!(benches, bench_syscall_overhead, bench_memory_allocation);
criterion_main!(benches);
```

#### Latency Testing

```rust
// tests/performance/latency.rs
#[test]
fn test_interrupt_latency() {
    let mut latencies = Vec::new();
    
    for _ in 0..1000 {
        let start = rdtsc();
        trigger_test_interrupt();
        wait_for_interrupt_handler();
        let end = rdtsc();
        
        latencies.push(end - start);
    }
    
    latencies.sort();
    let p50 = latencies[500];
    let p99 = latencies[990];
    let p999 = latencies[999];
    
    // Assert latency requirements
    assert!(cycles_to_ns(p50) < 1000);   // 50th percentile < 1μs
    assert!(cycles_to_ns(p99) < 5000);   // 99th percentile < 5μs
    assert!(cycles_to_ns(p999) < 10000); // 99.9th percentile < 10μs
}
```

### 5. Security Testing

Security tests validate protection mechanisms and check for vulnerabilities.

#### Fuzzing

```rust
// fuzz/targets/syscall_fuzzer.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }
    
    // Parse syscall number and arguments from fuzz input
    let syscall_nr = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let args = &data[4..];
    
    // Set up isolated test environment
    let env = IsolatedTestEnv::new();
    
    // Execute syscall with fuzzed inputs
    let _ = env.syscall(syscall_nr, args);
    
    // Verify system integrity
    assert!(env.verify_integrity());
});
```

#### Capability Testing

```rust
// tests/security/capabilities.rs
#[test]
fn test_capability_enforcement() {
    let env = TestEnvironment::new();
    
    // Create two processes
    let proc1 = env.create_process("untrusted");
    let proc2 = env.create_process("target");
    
    // Create resource without granting capability
    let resource = env.create_resource(proc2);
    
    // Attempt unauthorized access
    let result = proc1.access_resource(resource);
    assert!(matches!(result, Err(Error::PermissionDenied)));
    
    // Grant read capability
    env.grant_capability(proc1, resource, Permissions::READ);
    
    // Read should succeed
    let result = proc1.read_resource(resource);
    assert!(result.is_ok());
    
    // Write should still fail
    let result = proc1.write_resource(resource, &[0u8]);
    assert!(matches!(result, Err(Error::PermissionDenied)));
}

#[test]
fn test_capability_revocation() {
    let env = TestEnvironment::new();
    
    // Create capability hierarchy
    let root_cap = env.create_root_capability();
    let derived_cap = env.derive_capability(root_cap, Permissions::READ);
    let sub_derived = env.derive_capability(derived_cap, Permissions::READ);
    
    // All capabilities should work
    assert!(env.use_capability(root_cap).is_ok());
    assert!(env.use_capability(derived_cap).is_ok());
    assert!(env.use_capability(sub_derived).is_ok());
    
    // Revoke middle capability
    env.revoke_capability(derived_cap);
    
    // Root should still work
    assert!(env.use_capability(root_cap).is_ok());
    
    // Derived capabilities should fail
    assert!(matches!(
        env.use_capability(derived_cap),
        Err(Error::RevokedCapability)
    ));
    assert!(matches!(
        env.use_capability(sub_derived),
        Err(Error::RevokedCapability)
    ));
}
```

### 6. Regression Testing

Regression tests ensure fixed bugs don't reappear.

```rust
// tests/regression/issue_42.rs
/// Regression test for issue #42: Race condition in scheduler
#[test]
fn test_scheduler_race_condition() {
    // This specific sequence triggered the bug
    let env = TestEnvironment::new();
    
    // Create high-priority process
    let high_prio = env.create_process("high")
        .priority(Priority::High)
        .build();
        
    // Create many normal priority processes
    let normal_procs: Vec<_> = (0..100)
        .map(|i| {
            env.create_process(&format!("normal{}", i))
                .priority(Priority::Normal)
                .build()
        })
        .collect();
    
    // Start all processes simultaneously
    high_prio.start();
    for proc in &normal_procs {
        proc.start();
    }
    
    // High priority should run first
    let first_scheduled = env.get_first_scheduled_process();
    assert_eq!(first_scheduled, high_prio.id());
    
    // Verify no crashes or deadlocks
    env.run_for(Duration::from_secs(10));
    assert!(env.is_healthy());
}
```

## Test Infrastructure

### Test Harness

```rust
// test-harness/src/lib.rs
pub struct TestEnvironment {
    kernel: TestKernel,
    processes: HashMap<ProcessId, TestProcess>,
    resources: HashMap<ResourceId, TestResource>,
}

impl TestEnvironment {
    pub fn new() -> Self {
        Self::with_config(TestConfig::default())
    }
    
    pub fn with_config(config: TestConfig) -> Self {
        let kernel = TestKernel::new(config);
        Self {
            kernel,
            processes: HashMap::new(),
            resources: HashMap::new(),
        }
    }
    
    pub fn create_process(&mut self, name: &str) -> TestProcess {
        let proc = self.kernel.create_process(name);
        self.processes.insert(proc.id(), proc.clone());
        proc
    }
    
    pub fn run_until<F>(&mut self, condition: F) -> Duration
    where
        F: Fn(&TestEnvironment) -> bool,
    {
        let start = Instant::now();
        
        while !condition(self) {
            self.kernel.tick();
            
            if start.elapsed() > Duration::from_secs(30) {
                panic!("Test timeout");
            }
        }
        
        start.elapsed()
    }
}
```

### Mock Frameworks

```rust
// test-harness/src/mocks.rs
use mockall::*;

#[automock]
pub trait FileSystem {
    fn open(&self, path: &Path, flags: OpenFlags) -> Result<FileHandle, Error>;
    fn read(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, Error>;
    fn write(&self, handle: FileHandle, buf: &[u8]) -> Result<usize, Error>;
    fn close(&self, handle: FileHandle) -> Result<(), Error>;
}

#[automock]
pub trait NetworkInterface {
    fn send_packet(&mut self, packet: &[u8]) -> Result<(), Error>;
    fn receive_packet(&mut self) -> Result<Vec<u8>, Error>;
    fn set_address(&mut self, addr: IpAddr) -> Result<(), Error>;
}
```

## Continuous Integration

### GitHub Actions Workflow

```yaml
name: Test Suite

on: [push, pull_request]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        target: [x86_64, aarch64, riscv64]
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@nightly
      - run: cargo test --target ${{ matrix.target }}-unknown-none

  integration-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@nightly
      - run: cargo test --test '*' --features integration-tests

  system-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Install QEMU
        run: sudo apt-get install -y qemu-system
      - run: cargo test --test system_* --features system-tests

  benchmarks:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@nightly
      - run: cargo bench --no-run # Build only
      - run: cargo bench -- --save-baseline pr-${{ github.event.number }}
      
  security-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Run fuzzer
        run: |
          cargo install cargo-fuzz
          cargo fuzz run syscall_fuzzer -- -max_total_time=300
```

## Test Coverage

### Coverage Requirements

- **Kernel Core**: >95% coverage
- **Drivers**: >90% coverage  
- **System Services**: >90% coverage
- **Libraries**: >95% coverage

### Coverage Reporting

```toml
# .cargo/config.toml
[target.x86_64-unknown-none]
rustflags = [
    "-C", "instrument-coverage",
    "-C", "link-arg=-Tkernel/src/arch/x86_64/linker.ld",
]

[alias]
cov = "tarpaulin --out Html --output-dir coverage"
```

## Test Documentation

Each test should be documented with:

1. **Purpose**: What the test validates
2. **Setup**: Required preconditions
3. **Steps**: What the test does
4. **Verification**: Expected outcomes
5. **Cleanup**: Any cleanup needed

Example:
```rust
/// Test: Concurrent memory allocation under pressure
/// 
/// Purpose: Verify the memory allocator handles concurrent allocation
///          requests correctly when memory is nearly exhausted.
///
/// Setup: Configure test environment with limited memory (64MB)
/// 
/// Steps:
///   1. Spawn 10 threads
///   2. Each thread allocates random-sized chunks (1-64 pages)
///   3. Randomly free allocations
///   4. Continue until OOM
///
/// Verification:
///   - No double allocations
///   - No crashes or panics
///   - Graceful OOM handling
///   - Memory properly freed
#[test]
fn test_concurrent_allocation_pressure() {
    // Test implementation...
}
```

## Best Practices

1. **Deterministic Tests**: Avoid timing-dependent tests
2. **Isolated Tests**: Each test should be independent
3. **Fast Tests**: Keep unit tests under 100ms
4. **Meaningful Names**: Test names should describe what they test
5. **Failure Messages**: Provide context on assertion failures
6. **Test Data**: Use property-based testing for better coverage
7. **Mock External Dependencies**: Don't rely on external services
8. **Clean Up**: Always clean up test resources

## Conclusion

Comprehensive testing is essential for VeridianOS's reliability and security. This strategy ensures that all components are thoroughly validated at multiple levels, from individual functions to system-wide behavior. Regular execution of these tests in CI/CD pipelines helps maintain quality throughout development.