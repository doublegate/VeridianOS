//! Hardware discovery commands (PCI, USB, block devices).

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String, vec::Vec};

use super::pci_class_name;
use crate::{
    error::KernelError,
    services::shell::{BuiltinCommand, CommandResult, Shell},
};

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

pub(in crate::services::shell) struct NvmeCommand;
impl BuiltinCommand for NvmeCommand {
    fn name(&self) -> &str {
        "nvme"
    }
    fn description(&self) -> &str {
        "List NVMe namespaces; 'nvme selftest' does a write/read-back round trip"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        use crate::{drivers::nvme, fs::blockdev::BlockDevice};

        let count = nvme::controller_count();
        if count == 0 {
            crate::println!("nvme: no controllers");
            return CommandResult::Success(1);
        }
        if args.first().map(String::as_str) != Some("selftest") {
            for i in 0..count {
                nvme::with_controller(i, |c| {
                    crate::println!(
                        "{}: {} blocks x {} bytes",
                        c.name(),
                        c.block_count(),
                        c.block_size()
                    );
                });
            }
            return CommandResult::Success(0);
        }

        // Round trip on the last 8 KiB of nvme0n1, restoring the original
        // contents afterwards. Spans two pages, so PRP2 is exercised.
        let result = nvme::with_controller(0, |c| block_selftest(c));
        match result {
            Some(Ok(true)) => {
                crate::println!("NVME-SELFTEST: PASS");
                CommandResult::Success(0)
            }
            Some(Ok(false)) => {
                crate::println!("NVME-SELFTEST: FAIL");
                CommandResult::Success(1)
            }
            Some(Err(e)) => {
                crate::println!("nvme: selftest error: {:?}", e);
                crate::println!("NVME-SELFTEST: FAIL");
                CommandResult::Success(1)
            }
            None => CommandResult::Success(1),
        }
    }
}

/// Bytes the self-test overwrites at the end of the device: two pages, so
/// NVMe PRP2 is exercised.
const SELFTEST_SPAN: usize = 8192;

/// Write a pattern over the last `SELFTEST_SPAN` bytes of `dev`, read it
/// back, and restore the original contents, then check that out-of-range and
/// empty reads are rejected. Returns whether every check passed.
///
/// The restore runs whenever the pattern write was attempted, even if a
/// later step failed, because a failed or partial write may already have
/// changed the blocks (review of the v0.26.0 stack, PR #10).
fn block_selftest(dev: &mut dyn crate::fs::blockdev::BlockDevice) -> Result<bool, KernelError> {
    let bs = dev.block_size();
    if bs == 0 || bs > SELFTEST_SPAN || !SELFTEST_SPAN.is_multiple_of(bs) {
        return Err(KernelError::InvalidArgument {
            name: "block_size",
            value: "must divide the 8 KiB self-test span",
        });
    }
    let n = (SELFTEST_SPAN / bs) as u64;
    let lba = dev
        .block_count()
        .checked_sub(n)
        .ok_or(KernelError::InvalidArgument {
            name: "block_count",
            value: "smaller than the 8 KiB self-test span",
        })?;

    let mut first = alloc::vec![0u8; bs];
    dev.read_blocks(0, &mut first)?;
    let sig: String = first[..first.len().min(32)]
        .iter()
        .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
        .collect();
    crate::println!("NVME-SELFTEST: lba0={}", sig);

    let mut saved = alloc::vec![0u8; SELFTEST_SPAN];
    dev.read_blocks(lba, &mut saved)?;
    let pattern: Vec<u8> = (0..SELFTEST_SPAN as u32)
        .map(|i| (i.wrapping_mul(31) ^ 0x5A) as u8)
        .collect();

    let roundtrip = pattern_roundtrip(dev, lba, &pattern);
    let restore = dev.write_blocks(lba, &saved).and_then(|()| dev.flush());
    let ok = match (roundtrip, restore) {
        (Ok(ok), Ok(())) => ok,
        (Err(e), Ok(())) => {
            crate::println!("NVME-SELFTEST: round trip failed: {:?} (data restored)", e);
            return Err(e);
        }
        (Ok(_), Err(e)) => {
            crate::println!("NVME-SELFTEST: restore failed: {:?}", e);
            return Err(e);
        }
        (Err(test), Err(restore)) => {
            crate::println!(
                "NVME-SELFTEST: round trip failed: {:?}; restore failed: {:?}",
                test,
                restore
            );
            return Err(test);
        }
    };

    // Out-of-range access must fail, not wrap or underflow.
    let mut one = alloc::vec![0u8; bs];
    let oob = dev.read_blocks(dev.block_count(), &mut one).is_err();
    let empty = dev.read_blocks(0, &mut []).is_err();
    crate::println!(
        "NVME-SELFTEST: roundtrip={} oob_rejected={} empty_rejected={}",
        ok,
        oob,
        empty
    );
    Ok(ok && oob && empty)
}

