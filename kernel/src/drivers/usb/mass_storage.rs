//! USB Mass Storage Class Driver
//!
//! Implements the USB Mass Storage Bulk-Only Transport (BOT) protocol with
//! SCSI Transparent Command Set for block device access. Supports standard
//! SCSI commands: INQUIRY, TEST UNIT READY, READ CAPACITY(10), READ(10),
//! WRITE(10), and REQUEST SENSE.
//!
//! Reference: USB Mass Storage Class Bulk-Only Transport Specification Rev 1.0

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// SCSI Command Opcodes
// ---------------------------------------------------------------------------

/// SCSI command opcodes used with USB Mass Storage BOT
mod scsi_opcodes {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const REQUEST_SENSE: u8 = 0x03;
    pub const INQUIRY: u8 = 0x12;
    pub const READ_CAPACITY_10: u8 = 0x25;
    pub const READ_10: u8 = 0x28;
    pub const WRITE_10: u8 = 0x2A;
}

// ---------------------------------------------------------------------------
// SCSI Sense Key Definitions
// ---------------------------------------------------------------------------

/// SCSI sense keys for error reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SenseKey {
    NoSense = 0x00,
    RecoveredError = 0x01,
    NotReady = 0x02,
    MediumError = 0x03,
    HardwareError = 0x04,
    IllegalRequest = 0x05,
    UnitAttention = 0x06,
    DataProtect = 0x07,
    BlankCheck = 0x08,
    AbortedCommand = 0x0B,
    VolumeOverflow = 0x0D,
    Miscompare = 0x0E,
}

impl SenseKey {
    /// Parse a sense key from a raw byte value
    pub fn from_byte(byte: u8) -> Self {
        match byte & 0x0F {
            0x00 => Self::NoSense,
            0x01 => Self::RecoveredError,
            0x02 => Self::NotReady,
            0x03 => Self::MediumError,
            0x04 => Self::HardwareError,
            0x05 => Self::IllegalRequest,
            0x06 => Self::UnitAttention,
            0x07 => Self::DataProtect,
            0x08 => Self::BlankCheck,
            0x0B => Self::AbortedCommand,
            0x0D => Self::VolumeOverflow,
            0x0E => Self::Miscompare,
            _ => Self::HardwareError,
        }
    }
}

// ---------------------------------------------------------------------------
// Command Block Wrapper (CBW)
// ---------------------------------------------------------------------------

/// CBW signature: "USBC" in little-endian
pub const CBW_SIGNATURE: u32 = 0x43425355;

/// CBW direction flags
pub const CBW_DIRECTION_OUT: u8 = 0x00;
pub const CBW_DIRECTION_IN: u8 = 0x80;

/// Command Block Wrapper (31 bytes)
///
/// Sent from host to device on the Bulk-Out endpoint to initiate a command.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct CommandBlockWrapper {
    /// Signature: must be CBW_SIGNATURE (0x43425355)
    pub signature: u32,
    /// Tag: unique per-command identifier, echoed in CSW
    pub tag: u32,
    /// Data transfer length in bytes
    pub data_transfer_length: u32,
    /// Flags: bit 7 = direction (0 = OUT, 1 = IN)
    pub flags: u8,
    /// Logical Unit Number (bits 3:0)
    pub lun: u8,
    /// Length of the command block (1-16)
    pub cb_length: u8,
    /// SCSI Command Descriptor Block (16 bytes, zero-padded)
    pub cb: [u8; 16],
}

impl CommandBlockWrapper {
    /// Size of a CBW in bytes
    pub const SIZE: usize = 31;

    /// Create a new CBW with default values
    pub fn new(tag: u32, data_length: u32, direction: u8, lun: u8, command: &[u8]) -> Self {
        let cb_len = command.len().min(16) as u8;
        let mut cb = [0u8; 16];
        let copy_len = command.len().min(16);
        cb[..copy_len].copy_from_slice(&command[..copy_len]);

        Self {
            signature: CBW_SIGNATURE,
            tag,
            data_transfer_length: data_length,
            flags: direction,
            lun: lun & 0x0F,
            cb_length: cb_len,
            cb,
        }
    }

