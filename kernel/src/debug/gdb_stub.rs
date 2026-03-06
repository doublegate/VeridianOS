//! GDB Remote Serial Protocol (RSP) Stub
//!
//! Implements the GDB remote serial protocol over COM2 (I/O port 0x2F8).
//! Supports core RSP commands: register read/write, memory read/write,
//! continue/step, breakpoints, and thread queries.
//!
//! Protocol: `$packet-data#checksum` framing with `+`/`-` acknowledgment.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::sync::once_lock::OnceLock;

// COM2 I/O port base
const COM2_BASE: u16 = 0x2F8;
const COM2_DATA: u16 = COM2_BASE;
const COM2_IER: u16 = COM2_BASE + 1;
const COM2_FCR: u16 = COM2_BASE + 2;
const COM2_LCR: u16 = COM2_BASE + 3;
const COM2_MCR: u16 = COM2_BASE + 4;
const COM2_LSR: u16 = COM2_BASE + 5;
const COM2_DLL: u16 = COM2_BASE;
const COM2_DLH: u16 = COM2_BASE + 1;

// LSR bits
const LSR_DATA_READY: u8 = 0x01;
const LSR_TX_EMPTY: u8 = 0x20;

/// Maximum packet size (register dump + overhead)
const MAX_PACKET_SIZE: usize = 4096;

/// GDB is actively connected and should handle exceptions
static GDB_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Saved register state from the last exception/breakpoint
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct GdbRegisters {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cs: u64,
    pub ss: u64,
    pub ds: u64,
    pub es: u64,
    pub fs: u64,
    pub gs: u64,
}

impl GdbRegisters {
    const NUM_REGS: usize = 24;

    fn reg_by_index(&self, idx: usize) -> Option<u64> {
        match idx {
            0 => Some(self.rax),
            1 => Some(self.rbx),
            2 => Some(self.rcx),
            3 => Some(self.rdx),
            4 => Some(self.rsi),
            5 => Some(self.rdi),
            6 => Some(self.rbp),
            7 => Some(self.rsp),
            8 => Some(self.r8),
            9 => Some(self.r9),
            10 => Some(self.r10),
            11 => Some(self.r11),
            12 => Some(self.r12),
            13 => Some(self.r13),
            14 => Some(self.r14),
            15 => Some(self.r15),
            16 => Some(self.rip),
            17 => Some(self.rflags),
            18 => Some(self.cs),
            19 => Some(self.ss),
            20 => Some(self.ds),
            21 => Some(self.es),
            22 => Some(self.fs),
            23 => Some(self.gs),
            _ => None,
        }
    }

    fn set_reg_by_index(&mut self, idx: usize, val: u64) -> bool {
        match idx {
            0 => self.rax = val,
            1 => self.rbx = val,
            2 => self.rcx = val,
            3 => self.rdx = val,
            4 => self.rsi = val,
            5 => self.rdi = val,
            6 => self.rbp = val,
            7 => self.rsp = val,
            8 => self.r8 = val,
            9 => self.r9 = val,
            10 => self.r10 = val,
            11 => self.r11 = val,
            12 => self.r12 = val,
            13 => self.r13 = val,
            14 => self.r14 = val,
            15 => self.r15 = val,
            16 => self.rip = val,
            17 => self.rflags = val,
            18 => self.cs = val,
            19 => self.ss = val,
            20 => self.ds = val,
            21 => self.es = val,
            22 => self.fs = val,
            23 => self.gs = val,
            _ => return false,
        }
        true
    }
}

/// GDB stub state
struct GdbState {
    registers: GdbRegisters,
    connected: bool,
    no_ack_mode: bool,
    /// Currently selected thread for register/memory operations
    current_thread: u64,
    /// Thread enumeration state for qsThreadInfo
    thread_enum_index: usize,
    /// Cached thread IDs for enumeration
    thread_ids_cache: Vec<u64>,
}

impl GdbState {
    fn new() -> Self {
        Self {
            registers: GdbRegisters::default(),
            connected: false,
            no_ack_mode: false,
            current_thread: 1,
            thread_enum_index: 0,
            thread_ids_cache: Vec::new(),
        }
    }
}

