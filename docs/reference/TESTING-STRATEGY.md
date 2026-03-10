# Veridian OS Testing Strategy Guide

**Current Status:** Phase 1 COMPLETE (v0.2.1 - June 17, 2025)
- Latest release: v0.2.1 - Maintenance Release
- All three architectures (x86_64, AArch64, RISC-V) boot to Stage 6
- Zero warnings and clippy-clean across all architectures
- Ready for Phase 2 User Space Foundation development

## Table of Contents

1. [Testing Philosophy](#testing-philosophy)
1. [Testing Pyramid](#testing-pyramid)
1. [Unit Testing](#unit-testing)
1. [Integration Testing](#integration-testing)
1. [System Testing](#system-testing)
1. [Performance Testing](#performance-testing)
1. [Security Testing](#security-testing)
1. [Hardware Testing](#hardware-testing)
1. [Continuous Testing](#continuous-testing)
1. [Test Infrastructure](#test-infrastructure)

## Testing Philosophy

### Core Principles

1. **Test Early, Test Often**: Catch bugs as close to introduction as possible
1. **Automate Everything**: Manual testing should be the exception
1. **Test at Multiple Levels**: From unit to system-wide testing
1. **Performance is Correctness**: Performance regressions are bugs
1. **Security by Testing**: Security properties must be continuously validated

### Quality Metrics

|Metric                |Target|Critical Path Target|
|----------------------|------|--------------------|
|Code Coverage         |>80%  |>95%                |
|Test Success Rate     |>99.9%|100%                |
|Performance Regression|<5%   |<1%                 |
|Security Test Coverage|100%  |100%                |
|Hardware Compatibility|>90%  |>95%                |

## Testing Pyramid

```
                    ┌─────┐
                   /       \
                  /   E2E   \         5%
                 /   Tests   \
                /─────────────\
               /               \
              /  Integration    \    15%
             /      Tests        \
            /─────────────────────\
           /                       \
          /      Unit Tests         \  80%
         /───────────────────────────\
```

### Test Distribution

- **Unit Tests (80%)**: Fast, isolated component tests
- **Integration Tests (15%)**: Component interaction tests
- **End-to-End Tests (5%)**: Full system validation

## Unit Testing

### Framework Setup

```rust
// Cargo.toml
[dev-dependencies]
proptest = "1.0"
quickcheck = "1.0"
mockall = "0.11"
rstest = "0.18"
test-case = "3.0"

[profile.test]
opt-level = 2  # Optimize tests for speed
```

### Basic Unit Test Structure

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    /// Test fixture for common test setup
    struct TestFixture {
        allocator: FrameAllocator,
        memory_map: MemoryMap,
    }
    
    impl TestFixture {
        fn new() -> Self {
            Self {
                allocator: FrameAllocator::new_test(),
                memory_map: MemoryMap::empty(),
            }
        }
    }
    
    #[test]
    fn test_frame_allocation() {
        let mut fixture = TestFixture::new();
        
        // Arrange
        fixture.memory_map.add_region(MemoryRegion {
            start: PhysAddr::new(0x1000),
            end: PhysAddr::new(0x10000),
            kind: MemoryRegionKind::Usable,
        });
        
        // Act
        let frame = fixture.allocator.allocate();
        
        // Assert
        assert!(frame.is_some());
        let frame = frame.unwrap();
        assert_eq!(frame.size(), 4096);
        assert!(frame.start_address().is_aligned(4096));
    }
}
```

### Property-Based Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_capability_rights_subset(
        parent_rights in any::<u32>(),
        child_rights in any::<u32>()
    ) {
        let parent = CapabilityRights::from_bits_truncate(parent_rights);
        let child = CapabilityRights::from_bits_truncate(child_rights);
        
        // Property: derived capabilities cannot have more rights
        let derived = parent.derive(child);
        prop_assert!(derived.bits() <= parent.bits());
        prop_assert!((derived.bits() & child.bits()) == derived.bits());
    }
    
    #[test]
    fn test_memory_alignment(
        size in 1usize..=1048576,
        align in prop::sample::select(vec![1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096])
    ) {
        let layout = Layout::from_size_align(size, align);
        
        if let Ok(layout) = layout {
            let ptr = unsafe { ALLOCATOR.alloc(layout) };
            
            if !ptr.is_null() {
                prop_assert!(ptr as usize % align == 0);
                unsafe { ALLOCATOR.dealloc(ptr, layout); }
            }
        }
    }
}
```

### Parameterized Tests

```rust
use rstest::rstest;

#[rstest]
#[case(PageSize::Size4K, 4096)]
#[case(PageSize::Size2M, 2 * 1024 * 1024)]
#[case(PageSize::Size1G, 1024 * 1024 * 1024)]
fn test_page_size_calculations(
    #[case] page_size: PageSize,
    #[case] expected_bytes: usize
) {
    assert_eq!(page_size.bytes(), expected_bytes);
    assert_eq!(PageSize::from_bytes(expected_bytes), Some(page_size));
}

#[rstest]
#[case::empty_list(vec![], 0)]
#[case::single_item(vec![1], 1)]
#[case::multiple_items(vec![1, 2, 3, 4, 5], 5)]
#[case::with_duplicates(vec![1, 1, 2, 2, 3], 5)]
fn test_list_operations(
    #[case] input: Vec<i32>,
    #[case] expected_len: usize
) {
    let list = List::from_vec(input);
    assert_eq!(list.len(), expected_len);
}
```

### Mock Testing

```rust
use mockall::*;

#[automock]
trait MemoryManager {
    fn allocate(&mut self, size: usize) -> Result<*mut u8, AllocationError>;
    fn deallocate(&mut self, ptr: *mut u8, size: usize);
}

#[test]
fn test_with_mock_memory_manager() {
    let mut mock = MockMemoryManager::new();
    
    // Set expectations
    mock.expect_allocate()
        .with(eq(4096))
        .times(1)
        .returning(|_| Ok(0x1000 as *mut u8));
    
    mock.expect_deallocate()
        .with(eq(0x1000 as *mut u8), eq(4096))
        .times(1)
        .return_const(());
    
    // Use mock in test
    let result = some_function_using_memory_manager(&mut mock);
    assert!(result.is_ok());
}
```

### Testing Unsafe Code

```rust
#[cfg(test)]
mod unsafe_tests {
    use super::*;
    
    /// Tests for unsafe memory operations
    /// 
    /// SAFETY: These tests operate on controlled memory regions
    /// allocated specifically for testing. No production memory
    /// is accessed.
    #[test]
    fn test_unsafe_memory_operations() {
        // Allocate test buffer
        let mut buffer = vec![0u8; 4096];
        let ptr = buffer.as_mut_ptr();
        
        unsafe {
            // Test unaligned access handling
            let unaligned_ptr = ptr.add(1) as *mut u32;
            
            // SAFETY: We know the buffer is large enough and we're
            // testing the unaligned access handling code
            write_unaligned(unaligned_ptr, 0x12345678);
            let value = read_unaligned(unaligned_ptr);
            assert_eq!(value, 0x12345678);
        }
    }
    
    #[test]
    #[should_panic(expected = "assertion failed")]
    fn test_safety_assertions() {
        unsafe {
            // SAFETY: This intentionally violates safety to test assertions
            let null_ptr: *mut u8 = ptr::null_mut();
            // This should trigger a debug assertion
            debug_assert!(!null_ptr.is_null(), "Null pointer detected");
        }
    }
}
```

## Integration Testing

### Test Organization

```
tests/
├── common/
│   ├── mod.rs          # Common test utilities
│   ├── fixtures.rs     # Test fixtures
│   └── helpers.rs      # Helper functions
├── kernel/
│   ├── memory.rs       # Memory subsystem tests
│   ├── scheduler.rs    # Scheduler tests
│   └── ipc.rs          # IPC tests
└── system/
    ├── boot.rs         # Boot sequence tests
    └── drivers.rs      # Driver integration tests
```

### Integration Test Framework

```rust
// tests/common/mod.rs
use veridian_kernel::test_harness::*;

pub struct TestEnvironment {
    kernel: TestKernel,
    processes: Vec<TestProcess>,
    drivers: Vec<TestDriver>,
}

impl TestEnvironment {
    pub fn new() -> Self {
        let kernel = TestKernel::new()
            .with_memory_size(128 * 1024 * 1024)
            .with_cpu_count(4)
            .build();
        
        Self {
            kernel,
            processes: Vec::new(),
            drivers: Vec::new(),
        }
    }
    
    pub fn spawn_process(&mut self, name: &str) -> ProcessId {
        let process = TestProcess::new(name);
        let pid = process.id();
        self.processes.push(process);
        self.kernel.register_process(pid);
        pid
    }
}
```

### IPC Integration Tests

```rust
// tests/kernel/ipc.rs
use common::*;

#[test]
fn test_ipc_message_passing() {
    let mut env = TestEnvironment::new();
    
    // Create two processes
    let sender = env.spawn_process("sender");
    let receiver = env.spawn_process("receiver");
    
    // Create IPC channel
    let channel = env.kernel.create_channel();
    
    // Grant capabilities
    env.kernel.grant_capability(sender, channel, CapabilityRights::SEND);
    env.kernel.grant_capability(receiver, channel, CapabilityRights::RECEIVE);
    
    // Send message
    let message = Message::new(b"Hello, IPC!");
    env.kernel.send_message(sender, channel, message.clone()).unwrap();
    
    // Receive message
    let received = env.kernel.receive_message(receiver, channel).unwrap();
    
    assert_eq!(received, message);
}

#[test]
fn test_ipc_capability_passing() {
    let mut env = TestEnvironment::new();
    
    let process_a = env.spawn_process("process_a");
    let process_b = env.spawn_process("process_b");
    let process_c = env.spawn_process("process_c");
    
    // Create resource
    let resource = env.kernel.create_resource();
    
    // Give capability to A
    let cap_a = env.kernel.grant_capability(
        process_a,
        resource,
        CapabilityRights::all()
    );
    
    // A delegates to B with reduced rights
    let cap_b = env.kernel.delegate_capability(
        process_a,
        process_b,
        cap_a,
        CapabilityRights::READ
    ).unwrap();
    
    // Verify B cannot write
    let write_result = env.kernel.write_resource(process_b, cap_b, b"data");
    assert!(write_result.is_err());
    
    // Verify B can read
    let read_result = env.kernel.read_resource(process_b, cap_b);
    assert!(read_result.is_ok());
}
```

### Driver Integration Tests

```rust
// tests/system/drivers.rs
#[test]
fn test_driver_loading_and_communication() {
    let mut env = TestEnvironment::new();
    
    // Load storage driver
    let driver = TestDriver::new("storage_driver");
    let driver_id = env.kernel.load_driver(driver);
    
    // Create user process
    let app = env.spawn_process("storage_app");
    
    // Request storage access
    let storage_cap = env.kernel.request_device_access(
        app,
        DeviceClass::Storage,
        AccessMode::ReadWrite
    ).unwrap();
    
    // Perform I/O operation
    let write_data = b"Test data";
    env.kernel.device_write(app, storage_cap, 0, write_data).unwrap();
    
    let mut read_buffer = vec![0u8; write_data.len()];
    env.kernel.device_read(app, storage_cap, 0, &mut read_buffer).unwrap();
    
    assert_eq!(&read_buffer, write_data);
}
```

## System Testing

### QEMU-Based System Tests

```rust
// tests/system/boot.rs
use veridian_test::qemu::*;

#[test]
fn test_full_boot_sequence() {
    let mut qemu = QemuBuilder::new()
        .kernel_image("target/x86_64-unknown-none/release/veridian.img")
        .memory(512)
        .cpus(4)
        .timeout(Duration::from_secs(30))
        .build();
    
    qemu.start().expect("Failed to start QEMU");
    
    // Wait for boot message
    qemu.wait_for_output("Veridian OS v", Duration::from_secs(10))
        .expect("Boot message not found");
    
    // Verify kernel initialization
    qemu.wait_for_output("Kernel initialized", Duration::from_secs(5))
        .expect("Kernel initialization failed");
    
    // Test basic command
    qemu.send_input("help\n");
    qemu.wait_for_output("Available commands:", Duration::from_secs(2))
        .expect("Help command failed");
    
    // Graceful shutdown
    qemu.send_input("shutdown\n");
    qemu.wait_for_exit(Duration::from_secs(5))
        .expect("Shutdown failed");
}
```

### Multi-Architecture Testing

```rust
#[rstest]
#[case("x86_64", "target/x86_64-unknown-none/release/veridian.img")]
#[case("aarch64", "target/aarch64-unknown-none/release/veridian.img")]
#[case("riscv64", "target/riscv64gc-unknown-none-elf/release/veridian.img")]
fn test_architecture_boot(
    #[case] arch: &str,
    #[case] image_path: &str
) {
    let qemu_binary = match arch {
        "x86_64" => "qemu-system-x86_64",
        "aarch64" => "qemu-system-aarch64",
        "riscv64" => "qemu-system-riscv64",
        _ => panic!("Unknown architecture"),
    };
    
    let mut qemu = QemuBuilder::new()
        .binary(qemu_binary)
        .kernel_image(image_path)
        .architecture_defaults(arch)
        .build();
    
    qemu.start().expect("Failed to start QEMU");
    
    // Architecture-agnostic boot verification
    qemu.wait_for_output("Veridian OS", Duration::from_secs(30))
        .expect("Boot failed");
}
```

## Performance Testing

### Benchmark Framework

```rust
// benches/scheduler.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use veridian_kernel::scheduler::*;

fn benchmark_context_switch(c: &mut Criterion) {
    let mut group = c.benchmark_group("context_switch");
    
    // Test different thread counts
    for thread_count in [2, 4, 8, 16, 32].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(thread_count),
            thread_count,
            |b, &thread_count| {
                let mut scheduler = Scheduler::new();
                let threads: Vec<_> = (0..thread_count)
                    .map(|_| scheduler.create_thread())
                    .collect();
                
                b.iter(|| {
                    for _ in 0..100 {
                        let from = threads[0];
                        let to = threads[1];
                        scheduler.switch_context(from, to);
                    }
                });
            }
        );
    }
    
    group.finish();
}

fn benchmark_memory_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_allocation");
    
    // Different allocation sizes
    for size in [4096, 16384, 65536, 262144, 1048576].iter() {
        group.bench_with_input(
            BenchmarkId::new("size", size),
            size,
            |b, &size| {
                let mut allocator = FrameAllocator::new();
                
                b.iter(|| {
                    let frames: Vec<_> = (0..10)
                        .map(|_| allocator.allocate_contiguous(size / 4096))
                        .collect();
                    
                    for frame in frames {
                        if let Some(f) = frame {
                            allocator.deallocate_contiguous(f);
                        }
                    }
                });
            }
        );
    }
    
    group.finish();
}

criterion_group!(benches, benchmark_context_switch, benchmark_memory_allocation);
criterion_main!(benches);
```

### Performance Regression Detection

```yaml
# .github/workflows/performance.yml
name: Performance Tests

on:
  pull_request:
    paths:
      - 'kernel/**'
      - 'benches/**'

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Run benchmarks
        run: |
          cargo bench --all-features -- --save-baseline pr
      
      - name: Compare with main
        run: |
          git checkout main
          cargo bench --all-features -- --save-baseline main
          cargo bench --all-features -- --load-baseline main --baseline pr
      
      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: target/criterion
```

### Latency Testing

```rust
use hdrhistogram::Histogram;

#[test]
fn test_ipc_latency_distribution() {
    let mut histogram = Histogram::<u64>::new(3).unwrap();
    let mut env = TestEnvironment::new();
    
    let sender = env.spawn_process("sender");
    let receiver = env.spawn_process("receiver");
    let channel = env.kernel.create_channel();
    
    // Warm up
    for _ in 0..1000 {
        let start = Instant::now();
        env.kernel.send_message(sender, channel, Message::empty()).unwrap();
        env.kernel.receive_message(receiver, channel).unwrap();
        let _ = start.elapsed();
    }
    
    // Measure
    for _ in 0..10000 {
        let start = Instant::now();
        env.kernel.send_message(sender, channel, Message::empty()).unwrap();
        env.kernel.receive_message(receiver, channel).unwrap();
        let latency = start.elapsed().as_nanos() as u64;
        
        histogram.record(latency).unwrap();
    }
    
    // Verify latency requirements
    assert!(histogram.value_at_percentile(50.0) < 1000);  // p50 < 1µs
    assert!(histogram.value_at_percentile(95.0) < 5000);  // p95 < 5µs
    assert!(histogram.value_at_percentile(99.0) < 10000); // p99 < 10µs
    
    println!("IPC Latency Distribution:");
    println!("p50: {}ns", histogram.value_at_percentile(50.0));
    println!("p95: {}ns", histogram.value_at_percentile(95.0));
    println!("p99: {}ns", histogram.value_at_percentile(99.0));
    println!("p99.9: {}ns", histogram.value_at_percentile(99.9));
}
```

## Security Testing

### Fuzzing Setup

```rust
// fuzz/fuzz_targets/syscall_fuzzer.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use arbitrary::{Arbitrary, Unstructured};

#[derive(Debug, Arbitrary)]
struct SyscallInput {
    syscall_num: u32,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    if let Ok(input) = SyscallInput::arbitrary(&mut u) {
        // Create isolated test environment
        let env = FuzzEnvironment::new();
        
        // Execute syscall with fuzzing input
        let _ = env.execute_syscall(
            input.syscall_num,
            input.arg1,
            input.arg2,
            input.arg3,
            input.arg4,
            input.arg5,
            input.arg6,
        );
        
        // Verify no kernel panic occurred
        assert!(env.kernel_alive());
    }
});
```

### Security Property Testing

```rust
#[test]
fn test_capability_unforgeability() {
    let mut env = TestEnvironment::new();
    
    let process_a = env.spawn_process("process_a");
    let process_b = env.spawn_process("process_b");
    
    // Create resource accessible only to A
    let resource = env.kernel.create_resource();
    let cap_a = env.kernel.grant_capability(
        process_a,
        resource,
        CapabilityRights::all()
    );
    
    // Try to forge capability in process B
    for possible_cap in 0..1000000 {
        let forged_cap = CapabilityId(possible_cap);
        let result = env.kernel.use_capability(process_b, forged_cap);
        
        // Should never succeed unless it's a legitimately granted capability
        if result.is_ok() && forged_cap != cap_a {
            panic!("Capability forgery successful!");
        }
    }
}

#[test]
fn test_memory_isolation() {
    let mut env = TestEnvironment::new();
    
    let process_a = env.spawn_process("process_a");
    let process_b = env.spawn_process("process_b");
    
    // Allocate memory in process A
    let addr_a = env.kernel.allocate_memory(process_a, 4096).unwrap();
    env.kernel.write_memory(process_a, addr_a, b"secret").unwrap();
    
    // Try to read from process B
    for addr in (0..0xFFFF_FFFF_FFFF).step_by(4096) {
        let result = env.kernel.read_memory(process_b, VirtAddr::new(addr), 6);
        
        // Should never be able to read another process's memory
        if result.is_ok() {
            let data = result.unwrap();
            assert_ne!(&data, b"secret", "Memory isolation breach!");
        }
    }
}
```

### Penetration Testing

```rust
// tests/security/penetration.rs
#[test]
fn test_privilege_escalation_attempt() {
    let mut env = TestEnvironment::new();
    
    // Create unprivileged process
    let attacker = env.spawn_process("attacker");
    env.kernel.set_process_privileges(attacker, Privileges::USER);
    
    // Attempt various privilege escalation techniques
    
    // 1. Try to modify kernel memory
    let kernel_addr = VirtAddr::new(0xFFFF_8000_0000_0000);
    let result = env.kernel.write_memory(attacker, kernel_addr, b"pwned");
    assert!(result.is_err());
    
    // 2. Try to access privileged syscalls
    let result = env.kernel.execute_syscall_as(
        attacker,
        SYSCALL_LOAD_DRIVER,
        0, 0, 0, 0, 0, 0
    );
    assert_eq!(result, Err(SyscallError::PermissionDenied));
    
    // 3. Try to access other process's capabilities
    let privileged = env.spawn_process("privileged");
    let priv_cap = env.kernel.grant_capability(
        privileged,
        env.kernel.create_resource(),
        CapabilityRights::all()
    );
    
    let result = env.kernel.use_capability(attacker, priv_cap);
    assert!(result.is_err());
}
```

## Hardware Testing

### Hardware-in-the-Loop Framework

```python
# tests/hil/framework.py
import serial
import time
from typing import Optional, List

class HardwareTest:
    def __init__(self, board: str, port: str):
        self.board = board
        self.serial = serial.Serial(port, 115200, timeout=30)
        
    def flash_image(self, image_path: str) -> bool:
        """Flash OS image to hardware"""
        if self.board == "rpi4":
            return self._flash_rpi4(image_path)
        elif self.board == "rock5b":
            return self._flash_rock5b(image_path)
        else:
            raise ValueError(f"Unknown board: {self.board}")
    
    def wait_for_boot(self, timeout: float = 60) -> bool:
        """Wait for OS to boot"""
        start_time = time.time()
        
        while time.time() - start_time < timeout:
            line = self.serial.readline().decode('utf-8', errors='ignore')
            if "Veridian OS" in line:
                return True
                
        return False
    
    def run_test_suite(self, tests: List[str]) -> dict:
        """Run test suite on hardware"""
        results = {}
        
        for test in tests:
            self.serial.write(f"test {test}\n".encode())
            result = self._wait_for_result(test)
            results[test] = result
            
        return results
```

### Device Driver Testing

```rust
// tests/drivers/network.rs
#[cfg(feature = "hardware_tests")]
#[test]
fn test_real_network_driver() {
    use veridian_test::hardware::*;
    
    let mut hw_test = HardwareTest::connect("10.0.0.100:5555").unwrap();
    
    // Load network driver
    hw_test.execute("modprobe e1000e").unwrap();
    
    // Verify driver loaded
    let output = hw_test.execute("lsmod | grep e1000e").unwrap();
    assert!(output.contains("e1000e"));
    
    // Test network functionality
    hw_test.execute("ip link set eth0 up").unwrap();
    hw_test.execute("dhclient eth0").unwrap();
    
    // Verify connectivity
    let ping_result = hw_test.execute("ping -c 1 8.8.8.8").unwrap();
    assert!(ping_result.contains("1 packets transmitted, 1 received"));
    
    // Test performance
    let iperf_result = hw_test.execute("iperf3 -c 10.0.0.1 -t 10").unwrap();
    let throughput = parse_throughput(&iperf_result);
    assert!(throughput > 900_000_000, "Expected >900 Mbps, got {}", throughput);
}

#[cfg(feature = "hardware_tests")]
#[test]
fn test_storage_driver_performance() {
    let mut hw_test = HardwareTest::connect("10.0.0.101:5555").unwrap();
    
    // Test sequential write performance
    let write_result = hw_test.execute(
        "dd if=/dev/zero of=/tmp/test bs=1M count=1024 conv=fdatasync"
    ).unwrap();
    
    let write_speed = parse_dd_speed(&write_result);
    assert!(write_speed > 100_000_000, "Write speed too low: {} MB/s", write_speed / 1_000_000);
    
    // Test random I/O performance
    let fio_result = hw_test.execute(
        "fio --name=randread --ioengine=libaio --iodepth=32 \
         --rw=randread --bs=4k --direct=1 --size=1G --numjobs=4 \
         --runtime=60 --group_reporting"
    ).unwrap();
    
    let iops = parse_fio_iops(&fio_result);
    assert!(iops > 10_000, "Random read IOPS too low: {}", iops);
}

### Multi-Board Testing Matrix

```yaml
# tests/hil/test_matrix.yml
boards:
  - name: rpi4
    arch: aarch64
    connection: serial:/dev/ttyUSB0
    tests:
      - boot
      - memory
      - usb
      - network
      - gpio
    
  - name: rock5b
    arch: aarch64
    connection: network:10.0.0.102:5555
    tests:
      - boot
      - memory
      - pcie
      - nvme
      - network
    
  - name: visionfive2
    arch: riscv64
    connection: serial:/dev/ttyUSB1
    tests:
      - boot
      - memory
      - sdcard
      - network

  - name: x86_testbed
    arch: x86_64
    connection: network:10.0.0.103:5555
    tests:
      - boot
      - memory
      - sata
      - network
      - virtualization
```

## Continuous Testing

### CI/CD Test Pipeline

```yaml
# .github/workflows/continuous-testing.yml
name: Continuous Testing

on:
  push:
    branches: [main, develop]
  pull_request:
  schedule:
    - cron: '0 */6 * * *'  # Every 6 hours

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        toolchain: [stable, nightly]
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.toolchain }}
      
      - name: Run unit tests
        run: |
          cargo test --all-features --lib
          cargo test --all-features --doc
      
      - name: Generate coverage report
        if: matrix.toolchain == 'nightly'
        run: |
          cargo install cargo-tarpaulin
          cargo tarpaulin --out Html --output-dir coverage
      
      - name: Upload coverage
        uses: codecov/codecov-action@v3
        with:
          files: ./coverage/tarpaulin-report.html

  integration-tests:
    runs-on: ubuntu-latest
    needs: unit-tests
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup QEMU
        run: |
          sudo apt-get update
          sudo apt-get install -y qemu-system-x86 qemu-system-aarch64
      
      - name: Run integration tests
        run: cargo test --test '*' --features integration-tests
      
      - name: Upload test logs
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: integration-test-logs
          path: tests/logs/

  performance-tests:
    runs-on: [self-hosted, performance]
    if: github.event_name == 'pull_request'
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      
      - name: Checkout base branch
        run: git checkout ${{ github.base_ref }}
      
      - name: Run baseline benchmarks
        run: |
          cargo bench --all-features -- --save-baseline base
      
      - name: Checkout PR branch
        run: git checkout ${{ github.head_ref }}
      
      - name: Run PR benchmarks
        run: |
          cargo bench --all-features -- --baseline base
      
      - name: Comment results
        uses: actions/github-script@v6
        with:
          script: |
            const fs = require('fs');
            const results = fs.readFileSync('target/criterion/report/index.html', 'utf8');
            github.rest.issues.createComment({
              issue_number: context.issue.number,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: '## Performance Test Results\n' + results
            });

  security-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install security tools
        run: |
          cargo install cargo-audit
          cargo install cargo-geiger
      
      - name: Security audit
        run: |
          cargo audit
          cargo geiger --all-features
      
      - name: Run fuzzing
        run: |
          cargo install cargo-fuzz
          cargo fuzz run syscall_fuzzer -- -max_total_time=300
      
      - name: Upload crash artifacts
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: fuzzing-crashes
          path: fuzz/artifacts/

  hardware-tests:
    runs-on: [self-hosted, hil]
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    strategy:
      matrix:
        board: [rpi4, rock5b, visionfive2]
    steps:
      - uses: actions/checkout@v4
      
      - name: Build OS image
        run: |
          ./scripts/build-image.sh --board ${{ matrix.board }}
      
      - name: Flash and test
        run: |
          python3 tests/hil/run_tests.py \
            --board ${{ matrix.board }} \
            --image build/${{ matrix.board }}/veridian.img
      
      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: hil-results-${{ matrix.board }}
          path: tests/hil/results/
```

### Test Result Aggregation

```python
# scripts/aggregate_test_results.py
import json
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Dict, List

class TestResultAggregator:
    def __init__(self):
        self.results = {
            'unit': {},
            'integration': {},
            'performance': {},
            'security': {},
            'hardware': {}
        }
    
    def parse_junit_xml(self, xml_path: Path) -> Dict:
        """Parse JUnit XML test results"""
        tree = ET.parse(xml_path)
        root = tree.getroot()
        
        return {
            'total': int(root.get('tests', 0)),
            'passed': int(root.get('tests', 0)) - int(root.get('failures', 0)) - int(root.get('errors', 0)),
            'failed': int(root.get('failures', 0)),
            'errors': int(root.get('errors', 0)),
            'skipped': int(root.get('skipped', 0)),
            'time': float(root.get('time', 0))
        }
    
    def parse_coverage_report(self, lcov_path: Path) -> Dict:
        """Parse LCOV coverage report"""
        total_lines = 0
        covered_lines = 0
        
        with open(lcov_path, 'r') as f:
            for line in f:
                if line.startswith('LF:'):
                    total_lines += int(line.split(':')[1])
                elif line.startswith('LH:'):
                    covered_lines += int(line.split(':')[1])
        
        return {
            'lines_total': total_lines,
            'lines_covered': covered_lines,
            'coverage_percent': (covered_lines / total_lines * 100) if total_lines > 0 else 0
        }
    
    def generate_report(self) -> str:
        """Generate markdown report"""
        report = """# Test Results Summary

## Overall Status: {}

### Test Categories

| Category | Total | Passed | Failed | Coverage |
|----------|-------|--------|--------|----------|
""".format(self._overall_status())
        
        for category, data in self.results.items():
            if data:
                report += f"| {category.title()} | {data.get('total', 0)} | {data.get('passed', 0)} | {data.get('failed', 0)} | {data.get('coverage_percent', 'N/A'):.1f}% |\n"
        
        return report
    
    def _overall_status(self) -> str:
        """Determine overall test status"""
        for data in self.results.values():
            if data.get('failed', 0) > 0:
                return "❌ FAILED"
        return "✅ PASSED"
```

### Nightly Test Runs

```yaml
# .github/workflows/nightly-tests.yml
name: Nightly Comprehensive Tests

on:
  schedule:
    - cron: '0 0 * * *'  # Midnight UTC
  workflow_dispatch:

jobs:
  extended-tests:
    runs-on: ubuntu-latest
    timeout-minutes: 480  # 8 hours
    steps:
      - uses: actions/checkout@v4
      
      - name: Run extended test suite
        run: |
          cargo test --all-features --release -- --ignored
      
      - name: Long-running stress tests
        run: |
          cargo run --example stress_test -- --duration 3600
      
      - name: Memory leak detection
        run: |
          cargo test --features leak-detection -- --test-threads=1
      
      - name: Exhaustive fuzzing
        run: |
          for target in fuzz/fuzz_targets/*.rs; do
            name=$(basename $target .rs)
            cargo fuzz run $name -- -max_total_time=3600
          done
```

## Test Infrastructure

### Test Environment Management

```rust
// tests/common/environment.rs
use std::sync::Mutex;
use once_cell::sync::Lazy;

/// Global test environment manager
static TEST_ENV: Lazy<Mutex<TestEnvironmentManager>> = Lazy::new(|| {
    Mutex::new(TestEnvironmentManager::new())
});

pub struct TestEnvironmentManager {
    qemu_instances: Vec<QemuInstance>,
    temp_dirs: Vec<TempDir>,
    network_namespaces: Vec<NetworkNamespace>,
}

impl TestEnvironmentManager {
    pub fn get_isolated_environment(&mut self) -> IsolatedTestEnvironment {
        let temp_dir = TempDir::new().unwrap();
        let network_ns = NetworkNamespace::create().unwrap();
        
        let qemu = QemuInstance::new()
            .with_network_namespace(&network_ns)
            .with_temp_directory(&temp_dir)
            .spawn()
            .unwrap();
        
        self.qemu_instances.push(qemu.clone());
        self.temp_dirs.push(temp_dir.clone());
        self.network_namespaces.push(network_ns.clone());
        
        IsolatedTestEnvironment {
            qemu,
            temp_dir,
            network_ns,
        }
    }
}

impl Drop for TestEnvironmentManager {
    fn drop(&mut self) {
        // Clean up all test resources
        for qemu in &mut self.qemu_instances {
            let _ = qemu.terminate();
        }
        for ns in &self.network_namespaces {
            let _ = ns.destroy();
        }
    }
}
```

### Test Data Generation

```rust
// tests/common/generators.rs
use proptest::prelude::*;
use arbitrary::{Arbitrary, Unstructured};

/// Generate valid kernel configurations
pub fn kernel_config_strategy() -> impl Strategy<Value = KernelConfig> {
    (
        1usize..=128,  // CPU count
        (1usize..=16).prop_map(|n| n * 1024 * 1024 * 1024),  // Memory size (GB)
        bool::ANY,     // Enable SMP
        bool::ANY,     // Enable NUMA
        prop::collection::vec(device_strategy(), 0..10),  // Devices
    ).prop_map(|(cpus, memory, smp, numa, devices)| {
        KernelConfig {
            cpus,
            memory,
            smp,
            numa,
            devices,
        }
    })
}

/// Generate arbitrary but valid capability sets
pub fn capability_set_strategy() -> impl Strategy<Value = CapabilitySet> {
    prop::collection::vec(
        (0u64..1000, any::<CapabilityRights>()),
        0..20
    ).prop_map(|caps| {
        let mut set = CapabilitySet::new();
        for (id, rights) in caps {
            set.insert(CapabilityId(id), rights);
        }
        set
    })
}

/// Generate filesystem operations for testing
#[derive(Debug, Clone, Arbitrary)]
pub enum FsOperation {
    Create { path: String, content: Vec<u8> },
    Read { path: String },
    Write { path: String, content: Vec<u8> },
    Delete { path: String },
    Rename { from: String, to: String },
    Mkdir { path: String },
    List { path: String },
}

pub fn fs_operation_sequence() -> impl Strategy<Value = Vec<FsOperation>> {
    prop::collection::vec(any::<FsOperation>(), 0..100)
        .prop_filter("valid paths", |ops| {
            ops.iter().all(|op| match op {
                FsOperation::Create { path, .. } |
                FsOperation::Read { path } |
                FsOperation::Write { path, .. } |
                FsOperation::Delete { path } |
                FsOperation::Mkdir { path } |
                FsOperation::List { path } => !path.is_empty() && !path.contains('\0'),
                FsOperation::Rename { from, to } => {
                    !from.is_empty() && !to.is_empty() && 
                    !from.contains('\0') && !to.contains('\0')
                }
            })
        })
}
```

### Test Utilities

```rust
// tests/common/utils.rs

/// Retry flaky operations with exponential backoff
pub fn retry_with_backoff<T, E, F>(
    mut operation: F,
    max_attempts: u32,
    initial_delay: Duration,
) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
{
    let mut delay = initial_delay;
    
    for attempt in 0..max_attempts {
        match operation() {
            Ok(result) => return Ok(result),
            Err(e) if attempt < max_attempts - 1 => {
                thread::sleep(delay);
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    
    unreachable!()
}

/// Wait for condition with timeout
pub fn wait_for<F>(
    mut condition: F,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), TimeoutError>
where
    F: FnMut() -> bool,
{
    let start = Instant::now();
    
    while start.elapsed() < timeout {
        if condition() {
            return Ok(());
        }
        thread::sleep(poll_interval);
    }
    
    Err(TimeoutError)
}

/// Capture and assert on kernel logs
pub struct LogCapture {
    buffer: Arc<Mutex<Vec<String>>>,
}

impl LogCapture {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    pub fn assert_contains(&self, pattern: &str) {
        let logs = self.buffer.lock().unwrap();
        assert!(
            logs.iter().any(|line| line.contains(pattern)),
            "Pattern '{}' not found in logs:\n{}",
            pattern,
            logs.join("\n")
        );
    }
    
    pub fn assert_not_contains(&self, pattern: &str) {
        let logs = self.buffer.lock().unwrap();
        assert!(
            !logs.iter().any(|line| line.contains(pattern)),
            "Pattern '{}' found in logs but shouldn't be:\n{}",
            pattern,
            logs.join("\n")
        );
    }
}
```

### Test Reporting

```rust
// tests/common/reporting.rs
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct TestReport {
    pub timestamp: DateTime<Utc>,
    pub duration: Duration,
    pub environment: TestEnvironmentInfo,
    pub results: TestResults,
    pub coverage: Option<CoverageReport>,
    pub performance: Option<PerformanceReport>,
}

impl TestReport {
    pub fn generate_html(&self) -> String {
        let template = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Veridian OS Test Report</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; }
        .summary { background: #f0f0f0; padding: 10px; border-radius: 5px; }
        .passed { color: green; }
        .failed { color: red; }
        table { border-collapse: collapse; width: 100%; margin-top: 20px; }
        th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
        th { background-color: #f2f2f2; }
    </style>
</head>
<body>
    <h1>Veridian OS Test Report</h1>
    <div class="summary">
        <p><strong>Date:</strong> {{ timestamp }}</p>
        <p><strong>Duration:</strong> {{ duration }}</p>
        <p><strong>Status:</strong> <span class="{{ status_class }}">{{ status }}</span></p>
    </div>
    
    <h2>Test Results</h2>
    <table>
        <tr>
            <th>Category</th>
            <th>Total</th>
            <th>Passed</th>
            <th>Failed</th>
            <th>Skipped</th>
        </tr>
        {{ test_rows }}
    </table>
    
    {{ coverage_section }}
    {{ performance_section }}
</body>
</html>
        "#;
        
        // Render template with actual data
        self.render_template(template)
    }
}
```

## Best Practices

### Test Naming Conventions

```rust
// Good test names
#[test]
fn test_allocator_returns_aligned_memory() { }

#[test]
fn test_ipc_rejects_invalid_capability() { }

#[test]
fn test_scheduler_handles_priority_inversion() { }

// Bad test names
#[test]
fn test1() { }

#[test]
fn allocator_test() { }

#[test]
fn it_works() { }
```

### Test Organization

1. **Group related tests**: Use modules to organize tests logically
1. **Share test utilities**: Create common test helpers
1. **Isolate test data**: Each test should create its own data
1. **Clean up resources**: Use RAII patterns for test cleanup
1. **Document complex tests**: Add comments explaining test logic

### Performance Testing Guidelines

1. **Warm up before measuring**: Run operations multiple times before timing
1. **Use statistical analysis**: Don’t rely on single measurements
1. **Control for variance**: Disable CPU frequency scaling during tests
1. **Test realistic scenarios**: Use production-like workloads
1. **Track trends**: Monitor performance over time

### Security Testing Guidelines

1. **Think like an attacker**: Test boundary conditions and error paths
1. **Fuzz all inputs**: Especially system calls and IPC interfaces
1. **Test isolation**: Verify process and resource isolation
1. **Check error handling**: Ensure errors don’t leak information
1. **Validate assumptions**: Test that security invariants hold

## Conclusion

A comprehensive testing strategy is essential for building a reliable, secure, and performant operating system. Veridian OS employs multiple layers of testing, from unit tests that validate individual components to system-wide tests that ensure the OS functions correctly as a whole.

Key takeaways:

- **Automate everything**: Manual testing doesn’t scale
- **Test at multiple levels**: Each level catches different bugs
- **Measure continuously**: Track metrics over time
- **Security is paramount**: Test security properties explicitly
- **Performance matters**: Treat performance regressions as bugs

By following this testing strategy, Veridian OS can maintain high quality standards while enabling rapid development and innovation. Remember: a test that isn’t run automatically might as well not exist.

Regular review and updates of this testing strategy ensure it remains effective as the project evolves. Testing is not a phase—it’s an integral part of every aspect of OS development.