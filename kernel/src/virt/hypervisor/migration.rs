//! Live Migration
//!
//! VMCS serialization, dirty page pre-copy, stop-and-copy for VM migration.

#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};

use super::{BITS_PER_U64, PAGE_SIZE, PRECOPY_BATCH_SIZE};
use crate::virt::{vmx::VmcsFields, VmError};

// ---------------------------------------------------------------------------
// 3. Live Migration
// ---------------------------------------------------------------------------

/// Migration state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MigrationState {
    /// Not migrating
    #[default]
    Idle,
    /// Initial setup phase
    Setup,
    /// Pre-copy: iteratively send dirty pages
    PreCopy,
    /// Stop-and-copy: VM paused, final state transfer
    StopAndCopy,
    /// Completing migration
    Completing,
    /// Migration complete
    Complete,
    /// Migration failed
    Failed,
}

/// VMCS field group for serialization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmcsFieldGroup {
    /// Guest register state (RIP, RSP, RFLAGS, etc.)
    GuestRegisterState,
    /// Guest segment state (selectors, bases, limits, AR)
    GuestSegmentState,
    /// Guest control state (CR0, CR3, CR4, DR7)
    GuestControlState,
    /// Host state fields
    HostState,
    /// Execution control fields
    ExecutionControls,
    /// Exit/entry control fields
    ExitEntryControls,
    /// Read-only data fields
    ReadOnlyData,
}

/// Serialized VMCS field
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerializedVmcsField {
    pub encoding: u32,
    pub value: u64,
}

/// Serialized VMCS state for migration
#[cfg(feature = "alloc")]
pub struct SerializedVmcs {
    pub fields: Vec<SerializedVmcsField>,
}

#[cfg(feature = "alloc")]
impl Default for SerializedVmcs {
    fn default() -> Self {
        Self::new()
    }
}

impl SerializedVmcs {
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    pub fn add_field(&mut self, encoding: u32, value: u64) {
        self.fields.push(SerializedVmcsField { encoding, value });
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    pub fn find_field(&self, encoding: u32) -> Option<u64> {
        self.fields
            .iter()
            .find(|f| f.encoding == encoding)
            .map(|f| f.value)
    }

    /// Get all guest register state fields for serialization
    pub fn serialize_guest_registers() -> &'static [u32] {
        &[
            VmcsFields::GUEST_RIP,
            VmcsFields::GUEST_RSP,
            VmcsFields::GUEST_RFLAGS,
            VmcsFields::GUEST_CR0,
            VmcsFields::GUEST_CR3,
            VmcsFields::GUEST_CR4,
            VmcsFields::GUEST_DR7,
            VmcsFields::GUEST_SYSENTER_CS,
            VmcsFields::GUEST_SYSENTER_ESP,
            VmcsFields::GUEST_SYSENTER_EIP,
            VmcsFields::GUEST_IA32_EFER,
            VmcsFields::GUEST_IA32_PAT,
        ]
    }

    /// Get all guest segment state fields
    pub fn serialize_guest_segments() -> &'static [u32] {
        &[
            VmcsFields::GUEST_CS_SELECTOR,
            VmcsFields::GUEST_CS_BASE,
            VmcsFields::GUEST_CS_LIMIT,
            VmcsFields::GUEST_CS_ACCESS_RIGHTS,
            VmcsFields::GUEST_SS_SELECTOR,
            VmcsFields::GUEST_SS_BASE,
            VmcsFields::GUEST_SS_LIMIT,
            VmcsFields::GUEST_SS_ACCESS_RIGHTS,
            VmcsFields::GUEST_DS_SELECTOR,
            VmcsFields::GUEST_DS_BASE,
            VmcsFields::GUEST_DS_LIMIT,
            VmcsFields::GUEST_DS_ACCESS_RIGHTS,
            VmcsFields::GUEST_ES_SELECTOR,
            VmcsFields::GUEST_ES_BASE,
            VmcsFields::GUEST_ES_LIMIT,
            VmcsFields::GUEST_ES_ACCESS_RIGHTS,
            VmcsFields::GUEST_FS_SELECTOR,
            VmcsFields::GUEST_FS_BASE,
            VmcsFields::GUEST_FS_LIMIT,
            VmcsFields::GUEST_FS_ACCESS_RIGHTS,
            VmcsFields::GUEST_GS_SELECTOR,
            VmcsFields::GUEST_GS_BASE,
            VmcsFields::GUEST_GS_LIMIT,
            VmcsFields::GUEST_GS_ACCESS_RIGHTS,
            VmcsFields::GUEST_TR_SELECTOR,
            VmcsFields::GUEST_TR_BASE,
            VmcsFields::GUEST_TR_LIMIT,
            VmcsFields::GUEST_TR_ACCESS_RIGHTS,
            VmcsFields::GUEST_LDTR_SELECTOR,
            VmcsFields::GUEST_LDTR_BASE,
            VmcsFields::GUEST_LDTR_LIMIT,
            VmcsFields::GUEST_LDTR_ACCESS_RIGHTS,
            VmcsFields::GUEST_GDTR_BASE,
            VmcsFields::GUEST_GDTR_LIMIT,
            VmcsFields::GUEST_IDTR_BASE,
            VmcsFields::GUEST_IDTR_LIMIT,
        ]
    }
}