/// Collect thread IDs from the kernel task registry
#[cfg(feature = "alloc")]
fn collect_thread_ids() -> Vec<u64> {
    // Use process table to enumerate known PIDs
    // Fall back to just thread 1 if registry is empty
    let mut ids = Vec::new();
    for pid in 1..=256u64 {
        if crate::sched::scheduler::get_task_ptr(pid).is_some() {
            ids.push(pid);
        }
    }
    if ids.is_empty() {
        alloc::vec![1] // fallback: report at least thread 1
    } else {
        ids
    }
}

/// Check if a thread exists in the task registry
fn thread_exists(tid: u64) -> bool {
    crate::sched::scheduler::get_task_ptr(tid).is_some() || tid == 1
}

/// Format a thread ID as hex bytes
#[cfg(feature = "alloc")]
fn format_thread_id_hex(tid: u64) -> Vec<u8> {
    if tid == 0 {
        return alloc::vec![b'0'];
    }
    let mut result = Vec::new();
    let val = tid;
    let mut started = false;
    for shift in (0..16).rev() {
        let nibble = ((val >> (shift * 4)) & 0xF) as u8;
        if nibble != 0 || started {
            started = true;
            result.push(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
    }
    if result.is_empty() {
        result.push(b'0');
    }
    result
}

/// Load registers from a thread's saved context
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn load_thread_registers(tid: u64) -> Option<GdbRegisters> {
    let task_ptr = crate::sched::scheduler::get_task_ptr(tid)?;
    let task = unsafe { task_ptr.as_ref() };
    match &task.context {
        crate::sched::task::TaskContext::X86_64(ctx) => Some(GdbRegisters {
            rax: ctx.rax,
            rbx: ctx.rbx,
            rcx: ctx.rcx,
            rdx: ctx.rdx,
            rsi: ctx.rsi,
            rdi: ctx.rdi,
            rbp: ctx.rbp,
            rsp: ctx.rsp,
            r8: ctx.r8,
            r9: ctx.r9,
            r10: ctx.r10,
            r11: ctx.r11,
            r12: ctx.r12,
            r13: ctx.r13,
            r14: ctx.r14,
            r15: ctx.r15,
            rip: ctx.rip,
            rflags: ctx.rflags,
            cs: ctx.cs as u64,
            ss: ctx.ss as u64,
            ds: ctx.ds as u64,
            es: ctx.es as u64,
            fs: ctx.fs as u64,
            gs: ctx.gs as u64,
        }),
    }
}

#[cfg(all(feature = "alloc", not(target_arch = "x86_64")))]
fn load_thread_registers(_tid: u64) -> Option<GdbRegisters> {
    None // GDB stub is x86_64 only
}

static GDB_STATE: OnceLock<Mutex<GdbState>> = OnceLock::new();

// ---------------------------------------------------------------------------
// COM2 low-level I/O (x86_64 only)
// ---------------------------------------------------------------------------

#[cfg(target_os = "none")]
fn com2_init() {
    unsafe {
        // Disable interrupts
        outb(COM2_IER, 0x00);
        // Enable DLAB for baud rate
        outb(COM2_LCR, 0x80);
        // 115200 baud (divisor = 1)
        outb(COM2_DLL, 0x01);
        outb(COM2_DLH, 0x00);
        // 8N1, disable DLAB
        outb(COM2_LCR, 0x03);
        // Enable FIFO, clear, 14-byte threshold
        outb(COM2_FCR, 0xC7);
        // RTS/DSR set, IRQs enabled
        outb(COM2_MCR, 0x0B);
    }
}

#[cfg(not(target_os = "none"))]
fn com2_init() {}

#[cfg(target_os = "none")]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nostack, preserves_flags));
}

#[cfg(target_os = "none")]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nostack, preserves_flags));
    val
}

#[cfg(target_os = "none")]
fn com2_read_byte() -> u8 {
    unsafe {
        // Wait for data ready
        while (inb(COM2_LSR) & LSR_DATA_READY) == 0 {
            core::hint::spin_loop();
        }
        inb(COM2_DATA)
    }
}

#[cfg(target_os = "none")]
fn com2_write_byte(byte: u8) {
    unsafe {
        // Wait for TX empty
        while (inb(COM2_LSR) & LSR_TX_EMPTY) == 0 {
            core::hint::spin_loop();
        }
        outb(COM2_DATA, byte);
    }
}

