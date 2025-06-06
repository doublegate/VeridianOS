# VeridianOS Troubleshooting Guide

## Build Issues

### Rust Toolchain Problems

#### Error: "no override and no default toolchain set"
```bash
# Solution: Install and set default toolchain
rustup toolchain install nightly-2025-01-15
rustup default nightly-2025-01-15
```

#### Error: "can't find crate for `core`"
```bash
# Solution: Add rust-src component
rustup component add rust-src
```

#### Error: "error[E0463]: can't find crate for `alloc`"
```bash
# Solution: Ensure correct target and rust-src
rustup target add x86_64-unknown-none
rustup component add rust-src
```

### Linker Errors

#### Error: "rust-lld: error: undefined symbol"
Common causes and solutions:

1. **Missing `#[no_mangle]` on entry point:**
```rust
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // ...
}
```

2. **Missing panic handler:**
```rust
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
```

3. **Missing memory functions:**
```rust
#[no_mangle]
pub extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    // Implementation
}
```

#### Error: "relocation R_X86_64_32S out of range"
This indicates position-dependent code in kernel:

```toml
# Fix in Cargo.toml
[profile.dev]
panic = "abort"

[profile.release]
panic = "abort"
```

And ensure PIC/PIE is disabled in linker script.

### Build Performance Issues

#### Slow Compilation

1. **Enable incremental compilation:**
```bash
export CARGO_INCREMENTAL=1
```

2. **Use sccache:**
```bash
cargo install sccache
export RUSTC_WRAPPER=sccache
```

3. **Reduce codegen units for dev builds:**
```toml
[profile.dev]
codegen-units = 256
```

4. **Use mold linker:**
```bash
sudo apt install mold
export RUSTFLAGS="-C link-arg=-fuse-ld=mold"
```

## Runtime Issues

### Boot Problems

#### System Hangs After "Booting kernel..."

1. **Check serial output:**
```bash
just run -- -serial stdio
```

2. **Enable early boot debugging:**
```rust
// In kernel main
pub fn kernel_main() -> ! {
    serial::init();
    serial_println!("Kernel starting...");
    // Rest of initialization
}
```

3. **Verify memory map:**
```bash
just run -- -d int,cpu_reset
```

#### Triple Fault on Boot

Common causes:

1. **Invalid GDT:**
```rust
// Ensure GDT is properly aligned and valid
#[repr(align(8))]
static GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();
```

2. **Stack issues:**
```assembly
// Ensure stack is set up properly
mov $stack_top, %rsp
and $-16, %rsp  // Align to 16 bytes
```

3. **Invalid page tables:**
   - Check page table alignment (4KB)
   - Verify no overlapping mappings
   - Ensure identity mapping for kernel

### Memory Issues

#### Page Fault

Debug page faults with detailed information:

```rust
extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let cr2: usize;
    unsafe {
        asm!("mov %cr2, {}", out(reg) cr2);
    }
    
    panic!(
        "Page fault at {:#x}\n\
         Error: {:?}\n\
         Instruction: {:#x}",
        cr2,
        error_code,
        stack_frame.instruction_pointer
    );
}
```

#### Out of Memory

1. **Check memory usage:**
```rust
fn debug_memory_usage() {
    let stats = ALLOCATOR.stats();
    serial_println!("Used: {} KB", stats.used / 1024);
    serial_println!("Free: {} KB", stats.free / 1024);
}
```

2. **Enable memory leak detection:**
```rust
#[cfg(debug_assertions)]
static ALLOCATION_TRACKER: Mutex<BTreeMap<usize, AllocationInfo>> = 
    Mutex::new(BTreeMap::new());
```

### Scheduling Issues

#### Deadlock Detection

Enable deadlock detection in debug builds:

```rust
#[cfg(debug_assertions)]
impl Scheduler {
    fn detect_deadlock(&self) {
        let waiting_graph = self.build_waiting_graph();
        if let Some(cycle) = find_cycle(&waiting_graph) {
            panic!("Deadlock detected: {:?}", cycle);
        }
    }
}
```

#### CPU Stuck at 100%

Check for infinite loops:

```rust
// Add loop detection
static LOOP_DETECTOR: AtomicU64 = AtomicU64::new(0);

fn suspicious_loop() {
    let start = LOOP_DETECTOR.fetch_add(1, Ordering::Relaxed);
    
    // Your loop here
    
    if LOOP_DETECTOR.load(Ordering::Relaxed) - start > 1_000_000 {
        panic!("Possible infinite loop detected");
    }
}
```

## QEMU Issues

### QEMU Won't Start

#### "qemu-system-x86_64: command not found"

Install QEMU:
```bash
# Ubuntu/Debian
sudo apt install qemu-system-x86

# Fedora
sudo dnf install qemu-system-x86

# macOS
brew install qemu
```

#### "Could not access KVM kernel module"

1. **Enable virtualization in BIOS**

2. **Load KVM module:**
```bash
sudo modprobe kvm
sudo modprobe kvm_intel  # or kvm_amd
```

3. **Add user to kvm group:**
```bash
sudo usermod -aG kvm $USER
# Log out and back in
```

### QEMU Debugging

#### Enable QEMU Monitor

```bash
just run -- -monitor stdio
```