/// Dirty page bitmap for tracking modified guest pages during migration
#[cfg(feature = "alloc")]
pub struct DirtyPageBitmap {
    /// Bitmap: 1 bit per page
    bitmap: Vec<u64>,
    /// Total number of pages tracked
    total_pages: u64,
    /// Count of currently dirty pages
    dirty_count: u64,
}

#[cfg(feature = "alloc")]
impl DirtyPageBitmap {
    pub fn new(total_pages: u64) -> Self {
        let words = total_pages.div_ceil(BITS_PER_U64) as usize;
        Self {
            bitmap: vec![0u64; words],
            total_pages,
            dirty_count: 0,
        }
    }

    /// Mark a page as dirty
    pub fn set_dirty(&mut self, page_index: u64) {
        if page_index >= self.total_pages {
            return;
        }
        let word = (page_index / BITS_PER_U64) as usize;
        let bit = page_index % BITS_PER_U64;
        if self.bitmap[word] & (1u64 << bit) == 0 {
            self.bitmap[word] |= 1u64 << bit;
            self.dirty_count += 1;
        }
    }

    /// Check if a page is dirty
    pub fn is_dirty(&self, page_index: u64) -> bool {
        if page_index >= self.total_pages {
            return false;
        }
        let word = (page_index / BITS_PER_U64) as usize;
        let bit = page_index % BITS_PER_U64;
        self.bitmap[word] & (1u64 << bit) != 0
    }

    /// Clear a page's dirty bit
    pub fn clear_dirty(&mut self, page_index: u64) {
        if page_index >= self.total_pages {
            return;
        }
        let word = (page_index / BITS_PER_U64) as usize;
        let bit = page_index % BITS_PER_U64;
        if self.bitmap[word] & (1u64 << bit) != 0 {
            self.bitmap[word] &= !(1u64 << bit);
            if self.dirty_count > 0 {
                self.dirty_count -= 1;
            }
        }
    }

    /// Clear all dirty bits and return previous dirty count
    pub fn clear_all(&mut self) -> u64 {
        let count = self.dirty_count;
        for word in &mut self.bitmap {
            *word = 0;
        }
        self.dirty_count = 0;
        count
    }

    /// Iterate over dirty page indices
    pub fn dirty_pages(&self) -> DirtyPageIter<'_> {
        DirtyPageIter {
            bitmap: self,
            current_word: 0,
            current_bit: 0,
        }
    }

    pub fn dirty_count(&self) -> u64 {
        self.dirty_count
    }

    pub fn total_pages(&self) -> u64 {
        self.total_pages
    }
}

/// Iterator over dirty pages
#[cfg(feature = "alloc")]
pub struct DirtyPageIter<'a> {
    bitmap: &'a DirtyPageBitmap,
    current_word: usize,
    current_bit: u64,
}

