//! GDB Breakpoint and Watchpoint Management
//!
//! Implements INT3 software breakpoints (Z0/z0), hardware watchpoints via
//! x86_64 debug registers DR0-DR3 (Z2/z2 write, Z3/z3 read, Z4/z4 access),
//! and single-step via EFLAGS TF bit.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Maximum software breakpoints
const MAX_SW_BREAKPOINTS: usize = 256;

/// Maximum hardware watchpoints (DR0-DR3)
const MAX_HW_WATCHPOINTS: usize = 4;

/// Software breakpoint entry
#[derive(Debug, Clone, Copy, Default)]
struct SwBreakpoint {
    addr: u64,
    original_byte: u8,
    active: bool,
}

/// Hardware watchpoint type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchpointType {
    Write,
    Read,
    Access,
}

/// Hardware watchpoint entry
#[derive(Debug, Clone, Copy)]
struct HwWatchpoint {
    addr: u64,
    len: u8,
    wp_type: WatchpointType,
    active: bool,
}

impl Default for HwWatchpoint {
    fn default() -> Self {
        Self {
            addr: 0,
            len: 1,
            wp_type: WatchpointType::Write,
            active: false,
        }
    }
}

/// Breakpoint manager state
struct BreakpointManager {
    sw_breakpoints: [SwBreakpoint; MAX_SW_BREAKPOINTS],
    sw_count: usize,
    hw_watchpoints: [HwWatchpoint; MAX_HW_WATCHPOINTS],
}

impl BreakpointManager {
    const fn new() -> Self {
        Self {
            sw_breakpoints: [SwBreakpoint {
                addr: 0,
                original_byte: 0,
                active: false,
            }; MAX_SW_BREAKPOINTS],
            sw_count: 0,
            hw_watchpoints: [HwWatchpoint {
                addr: 0,
                len: 1,
                wp_type: WatchpointType::Write,
                active: false,
            }; MAX_HW_WATCHPOINTS],
        }
    }

    fn insert_sw_breakpoint(&mut self, addr: u64) -> bool {
        // Check for existing
        for bp in &self.sw_breakpoints[..self.sw_count] {
            if bp.addr == addr && bp.active {
                return true; // Already set
            }
        }

        if self.sw_count >= MAX_SW_BREAKPOINTS {
            return false;
        }

        // Read original byte and replace with INT3 (0xCC)
        let ptr = addr as *mut u8;
        // SAFETY: GDB has requested a breakpoint at this address. We trust that
        // GDB provides valid code addresses. The original byte is preserved for
        // restoration when the breakpoint is removed.
        let original = unsafe { core::ptr::read_volatile(ptr) };
        unsafe {
            core::ptr::write_volatile(ptr, 0xCC);
        }

        self.sw_breakpoints[self.sw_count] = SwBreakpoint {
            addr,
            original_byte: original,
            active: true,
        };
        self.sw_count += 1;
        true
    }

    fn remove_sw_breakpoint(&mut self, addr: u64) -> bool {
        for bp in &mut self.sw_breakpoints[..self.sw_count] {
            if bp.addr == addr && bp.active {
                // Restore original byte
                let ptr = addr as *mut u8;
                // SAFETY: Restoring the original instruction byte that was saved
                // when the breakpoint was inserted.
                unsafe {
                    core::ptr::write_volatile(ptr, bp.original_byte);
                }
                bp.active = false;
                return true;
            }
        }
        false
    }

    fn insert_hw_watchpoint(&mut self, addr: u64, len: u8, wp_type: WatchpointType) -> bool {
        // Find free DR slot
        for (i, wp) in self.hw_watchpoints.iter_mut().enumerate() {
            if !wp.active {
                wp.addr = addr;
                wp.len = len;
                wp.wp_type = wp_type;
                wp.active = true;

                #[cfg(all(target_arch = "x86_64", target_os = "none"))]
                set_debug_register(i, addr, len, wp_type);

                return true;
            }
        }
        false
    }

