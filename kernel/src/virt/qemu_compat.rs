//! QEMU compatibility layer for device model and live migration
//!
//! Provides a device multiplexer for I/O and MMIO dispatch, migration v3
//! format serialization, and pre-copy dirty page tracking for live migration.
//!
//! Sprints W5-S4 (device model interface), W5-S5 (migration format + pre-copy).

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

use super::VmError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Migration format magic number ("QEMI" in little-endian)
const MIGRATION_MAGIC: u32 = 0x5145_4D49;

/// Migration format version
const MIGRATION_VERSION: u32 = 3;

/// Default page size for dirty tracking
const PAGE_SIZE: u64 = 4096;

/// Bits per u64 word in dirty bitmap
const BITS_PER_WORD: u64 = 64;

/// Maximum I/O handlers
const MAX_IO_HANDLERS: usize = 256;

/// Maximum MMIO handlers
const MAX_MMIO_HANDLERS: usize = 128;

/// Maximum dirty page threshold for stop-and-copy transition
const DEFAULT_DIRTY_THRESHOLD: u64 = 256;

// ---------------------------------------------------------------------------
// Device Model Interface
// ---------------------------------------------------------------------------

/// Trait for device models that handle I/O and MMIO
pub trait DeviceModelInterface: Send {
    /// Handle an I/O port access
    fn handle_io(&mut self, port: u16, is_write: bool, data: &mut [u8]) -> Result<(), VmError>;

    /// Handle an MMIO access
    fn handle_mmio(&mut self, addr: u64, is_write: bool, data: &mut [u8]) -> Result<(), VmError>;

    /// Get the serialized device state for migration
    fn get_state(&self) -> DeviceState;

    /// Restore device state from migration data
    fn set_state(&mut self, state: &DeviceState) -> Result<(), VmError>;
}

// ---------------------------------------------------------------------------
// I/O Handler
// ---------------------------------------------------------------------------

/// I/O port handler registration
#[derive(Debug, Clone, Copy)]
pub struct IoHandler {
    /// Starting port of the I/O range
    pub port_start: u16,
    /// Number of ports in the range
    pub port_count: u16,
    /// Device identifier
    pub device_id: u32,
}

impl IoHandler {
    /// Create a new I/O handler
    pub fn new(port_start: u16, port_count: u16, device_id: u32) -> Self {
        Self {
            port_start,
            port_count,
            device_id,
        }
    }

    /// Check if a port falls within this handler's range
    pub fn contains_port(&self, port: u16) -> bool {
        port >= self.port_start && port < self.port_start + self.port_count
    }

    /// Get the port end (exclusive)
    pub fn port_end(&self) -> u16 {
        self.port_start + self.port_count
    }