    /// Serialize the CBW to a 31-byte array (little-endian)
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        let sig = self.signature.to_le_bytes();
        buf[0..4].copy_from_slice(&sig);
        let tag = self.tag.to_le_bytes();
        buf[4..8].copy_from_slice(&tag);
        let dtl = self.data_transfer_length.to_le_bytes();
        buf[8..12].copy_from_slice(&dtl);
        buf[12] = self.flags;
        buf[13] = self.lun;
        buf[14] = self.cb_length;
        buf[15..31].copy_from_slice(&self.cb);
        buf
    }

    /// Deserialize a CBW from a 31-byte slice
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < Self::SIZE {
            return Err(KernelError::InvalidArgument {
                name: "cbw_data",
                value: "buffer too small for CBW",
            });
        }
        let signature = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if signature != CBW_SIGNATURE {
            return Err(KernelError::InvalidArgument {
                name: "cbw_signature",
                value: "invalid CBW signature",
            });
        }
        let tag = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let data_transfer_length = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let flags = data[12];
        let lun = data[13];
        let cb_length = data[14];
        let mut cb = [0u8; 16];
        cb.copy_from_slice(&data[15..31]);

        Ok(Self {
            signature,
            tag,
            data_transfer_length,
            flags,
            lun,
            cb_length,
            cb,
        })
    }
}

// ---------------------------------------------------------------------------
// Command Status Wrapper (CSW)
// ---------------------------------------------------------------------------

/// CSW signature: "USBS" in little-endian
pub const CSW_SIGNATURE: u32 = 0x53425355;

/// CSW status values
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CswStatus {
    /// Command passed (good status)
    Passed = 0x00,
    /// Command failed
    Failed = 0x01,
    /// Phase error (requires reset recovery)
    PhaseError = 0x02,
}

impl CswStatus {
    /// Parse status from raw byte
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::Passed,
            0x01 => Self::Failed,
            _ => Self::PhaseError,
        }
    }
}

/// Command Status Wrapper (13 bytes)
///
/// Sent from device to host on the Bulk-In endpoint after command completion.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct CommandStatusWrapper {
    /// Signature: must be CSW_SIGNATURE (0x53425355)
    pub signature: u32,
    /// Tag: must match the tag from the corresponding CBW
    pub tag: u32,
    /// Data residue: difference between expected and actual data transferred
    pub data_residue: u32,
    /// Status byte
    pub status: u8,
}

impl CommandStatusWrapper {
    /// Size of a CSW in bytes
    pub const SIZE: usize = 13;

    /// Deserialize a CSW from a 13-byte slice
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < Self::SIZE {
            return Err(KernelError::InvalidArgument {
                name: "csw_data",
                value: "buffer too small for CSW",
            });
        }
        let signature = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if signature != CSW_SIGNATURE {
            return Err(KernelError::InvalidArgument {
                name: "csw_signature",
                value: "invalid CSW signature",
            });
        }
        let tag = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let data_residue = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let status = data[12];

        Ok(Self {
            signature,
            tag,
            data_residue,
            status,
        })
    }

    /// Serialize the CSW to a 13-byte array
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.signature.to_le_bytes());
        buf[4..8].copy_from_slice(&self.tag.to_le_bytes());
        buf[8..12].copy_from_slice(&self.data_residue.to_le_bytes());
        buf[12] = self.status;
        buf
    }

    /// Get the status as a typed enum
    pub fn get_status(&self) -> CswStatus {
        CswStatus::from_byte(self.status)
    }
}

// ---------------------------------------------------------------------------
// SCSI Sense Data
// ---------------------------------------------------------------------------

/// Parsed SCSI sense data from REQUEST SENSE response
#[derive(Debug, Clone, Copy)]
pub struct SenseData {
    /// Response code (0x70 = current, 0x71 = deferred)
    pub response_code: u8,
    /// Sense key
    pub sense_key: SenseKey,
    /// Additional Sense Code (ASC)
    pub asc: u8,
    /// Additional Sense Code Qualifier (ASCQ)
    pub ascq: u8,
}

impl SenseData {
    /// Parse sense data from a REQUEST SENSE response buffer
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 14 {
            return Err(KernelError::InvalidArgument {
                name: "sense_data",
                value: "buffer too small for sense data",
            });
        }
        Ok(Self {
            response_code: data[0] & 0x7F,
            sense_key: SenseKey::from_byte(data[2]),
            asc: data[12],
            ascq: data[13],
        })
    }

    /// Returns true if no error condition is present
    pub fn is_ok(&self) -> bool {
        self.sense_key == SenseKey::NoSense
    }
}

// ---------------------------------------------------------------------------
// SCSI Inquiry Data
// ---------------------------------------------------------------------------