#[cfg(feature = "alloc")]
impl<'a> Iterator for DirtyPageIter<'a> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        while self.current_word < self.bitmap.bitmap.len() {
            let word = self.bitmap.bitmap[self.current_word];
            while self.current_bit < BITS_PER_U64 {
                let bit = self.current_bit;
                self.current_bit += 1;
                if word & (1u64 << bit) != 0 {
                    let page_idx = (self.current_word as u64)
                        .checked_mul(BITS_PER_U64)
                        .and_then(|v| v.checked_add(bit));
                    if let Some(idx) = page_idx {
                        if idx < self.bitmap.total_pages {
                            return Some(idx);
                        }
                    }
                }
            }
            self.current_word += 1;
            self.current_bit = 0;
        }
        None
    }
}

/// Migration progress tracking (integer-only bandwidth/progress estimation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MigrationProgress {
    /// Total bytes to transfer
    pub total_bytes: u64,
    /// Bytes transferred so far
    pub transferred_bytes: u64,
    /// Current iteration number
    pub iteration: u32,
    /// Dirty pages in current iteration
    pub current_dirty_pages: u64,
    /// Dirty pages from previous iteration
    pub previous_dirty_pages: u64,
    /// Estimated bandwidth in bytes per millisecond (integer)
    pub bandwidth_bytes_per_ms: u64,
    /// Estimated remaining time in milliseconds
    pub estimated_remaining_ms: u64,
}

impl MigrationProgress {
    /// Update bandwidth estimate (integer math)
    /// `bytes_sent`: bytes transferred in this iteration
    /// `elapsed_ms`: time for this iteration in milliseconds
    pub fn update_bandwidth(&mut self, bytes_sent: u64, elapsed_ms: u64) {
        if elapsed_ms > 0 {
            self.bandwidth_bytes_per_ms = bytes_sent / elapsed_ms;
        }
        self.transferred_bytes = self.transferred_bytes.saturating_add(bytes_sent);
    }

    /// Estimate remaining transfer time
    pub fn estimate_remaining(&mut self) {
        if self.bandwidth_bytes_per_ms > 0 {
            let remaining = self.total_bytes.saturating_sub(self.transferred_bytes);
            self.estimated_remaining_ms = remaining / self.bandwidth_bytes_per_ms;
        }
    }

    /// Calculate completion percentage (0-100, integer)
    pub fn completion_percent(&self) -> u32 {
        if self.total_bytes == 0 {
            return 100;
        }
        // Use checked_mul to avoid overflow on large memory sizes
        let percent = self
            .transferred_bytes
            .checked_mul(100)
            .map(|v| v / self.total_bytes)
            .unwrap_or(100);
        if percent > 100 {
            100
        } else {
            percent as u32
        }
    }

    /// Check if dirty page convergence threshold is met
    /// Returns true if dirty pages decreased by at least the given percentage
    pub fn has_converged(&self, threshold_percent: u32) -> bool {
        if self.previous_dirty_pages == 0 {
            return true;
        }
        // current_dirty < previous_dirty * (100 - threshold) / 100
        let threshold_pages = self
            .previous_dirty_pages
            .checked_mul((100 - threshold_percent) as u64)
            .map(|v| v / 100)
            .unwrap_or(0);
        self.current_dirty_pages <= threshold_pages
    }
}

/// Live migration controller
#[cfg(feature = "alloc")]
pub struct MigrationController {
    /// Current migration state
    state: MigrationState,
    /// Progress tracking
    progress: MigrationProgress,
    /// Dirty page bitmap
    dirty_bitmap: Option<DirtyPageBitmap>,
    /// Serialized VMCS state
    vmcs_state: Option<SerializedVmcs>,
    /// Source VM ID
    source_vm_id: u64,
    /// Convergence threshold (percent reduction in dirty pages)
    convergence_threshold: u32,
    /// Maximum pre-copy iterations before stop-and-copy
    max_precopy_iterations: u32,
}

#[cfg(feature = "alloc")]
impl MigrationController {
    pub fn new(source_vm_id: u64) -> Self {
        Self {
            state: MigrationState::Idle,
            progress: MigrationProgress::default(),
            dirty_bitmap: None,
            vmcs_state: None,
            source_vm_id,
            convergence_threshold: 20, // 20% reduction required
            max_precopy_iterations: 30,
        }
    }