    fn remove_hw_watchpoint(&mut self, addr: u64, wp_type: WatchpointType) -> bool {
        for (i, wp) in self.hw_watchpoints.iter_mut().enumerate() {
            if wp.active && wp.addr == addr && wp.wp_type == wp_type {
                wp.active = false;

                #[cfg(all(target_arch = "x86_64", target_os = "none"))]
                clear_debug_register(i);

                return true;
            }
        }
        false
    }
}

static BP_MANAGER: spin::Mutex<BreakpointManager> = spin::Mutex::new(BreakpointManager::new());

// ---------------------------------------------------------------------------
// Debug register manipulation (x86_64 bare-metal only)
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn set_debug_register(idx: usize, addr: u64, len: u8, wp_type: WatchpointType) {
    unsafe {
        // Set address in DR0-DR3
        match idx {
            0 => core::arch::asm!("mov dr0, {}", in(reg) addr, options(nostack)),
            1 => core::arch::asm!("mov dr1, {}", in(reg) addr, options(nostack)),
            2 => core::arch::asm!("mov dr2, {}", in(reg) addr, options(nostack)),
            3 => core::arch::asm!("mov dr3, {}", in(reg) addr, options(nostack)),
            _ => return,
        }

        // Configure DR7
        let mut dr7: u64;
        core::arch::asm!("mov {}, dr7", out(reg) dr7, options(nostack));

        // Enable local breakpoint for this slot
        dr7 |= 1 << (idx * 2);

        // Set condition bits (bits 16-17 for DR0, 20-21 for DR1, etc.)
        let condition = match wp_type {
            WatchpointType::Write => 0b01,  // Write only
            WatchpointType::Read => 0b11,   // Read/Write (x86 has no read-only)
            WatchpointType::Access => 0b11, // Read/Write
        };
        let cond_shift = 16 + (idx * 4);
        dr7 &= !(0b11 << cond_shift);
        dr7 |= condition << cond_shift;

        // Set length bits
        let len_bits = match len {
            1 => 0b00,
            2 => 0b01,
            4 => 0b11,
            8 => 0b10,
            _ => 0b00,
        };
        let len_shift = 18 + (idx * 4);
        dr7 &= !(0b11 << len_shift);
        dr7 |= len_bits << len_shift;

        core::arch::asm!("mov dr7, {}", in(reg) dr7, options(nostack));
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn clear_debug_register(idx: usize) {
    unsafe {
        let mut dr7: u64;
        core::arch::asm!("mov {}, dr7", out(reg) dr7, options(nostack));

        // Disable local breakpoint
        dr7 &= !(1 << (idx * 2));

        // Clear condition and length
        let cond_shift = 16 + (idx * 4);
        dr7 &= !(0b1111 << cond_shift);

        core::arch::asm!("mov dr7, {}", in(reg) dr7, options(nostack));
    }
}

// ---------------------------------------------------------------------------
// RSP Z/z command handlers
// ---------------------------------------------------------------------------

fn parse_z_command(data: &[u8]) -> Option<(u8, u64, u64)> {
    // Format: type,addr,kind
    if data.is_empty() {
        return None;
    }

    let parts: Vec<&[u8]> = data.split(|&b| b == b',').collect();
    if parts.len() < 3 {
        return None;
    }

    let bp_type = super::gdb_stub::hex_digit(parts[0][0])?;
    let addr = super::gdb_stub::parse_hex_u64(parts[1])?;
    let kind = super::gdb_stub::parse_hex_u64(parts[2])?;

    Some((bp_type, addr, kind))
}

/// Handle Z (insert breakpoint/watchpoint) command
pub fn handle_insert(data: &[u8]) -> Option<Vec<u8>> {
    let (bp_type, addr, kind) = parse_z_command(data)?;
    let mut mgr = BP_MANAGER.lock();

    match bp_type {
        // Software breakpoint
        0 => {
            if mgr.insert_sw_breakpoint(addr) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        // Hardware breakpoint (use sw bp as fallback)
        1 => {
            if mgr.insert_sw_breakpoint(addr) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        // Write watchpoint
        2 => {
            if mgr.insert_hw_watchpoint(addr, kind as u8, WatchpointType::Write) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        // Read watchpoint
        3 => {
            if mgr.insert_hw_watchpoint(addr, kind as u8, WatchpointType::Read) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        // Access watchpoint
        4 => {
            if mgr.insert_hw_watchpoint(addr, kind as u8, WatchpointType::Access) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        _ => None,
    }
}

/// Handle z (remove breakpoint/watchpoint) command
pub fn handle_remove(data: &[u8]) -> Option<Vec<u8>> {
    let (bp_type, addr, _kind) = parse_z_command(data)?;
    let mut mgr = BP_MANAGER.lock();

    match bp_type {
        0 | 1 => {
            if mgr.remove_sw_breakpoint(addr) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        2 => {
            if mgr.remove_hw_watchpoint(addr, WatchpointType::Write) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        3 => {
            if mgr.remove_hw_watchpoint(addr, WatchpointType::Read) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        4 => {
            if mgr.remove_hw_watchpoint(addr, WatchpointType::Access) {
                Some(b"OK".to_vec())
            } else {
                Some(b"E01".to_vec())
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_breakpoint_manager_creation() {
        let mgr = BreakpointManager::new();
        assert_eq!(mgr.sw_count, 0);
        for wp in &mgr.hw_watchpoints {
            assert!(!wp.active);
        }
    }

    #[test]
    fn test_watchpoint_type_eq() {
        assert_eq!(WatchpointType::Write, WatchpointType::Write);
        assert_ne!(WatchpointType::Write, WatchpointType::Read);
        assert_ne!(WatchpointType::Read, WatchpointType::Access);
    }

    #[test]
    fn test_parse_z_command() {
        let result = parse_z_command(b"0,401000,1");
        assert!(result.is_some());
        let (tp, addr, kind) = result.unwrap();
        assert_eq!(tp, 0);
        assert_eq!(addr, 0x401000);
        assert_eq!(kind, 1);
    }

    #[test]
    fn test_parse_z_command_watchpoint() {
        let result = parse_z_command(b"2,7ffe1234,4");
        assert!(result.is_some());
        let (tp, addr, kind) = result.unwrap();
        assert_eq!(tp, 2);
        assert_eq!(addr, 0x7FFE1234);
        assert_eq!(kind, 4);
    }

    #[test]
    fn test_parse_z_command_invalid() {
        assert!(parse_z_command(b"").is_none());
        assert!(parse_z_command(b"0").is_none());
    }

    #[test]
    fn test_hw_watchpoint_slots() {
        let mut mgr = BreakpointManager::new();
        // Fill all 4 slots
        for i in 0..4 {
            assert!(mgr.insert_hw_watchpoint(0x1000 + i * 8, 4, WatchpointType::Write));
        }
        // 5th should fail
        assert!(!mgr.insert_hw_watchpoint(0x2000, 4, WatchpointType::Write));

        // Remove one and try again
        assert!(mgr.remove_hw_watchpoint(0x1000, WatchpointType::Write));
        assert!(mgr.insert_hw_watchpoint(0x2000, 4, WatchpointType::Write));
    }

    #[test]
    fn test_remove_nonexistent_watchpoint() {
        let mut mgr = BreakpointManager::new();
        assert!(!mgr.remove_hw_watchpoint(0xDEAD, WatchpointType::Write));
    }

    #[test]
    fn test_sw_breakpoint_default() {
        let bp = SwBreakpoint::default();
        assert_eq!(bp.addr, 0);
        assert_eq!(bp.original_byte, 0);
        assert!(!bp.active);
    }

    #[test]
    fn test_hw_watchpoint_default() {
        let wp = HwWatchpoint::default();
        assert_eq!(wp.addr, 0);
        assert_eq!(wp.len, 1);
        assert_eq!(wp.wp_type, WatchpointType::Write);
        assert!(!wp.active);
    }

    #[test]
    fn test_handle_insert_parse() {
        // Test parsing only (not actual insertion since that touches memory)
        let result = parse_z_command(b"2,deadbeef,8");
        assert!(result.is_some());
        let (tp, addr, kind) = result.unwrap();
        assert_eq!(tp, 2);
        assert_eq!(addr, 0xDEADBEEF);
        assert_eq!(kind, 8);
    }
}