/// Parsed SCSI INQUIRY response
#[derive(Debug, Clone, Copy)]
pub struct InquiryData {
    /// Peripheral device type (e.g., 0x00 = direct access block device)
    pub device_type: u8,
    /// Removable media indicator
    pub removable: bool,
    /// SCSI version supported
    pub version: u8,
    /// Vendor identification (8 bytes, space-padded ASCII)
    pub vendor: [u8; 8],
    /// Product identification (16 bytes, space-padded ASCII)
    pub product: [u8; 16],
    /// Product revision level (4 bytes)
    pub revision: [u8; 4],
}

impl InquiryData {
    /// Parse INQUIRY response data (minimum 36 bytes standard response)
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 36 {
            return Err(KernelError::InvalidArgument {
                name: "inquiry_data",
                value: "buffer too small for inquiry response",
            });
        }
        let mut vendor = [0u8; 8];
        vendor.copy_from_slice(&data[8..16]);
        let mut product = [0u8; 16];
        product.copy_from_slice(&data[16..32]);
        let mut revision = [0u8; 4];
        revision.copy_from_slice(&data[32..36]);

        Ok(Self {
            device_type: data[0] & 0x1F,
            removable: (data[1] & 0x80) != 0,
            version: data[2],
            vendor,
            product,
            revision,
        })
    }
}

// ---------------------------------------------------------------------------
// Device State
// ---------------------------------------------------------------------------

/// Mass storage device states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MassStorageState {
    /// Device has not been initialized
    Uninitialized,
    /// Device is ready for I/O
    Ready,
    /// Device encountered an error
    Error,
}

// ---------------------------------------------------------------------------
// Block Device Trait
// ---------------------------------------------------------------------------

/// Trait for block-level I/O on a storage device
pub trait BlockDevice {
    /// Read `count` blocks starting at logical block address `lba` into `buf`.
    /// Returns the number of bytes actually read.
    fn read_blocks(&self, lba: u64, count: u32, buf: &mut [u8]) -> Result<usize, KernelError>;

    /// Write `count` blocks starting at logical block address `lba` from `buf`.
    /// Returns the number of bytes actually written.
    fn write_blocks(&self, lba: u64, count: u32, buf: &[u8]) -> Result<usize, KernelError>;

    /// Get the block (sector) size in bytes
    fn block_size(&self) -> u32;

    /// Get the total number of blocks on the device
    fn total_blocks(&self) -> u64;
}

// ---------------------------------------------------------------------------
// Mass Storage Device
// ---------------------------------------------------------------------------

/// Global tag counter for CBW/CSW matching
static NEXT_TAG: AtomicU32 = AtomicU32::new(1);

/// Generate a unique tag for CBW/CSW pairs
fn next_tag() -> u32 {
    NEXT_TAG.fetch_add(1, Ordering::Relaxed)
}

/// USB Mass Storage device using Bulk-Only Transport (BOT)
#[derive(Debug)]
pub struct MassStorageDevice {
    /// USB device address on the bus
    pub device_address: u8,
    /// Bulk-In endpoint address
    pub bulk_in_ep: u8,
    /// Bulk-Out endpoint address
    pub bulk_out_ep: u8,
    /// Active Logical Unit Number
    pub lun: u8,
    /// Maximum number of LUNs supported by the device
    pub max_lun: u8,
    /// Cached block (sector) size in bytes
    pub cached_block_size: u32,
    /// Cached total block count
    pub cached_total_blocks: u64,
    /// Current device state
    pub state: MassStorageState,
    /// Last sense data from REQUEST SENSE
    pub last_sense: Option<SenseData>,
}

impl MassStorageDevice {
    /// Create a new mass storage device handle.
    ///
    /// `device_address`: USB device address on the bus
    /// `bulk_in_ep`: endpoint address for Bulk-In transfers
    /// `bulk_out_ep`: endpoint address for Bulk-Out transfers
    pub fn new(device_address: u8, bulk_in_ep: u8, bulk_out_ep: u8) -> Self {
        Self {
            device_address,
            bulk_in_ep,
            bulk_out_ep,
            lun: 0,
            max_lun: 0,
            cached_block_size: 0,
            cached_total_blocks: 0,
            state: MassStorageState::Uninitialized,
            last_sense: None,
        }
    }

    /// Set the active Logical Unit Number
    pub fn set_lun(&mut self, lun: u8) {
        self.lun = lun;
    }