    /// Begin migration setup
    pub fn begin_setup(&mut self, total_memory_pages: u64) -> Result<(), VmError> {
        if self.state != MigrationState::Idle {
            return Err(VmError::InvalidVmState);
        }

        self.dirty_bitmap = Some(DirtyPageBitmap::new(total_memory_pages));
        self.progress.total_bytes = total_memory_pages
            .checked_mul(PAGE_SIZE)
            .ok_or(VmError::GuestMemoryError)?;
        self.state = MigrationState::Setup;
        Ok(())
    }

    /// Transition to pre-copy phase
    pub fn begin_precopy(&mut self) -> Result<(), VmError> {
        if self.state != MigrationState::Setup {
            return Err(VmError::InvalidVmState);
        }

        // Mark all pages dirty for initial transfer
        if let Some(ref mut bitmap) = self.dirty_bitmap {
            let total = bitmap.total_pages();
            for i in 0..total {
                bitmap.set_dirty(i);
            }
        }

        self.progress.iteration = 0;
        self.state = MigrationState::PreCopy;
        Ok(())
    }

    /// Perform one pre-copy iteration: returns list of dirty page indices to
    /// send
    pub fn precopy_iteration(&mut self) -> Result<Vec<u64>, VmError> {
        if self.state != MigrationState::PreCopy {
            return Err(VmError::InvalidVmState);
        }

        let dirty_pages: Vec<u64> = if let Some(ref bitmap) = self.dirty_bitmap {
            bitmap
                .dirty_pages()
                .take(PRECOPY_BATCH_SIZE as usize)
                .collect()
        } else {
            return Err(VmError::InvalidVmState);
        };

        // Update progress
        self.progress.previous_dirty_pages = self.progress.current_dirty_pages;
        self.progress.current_dirty_pages = if let Some(ref bitmap) = self.dirty_bitmap {
            bitmap.dirty_count()
        } else {
            0
        };
        self.progress.iteration += 1;

        // Clear sent pages from bitmap
        if let Some(ref mut bitmap) = self.dirty_bitmap {
            for &page_idx in &dirty_pages {
                bitmap.clear_dirty(page_idx);
            }
        }

        // Check convergence or max iterations
        if self.progress.has_converged(self.convergence_threshold)
            || self.progress.iteration >= self.max_precopy_iterations
        {
            // Time to stop and copy
            self.state = MigrationState::StopAndCopy;
        }

        Ok(dirty_pages)
    }

    /// Begin stop-and-copy phase (VM must be paused)
    pub fn begin_stop_and_copy(&mut self) -> Result<(), VmError> {
        if self.state != MigrationState::PreCopy && self.state != MigrationState::StopAndCopy {
            return Err(VmError::InvalidVmState);
        }
        self.state = MigrationState::StopAndCopy;
        Ok(())
    }

    /// Serialize VMCS state for transfer
    pub fn serialize_vmcs(&mut self, fields: &[(u32, u64)]) -> Result<(), VmError> {
        let mut vmcs = SerializedVmcs::new();
        for &(encoding, value) in fields {
            vmcs.add_field(encoding, value);
        }
        self.vmcs_state = Some(vmcs);
        Ok(())
    }

    /// Get remaining dirty pages for stop-and-copy final transfer
    pub fn final_dirty_pages(&self) -> Result<Vec<u64>, VmError> {
        if self.state != MigrationState::StopAndCopy {
            return Err(VmError::InvalidVmState);
        }
        if let Some(ref bitmap) = self.dirty_bitmap {
            Ok(bitmap.dirty_pages().collect())
        } else {
            Err(VmError::InvalidVmState)
        }
    }

    /// Complete the migration
    pub fn complete(&mut self) -> Result<(), VmError> {
        if self.state != MigrationState::StopAndCopy {
            return Err(VmError::InvalidVmState);
        }
        self.state = MigrationState::Complete;
        Ok(())
    }

    /// Mark migration as failed
    pub fn fail(&mut self) {
        self.state = MigrationState::Failed;
    }

    pub fn state(&self) -> MigrationState {
        self.state
    }

    pub fn progress(&self) -> &MigrationProgress {
        &self.progress
    }

    pub fn source_vm_id(&self) -> u64 {
        self.source_vm_id
    }

    pub fn vmcs_state(&self) -> Option<&SerializedVmcs> {
        self.vmcs_state.as_ref()
    }
}
