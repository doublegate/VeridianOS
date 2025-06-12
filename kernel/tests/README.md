# VeridianOS Kernel Tests

This directory contains integration tests for the VeridianOS kernel.

## Running Tests

Due to the nature of `no_std` kernel development and the use of custom targets, tests must be run individually to avoid duplicate lang item errors.

### Running a Single Test

```bash
cargo test --test <test_name> --target x86_64-unknown-none -- --nocapture
```

For example:
```bash
cargo test --test basic_boot --target x86_64-unknown-none -- --nocapture
```

### Available Tests

- `basic_boot` - Basic kernel boot and initialization tests
- `memory_tests` - Memory management subsystem tests
- `ipc_basic` - Basic IPC functionality tests
- `ipc_integration_tests` - Comprehensive IPC integration tests
- `scheduler_tests` - Scheduler and task management tests
- `process_tests` - Process lifecycle and management tests
- `should_panic` - Test framework validation (should panic)

### Running Tests with the Test Script

A convenience script is provided to run tests individually:

```bash
./run-tests.sh
```

This script will:
1. Build each test individually
2. Run it in QEMU
3. Report pass/fail status
4. Provide a summary at the end

### Test Structure

Each test file must:
1. Define `#![no_std]` and `#![no_main]`
2. Provide its own `_start` entry point
3. Define a panic handler
4. Use `harness = false` in Cargo.toml

### Known Issues

**Duplicate Lang Items Error**: This is a fundamental limitation when using `-Zbuild-std` with bare metal targets:

- Multiple versions of `core` get linked when building tests
- Dependencies (bitflags, bootloader, etc.) each bring their own `core` version
- The Rust test framework conflicts with no_std kernel development
- This affects even individual test compilation

**Current Status**: The duplicate lang items issue cannot be resolved with the current Rust toolchain and bare metal target configuration. This is a known limitation in the Rust ecosystem for no_std kernel development.

**Alternative Testing Approaches**:
1. **Unit Tests**: Use `#[cfg(test)]` modules within source files for simple unit tests
2. **Integration via QEMU**: Manual testing by running kernel binaries in QEMU
3. **Future Solution**: Wait for Rust toolchain improvements for no_std testing

This limitation does not affect the kernel's functionality - only the automated test suite.

## Writing New Tests

To add a new test:

1. Create a new file in `tests/` directory
2. Add the test configuration to `Cargo.toml`:
   ```toml
   [[test]]
   name = "your_test_name"
   harness = false
   ```

3. Structure your test file:
   ```rust
   #![no_std]
   #![no_main]

   use core::panic::PanicInfo;
   use veridian_kernel::{serial_println, exit_qemu, QemuExitCode};

   #[no_mangle]
   pub extern "C" fn _start() -> ! {
       serial_println!("Running your test...");
       // Your test code here
       exit_qemu(QemuExitCode::Success)
   }

   #[panic_handler]
   fn panic(info: &PanicInfo) -> ! {
       veridian_kernel::test_panic_handler(info)
   }
   ```

4. Add your test to the `TESTS` array in `run-tests.sh`

## Test Guidelines

1. Each test should be focused on a specific subsystem or feature
2. Use descriptive names for test functions
3. Print clear status messages during test execution
4. Always exit with appropriate exit codes
5. Handle panics appropriately (expected vs unexpected)

## Debugging Failed Tests

If a test fails:

1. Run it individually with verbose output
2. Check QEMU serial output for error messages
3. Use GDB with QEMU for debugging:
   ```bash
   qemu-system-x86_64 -s -S -kernel <test_binary>
   # In another terminal:
   gdb <test_binary>
   target remote :1234
   ```