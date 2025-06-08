# VeridianOS Device Drivers

This directory contains user-space device drivers for VeridianOS.

## Overview

In VeridianOS's microkernel architecture, all device drivers run in user space for:
- **Fault Isolation**: Driver crashes don't bring down the system
- **Security**: Drivers have only the capabilities they need
- **Flexibility**: Easy to add/remove/update drivers

## Structure

```
drivers/
├── common/            # Shared driver framework
├── storage/           # Storage device drivers
│   ├── ahci/         # AHCI/SATA driver
│   ├── nvme/         # NVMe driver
│   └── virtio-blk/   # VirtIO block driver
├── network/          # Network drivers
│   ├── e1000/        # Intel E1000 driver
│   ├── rtl8139/      # Realtek RTL8139 driver
│   └── virtio-net/   # VirtIO network driver
├── input/            # Input device drivers
│   ├── ps2/          # PS/2 keyboard/mouse
│   └── usb-hid/      # USB HID devices
├── display/          # Display drivers
│   ├── vga/          # Basic VGA driver
│   └── virtio-gpu/   # VirtIO GPU driver
└── bus/              # Bus drivers
    ├── pci/          # PCI bus driver
    └── usb/          # USB host controller
```

## Driver Framework

All drivers implement the common driver trait:

```rust
pub trait Driver: Send + Sync {
    /// Driver name
    fn name(&self) -> &str;
    
    /// Initialize the driver
    fn init(&mut self) -> Result<(), DriverError>;
    
    /// Handle device interrupt
    fn handle_interrupt(&mut self) -> Result<(), DriverError>;
    
    /// Shutdown the driver
    fn shutdown(&mut self) -> Result<(), DriverError>;
}
```

## Capabilities

Drivers require specific capabilities:
- `CAP_DEVICE`: Access to device memory/ports
- `CAP_INTERRUPT`: Register interrupt handlers
- `CAP_DMA`: Perform DMA operations
- `CAP_MMIO`: Memory-mapped I/O access

## Communication

Drivers communicate with:
- **Kernel**: Via system calls with capabilities
- **Applications**: Through IPC endpoints
- **Other Drivers**: Via the driver manager service

## Building Drivers

```bash
# Build all drivers
just build-drivers

# Build specific driver
cd drivers/storage/ahci && cargo build
```

## Testing

Each driver includes:
- Unit tests for logic
- Integration tests with mock hardware
- QEMU-based system tests

## Adding a New Driver

1. Create directory under appropriate category
2. Implement the `Driver` trait
3. Register capabilities needed
4. Add to driver manager configuration
5. Write tests and documentation

## Safety Considerations

Even in user space, drivers must:
- Validate all hardware responses
- Handle DMA buffers safely
- Implement timeouts for hardware operations
- Clean up resources on failure

## Performance

Key metrics for drivers:
- Interrupt latency: < 10μs
- DMA setup: < 50μs
- Context switch overhead: < 10μs

## Status

| Driver | Status | Phase | Notes |
|--------|--------|-------|-------|
| PCI Bus | Planned | 2 | Core infrastructure |
| AHCI | Planned | 2 | Storage support |
| E1000 | Planned | 2 | Network support |
| PS/2 | Planned | 2 | Basic input |
| VGA | Planned | 3 | Basic display |

## Resources

- [Driver Development Guide](../docs/driver-guide.md)
- [Hardware Documentation](../docs/hardware/)
- [IPC Protocol Specs](../docs/ipc-protocols/)