#[cfg(target_os = "none")]
fn _com2_data_available() -> bool {
    unsafe { (inb(COM2_LSR) & LSR_DATA_READY) != 0 }
}

#[cfg(not(target_os = "none"))]
fn com2_read_byte() -> u8 {
    0
}

#[cfg(not(target_os = "none"))]
fn com2_write_byte(_byte: u8) {}

// ---------------------------------------------------------------------------
// RSP packet framing
// ---------------------------------------------------------------------------

fn compute_checksum(data: &[u8]) -> u8 {
    let mut sum: u8 = 0;
    for &b in data {
        sum = sum.wrapping_add(b);
    }
    sum
}

fn hex_char(nibble: u8) -> u8 {
    let n = nibble & 0x0F;
    if n < 10 {
        b'0' + n
    } else {
        b'a' + (n - 10)
    }
}

pub(crate) fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_to_u8(hi: u8, lo: u8) -> Option<u8> {
    let h = hex_digit(hi)?;
    let l = hex_digit(lo)?;
    Some((h << 4) | l)
}

/// Parse a hex string into a u64
pub(crate) fn parse_hex_u64(s: &[u8]) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut val: u64 = 0;
    for &c in s {
        let d = hex_digit(c)?;
        val = val.checked_shl(4)?.wrapping_add(d as u64);
    }
    Some(val)
}

/// Receive a single RSP packet (blocking).
/// Returns the packet data (between `$` and `#`), or None on error.
fn receive_packet() -> Option<Vec<u8>> {
    // Wait for '$'
    loop {
        let c = com2_read_byte();
        if c == b'$' {
            break;
        }
        if c == 0x03 {
            // Ctrl-C: interrupt
            return Some(alloc::vec![0x03]);
        }
    }

    let mut buf = Vec::with_capacity(MAX_PACKET_SIZE);

    // Read until '#'
    loop {
        let c = com2_read_byte();
        if c == b'#' {
            break;
        }
        if buf.len() >= MAX_PACKET_SIZE {
            return None;
        }
        buf.push(c);
    }

    // Read 2-char hex checksum
    let hi = com2_read_byte();
    let lo = com2_read_byte();
    let checksum_received = hex_to_u8(hi, lo).unwrap_or(0);

    let computed = compute_checksum(&buf);
    if computed == checksum_received {
        // ACK
        com2_write_byte(b'+');
        Some(buf)
    } else {
        // NAK
        com2_write_byte(b'-');
        None
    }
}

/// Send an RSP packet
fn send_packet(data: &[u8]) {
    com2_write_byte(b'$');
    for &b in data {
        com2_write_byte(b);
    }
    com2_write_byte(b'#');
    let cksum = compute_checksum(data);
    com2_write_byte(hex_char(cksum >> 4));
    com2_write_byte(hex_char(cksum & 0x0F));
}

/// Send an OK response
fn send_ok() {
    send_packet(b"OK");
}

/// Send an error response
fn send_error(code: u8) {
    let mut buf = [b'E', 0, 0];
    buf[1] = hex_char(code >> 4);
    buf[2] = hex_char(code & 0x0F);
    send_packet(&buf);
}

/// Send an empty response (unsupported command)
fn send_empty() {
    send_packet(b"");
}

// ---------------------------------------------------------------------------
// Hex encoding helpers
// ---------------------------------------------------------------------------

fn encode_hex_u64(val: u64, buf: &mut Vec<u8>) {
    // GDB expects little-endian byte order for register values
    for i in 0..8 {
        let byte = ((val >> (i * 8)) & 0xFF) as u8;
        buf.push(hex_char(byte >> 4));
        buf.push(hex_char(byte & 0x0F));
    }
}