    /// Dispatch an I/O access (returns the offset within the range)
    pub fn dispatch_io(&self, port: u16) -> Option<u16> {
        if self.contains_port(port) {
            Some(port - self.port_start)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// MMIO Handler
// ---------------------------------------------------------------------------

/// MMIO region handler registration
#[derive(Debug, Clone, Copy)]
pub struct MmioHandler {
    /// Base address of the MMIO region
    pub base_addr: u64,
    /// Size of the MMIO region in bytes
    pub size: u64,
    /// Device identifier
    pub device_id: u32,
}

impl MmioHandler {
    /// Create a new MMIO handler
    pub fn new(base_addr: u64, size: u64, device_id: u32) -> Self {
        Self {
            base_addr,
            size,
            device_id,
        }
    }

    /// Check if an address falls within this handler's range
    pub fn contains_addr(&self, addr: u64) -> bool {
        addr >= self.base_addr && addr < self.base_addr + self.size
    }

    /// Get the address end (exclusive)
    pub fn addr_end(&self) -> u64 {
        self.base_addr + self.size
    }

    /// Dispatch an MMIO access (returns the offset within the region)
    pub fn dispatch_mmio(&self, addr: u64) -> Option<u64> {
        if self.contains_addr(addr) {
            Some(addr - self.base_addr)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Device Multiplexer
// ---------------------------------------------------------------------------

/// Multiplexer for routing I/O and MMIO to registered device handlers
#[cfg(feature = "alloc")]
pub struct DeviceMultiplexer {
    /// I/O handlers keyed by starting port
    io_handlers: BTreeMap<u16, IoHandler>,
    /// MMIO handlers keyed by base address
    mmio_handlers: BTreeMap<u64, MmioHandler>,
}

#[cfg(feature = "alloc")]
impl Default for DeviceMultiplexer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl DeviceMultiplexer {
    /// Create a new empty device multiplexer
    pub fn new() -> Self {
        Self {
            io_handlers: BTreeMap::new(),
            mmio_handlers: BTreeMap::new(),
        }
    }

    /// Register an I/O handler
    pub fn register_io(&mut self, handler: IoHandler) -> Result<(), VmError> {
        if self.io_handlers.len() >= MAX_IO_HANDLERS {
            return Err(VmError::DeviceError);
        }

        // Check for overlaps
        for existing in self.io_handlers.values() {
            if handler.port_start < existing.port_end() && handler.port_end() > existing.port_start
            {
                return Err(VmError::DeviceError);
            }
        }

        self.io_handlers.insert(handler.port_start, handler);
        Ok(())
    }

    /// Register an MMIO handler
    pub fn register_mmio(&mut self, handler: MmioHandler) -> Result<(), VmError> {
        if self.mmio_handlers.len() >= MAX_MMIO_HANDLERS {
            return Err(VmError::DeviceError);
        }

        // Check for overlaps
        for existing in self.mmio_handlers.values() {
            if handler.base_addr < existing.addr_end() && handler.addr_end() > existing.base_addr {
                return Err(VmError::DeviceError);
            }
        }

        self.mmio_handlers.insert(handler.base_addr, handler);
        Ok(())
    }

    /// Dispatch an I/O access, returning (device_id, offset)
    pub fn dispatch_io(&self, port: u16) -> Option<(u32, u16)> {
        // Find the handler whose range includes this port
        for handler in self.io_handlers.values() {
            if let Some(offset) = handler.dispatch_io(port) {
                return Some((handler.device_id, offset));
            }
        }
        None
    }

    /// Dispatch an MMIO access, returning (device_id, offset)
    pub fn dispatch_mmio(&self, addr: u64) -> Option<(u32, u64)> {
        for handler in self.mmio_handlers.values() {
            if let Some(offset) = handler.dispatch_mmio(addr) {
                return Some((handler.device_id, offset));
            }
        }
        None
    }

    /// Unregister an I/O handler by starting port
    pub fn unregister_io(&mut self, port_start: u16) -> bool {
        self.io_handlers.remove(&port_start).is_some()
    }

    /// Unregister an MMIO handler by base address
    pub fn unregister_mmio(&mut self, base_addr: u64) -> bool {
        self.mmio_handlers.remove(&base_addr).is_some()
    }

    /// Get number of registered I/O handlers
    pub fn io_handler_count(&self) -> usize {
        self.io_handlers.len()
    }

    /// Get number of registered MMIO handlers
    pub fn mmio_handler_count(&self) -> usize {
        self.mmio_handlers.len()
    }
}

// ---------------------------------------------------------------------------
// Migration Format: Device State
// ---------------------------------------------------------------------------

/// Serialized state of a single device
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct DeviceState {
    /// Device name identifier
    pub device_name: String,
    /// Serialized device data
    pub data: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl DeviceState {
    /// Create a new device state
    pub fn new(name: &str) -> Self {
        Self {
            device_name: String::from(name),
            data: Vec::new(),
        }
    }

    /// Create a device state with data
    pub fn with_data(name: &str, data: Vec<u8>) -> Self {
        Self {
            device_name: String::from(name),
            data,
        }
    }

    /// Write a u32 value to the state data
    pub fn write_u32(&mut self, value: u32) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a u64 value to the state data
    pub fn write_u64(&mut self, value: u64) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a byte slice to the state data
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_u32(bytes.len() as u32);
        self.data.extend_from_slice(bytes);
    }

    /// Read a u32 from state data at offset, advancing offset
    pub fn read_u32(&self, offset: &mut usize) -> Option<u32> {
        if *offset + 4 > self.data.len() {
            return None;
        }
        let bytes: [u8; 4] = self.data[*offset..*offset + 4].try_into().ok()?;
        *offset += 4;
        Some(u32::from_le_bytes(bytes))
    }

    /// Read a u64 from state data at offset, advancing offset
    pub fn read_u64(&self, offset: &mut usize) -> Option<u64> {
        if *offset + 8 > self.data.len() {
            return None;
        }
        let bytes: [u8; 8] = self.data[*offset..*offset + 8].try_into().ok()?;
        *offset += 8;
        Some(u64::from_le_bytes(bytes))
    }

    /// Read bytes from state data at offset, advancing offset
    pub fn read_bytes(&self, offset: &mut usize) -> Option<Vec<u8>> {
        let len = self.read_u32(offset)? as usize;
        if *offset + len > self.data.len() {
            return None;
        }
        let data = self.data[*offset..*offset + len].to_vec();
        *offset += len;
        Some(data)
    }

    /// Get total serialized size
    pub fn serialized_size(&self) -> usize {
        4 + self.device_name.len() + self.data.len() // name_len + name + data
    }
}

// ---------------------------------------------------------------------------
// Migration Format: VM State
// ---------------------------------------------------------------------------

/// Complete VM state for migration
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Default)]
pub struct VmState {
    /// Per-vCPU register state (serialized as bytes)
    pub vcpu_regs: Vec<Vec<u8>>,
    /// Hash of guest memory for integrity verification
    pub memory_hash: u64,
    /// Memory size in bytes
    pub memory_size: u64,
    /// Number of vCPUs
    pub num_vcpus: u32,
}

#[cfg(feature = "alloc")]
impl VmState {
    /// Create a new VM state
    pub fn new(num_vcpus: u32, memory_size: u64) -> Self {
        Self {
            vcpu_regs: Vec::with_capacity(num_vcpus as usize),
            memory_hash: 0,
            memory_size,
            num_vcpus,
        }
    }

    /// Add vCPU register state
    pub fn add_vcpu_state(&mut self, regs: Vec<u8>) {
        self.vcpu_regs.push(regs);
    }

    /// Set memory hash
    pub fn set_memory_hash(&mut self, hash: u64) {
        self.memory_hash = hash;
    }
}

// ---------------------------------------------------------------------------
// Migration Header
// ---------------------------------------------------------------------------

/// Migration stream header (v3 format)
#[derive(Debug, Clone, Copy)]
pub struct MigrationHeader {
    /// Magic number (MIGRATION_MAGIC)
    pub magic: u32,
    /// Format version
    pub version: u32,
    /// Size of VM state section in bytes
    pub vm_state_size: u32,
    /// Size of device state section in bytes
    pub device_state_size: u32,
    /// Number of device state entries
    pub device_count: u32,
    /// Flags (reserved)
    pub flags: u32,
}

impl MigrationHeader {
    /// Create a new migration header
    pub fn new(vm_state_size: u32, device_state_size: u32, device_count: u32) -> Self {
        Self {
            magic: MIGRATION_MAGIC,
            version: MIGRATION_VERSION,
            vm_state_size,
            device_state_size,
            device_count,
            flags: 0,
        }
    }

    /// Validate the header
    pub fn validate(&self) -> Result<(), VmError> {
        if self.magic != MIGRATION_MAGIC {
            return Err(VmError::InvalidVmState);
        }
        if self.version != MIGRATION_VERSION {
            return Err(VmError::InvalidVmState);
        }
        Ok(())
    }

    /// Serialize header to bytes
    #[cfg(feature = "alloc")]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(24);
        buf.extend_from_slice(&self.magic.to_le_bytes());
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(&self.vm_state_size.to_le_bytes());
        buf.extend_from_slice(&self.device_state_size.to_le_bytes());
        buf.extend_from_slice(&self.device_count.to_le_bytes());
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf
    }

    /// Deserialize header from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, VmError> {
        if data.len() < 24 {
            return Err(VmError::InvalidVmState);
        }
        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let vm_state_size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let device_state_size = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        let device_count = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        let flags = u32::from_le_bytes([data[20], data[21], data[22], data[23]]);

        let header = Self {
            magic,
            version,
            vm_state_size,
            device_state_size,
            device_count,
            flags,
        };
        header.validate()?;
        Ok(header)
    }
}

// ---------------------------------------------------------------------------
// State Serialization Functions
// ---------------------------------------------------------------------------

/// Serialize VM and device states into a migration stream
#[cfg(feature = "alloc")]
pub fn serialize_state(vm_state: &VmState, device_states: &[DeviceState]) -> Vec<u8> {
    let mut vm_data = Vec::new();
    // Serialize VM state
    vm_data.extend_from_slice(&vm_state.num_vcpus.to_le_bytes());
    vm_data.extend_from_slice(&vm_state.memory_size.to_le_bytes());
    vm_data.extend_from_slice(&vm_state.memory_hash.to_le_bytes());
    // Serialize vCPU states
    for vcpu_regs in &vm_state.vcpu_regs {
        vm_data.extend_from_slice(&(vcpu_regs.len() as u32).to_le_bytes());
        vm_data.extend_from_slice(vcpu_regs);
    }

    let mut device_data = Vec::new();
    for ds in device_states {
        // Name length + name
        device_data.extend_from_slice(&(ds.device_name.len() as u32).to_le_bytes());
        device_data.extend_from_slice(ds.device_name.as_bytes());
        // Data length + data
        device_data.extend_from_slice(&(ds.data.len() as u32).to_le_bytes());
        device_data.extend_from_slice(&ds.data);
    }

    let header = MigrationHeader::new(
        vm_data.len() as u32,
        device_data.len() as u32,
        device_states.len() as u32,
    );

    let mut stream = header.to_bytes();
    stream.extend_from_slice(&vm_data);
    stream.extend_from_slice(&device_data);
    stream
}

/// Deserialize VM state from a migration stream
#[cfg(feature = "alloc")]
pub fn deserialize_state(data: &[u8]) -> Result<(VmState, Vec<DeviceState>), VmError> {
    let header = MigrationHeader::from_bytes(data)?;

    let vm_offset = 24; // After header
    let device_offset = vm_offset + header.vm_state_size as usize;

    if data.len() < device_offset + header.device_state_size as usize {
        return Err(VmError::InvalidVmState);
    }

    // Parse VM state
    let vm_data = &data[vm_offset..device_offset];
    if vm_data.len() < 20 {
        return Err(VmError::InvalidVmState);
    }

    let num_vcpus = u32::from_le_bytes([vm_data[0], vm_data[1], vm_data[2], vm_data[3]]);
    let memory_size = u64::from_le_bytes([
        vm_data[4],
        vm_data[5],
        vm_data[6],
        vm_data[7],
        vm_data[8],
        vm_data[9],
        vm_data[10],
        vm_data[11],
    ]);
    let memory_hash = u64::from_le_bytes([
        vm_data[12],
        vm_data[13],
        vm_data[14],
        vm_data[15],
        vm_data[16],
        vm_data[17],
        vm_data[18],
        vm_data[19],
    ]);

    let mut vm_state = VmState::new(num_vcpus, memory_size);
    vm_state.set_memory_hash(memory_hash);

    let mut pos = 20;
    for _ in 0..num_vcpus {
        if pos + 4 > vm_data.len() {
            break;
        }
        let len = u32::from_le_bytes([
            vm_data[pos],
            vm_data[pos + 1],
            vm_data[pos + 2],
            vm_data[pos + 3],
        ]) as usize;
        pos += 4;
        if pos + len > vm_data.len() {
            break;
        }
        vm_state.add_vcpu_state(vm_data[pos..pos + len].to_vec());
        pos += len;
    }

    // Parse device states
    let dev_data = &data[device_offset..device_offset + header.device_state_size as usize];
    let mut device_states = Vec::new();
    let mut dpos = 0;

    for _ in 0..header.device_count {
        if dpos + 4 > dev_data.len() {
            break;
        }
        let name_len = u32::from_le_bytes([
            dev_data[dpos],
            dev_data[dpos + 1],
            dev_data[dpos + 2],
            dev_data[dpos + 3],
        ]) as usize;
        dpos += 4;

        if dpos + name_len > dev_data.len() {
            break;
        }
        let name = core::str::from_utf8(&dev_data[dpos..dpos + name_len]).unwrap_or("unknown");
        dpos += name_len;

        if dpos + 4 > dev_data.len() {
            break;
        }
        let data_len = u32::from_le_bytes([
            dev_data[dpos],
            dev_data[dpos + 1],
            dev_data[dpos + 2],
            dev_data[dpos + 3],
        ]) as usize;
        dpos += 4;

        if dpos + data_len > dev_data.len() {
            break;
        }
        let state_data = dev_data[dpos..dpos + data_len].to_vec();
        dpos += data_len;

        device_states.push(DeviceState::with_data(name, state_data));
    }

    Ok((vm_state, device_states))
}

// ---------------------------------------------------------------------------
// Dirty Page Tracker
// ---------------------------------------------------------------------------

/// Bitmap-based dirty page tracker for migration
#[cfg(feature = "alloc")]
pub struct DirtyPageTracker {
    /// Bitmap of dirty pages (1 bit per page)
    bitmap: Vec<u64>,
    /// Page size
    page_size: u64,
    /// Total number of pages tracked
    num_pages: u64,
    /// Number of currently dirty pages
    dirty_count: u64,
}

#[cfg(feature = "alloc")]
impl DirtyPageTracker {
    /// Create a new dirty page tracker for the given memory size
    pub fn new(memory_size: u64, page_size: u64) -> Self {
        let ps = if page_size == 0 { PAGE_SIZE } else { page_size };
        let num_pages = memory_size.div_ceil(ps);
        let bitmap_words = num_pages.div_ceil(BITS_PER_WORD) as usize;
        Self {
            bitmap: vec![0u64; bitmap_words],
            page_size: ps,
            num_pages,
            dirty_count: 0,
        }
    }

    /// Mark a page as dirty by its page frame number
    pub fn mark_dirty(&mut self, page_num: u64) {
        if page_num >= self.num_pages {
            return;
        }
        let word = (page_num / BITS_PER_WORD) as usize;
        let bit = page_num % BITS_PER_WORD;
        if word < self.bitmap.len() {
            let mask = 1u64 << bit;
            if self.bitmap[word] & mask == 0 {
                self.bitmap[word] |= mask;
                self.dirty_count = self.dirty_count.saturating_add(1);
            }
        }
    }

    /// Mark a page as dirty by physical address
    pub fn mark_dirty_addr(&mut self, addr: u64) {
        self.mark_dirty(addr / self.page_size);
    }

    /// Check if a page is dirty
    pub fn is_dirty(&self, page_num: u64) -> bool {
        if page_num >= self.num_pages {
            return false;
        }
        let word = (page_num / BITS_PER_WORD) as usize;
        let bit = page_num % BITS_PER_WORD;
        if word < self.bitmap.len() {
            self.bitmap[word] & (1u64 << bit) != 0
        } else {
            false
        }
    }

    /// Get list of dirty page numbers
    pub fn get_dirty_pages(&self) -> Vec<u64> {
        let mut pages = Vec::new();
        for (word_idx, &word) in self.bitmap.iter().enumerate() {
            if word == 0 {
                continue;
            }
            for bit in 0..64u64 {
                if word & (1u64 << bit) != 0 {
                    let page = word_idx as u64 * BITS_PER_WORD + bit;
                    if page < self.num_pages {
                        pages.push(page);
                    }
                }
            }
        }
        pages
    }

    /// Clear all dirty bits and return the list of previously dirty pages
    pub fn clear(&mut self) -> Vec<u64> {
        let dirty = self.get_dirty_pages();
        for word in &mut self.bitmap {
            *word = 0;
        }
        self.dirty_count = 0;
        dirty
    }

    /// Get the number of dirty pages
    pub fn dirty_count(&self) -> u64 {
        self.dirty_count
    }

    /// Get the total number of tracked pages
    pub fn total_pages(&self) -> u64 {
        self.num_pages
    }

    /// Get page size
    pub fn page_size(&self) -> u64 {
        self.page_size
    }

    /// Mark all pages as dirty (for initial transfer)
    pub fn mark_all_dirty(&mut self) {
        for (i, word) in self.bitmap.iter_mut().enumerate() {
            let remaining = self.num_pages.saturating_sub(i as u64 * BITS_PER_WORD);
            if remaining >= BITS_PER_WORD {
                *word = u64::MAX;
            } else if remaining > 0 {
                *word = (1u64 << remaining) - 1;
            }
        }
        self.dirty_count = self.num_pages;
    }
}

// ---------------------------------------------------------------------------
// Migration Phase
// ---------------------------------------------------------------------------

/// Phase of the live migration process
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MigrationPhase {
    /// Not started
    #[default]
    Idle,
    /// Pre-copy: iteratively transferring dirty pages
    PreCopy,
    /// Stop-and-copy: VM paused, final state transfer
    StopAndCopy,
    /// Migration completed
    Done,
    /// Migration failed
    Failed,
}

// ---------------------------------------------------------------------------
// Migration Stream
// ---------------------------------------------------------------------------

/// Live migration stream controller
#[cfg(feature = "alloc")]
pub struct MigrationStream {
    /// Current migration phase
    pub phase: MigrationPhase,
    /// Number of pre-copy rounds performed
    pub rounds: u32,
    /// Dirty page threshold for transitioning to stop-and-copy
    pub dirty_page_threshold: u64,
    /// Dirty page tracker
    pub tracker: DirtyPageTracker,
    /// Pages sent in current round
    pub pages_sent: u64,
    /// Total pages sent across all rounds
    pub total_pages_sent: u64,
    /// Total bytes sent
    pub total_bytes_sent: u64,
    /// Maximum rounds before forcing stop-and-copy
    pub max_rounds: u32,
}

#[cfg(feature = "alloc")]
impl MigrationStream {
    /// Create a new migration stream
    pub fn new(memory_size: u64) -> Self {
        Self {
            phase: MigrationPhase::Idle,
            rounds: 0,
            dirty_page_threshold: DEFAULT_DIRTY_THRESHOLD,
            tracker: DirtyPageTracker::new(memory_size, PAGE_SIZE),
            pages_sent: 0,
            total_pages_sent: 0,
            total_bytes_sent: 0,
            max_rounds: 32,
        }
    }

    /// Start the pre-copy phase
    pub fn start_precopy(&mut self) {
        self.phase = MigrationPhase::PreCopy;
        self.rounds = 0;
        self.tracker.mark_all_dirty();
    }

    /// Send dirty pages for the current round
    /// Returns the page numbers that were sent
    pub fn send_dirty_pages(&mut self) -> Vec<u64> {
        if self.phase != MigrationPhase::PreCopy {
            return Vec::new();
        }

        let dirty_pages = self.tracker.clear();
        let count = dirty_pages.len() as u64;
        self.pages_sent = count;
        self.total_pages_sent = self.total_pages_sent.saturating_add(count);
        self.total_bytes_sent = self.total_bytes_sent.saturating_add(count * PAGE_SIZE);
        self.rounds += 1;

        dirty_pages
    }

    /// Receive dirty pages on the destination (mark them as received)
    pub fn receive_dirty_pages(&mut self, pages: &[u64]) -> Result<(), VmError> {
        // On destination side, pages have been received and applied
        self.total_pages_sent = self.total_pages_sent.saturating_add(pages.len() as u64);
        Ok(())
    }

    /// Check if we should transition to stop-and-copy
    pub fn should_stop_and_copy(&self) -> bool {
        if self.phase != MigrationPhase::PreCopy {
            return false;
        }
        // Transition when dirty pages below threshold or max rounds exceeded
        self.tracker.dirty_count() <= self.dirty_page_threshold || self.rounds >= self.max_rounds
    }

    /// Transition to stop-and-copy phase
    pub fn stop_and_copy(&mut self) -> Vec<u64> {
        self.phase = MigrationPhase::StopAndCopy;

        // Get final dirty pages
        let final_pages = self.tracker.clear();
        let count = final_pages.len() as u64;
        self.total_pages_sent = self.total_pages_sent.saturating_add(count);
        self.total_bytes_sent = self.total_bytes_sent.saturating_add(count * PAGE_SIZE);

        final_pages
    }

    /// Complete the migration
    pub fn complete(&mut self) {
        self.phase = MigrationPhase::Done;
    }

    /// Mark migration as failed
    pub fn fail(&mut self) {
        self.phase = MigrationPhase::Failed;
    }

    /// Get migration statistics
    pub fn stats(&self) -> MigrationStats {
        MigrationStats {
            phase: self.phase,
            rounds: self.rounds,
            total_pages_sent: self.total_pages_sent,
            total_bytes_sent: self.total_bytes_sent,
            remaining_dirty: self.tracker.dirty_count(),
        }
    }
}

/// Migration statistics
#[derive(Debug, Clone, Copy)]
pub struct MigrationStats {
    /// Current phase
    pub phase: MigrationPhase,
    /// Number of rounds completed
    pub rounds: u32,
    /// Total pages sent
    pub total_pages_sent: u64,
    /// Total bytes sent
    pub total_bytes_sent: u64,
    /// Remaining dirty pages
    pub remaining_dirty: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_handler_contains() {
        let handler = IoHandler::new(0x3F8, 8, 1);
        assert!(handler.contains_port(0x3F8));
        assert!(handler.contains_port(0x3FF));
        assert!(!handler.contains_port(0x400));
        assert!(!handler.contains_port(0x3F7));
    }

    #[test]
    fn test_io_handler_dispatch() {
        let handler = IoHandler::new(0x3F8, 8, 1);
        assert_eq!(handler.dispatch_io(0x3F8), Some(0));
        assert_eq!(handler.dispatch_io(0x3FA), Some(2));
        assert_eq!(handler.dispatch_io(0x400), None);
    }

    #[test]
    fn test_mmio_handler_contains() {
        let handler = MmioHandler::new(0xFEE0_0000, 0x1000, 2);
        assert!(handler.contains_addr(0xFEE0_0000));
        assert!(handler.contains_addr(0xFEE0_0FFF));
        assert!(!handler.contains_addr(0xFEE0_1000));
    }

    #[test]
    fn test_mmio_handler_dispatch() {
        let handler = MmioHandler::new(0xFEE0_0000, 0x1000, 2);
        assert_eq!(handler.dispatch_mmio(0xFEE0_0010), Some(0x10));
        assert_eq!(handler.dispatch_mmio(0xFEE0_1000), None);
    }

    #[test]
    fn test_device_multiplexer_io() {
        let mut mux = DeviceMultiplexer::new();
        let h1 = IoHandler::new(0x3F8, 8, 1);
        let h2 = IoHandler::new(0x2F8, 8, 2);
        assert!(mux.register_io(h1).is_ok());
        assert!(mux.register_io(h2).is_ok());
        assert_eq!(mux.io_handler_count(), 2);

        assert_eq!(mux.dispatch_io(0x3F8), Some((1, 0)));
        assert_eq!(mux.dispatch_io(0x2FA), Some((2, 2)));
        assert_eq!(mux.dispatch_io(0x100), None);
    }

    #[test]
    fn test_device_multiplexer_io_overlap() {
        let mut mux = DeviceMultiplexer::new();
        let h1 = IoHandler::new(0x3F8, 8, 1);
        let h2 = IoHandler::new(0x3FC, 4, 2); // Overlaps
        assert!(mux.register_io(h1).is_ok());
        assert!(mux.register_io(h2).is_err());
    }

    #[test]
    fn test_device_multiplexer_mmio() {
        let mut mux = DeviceMultiplexer::new();
        let h = MmioHandler::new(0xFEE0_0000, 0x1000, 1);
        assert!(mux.register_mmio(h).is_ok());
        assert_eq!(mux.dispatch_mmio(0xFEE0_0020), Some((1, 0x20)));
    }

    #[test]
    fn test_device_multiplexer_unregister() {
        let mut mux = DeviceMultiplexer::new();
        mux.register_io(IoHandler::new(0x3F8, 8, 1)).unwrap();
        assert!(mux.unregister_io(0x3F8));
        assert_eq!(mux.io_handler_count(), 0);
        assert!(!mux.unregister_io(0x3F8)); // Already removed
    }

    #[test]
    fn test_migration_header_roundtrip() {
        let header = MigrationHeader::new(100, 200, 3);
        let bytes = header.to_bytes();
        let parsed = MigrationHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.magic, MIGRATION_MAGIC);
        assert_eq!(parsed.version, MIGRATION_VERSION);
        assert_eq!(parsed.vm_state_size, 100);
        assert_eq!(parsed.device_state_size, 200);
        assert_eq!(parsed.device_count, 3);
    }

    #[test]
    fn test_migration_header_invalid_magic() {
        let mut bytes = MigrationHeader::new(0, 0, 0).to_bytes();
        bytes[0] = 0xFF; // Corrupt magic
        assert!(MigrationHeader::from_bytes(&bytes).is_err());
    }

    #[test]
    fn test_device_state_write_read() {
        let mut state = DeviceState::new("uart0");
        state.write_u32(42);
        state.write_u64(0xDEAD_BEEF);
        state.write_bytes(&[1, 2, 3, 4]);

        let mut offset = 0;
        assert_eq!(state.read_u32(&mut offset), Some(42));
        assert_eq!(state.read_u64(&mut offset), Some(0xDEAD_BEEF));
        let bytes = state.read_bytes(&mut offset).unwrap();
        assert_eq!(bytes, &[1, 2, 3, 4]);
    }

    #[test]
    fn test_serialize_deserialize_state() {
        let mut vm_state = VmState::new(2, 0x1000_0000);
        vm_state.set_memory_hash(0x1234_5678);
        vm_state.add_vcpu_state(vec![1, 2, 3, 4]);
        vm_state.add_vcpu_state(vec![5, 6, 7, 8]);

        let device_states = vec![
            DeviceState::with_data("uart", vec![0xAA, 0xBB]),
            DeviceState::with_data("pic", vec![0xCC]),
        ];

        let data = serialize_state(&vm_state, &device_states);
        let (parsed_vm, parsed_devs) = deserialize_state(&data).unwrap();

        assert_eq!(parsed_vm.num_vcpus, 2);
        assert_eq!(parsed_vm.memory_size, 0x1000_0000);
        assert_eq!(parsed_vm.memory_hash, 0x1234_5678);
        assert_eq!(parsed_vm.vcpu_regs.len(), 2);
        assert_eq!(parsed_vm.vcpu_regs[0], &[1, 2, 3, 4]);
        assert_eq!(parsed_devs.len(), 2);
        assert_eq!(parsed_devs[0].device_name, "uart");
        assert_eq!(parsed_devs[0].data, &[0xAA, 0xBB]);
    }

    #[test]
    fn test_dirty_page_tracker_basic() {
        let mut tracker = DirtyPageTracker::new(0x10000, PAGE_SIZE);
        assert_eq!(tracker.total_pages(), 16);
        assert_eq!(tracker.dirty_count(), 0);

        tracker.mark_dirty(0);
        tracker.mark_dirty(5);
        assert!(tracker.is_dirty(0));
        assert!(tracker.is_dirty(5));
        assert!(!tracker.is_dirty(1));
        assert_eq!(tracker.dirty_count(), 2);
    }

    #[test]
    fn test_dirty_page_tracker_by_addr() {
        let mut tracker = DirtyPageTracker::new(0x10000, PAGE_SIZE);
        tracker.mark_dirty_addr(0x5000);
        assert!(tracker.is_dirty(5));
    }

    #[test]
    fn test_dirty_page_tracker_get_pages() {
        let mut tracker = DirtyPageTracker::new(0x10000, PAGE_SIZE);
        tracker.mark_dirty(1);
        tracker.mark_dirty(3);
        tracker.mark_dirty(7);
        let pages = tracker.get_dirty_pages();
        assert_eq!(pages, &[1, 3, 7]);
    }

    #[test]
    fn test_dirty_page_tracker_clear() {
        let mut tracker = DirtyPageTracker::new(0x10000, PAGE_SIZE);
        tracker.mark_dirty(0);
        tracker.mark_dirty(1);
        let cleared = tracker.clear();
        assert_eq!(cleared.len(), 2);
        assert_eq!(tracker.dirty_count(), 0);
        assert!(!tracker.is_dirty(0));
    }

    #[test]
    fn test_dirty_page_tracker_mark_all() {
        let mut tracker = DirtyPageTracker::new(0x10000, PAGE_SIZE);
        tracker.mark_all_dirty();
        assert_eq!(tracker.dirty_count(), 16);
        for i in 0..16 {
            assert!(tracker.is_dirty(i));
        }
    }

    #[test]
    fn test_dirty_page_tracker_double_mark() {
        let mut tracker = DirtyPageTracker::new(0x10000, PAGE_SIZE);
        tracker.mark_dirty(5);
        tracker.mark_dirty(5); // Should not double-count
        assert_eq!(tracker.dirty_count(), 1);
    }

    #[test]
    fn test_migration_stream_precopy() {
        let mut stream = MigrationStream::new(0x10000);
        assert_eq!(stream.phase, MigrationPhase::Idle);

        stream.start_precopy();
        assert_eq!(stream.phase, MigrationPhase::PreCopy);

        let pages = stream.send_dirty_pages();
        assert_eq!(pages.len(), 16); // All pages dirty initially
        assert_eq!(stream.rounds, 1);
        assert_eq!(stream.total_pages_sent, 16);
    }

    #[test]
    fn test_migration_stream_stop_and_copy() {
        let mut stream = MigrationStream::new(0x10000);
        stream.start_precopy();
        let _ = stream.send_dirty_pages(); // Round 1

        // Mark a few pages dirty
        stream.tracker.mark_dirty(1);
        stream.tracker.mark_dirty(3);

        let final_pages = stream.stop_and_copy();
        assert_eq!(stream.phase, MigrationPhase::StopAndCopy);
        assert_eq!(final_pages.len(), 2);

        stream.complete();
        assert_eq!(stream.phase, MigrationPhase::Done);
    }

    #[test]
    fn test_migration_stream_should_stop() {
        let mut stream = MigrationStream::new(0x10000);
        stream.start_precopy();
        let _ = stream.send_dirty_pages();
        // After clearing, dirty count is 0 which is below threshold
        assert!(stream.should_stop_and_copy());
    }

    #[test]
    fn test_migration_stats() {
        let mut stream = MigrationStream::new(0x10000);
        stream.start_precopy();
        let _ = stream.send_dirty_pages();
        let stats = stream.stats();
        assert_eq!(stats.rounds, 1);
        assert_eq!(stats.total_pages_sent, 16);
        assert_eq!(stats.total_bytes_sent, 16 * PAGE_SIZE);
    }
}