    /// Build a SCSI INQUIRY command (6-byte CDB)
    pub fn build_inquiry_cdb(allocation_length: u8) -> [u8; 6] {
        [
            scsi_opcodes::INQUIRY,
            0, // reserved
            0, // reserved
            0, // reserved
            allocation_length,
            0, // control
        ]
    }

    /// Build a SCSI TEST UNIT READY command (6-byte CDB)
    pub fn build_test_unit_ready_cdb() -> [u8; 6] {
        [
            scsi_opcodes::TEST_UNIT_READY,
            0,
            0,
            0,
            0,
            0, // control
        ]
    }

    /// Build a SCSI REQUEST SENSE command (6-byte CDB)
    pub fn build_request_sense_cdb(allocation_length: u8) -> [u8; 6] {
        [
            scsi_opcodes::REQUEST_SENSE,
            0,
            0,
            0,
            allocation_length,
            0, // control
        ]
    }

    /// Build a SCSI READ CAPACITY(10) command (10-byte CDB)
    pub fn build_read_capacity_10_cdb() -> [u8; 10] {
        [
            scsi_opcodes::READ_CAPACITY_10,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0, // control
        ]
    }

    /// Build a SCSI READ(10) command (10-byte CDB)
    pub fn build_read_10_cdb(lba: u32, transfer_length: u16) -> [u8; 10] {
        let lba_bytes = lba.to_be_bytes();
        let len_bytes = transfer_length.to_be_bytes();
        [
            scsi_opcodes::READ_10,
            0, // flags
            lba_bytes[0],
            lba_bytes[1],
            lba_bytes[2],
            lba_bytes[3],
            0, // group number
            len_bytes[0],
            len_bytes[1],
            0, // control
        ]
    }

    /// Build a SCSI WRITE(10) command (10-byte CDB)
    pub fn build_write_10_cdb(lba: u32, transfer_length: u16) -> [u8; 10] {
        let lba_bytes = lba.to_be_bytes();
        let len_bytes = transfer_length.to_be_bytes();
        [
            scsi_opcodes::WRITE_10,
            0, // flags
            lba_bytes[0],
            lba_bytes[1],
            lba_bytes[2],
            lba_bytes[3],
            0, // group number
            len_bytes[0],
            len_bytes[1],
            0, // control
        ]
    }

    /// Build a CBW for the given SCSI command
    fn build_cbw(&self, command: &[u8], data_length: u32, direction: u8) -> CommandBlockWrapper {
        CommandBlockWrapper::new(next_tag(), data_length, direction, self.lun, command)
    }

    /// Validate a CSW against the expected tag and check status.
    fn validate_csw(
        &mut self,
        csw_bytes: &[u8],
        expected_tag: u32,
    ) -> Result<CommandStatusWrapper, KernelError> {
        let csw = CommandStatusWrapper::from_bytes(csw_bytes)?;
        if csw.tag != expected_tag {
            self.state = MassStorageState::Error;
            return Err(KernelError::HardwareError {
                device: "usb-mass-storage",
                code: 0x10, // tag mismatch
            });
        }
        match csw.get_status() {
            CswStatus::Passed => Ok(csw),
            CswStatus::Failed => {
                // Command failed -- caller should issue REQUEST SENSE
                Err(KernelError::HardwareError {
                    device: "usb-mass-storage",
                    code: 0x11, // command failed
                })
            }
            CswStatus::PhaseError => {
                self.state = MassStorageState::Error;
                Err(KernelError::HardwareError {
                    device: "usb-mass-storage",
                    code: 0x12, // phase error, needs reset recovery
                })
            }
        }
    }

