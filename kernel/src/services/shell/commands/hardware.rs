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