fn decode_hex_u64_le(data: &[u8]) -> Option<u64> {
    if data.len() < 16 {
        return None;
    }
    let mut val: u64 = 0;
    for i in 0..8 {
        let byte = hex_to_u8(data[i * 2], data[i * 2 + 1])?;
        val |= (byte as u64) << (i * 8);
    }
    Some(val)
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

/// Handle 'g' command: read all registers
fn handle_read_registers(state: &GdbState) -> Vec<u8> {
    let mut buf = Vec::with_capacity(GdbRegisters::NUM_REGS * 16);
    for i in 0..GdbRegisters::NUM_REGS {
        if let Some(val) = state.registers.reg_by_index(i) {
            encode_hex_u64(val, &mut buf);
        }
    }
    buf
}

/// Handle 'G' command: write all registers
fn handle_write_registers(state: &mut GdbState, data: &[u8]) -> bool {
    if data.len() < GdbRegisters::NUM_REGS * 16 {
        return false;
    }
    for i in 0..GdbRegisters::NUM_REGS {
        let offset = i * 16;
        if let Some(val) = decode_hex_u64_le(&data[offset..offset + 16]) {
            state.registers.set_reg_by_index(i, val);
        }
    }
    true
}

/// Handle 'p' command: read single register
fn handle_read_single_reg(state: &GdbState, data: &[u8]) -> Option<Vec<u8>> {
    let reg_num = parse_hex_u64(data)? as usize;
    let val = state.registers.reg_by_index(reg_num)?;
    let mut buf = Vec::with_capacity(16);
    encode_hex_u64(val, &mut buf);
    Some(buf)
}

/// Handle 'P' command: write single register
fn handle_write_single_reg(state: &mut GdbState, data: &[u8]) -> bool {
    // Format: Pnn=rrrrrrrrrrrrrrrrr
    let eq_pos = data.iter().position(|&c| c == b'=');
    let eq_pos = match eq_pos {
        Some(p) => p,
        None => return false,
    };

    let reg_num = match parse_hex_u64(&data[..eq_pos]) {
        Some(n) => n as usize,
        None => return false,
    };

    let val = match decode_hex_u64_le(&data[eq_pos + 1..]) {
        Some(v) => v,
        None => return false,
    };

    state.registers.set_reg_by_index(reg_num, val)
}

/// Handle 'm' command: read memory
fn handle_read_memory(data: &[u8]) -> Option<Vec<u8>> {
    // Format: maddr,length
    let comma = data.iter().position(|&c| c == b',')?;
    let addr = parse_hex_u64(&data[..comma])?;
    let len = parse_hex_u64(&data[comma + 1..])? as usize;

    if len > MAX_PACKET_SIZE / 2 {
        return None;
    }

    let mut buf = Vec::with_capacity(len * 2);

    for i in 0..len {
        let ptr = (addr + i as u64) as *const u8;
        // SAFETY: We read from the address GDB requested. If the address is
        // invalid, we may fault -- the page fault handler should catch this
        // in a production stub. For now, we do a best-effort read.
        let byte = unsafe { core::ptr::read_volatile(ptr) };
        buf.push(hex_char(byte >> 4));
        buf.push(hex_char(byte & 0x0F));
    }

    Some(buf)
}

/// Handle 'M' command: write memory
fn handle_write_memory(data: &[u8]) -> bool {
    // Format: Maddr,length:XX...
    let comma = match data.iter().position(|&c| c == b',') {
        Some(p) => p,
        None => return false,
    };
    let colon = match data.iter().position(|&c| c == b':') {
        Some(p) => p,
        None => return false,
    };

    let addr = match parse_hex_u64(&data[..comma]) {
        Some(a) => a,
        None => return false,
    };
    let len = match parse_hex_u64(&data[comma + 1..colon]) {
        Some(l) => l as usize,
        None => return false,
    };

    let hex_data = &data[colon + 1..];
    if hex_data.len() < len * 2 {
        return false;
    }

    for i in 0..len {
        let byte = match hex_to_u8(hex_data[i * 2], hex_data[i * 2 + 1]) {
            Some(b) => b,
            None => return false,
        };
        let ptr = (addr + i as u64) as *mut u8;
        // SAFETY: Writing to the address GDB requested. Same caveats as read.
        unsafe {
            core::ptr::write_volatile(ptr, byte);
        }
    }

    true
}

/// Handle '?' command: halt reason
fn handle_halt_reason() -> Vec<u8> {
    // Signal 5 = SIGTRAP (breakpoint)
    alloc::vec![b'S', b'0', b'5']
}

/// Handle 'q' queries
fn handle_query(_state: &mut GdbState, data: &[u8]) -> Option<Vec<u8>> {
    if data.starts_with(b"Supported") {
        return Some(
            b"PacketSize=1000;QStartNoAckMode+;qXfer:features:read+;multiprocess-".to_vec(),
        );
    }
    if data.starts_with(b"Attached") {
        return Some(b"1".to_vec());
    }
    if data == b"fThreadInfo" {
        // Enumerate all threads from task registry
        _state.thread_ids_cache = collect_thread_ids();
        _state.thread_enum_index = 0;
        if _state.thread_ids_cache.is_empty() {
            return Some(b"l".to_vec());
        }
        let mut response = alloc::vec![b'm'];
        for (i, &tid) in _state.thread_ids_cache.iter().enumerate() {
            if i > 0 {
                response.push(b',');
            }
            response.extend_from_slice(&format_thread_id_hex(tid));
        }
        _state.thread_enum_index = _state.thread_ids_cache.len();
        return Some(response);
    }
    if data == b"sThreadInfo" {
        // All threads reported in fThreadInfo
        return Some(b"l".to_vec());
    }
    if data == b"C" {
        // Current thread ID
        let mut resp = b"QC".to_vec();
        resp.extend_from_slice(&format_thread_id_hex(_state.current_thread));
        return Some(resp);
    }
    if data.starts_with(b"Xfer:features:read:target.xml:") {
        let xml = b"l<?xml version=\"1.0\"?>\
            <!DOCTYPE target SYSTEM \"gdb-target.dtd\">\
            <target version=\"1.0\">\
            <architecture>i386:x86-64</architecture>\
            </target>";
        return Some(xml.to_vec());
    }

    // Check for QStartNoAckMode
    if data.starts_with(b"StartNoAckMode") {
        // This is a 'Q' command, not 'q', handled separately
    }

    None
}

/// Handle 'Q' set commands
fn handle_set_command(state: &mut GdbState, data: &[u8]) -> Option<Vec<u8>> {
    if data.starts_with(b"StartNoAckMode") {
        state.no_ack_mode = true;
        return Some(b"OK".to_vec());
    }
    None
}

/// Handle 'H' command: set thread for subsequent operations
fn handle_set_thread(state: &mut GdbState, data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return b"E01".to_vec();
    }
    let _op = data[0]; // 'g' for register ops, 'c' for continue ops
    let tid_str = &data[1..];
    if tid_str.is_empty() {
        return b"E01".to_vec();
    }

    // Parse thread ID (hex)
    let tid = if tid_str == b"-1" {
        // -1 means "all threads"
        0xFFFF_FFFF_FFFF_FFFF
    } else {
        let mut val = 0u64;
        for &b in tid_str {
            let nibble = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return b"E01".to_vec(),
            };
            val = val.wrapping_shl(4) | nibble as u64;
        }
        val
    };

    // 0 means "any thread", -1 means "all threads" — both accepted
    if tid == 0 || tid == 0xFFFF_FFFF_FFFF_FFFF {
        // Keep current thread unchanged
        return b"OK".to_vec();
    }

    // Validate thread exists
    if thread_exists(tid) {
        state.current_thread = tid;
        b"OK".to_vec()
    } else {
        b"E01".to_vec()
    }
}

