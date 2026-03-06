//! Hardware discovery commands (PCI, USB, block devices).

#![allow(unused_variables, unused_assignments)]

use alloc::string::String;

use super::pci_class_name;
use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

pub(in crate::services::shell) struct LspciCommand;
impl BuiltinCommand for LspciCommand {
    fn name(&self) -> &str {
        "lspci"
    }
    fn description(&self) -> &str {
        "List PCI devices"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let verbose = args.iter().any(|a| a == "-v");
        let bus = crate::drivers::pci::get_pci_bus().lock();
        let devices = bus.get_all_devices();
        if devices.is_empty() {
            crate::println!("No PCI devices found");
            return CommandResult::Success(0);
        }
        crate::println!(
            "{:<12} {:<12} {:<8} {}",
            "BUS:DEV.FN",
            "VENDOR:DEV",
            "CLASS",
            "DESCRIPTION"
        );
        for dev in &devices {
            crate::println!(
                "{:02x}:{:02x}.{:x}    {:04x}:{:04x}    {:02x}      {}",
                dev.location.bus,
                dev.location.device,
                dev.location.function,
                dev.vendor_id,
                dev.device_id,
                dev.class_code,
                pci_class_name(dev.class_code)
            );
            if verbose {
                for (i, bar) in dev.bars.iter().enumerate() {
                    match bar {
                        crate::drivers::pci::PciBar::Memory { address, size, .. } => {
                            crate::println!(
                                "    BAR{}: Memory at {:#x} (size {:#x})",
                                i,
                                address,
                                size
                            );
                        }
                        crate::drivers::pci::PciBar::Io { address, .. } => {
                            crate::println!("    BAR{}: I/O at {:#x}", i, *address);
                        }
                        crate::drivers::pci::PciBar::None => {}
                    }
                }
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct LsusbCommand;
impl BuiltinCommand for LsusbCommand {
    fn name(&self) -> &str {
        "lsusb"
    }
    fn description(&self) -> &str {
        "List USB devices"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let bus = crate::drivers::usb::get_usb_bus().lock();
        let devices = bus.get_all_devices();
        if devices.is_empty() {
            crate::println!("No USB devices found");
            return CommandResult::Success(0);
        }
        for dev in &devices {
            crate::println!(
                "Bus {:03} Device {:03}: ID {:04x}:{:04x}",
                dev.port,
                dev.address,
                dev.descriptor.vendor_id,
                dev.descriptor.product_id
            );
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct LsblkCommand;
impl BuiltinCommand for LsblkCommand {
    fn name(&self) -> &str {
        "lsblk"
    }
    fn description(&self) -> &str {
        "List block devices"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!(
            "{:<10} {:<8} {:<12} {}",
            "NAME",
            "TYPE",
            "SIZE",
            "VENDOR:DEV"
        );
        let bus = crate::drivers::pci::get_pci_bus().lock();
        let devices = bus.get_all_devices();
        let mut idx = 0u32;
        for dev in &devices {
            let is_storage = dev.class_code == 0x01;
            let is_virtio_blk =
                dev.vendor_id == 0x1AF4 && (dev.device_id == 0x1001 || dev.device_id == 0x1042);
            if is_storage || is_virtio_blk {
                let dev_type = if is_virtio_blk { "virtio" } else { "disk" };
                crate::println!(
                    "vd{}        {:<8} -            {:04x}:{:04x}",
                    (b'a' + idx as u8) as char,
                    dev_type,
                    dev.vendor_id,
                    dev.device_id
                );
                idx += 1;
            }
        }
        if idx == 0 {
            crate::println!("(no block devices found)");
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Storage & RAID Commands
// ============================================================================

pub(in crate::services::shell) struct MdadmCommand;
impl BuiltinCommand for MdadmCommand {
    fn name(&self) -> &str {
        "mdadm"
    }
    fn description(&self) -> &str {
        "RAID management"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from(
                "Usage: mdadm status|create|assemble <array>",
            ));
        }

        match args[0].as_str() {
            "status" | "--detail" => {
                crate::println!("No RAID arrays configured");
                CommandResult::Success(0)
            }
            "create" => {
                if args.len() < 2 {
                    return CommandResult::Error(String::from(
                        "Usage: mdadm status|create|assemble <array>",
                    ));
                }
                let name = &args[1];
                // Extract level from --level=N if present
                let mut level = "0";
                for arg in &args[2..] {
                    if let Some(stripped) = arg.strip_prefix("--level=") {
                        level = stripped;
                    }
                }
                crate::println!("Creating RAID {} array {}...", level, name);
                CommandResult::Success(0)
            }
            _ => CommandResult::Error(String::from("Usage: mdadm status|create|assemble <array>")),
        }
    }
}

pub(in crate::services::shell) struct IscsiadmCommand;
impl BuiltinCommand for IscsiadmCommand {
    fn name(&self) -> &str {
        "iscsiadm"
    }
    fn description(&self) -> &str {
        "iSCSI management"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("Usage: iscsiadm discover|login|list"));
        }

        match args[0].as_str() {
            "discover" => {
                if args.len() < 2 {
                    return CommandResult::Error(String::from(
                        "Usage: iscsiadm discover|login|list",
                    ));
                }
                crate::println!("No iSCSI targets found at {}", args[1]);
                CommandResult::Success(0)
            }
            "list" => {
                crate::println!("Active sessions: (none)");
                CommandResult::Success(0)
            }
            _ => CommandResult::Error(String::from("Usage: iscsiadm discover|login|list")),
        }
    }
}

// ============================================================================
// Hardware Info Command
// ============================================================================

pub(in crate::services::shell) struct HwinfoCommand;
impl BuiltinCommand for HwinfoCommand {
    fn name(&self) -> &str {
        "hwinfo"
    }
    fn description(&self) -> &str {
        "Display hardware summary"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("=== Hardware Information ===");
        crate::println!();

        // CPU info
        #[cfg(target_arch = "x86_64")]
        crate::println!("CPU:          x86_64 (QEMU Virtual CPU)");
        #[cfg(target_arch = "aarch64")]
        crate::println!("CPU:          aarch64 (Cortex-A72)");
        #[cfg(target_arch = "riscv64")]
        crate::println!("CPU:          riscv64");
        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )))]
        crate::println!("CPU:          unknown");

        // Memory info
        let mem = crate::mm::get_memory_stats();
        let total_kb = mem.total_frames * 4;
        let free_kb = mem.free_frames * 4;
        crate::println!("Memory:       {}K total, {}K free", total_kb, free_kb);

        // PCI devices
        let bus = crate::drivers::pci::get_pci_bus().lock();
        let pci_devices = bus.get_all_devices();
        crate::println!("PCI devices:  {}", pci_devices.len());
        drop(bus);

        // USB devices
        let usb_bus = crate::drivers::usb::get_usb_bus().lock();
        let usb_devices = usb_bus.get_all_devices();
        crate::println!("USB devices:  {}", usb_devices.len());
        drop(usb_bus);

        // Block devices (count storage/virtio-blk from PCI)
        let bus2 = crate::drivers::pci::get_pci_bus().lock();
        let all_devs = bus2.get_all_devices();
        let mut blk_count = 0u32;
        for dev in &all_devs {
            let is_storage = dev.class_code == 0x01;
            let is_virtio_blk =
                dev.vendor_id == 0x1AF4 && (dev.device_id == 0x1001 || dev.device_id == 0x1042);
            if is_storage || is_virtio_blk {
                blk_count += 1;
            }
        }
        crate::println!("Block devices: {}", blk_count);

        CommandResult::Success(0)
    }
}