    /// Initialize the device: INQUIRY, TEST UNIT READY, READ CAPACITY.
    ///
    /// On success, `cached_block_size` and `cached_total_blocks` are populated
    /// and the device transitions to the `Ready` state.
    pub fn initialize(&mut self) -> Result<InquiryData, KernelError> {
        // Step 1: INQUIRY
        let inquiry_cdb = Self::build_inquiry_cdb(36);
        let cbw = self.build_cbw(&inquiry_cdb, 36, CBW_DIRECTION_IN);
        let inquiry_data = self.execute_command_in(&cbw, 36)?;
        let inquiry = InquiryData::from_bytes(&inquiry_data)?;

        // Verify this is a direct-access block device (type 0x00)
        if inquiry.device_type != 0x00 {
            self.state = MassStorageState::Error;
            return Err(KernelError::HardwareError {
                device: "usb-mass-storage",
                code: 0x20, // unsupported device type
            });
        }

        // Step 2: TEST UNIT READY
        let tur_cdb = Self::build_test_unit_ready_cdb();
        let cbw = self.build_cbw(&tur_cdb, 0, CBW_DIRECTION_OUT);
        if let Err(_e) = self.execute_command_none(&cbw) {
            // Device may need time -- try REQUEST SENSE and retry
            let _sense = self.request_sense();
            // Retry TEST UNIT READY once
            let cbw = self.build_cbw(&tur_cdb, 0, CBW_DIRECTION_OUT);
            self.execute_command_none(&cbw)?;
        }

        // Step 3: READ CAPACITY(10)
        let rc_cdb = Self::build_read_capacity_10_cdb();
        let cbw = self.build_cbw(&rc_cdb, 8, CBW_DIRECTION_IN);
        let cap_data = self.execute_command_in(&cbw, 8)?;

        if cap_data.len() >= 8 {
            let last_lba = u32::from_be_bytes([cap_data[0], cap_data[1], cap_data[2], cap_data[3]]);
            let block_size =
                u32::from_be_bytes([cap_data[4], cap_data[5], cap_data[6], cap_data[7]]);

            self.cached_total_blocks = (last_lba as u64) + 1;
            self.cached_block_size = block_size;
        }

        self.state = MassStorageState::Ready;
        Ok(inquiry)
    }

    /// Issue a SCSI REQUEST SENSE command and cache the result
    pub fn request_sense(&mut self) -> Result<SenseData, KernelError> {
        let cdb = Self::build_request_sense_cdb(18);
        let cbw = self.build_cbw(&cdb, 18, CBW_DIRECTION_IN);
        let sense_bytes = self.execute_command_in(&cbw, 18)?;
        let sense = SenseData::from_bytes(&sense_bytes)?;
        self.last_sense = Some(sense);
        Ok(sense)
    }

    /// Execute a BOT command with a data-in phase (device-to-host).
    ///
    /// This is a placeholder transport layer that builds the CBW bytes and
    /// simulates the three BOT phases (CBW out, data in, CSW in). In a real
    /// driver, each phase would use bulk endpoint transfers via the USB host
    /// controller.
    fn execute_command_in(
        &mut self,
        cbw: &CommandBlockWrapper,
        expected_len: u32,
    ) -> Result<Vec<u8>, KernelError> {
        // Phase 1: Send CBW on Bulk-Out
        let _cbw_bytes = cbw.to_bytes();

        // In a real implementation:
        //   usb_bulk_out(self.bulk_out_ep, &cbw_bytes)?;
        //   let data = usb_bulk_in(self.bulk_in_ep, expected_len)?;
        //   let csw_bytes = usb_bulk_in(self.bulk_in_ep, 13)?;
        //   self.validate_csw(&csw_bytes, cbw.tag)?;

        // Stub: return zeroed data of the requested length
        let data = alloc::vec![0u8; expected_len as usize];
        Ok(data)
    }

    /// Execute a BOT command with no data phase
    fn execute_command_none(&mut self, cbw: &CommandBlockWrapper) -> Result<(), KernelError> {
        let _cbw_bytes = cbw.to_bytes();

        // In a real implementation:
        //   usb_bulk_out(self.bulk_out_ep, &cbw_bytes)?;
        //   let csw_bytes = usb_bulk_in(self.bulk_in_ep, 13)?;
        //   self.validate_csw(&csw_bytes, cbw.tag)?;

        Ok(())
    }

    /// Execute a BOT command with a data-out phase (host-to-device)
    fn execute_command_out(
        &mut self,
        cbw: &CommandBlockWrapper,
        data: &[u8],
    ) -> Result<(), KernelError> {
        let _cbw_bytes = cbw.to_bytes();
        let _data_out = data;

        // In a real implementation:
        //   usb_bulk_out(self.bulk_out_ep, &cbw_bytes)?;
        //   usb_bulk_out(self.bulk_out_ep, data)?;
        //   let csw_bytes = usb_bulk_in(self.bulk_in_ep, 13)?;
        //   self.validate_csw(&csw_bytes, cbw.tag)?;

        Ok(())
    }
}