QEMU monitor commands:
- `info registers` - Show CPU registers
- `info mem` - Show memory mappings
- `info tlb` - Show TLB entries
- `x/10i $eip` - Disassemble 10 instructions
- `gva2gpa 0xffff800000000000` - Virtual to physical translation

#### Enable QEMU Logging

```bash
just run -- -d int,cpu_reset,guest_errors -D qemu.log
```

Log categories:
- `int` - Interrupts
- `cpu_reset` - CPU reset
- `guest_errors` - Guest errors
- `mmu` - MMU operations
- `in_asm` - Guest assembly

## GDB Debugging

### Connection Issues

#### "Remote connection closed"

1. **Ensure QEMU is waiting for GDB:**
```bash
just run -- -s -S
```

2. **Use correct GDB architecture:**
```bash
# For x86_64
gdb-multiarch

# For AArch64
aarch64-linux-gnu-gdb
```

### GDB Scripts

Create `.gdbinit` for kernel debugging:

```gdb
# Connect to QEMU
target remote localhost:1234

# Load symbols
symbol-file target/x86_64-veridian/debug/veridian-kernel

# Useful macros
define print-page-table
    set $pml4 = $arg0
    set $i = 0
    while $i < 512
        set $entry = ((uint64_t*)$pml4)[$i]
        if $entry & 1
            printf "PML4[%d] = %p\n", $i, $entry
        end
        set $i = $i + 1
    end
end

# Break on kernel panic
break rust_panic

# Break on page fault
break page_fault_handler
```

### Common GDB Commands

```gdb
# View current instruction
x/i $rip

# View stack
x/10gx $rsp

# View page tables
monitor info tlb

# Step through instructions
si

# Continue until next breakpoint
c

# View all registers
info registers all

# Backtrace
bt

# Switch between threads/CPUs
info threads
thread 2
```

## Performance Issues

### Profiling

#### Enable Performance Counters

```rust
// Read CPU cycle counter
fn rdtsc() -> u64 {
    unsafe {
        core::arch::x86_64::_rdtsc()
    }
}

// Measure function timing
let start = rdtsc();
expensive_function();
let cycles = rdtsc() - start;
serial_println!("Function took {} cycles", cycles);
```

#### Memory Profiling

Track allocations:

```rust
static ALLOCATION_STATS: Mutex<AllocationStats> = 
    Mutex::new(AllocationStats::new());

impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = self.inner_alloc(layout);
        
        if !ptr.is_null() {
            ALLOCATION_STATS.lock().record_alloc(layout.size());
        }
        
        ptr
    }
}
```

### Optimization Tips

1. **Profile before optimizing:**
```bash
cargo build --release
just profile
```

2. **Use release mode for performance testing:**
```bash
just run-release
```

3. **Enable CPU-specific optimizations:**
```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## Common Mistakes

### Forgetting `#![no_std]`

Symptom: "can't find crate for `std`"

Solution:
```rust
#![no_std]
#![no_main]
```

### Missing Boot Stack

Symptom: Random crashes on boot

Solution:
```assembly
.section .bss
.align 16
stack_bottom:
    .space 16384  # 16KB
stack_top:
```

### Incorrect Memory Mapping

Symptom: Page faults when accessing kernel memory

Solution: Ensure kernel is mapped correctly:
```rust
// Map kernel sections
map_kernel_section(".text", EXECUTABLE);
map_kernel_section(".rodata", READABLE);
map_kernel_section(".data", READABLE | WRITABLE);
map_kernel_section(".bss", READABLE | WRITABLE);
```

### Race Conditions

Symptom: Intermittent crashes

Solution: Use proper synchronization:
```rust
// Bad
static mut COUNTER: u32 = 0;

// Good
static COUNTER: AtomicU32 = AtomicU32::new(0);
```

## Getting Help

### Debug Checklist

1. [ ] Check serial output
2. [ ] Enable debug logging
3. [ ] Run with GDB attached
4. [ ] Check QEMU logs
5. [ ] Verify memory mappings
6. [ ] Test with minimal config
7. [ ] Compare with working version

### Reporting Issues

When reporting issues, include:

1. **System information:**
```bash
rustc --version
qemu-system-x86_64 --version
uname -a
```

2. **Build output:**
```bash
cargo build --target targets/x86_64-veridian.json 2>&1 | tee build.log
```

3. **Runtime output:**
```bash
just run -- -serial stdio -d int,guest_errors 2>&1 | tee run.log
```

4. **Minimal reproduction**

### Community Resources

- Discord: [VeridianOS Discord](#)
- Forums: [VeridianOS Forums](#)
- IRC: #veridian-os on irc.libera.chat
- Stack Overflow: Tag with `veridian-os`

## Quick Reference

### Useful Commands

```bash
# Clean rebuild
just clean && just build

# Run with maximum debugging
just debug -- -d int,cpu_reset,guest_errors,mmu

# Generate and view assembly
cargo rustc -- --emit asm
find target -name "*.s" | xargs less

# Check binary size
cargo bloat --release

# Find large functions
cargo bloat --release -n 20

# View dependency tree
cargo tree

# Update dependencies safely
cargo update --dry-run
```

### Environment Variables

```bash
# Enable debug output
export RUST_LOG=debug

# Verbose cargo output
export CARGO_VERBOSE=1

# Backtrace on panic
export RUST_BACKTRACE=1

# Use specific toolchain
export RUSTUP_TOOLCHAIN=nightly-2025-01-15
```