/// Handle 'T' command: is thread alive?
fn handle_thread_alive(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return b"OK".to_vec();
    }
    let mut tid = 0u64;
    for &b in data {
        let nibble = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return b"E01".to_vec(),
        };
        tid = tid.wrapping_shl(4) | nibble as u64;
    }
    if thread_exists(tid) {
        b"OK".to_vec()
    } else {
        b"E01".to_vec()
    }
}

/// Handle 'vAttach;pid': attach to a process
#[cfg(feature = "alloc")]
fn handle_vattach(state: &mut GdbState, data: &[u8]) -> Vec<u8> {
    // Parse PID from hex after "Attach;"
    if !data.starts_with(b"Attach;") {
        return b"E01".to_vec();
    }
    let pid_hex = &data[7..];
    let mut pid = 0u64;
    for &b in pid_hex {
        let nibble = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return b"E01".to_vec(),
        };
        pid = pid.wrapping_shl(4) | nibble as u64;
    }
    if thread_exists(pid) {
        state.current_thread = pid;
        b"S05".to_vec() // SIGTRAP stop reply
    } else {
        b"E01".to_vec()
    }
}

/// Handle 'vKill;pid': kill a process
#[cfg(feature = "alloc")]
fn handle_vkill(data: &[u8]) -> Vec<u8> {
    if !data.starts_with(b"Kill;") {
        return b"E01".to_vec();
    }
    let pid_hex = &data[5..];
    let mut pid = 0u64;
    for &b in pid_hex {
        let nibble = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return b"E01".to_vec(),
        };
        pid = pid.wrapping_shl(4) | nibble as u64;
    }
    // Signal the process to terminate
    let process_id = crate::process::pcb::ProcessId(pid);
    if crate::process::exit::kill_process(process_id, 9).is_ok() {
        b"OK".to_vec()
    } else {
        b"E01".to_vec()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the GDB stub on COM2
pub fn gdb_init() {
    com2_init();

    let state = GdbState::new();
    let _ = GDB_STATE.set(Mutex::new(state));

    #[cfg(target_os = "none")]
    crate::serial_println!("[GDB] Stub initialized on COM2 (0x2F8), waiting for connection...");
}

/// Check if GDB is active
pub fn is_gdb_active() -> bool {
    GDB_ACTIVE.load(Ordering::Relaxed)
}

/// Handle an exception/trap by entering GDB command loop.
/// `signal` is the Unix signal number (e.g., 5 for SIGTRAP).
/// `regs` provides the saved register context.
pub fn gdb_handle_exception(signal: u8, regs: &mut GdbRegisters) {
    let state_lock = match GDB_STATE.get() {
        Some(s) => s,
        None => return,
    };

    {
        let mut state = state_lock.lock();
        state.registers = *regs;
        state.connected = true;
    }
    GDB_ACTIVE.store(true, Ordering::Release);

    // Send stop reply
    let sig_reply = [b'S', hex_char(signal >> 4), hex_char(signal & 0x0F)];
    send_packet(&sig_reply);

    // Command loop
    loop {
        let packet = match receive_packet() {
            Some(p) => p,
            None => continue,
        };

        if packet.is_empty() {
            send_empty();
            continue;
        }

        let cmd = packet[0];
        let args = &packet[1..];

        match cmd {
            // Ctrl-C interrupt
            0x03 => {
                let reply = handle_halt_reason();
                send_packet(&reply);
            }
            // '?' - halt reason
            b'?' => {
                let reply = handle_halt_reason();
                send_packet(&reply);
            }
            // 'g' - read registers
            b'g' => {
                let state = state_lock.lock();
                let reply = handle_read_registers(&state);
                send_packet(&reply);
            }
            // 'G' - write registers
            b'G' => {
                let mut state = state_lock.lock();
                if handle_write_registers(&mut state, args) {
                    send_ok();
                } else {
                    send_error(0x01);
                }
            }
            // 'p' - read single register
            b'p' => {
                let state = state_lock.lock();
                match handle_read_single_reg(&state, args) {
                    Some(reply) => send_packet(&reply),
                    None => send_error(0x01),
                }
            }
            // 'P' - write single register
            b'P' => {
                let mut state = state_lock.lock();
                if handle_write_single_reg(&mut state, args) {
                    send_ok();
                } else {
                    send_error(0x01);
                }
            }
            // 'm' - read memory
            b'm' => match handle_read_memory(args) {
                Some(reply) => send_packet(&reply),
                None => send_error(0x01),
            },
            // 'M' - write memory
            b'M' => {
                if handle_write_memory(args) {
                    send_ok();
                } else {
                    send_error(0x01);
                }
            }
            // 'c' - continue execution
            b'c' => {
                // Optional address argument
                if !args.is_empty() {
                    if let Some(addr) = parse_hex_u64(args) {
                        let mut state = state_lock.lock();
                        state.registers.rip = addr;
                    }
                }
                // Update regs and return to execution
                let state = state_lock.lock();
                *regs = state.registers;
                return;
            }
            // 's' - single step
            b's' => {
                // Set TF (trap flag) in RFLAGS for single-step
                let mut state = state_lock.lock();
                if !args.is_empty() {
                    if let Some(addr) = parse_hex_u64(args) {
                        state.registers.rip = addr;
                    }
                }
                state.registers.rflags |= 0x100; // TF bit
                *regs = state.registers;
                return;
            }
            // 'D' - detach
            b'D' => {
                send_ok();
                GDB_ACTIVE.store(false, Ordering::Release);
                let mut state = state_lock.lock();
                state.connected = false;
                *regs = state.registers;
                return;
            }
            // 'k' - kill
            b'k' => {
                GDB_ACTIVE.store(false, Ordering::Release);
                let mut state = state_lock.lock();
                state.connected = false;
                *regs = state.registers;
                return;
            }
            // 'H' - set thread
            b'H' => {
                let mut state = state_lock.lock();
                let reply = handle_set_thread(&mut state, args);
                send_packet(&reply);
            }
            // 'T' - thread alive
            b'T' => {
                let reply = handle_thread_alive(args);
                send_packet(&reply);
            }
            // 'v' - extended/verbose commands
            b'v' => {
                if args.starts_with(b"Cont?") {
                    send_packet(b"vCont;c;s;t");
                } else if args.starts_with(b"Cont;c") {
                    let state = state_lock.lock();
                    *regs = state.registers;
                    return;
                } else if args.starts_with(b"Cont;s") {
                    let mut state = state_lock.lock();
                    state.registers.rflags |= 0x100;
                    *regs = state.registers;
                    return;
                } else if args.starts_with(b"Kill;") {
                    let reply = handle_vkill(args);
                    send_packet(&reply);
                    GDB_ACTIVE.store(false, Ordering::Release);
                    let mut state = state_lock.lock();
                    state.connected = false;
                    *regs = state.registers;
                    return;
                } else if args.starts_with(b"Attach;") {
                    let mut state = state_lock.lock();
                    let reply = handle_vattach(&mut state, args);
                    send_packet(&reply);
                } else {
                    send_empty();
                }
            }
            // 'q' - general query
            b'q' => {
                let mut state = state_lock.lock();
                match handle_query(&mut state, args) {
                    Some(reply) => send_packet(&reply),
                    None => send_empty(),
                }
            }
            // 'Q' - general set
            b'Q' => {
                let mut state = state_lock.lock();
                match handle_set_command(&mut state, args) {
                    Some(reply) => send_packet(&reply),
                    None => send_empty(),
                }
            }
            // 'Z' - insert breakpoint/watchpoint
            b'Z' => {
                if let Some(reply) = crate::debug::breakpoint::handle_insert(args) {
                    send_packet(&reply);
                } else {
                    send_empty();
                }
            }
            // 'z' - remove breakpoint/watchpoint
            b'z' => {
                if let Some(reply) = crate::debug::breakpoint::handle_remove(args) {
                    send_packet(&reply);
                } else {
                    send_empty();
                }
            }
            _ => {
                send_empty();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_compute_checksum() {
        assert_eq!(compute_checksum(b"OK"), b'O'.wrapping_add(b'K'));
        assert_eq!(compute_checksum(b""), 0);
        assert_eq!(
            compute_checksum(b"S05"),
            b'S'.wrapping_add(b'0').wrapping_add(b'5')
        );
    }

    #[test]
    fn test_hex_char() {
        assert_eq!(hex_char(0), b'0');
        assert_eq!(hex_char(9), b'9');
        assert_eq!(hex_char(10), b'a');
        assert_eq!(hex_char(15), b'f');
    }

    #[test]
    fn test_hex_digit() {
        assert_eq!(hex_digit(b'0'), Some(0));
        assert_eq!(hex_digit(b'9'), Some(9));
        assert_eq!(hex_digit(b'a'), Some(10));
        assert_eq!(hex_digit(b'f'), Some(15));
        assert_eq!(hex_digit(b'A'), Some(10));
        assert_eq!(hex_digit(b'F'), Some(15));
        assert_eq!(hex_digit(b'g'), None);
    }

    #[test]
    fn test_hex_to_u8() {
        assert_eq!(hex_to_u8(b'0', b'0'), Some(0x00));
        assert_eq!(hex_to_u8(b'f', b'f'), Some(0xFF));
        assert_eq!(hex_to_u8(b'4', b'2'), Some(0x42));
        assert_eq!(hex_to_u8(b'g', b'0'), None);
    }

    #[test]
    fn test_parse_hex_u64() {
        assert_eq!(parse_hex_u64(b"0"), Some(0));
        assert_eq!(parse_hex_u64(b"ff"), Some(0xFF));
        assert_eq!(parse_hex_u64(b"deadbeef"), Some(0xDEADBEEF));
        assert_eq!(parse_hex_u64(b""), None);
    }

    #[test]
    fn test_encode_decode_u64_le() {
        let val: u64 = 0x0102030405060708;
        let mut buf = Vec::new();
        encode_hex_u64(val, &mut buf);
        let decoded = decode_hex_u64_le(&buf);
        assert_eq!(decoded, Some(val));
    }

    #[test]
    fn test_registers_default() {
        let regs = GdbRegisters::default();
        assert_eq!(regs.rax, 0);
        assert_eq!(regs.rip, 0);
        assert_eq!(regs.rflags, 0);
    }

    #[test]
    fn test_register_read_write() {
        let mut regs = GdbRegisters::default();
        assert!(regs.set_reg_by_index(0, 0x42));
        assert_eq!(regs.reg_by_index(0), Some(0x42));
        assert_eq!(regs.rax, 0x42);

        assert!(regs.set_reg_by_index(16, 0xDEAD));
        assert_eq!(regs.rip, 0xDEAD);

        assert!(!regs.set_reg_by_index(99, 0));
        assert_eq!(regs.reg_by_index(99), None);
    }

    #[test]
    fn test_handle_read_registers() {
        let state = GdbState::new();
        let reply = handle_read_registers(&state);
        assert_eq!(reply.len(), GdbRegisters::NUM_REGS * 16);
        // All zeros
        assert!(reply.iter().all(|&b| b == b'0'));
    }

    #[test]
    fn test_handle_write_registers() {
        let mut state = GdbState::new();
        // Write register 0 (rax) = 0x0102030405060708 in LE hex
        let mut data = Vec::new();
        for _ in 0..GdbRegisters::NUM_REGS {
            encode_hex_u64(0x42, &mut data);
        }
        assert!(handle_write_registers(&mut state, &data));
        assert_eq!(state.registers.rax, 0x42);
        assert_eq!(state.registers.rip, 0x42);
    }

    #[test]
    fn test_handle_single_reg() {
        let mut state = GdbState::new();
        state.registers.rax = 0xCAFE;

        let reply = handle_read_single_reg(&state, b"0").unwrap();
        assert_eq!(reply.len(), 16);

        // Write P0=4200000000000000
        let mut cmd = Vec::new();
        cmd.extend_from_slice(b"0=");
        encode_hex_u64(0x1234, &mut cmd);
        assert!(handle_write_single_reg(&mut state, &cmd));
        assert_eq!(state.registers.rax, 0x1234);
    }

    #[test]
    fn test_halt_reason() {
        let reply = handle_halt_reason();
        assert_eq!(&reply, b"S05");
    }

    #[test]
    fn test_handle_set_thread_any() {
        let mut state = GdbState::new();
        let reply = handle_set_thread(&mut state, b"g0");
        assert_eq!(&reply, b"OK");
    }

    #[test]
    fn test_handle_set_thread_all() {
        let mut state = GdbState::new();
        let reply = handle_set_thread(&mut state, b"g-1");
        assert_eq!(&reply, b"OK");
    }

    #[test]
    fn test_handle_set_thread_specific() {
        let mut state = GdbState::new();
        // Thread 1 always exists as fallback
        let reply = handle_set_thread(&mut state, b"g1");
        assert_eq!(&reply, b"OK");
        assert_eq!(state.current_thread, 1);
    }

    #[test]
    fn test_handle_set_thread_empty_error() {
        let mut state = GdbState::new();
        let reply = handle_set_thread(&mut state, b"");
        assert_eq!(&reply, b"E01");
    }

    #[test]
    fn test_handle_set_thread_continue_op() {
        let mut state = GdbState::new();
        let reply = handle_set_thread(&mut state, b"c1");
        assert_eq!(&reply, b"OK");
    }

    #[test]
    fn test_format_thread_id_hex() {
        assert_eq!(format_thread_id_hex(0), vec![b'0']);
        assert_eq!(format_thread_id_hex(1), vec![b'1']);
        assert_eq!(format_thread_id_hex(0xFF), vec![b'f', b'f']);
        assert_eq!(format_thread_id_hex(0x1234), vec![b'1', b'2', b'3', b'4']);
    }

    #[test]
    fn test_gdb_state_default_thread() {
        let state = GdbState::new();
        assert_eq!(state.current_thread, 1);
    }

    #[test]
    fn test_collect_thread_ids_fallback() {
        // With empty registry, should return [1]
        let ids = collect_thread_ids();
        assert!(!ids.is_empty());
    }
}