impl BlockDevice for MassStorageDevice {
    fn read_blocks(&self, lba: u64, count: u32, buf: &mut [u8]) -> Result<usize, KernelError> {
        if self.state != MassStorageState::Ready {
            return Err(KernelError::InvalidState {
                expected: "Ready",
                actual: "not Ready",
            });
        }

        if self.cached_block_size == 0 {
            return Err(KernelError::InvalidState {
                expected: "initialized (block_size > 0)",
                actual: "block_size is 0",
            });
        }

        let total_bytes = (count as usize) * (self.cached_block_size as usize);
        if buf.len() < total_bytes {
            return Err(KernelError::InvalidArgument {
                name: "buf",
                value: "buffer too small for requested blocks",
            });
        }

        // Validate LBA range
        if lba.saturating_add(count as u64) > self.cached_total_blocks {
            return Err(KernelError::InvalidArgument {
                name: "lba+count",
                value: "exceeds device capacity",
            });
        }

        // READ(10) uses 32-bit LBA and 16-bit transfer count
        let lba32 = lba as u32;
        let cdb = Self::build_read_10_cdb(lba32, count as u16);
        let cbw = CommandBlockWrapper::new(
            next_tag(),
            total_bytes as u32,
            CBW_DIRECTION_IN,
            self.lun,
            &cdb,
        );
        let _cbw_bytes = cbw.to_bytes();

        // Stub: in a real driver, data would be read via bulk-in
        // For now, zero-fill the buffer to indicate successful stub execution
        for byte in buf[..total_bytes].iter_mut() {
            *byte = 0;
        }

        Ok(total_bytes)
    }

    fn write_blocks(&self, lba: u64, count: u32, buf: &[u8]) -> Result<usize, KernelError> {
        if self.state != MassStorageState::Ready {
            return Err(KernelError::InvalidState {
                expected: "Ready",
                actual: "not Ready",
            });
        }

        if self.cached_block_size == 0 {
            return Err(KernelError::InvalidState {
                expected: "initialized (block_size > 0)",
                actual: "block_size is 0",
            });
        }

        let total_bytes = (count as usize) * (self.cached_block_size as usize);
        if buf.len() < total_bytes {
            return Err(KernelError::InvalidArgument {
                name: "buf",
                value: "buffer too small for requested blocks",
            });
        }

        // Validate LBA range
        if lba.saturating_add(count as u64) > self.cached_total_blocks {
            return Err(KernelError::InvalidArgument {
                name: "lba+count",
                value: "exceeds device capacity",
            });
        }

        let lba32 = lba as u32;
        let cdb = Self::build_write_10_cdb(lba32, count as u16);
        let cbw = CommandBlockWrapper::new(
            next_tag(),
            total_bytes as u32,
            CBW_DIRECTION_OUT,
            self.lun,
            &cdb,
        );
        let _cbw_bytes = cbw.to_bytes();

        // Stub: in a real driver, data would be sent via bulk-out
        Ok(total_bytes)
    }

    fn block_size(&self) -> u32 {
        self.cached_block_size
    }

    fn total_blocks(&self) -> u64 {
        self.cached_total_blocks
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cbw_signature() {
        assert_eq!(CBW_SIGNATURE, 0x43425355);
    }

    #[test]
    fn test_csw_signature() {
        assert_eq!(CSW_SIGNATURE, 0x53425355);
    }

    #[test]
    fn test_cbw_serialization_roundtrip() {
        let command = [0x28, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00];
        let cbw = CommandBlockWrapper::new(42, 4096, CBW_DIRECTION_IN, 0, &command);
        let bytes = cbw.to_bytes();

        assert_eq!(bytes.len(), CommandBlockWrapper::SIZE);
        assert_eq!(bytes[0], 0x55); // 'U' (LE byte 0 of 0x43425355)
        assert_eq!(bytes[1], 0x53); // 'S'
        assert_eq!(bytes[2], 0x42); // 'B'
        assert_eq!(bytes[3], 0x43); // 'C'

        let parsed = CommandBlockWrapper::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.tag, 42);
        assert_eq!(parsed.data_transfer_length, 4096);
        assert_eq!(parsed.flags, CBW_DIRECTION_IN);
        assert_eq!(parsed.lun, 0);
        assert_eq!(parsed.cb[0], 0x28); // READ(10) opcode
    }