/// Write `pattern` at `lba`, flush, and report whether it reads back intact.
fn pattern_roundtrip(
    dev: &mut dyn crate::fs::blockdev::BlockDevice,
    lba: u64,
    pattern: &[u8],
) -> Result<bool, KernelError> {
    dev.write_blocks(lba, pattern)?;
    dev.flush()?;
    let mut back = alloc::vec![0u8; pattern.len()];
    dev.read_blocks(lba, &mut back)?;
    Ok(back == pattern)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::blockdev::BlockDevice;

    /// In-memory disk with injectable failures.
    struct FakeDisk {
        bs: usize,
        data: Vec<u8>,
        writes: usize,
        /// Fail every read issued after this many writes.
        fail_reads_after_writes: Option<usize>,
        /// Fail the write with this ordinal (0-based).
        fail_write_n: Option<usize>,
        /// Reads return zeros instead of the data.
        corrupt_reads: bool,
    }

    impl FakeDisk {
        fn new(bs: usize, blocks: usize) -> Self {
            let data = (0..bs * blocks).map(|i| (i % 251) as u8).collect();
            Self {
                bs,
                data,
                writes: 0,
                fail_reads_after_writes: None,
                fail_write_n: None,
                corrupt_reads: false,
            }
        }

        fn io_err() -> KernelError {
            KernelError::HardwareError {
                device: "fake",
                code: 1,
            }
        }
    }

    impl BlockDevice for FakeDisk {
        fn name(&self) -> &str {
            "fake"
        }
        fn block_size(&self) -> usize {
            self.bs
        }
        fn block_count(&self) -> u64 {
            (self.data.len() / self.bs) as u64
        }
        fn read_blocks(&self, start: u64, buf: &mut [u8]) -> Result<(), KernelError> {
            if buf.is_empty() {
                return Err(Self::io_err());
            }
            if self
                .fail_reads_after_writes
                .is_some_and(|n| self.writes > n)
            {
                return Err(Self::io_err());
            }
            let off = start as usize * self.bs;
            let end = off.checked_add(buf.len()).ok_or(Self::io_err())?;
            if end > self.data.len() {
                return Err(Self::io_err());
            }
            if self.corrupt_reads && self.writes > 0 {
                buf.fill(0);
            } else {
                buf.copy_from_slice(&self.data[off..end]);
            }
            Ok(())
        }
        fn write_blocks(&mut self, start: u64, buf: &[u8]) -> Result<(), KernelError> {
            let n = self.writes;
            self.writes += 1;
            if self.fail_write_n == Some(n) {
                return Err(Self::io_err());
            }
            let off = start as usize * self.bs;
            let end = off + buf.len();
            if end > self.data.len() {
                return Err(Self::io_err());
            }
            self.data[off..end].copy_from_slice(buf);
            Ok(())
        }
    }

    #[test]
    fn selftest_passes_and_leaves_data_unchanged() {
        let mut d = FakeDisk::new(512, 64);
        let before = d.data.clone();
        assert_eq!(block_selftest(&mut d), Ok(true));
        assert_eq!(d.data, before);
    }

    #[test]
    fn selftest_restores_data_when_readback_fails() {
        let mut d = FakeDisk::new(512, 64);
        let before = d.data.clone();
        // The pattern write is write 0; the read-back after it fails.
        d.fail_reads_after_writes = Some(0);
        assert!(block_selftest(&mut d).is_err());
        assert_eq!(d.data, before, "pattern left on disk");
    }

    #[test]
    fn selftest_reports_restore_failure() {
        let mut d = FakeDisk::new(512, 64);
        // Write 1 is the restore.
        d.fail_write_n = Some(1);
        assert!(block_selftest(&mut d).is_err());
    }

    #[test]
    fn selftest_reports_mismatch_as_failure_not_success() {
        let mut d = FakeDisk::new(512, 64);
        d.corrupt_reads = true;
        assert_eq!(block_selftest(&mut d), Ok(false));
    }

    #[test]
    fn selftest_rejects_device_smaller_than_span_without_writing() {
        let mut d = FakeDisk::new(512, 8);
        d.data.truncate(512 * 8 - 512); // 7 blocks < 16
        assert!(block_selftest(&mut d).is_err());
        assert_eq!(d.writes, 0);
    }

    #[test]
    fn selftest_rejects_block_size_larger_than_span() {
        let mut d = FakeDisk::new(16384, 4);
        assert!(block_selftest(&mut d).is_err());
        assert_eq!(d.writes, 0);
    }
}
