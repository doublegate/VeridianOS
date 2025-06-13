# Troubleshooting

## Boot Issues

### Process Init Hang

**Symptoms**: Kernel boots successfully but hangs when trying to create init process

**Status**: Expected behavior in Phase 1

**Reason**: The kernel tries to create an init process but the scheduler is not yet ready to handle user-space processes. This is normal for Phase 1 completion.

**Affected Architectures**: x86_64, RISC-V

### Memory Allocator Mutex Deadlock (RESOLVED)

**Symptoms**: RISC-V kernel hangs during memory allocator initialization

**Root Cause**: Stats tracking trying to allocate memory during initialization creates deadlock

**Solution**: Skip stats updates during initialization phase:
```rust
// In frame_allocator.rs
if !self.initialized {
    return Ok(frame);  // Skip stats during init
}
```

### AArch64 Boot Failure

**Symptoms**: kernel_main not reached from _start_rust

**Status**: Under investigation

**Details**: Assembly to Rust transition issue in boot sequence

## Build Issues

### R_X86_64_32S Relocation Errors (RESOLVED)

**Symptoms**: x86_64 kernel fails to link with relocation errors

**Solution**: Use custom target JSON with kernel code model:
```bash
./build-kernel.sh x86_64 dev
```

### Double Fault on Boot (RESOLVED)

**Symptoms**: Kernel crashes immediately after boot

**Solution**: Initialize PIC with interrupts masked:
```rust
const PIC1_DATA: u16 = 0x21;
const PIC2_DATA: u16 = 0xA1;
// Mask all interrupts
outb(PIC1_DATA, 0xFF);
outb(PIC2_DATA, 0xFF);
```