    #[test]
    fn test_cbw_from_bytes_invalid_signature() {
        let mut bytes = [0u8; 31];
        bytes[0] = 0xFF; // wrong signature
        let result = CommandBlockWrapper::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_cbw_from_bytes_too_short() {
        let bytes = [0u8; 10];
        let result = CommandBlockWrapper::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_csw_serialization_roundtrip() {
        let csw = CommandStatusWrapper {
            signature: CSW_SIGNATURE,
            tag: 42,
            data_residue: 0,
            status: CswStatus::Passed as u8,
        };
        let bytes = csw.to_bytes();
        assert_eq!(bytes.len(), CommandStatusWrapper::SIZE);

        let parsed = CommandStatusWrapper::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.tag, 42);
        assert_eq!(parsed.data_residue, 0);
        assert_eq!(parsed.get_status(), CswStatus::Passed);
    }

    #[test]
    fn test_csw_failed_status() {
        let csw = CommandStatusWrapper {
            signature: CSW_SIGNATURE,
            tag: 1,
            data_residue: 512,
            status: CswStatus::Failed as u8,
        };
        let bytes = csw.to_bytes();
        let parsed = CommandStatusWrapper::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.get_status(), CswStatus::Failed);
        assert_eq!(parsed.data_residue, 512);
    }

    #[test]
    fn test_csw_phase_error() {
        let status = CswStatus::from_byte(0x02);
        assert_eq!(status, CswStatus::PhaseError);
        // Unknown values also map to PhaseError
        let unknown = CswStatus::from_byte(0xFF);
        assert_eq!(unknown, CswStatus::PhaseError);
    }

    #[test]
    fn test_sense_key_parsing() {
        assert_eq!(SenseKey::from_byte(0x00), SenseKey::NoSense);
        assert_eq!(SenseKey::from_byte(0x02), SenseKey::NotReady);
        assert_eq!(SenseKey::from_byte(0x05), SenseKey::IllegalRequest);
        assert_eq!(SenseKey::from_byte(0x03), SenseKey::MediumError);
        // Upper nibble should be masked
        assert_eq!(SenseKey::from_byte(0xF5), SenseKey::IllegalRequest);
    }

    #[test]
    fn test_sense_data_parsing() {
        let mut raw = [0u8; 18];
        raw[0] = 0x70; // current errors, fixed format
        raw[2] = 0x05; // ILLEGAL REQUEST
        raw[12] = 0x24; // ASC: invalid field in CDB
        raw[13] = 0x00; // ASCQ
        let sense = SenseData::from_bytes(&raw).unwrap();
        assert_eq!(sense.response_code, 0x70);
        assert_eq!(sense.sense_key, SenseKey::IllegalRequest);
        assert_eq!(sense.asc, 0x24);
        assert_eq!(sense.ascq, 0x00);
        assert!(!sense.is_ok());
    }

    #[test]
    fn test_sense_data_no_sense() {
        let mut raw = [0u8; 18];
        raw[0] = 0x70;
        raw[2] = 0x00; // NO SENSE
        let sense = SenseData::from_bytes(&raw).unwrap();
        assert!(sense.is_ok());
    }

    #[test]
    fn test_inquiry_data_parsing() {
        let mut raw = [0u8; 36];
        raw[0] = 0x00; // direct access block device
        raw[1] = 0x80; // removable
        raw[2] = 0x05; // SPC-3
                       // Vendor: "VERIDIAN"
        raw[8..16].copy_from_slice(b"VERIDIAN");
        // Product: "USB DISK        "
        raw[16..32].copy_from_slice(b"USB DISK        ");
        // Revision: "1.00"
        raw[32..36].copy_from_slice(b"1.00");

        let inquiry = InquiryData::from_bytes(&raw).unwrap();
        assert_eq!(inquiry.device_type, 0x00);
        assert!(inquiry.removable);
        assert_eq!(inquiry.version, 0x05);
        assert_eq!(&inquiry.vendor, b"VERIDIAN");
        assert_eq!(&inquiry.product, b"USB DISK        ");
        assert_eq!(&inquiry.revision, b"1.00");
    }

    #[test]
    fn test_mass_storage_device_creation() {
        let dev = MassStorageDevice::new(1, 0x81, 0x02);
        assert_eq!(dev.device_address, 1);
        assert_eq!(dev.bulk_in_ep, 0x81);
        assert_eq!(dev.bulk_out_ep, 0x02);
        assert_eq!(dev.lun, 0);
        assert_eq!(dev.state, MassStorageState::Uninitialized);
        assert_eq!(dev.cached_block_size, 0);
        assert_eq!(dev.cached_total_blocks, 0);
    }

