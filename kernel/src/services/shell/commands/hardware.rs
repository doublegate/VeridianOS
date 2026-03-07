//! Hardware discovery commands (PCI, USB, block devices).

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String};

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
                let manager = crate::drivers::raid::manager::RaidManager::new();
                let count = manager.array_count();
                if count == 0 {
                    crate::println!("No RAID arrays configured");
                } else {
                    crate::println!("{} RAID array(s) configured", count);
                }
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
                let mut level_str = "0";
                for arg in &args[2..] {
                    if let Some(stripped) = arg.strip_prefix("--level=") {
                        level_str = stripped;
                    }
                }
                let raid_level = match level_str {
                    "0" => crate::drivers::raid::manager::RaidLevel::Raid0,
                    "1" => crate::drivers::raid::manager::RaidLevel::Raid1,
                    "5" => crate::drivers::raid::manager::RaidLevel::Raid5,
                    _ => {
                        crate::println!("mdadm: unsupported RAID level '{}'", level_str);
                        return CommandResult::Error(format!(
                            "mdadm: unsupported RAID level '{}'",
                            level_str
                        ));
                    }
                };

                // Collect device args (anything not starting with --)
                let mut disks = alloc::vec::Vec::new();
                for (i, arg) in args[2..].iter().enumerate() {
                    if !arg.starts_with("--") {
                        disks.push(crate::drivers::raid::manager::RaidDisk::new(
                            i as u32,
                            arg,
                            1024 * 1024, // 1M blocks default
                        ));
                    }
                }

                let mut manager = crate::drivers::raid::manager::RaidManager::new();
                match manager.create_array(name, raid_level, disks) {
                    Ok(()) => {
                        crate::println!("mdadm: created RAID {:?} array {}", raid_level, name);
                    }
                    Err(e) => {
                        crate::println!("mdadm: create failed: {:?}", e);
                    }
                }
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
                let portal = &args[1];
                crate::println!("Discovering targets at {}...", portal);
                let mut initiator = crate::drivers::iscsi::initiator::IscsiInitiator::new(portal);
                // Need a session to run discovery; attempt login first
                let initiator_name = "iqn.2026-03.os.veridian:initiator";
                let target_name = "iqn.2026-03.os.veridian:discovery";
                match initiator.login(initiator_name, target_name) {
                    Ok(session_idx) => match initiator.discovery(session_idx) {
                        Ok(targets) => {
                            if targets.is_empty() {
                                crate::println!("No iSCSI targets found at {}", portal);
                            } else {
                                for t in &targets {
                                    crate::println!("  {}", t);
                                }
                            }
                        }
                        Err(e) => {
                            crate::println!("iSCSI discovery failed: {:?}", e);
                        }
                    },
                    Err(e) => {
                        crate::println!(
                            "iSCSI login to {} failed: {:?} (no network route)",
                            portal,
                            e
                        );
                    }
                }
                CommandResult::Success(0)
            }
            "login" => {
                if args.len() < 2 {
                    return CommandResult::Error(String::from("Usage: iscsiadm login <portal>"));
                }
                let portal = &args[1];
                let mut initiator = crate::drivers::iscsi::initiator::IscsiInitiator::new(portal);
                let initiator_name = "iqn.2026-03.os.veridian:initiator";
                let target_name = args
                    .get(2)
                    .map(|s| s.as_str())
                    .unwrap_or("iqn.2026-03.os.veridian:target0");
                match initiator.login(initiator_name, target_name) {
                    Ok(idx) => {
                        crate::println!("iSCSI session {} established to {}", idx, portal);
                    }
                    Err(e) => {
                        crate::println!("iSCSI login failed: {:?}", e);
                    }
                }
                CommandResult::Success(0)
            }
            "list" => {
                let initiator = crate::drivers::iscsi::initiator::IscsiInitiator::new("localhost");
                let count = initiator.session_count();
                if count == 0 {
                    crate::println!("Active sessions: (none)");
                } else {
                    crate::println!("Active sessions: {}", count);
                    for i in 0..count {
                        if let Some(session) = initiator.session(i) {
                            crate::println!(
                                "  [{}] {} -> {}",
                                i,
                                session.initiator_name,
                                session.target_name
                            );
                        }
                    }
                }
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