    #[test]
    fn test_block_device_read_not_ready() {
        let dev = MassStorageDevice::new(1, 0x81, 0x02);
        let mut buf = [0u8; 512];
        let result = dev.read_blocks(0, 1, &mut buf);
        assert!(result.is_err()); // device is Uninitialized
    }

    #[test]
    fn test_block_device_write_not_ready() {
        let dev = MassStorageDevice::new(1, 0x81, 0x02);
        let buf = [0u8; 512];
        let result = dev.write_blocks(0, 1, &buf);
        assert!(result.is_err()); // device is Uninitialized
    }

    #[test]
    fn test_block_device_read_ready() {
        let mut dev = MassStorageDevice::new(1, 0x81, 0x02);
        dev.state = MassStorageState::Ready;
        dev.cached_block_size = 512;
        dev.cached_total_blocks = 1024;

        let mut buf = [0xFFu8; 512];
        let result = dev.read_blocks(0, 1, &mut buf);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 512);
    }

    #[test]
    fn test_block_device_write_ready() {
        let mut dev = MassStorageDevice::new(1, 0x81, 0x02);
        dev.state = MassStorageState::Ready;
        dev.cached_block_size = 512;
        dev.cached_total_blocks = 1024;

        let buf = [0xAAu8; 512];
        let result = dev.write_blocks(0, 1, &buf);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 512);
    }

    #[test]
    fn test_block_device_buffer_too_small() {
        let mut dev = MassStorageDevice::new(1, 0x81, 0x02);
        dev.state = MassStorageState::Ready;
        dev.cached_block_size = 512;
        dev.cached_total_blocks = 1024;

        let mut buf = [0u8; 256]; // too small for 1 block of 512
        let result = dev.read_blocks(0, 1, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_block_device_lba_out_of_range() {
        let mut dev = MassStorageDevice::new(1, 0x81, 0x02);
        dev.state = MassStorageState::Ready;
        dev.cached_block_size = 512;
        dev.cached_total_blocks = 100;

        let mut buf = [0u8; 512];
        // LBA 100 is past the end (0-99 valid)
        let result = dev.read_blocks(100, 1, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_scsi_read_10_cdb() {
        let cdb = MassStorageDevice::build_read_10_cdb(0x00000100, 8);
        assert_eq!(cdb[0], 0x28); // READ(10) opcode
                                  // LBA = 0x00000100, big-endian
        assert_eq!(cdb[2], 0x00);
        assert_eq!(cdb[3], 0x00);
        assert_eq!(cdb[4], 0x01);
        assert_eq!(cdb[5], 0x00);
        // Transfer length = 8 blocks, big-endian
        assert_eq!(cdb[7], 0x00);
        assert_eq!(cdb[8], 0x08);
    }

    #[test]
    fn test_scsi_write_10_cdb() {
        let cdb = MassStorageDevice::build_write_10_cdb(0x00001000, 1);
        assert_eq!(cdb[0], 0x2A); // WRITE(10) opcode
                                  // LBA big-endian
        assert_eq!(cdb[2], 0x00);
        assert_eq!(cdb[3], 0x00);
        assert_eq!(cdb[4], 0x10);
        assert_eq!(cdb[5], 0x00);
        // Transfer length = 1
        assert_eq!(cdb[7], 0x00);
        assert_eq!(cdb[8], 0x01);
    }

    #[test]
    fn test_lun_setting() {
        let mut dev = MassStorageDevice::new(1, 0x81, 0x02);
        assert_eq!(dev.lun, 0);
        dev.set_lun(3);
        assert_eq!(dev.lun, 3);
    }

    #[test]
    fn test_cbw_lun_masking() {
        let command = [0x00; 6]; // TEST UNIT READY
        let cbw = CommandBlockWrapper::new(1, 0, CBW_DIRECTION_OUT, 0xFF, &command);
        assert_eq!(cbw.lun, 0x0F); // only lower 4 bits
    }

    #[test]
    fn test_csw_from_bytes_invalid_signature() {
        let mut bytes = [0u8; 13];
        bytes[0] = 0x00; // wrong signature
        let result = CommandStatusWrapper::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_multi_block_read() {
        let mut dev = MassStorageDevice::new(1, 0x81, 0x02);
        dev.state = MassStorageState::Ready;
        dev.cached_block_size = 512;
        dev.cached_total_blocks = 2048;

        let mut buf = [0u8; 4096]; // 8 blocks
        let result = dev.read_blocks(0, 8, &mut buf);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 4096);
    }
}
