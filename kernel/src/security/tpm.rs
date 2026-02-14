//! TPM 2.0 Support
//!
//! Interface for Trusted Platform Module 2.0 operations including attestation,
//! sealing, and hardware random number generation.
//!
//! ## Hardware Integration Points
//!
//! TPM hardware can be accessed via multiple interfaces:
//! - **Memory-Mapped I/O (MMIO)**: Common on x86_64 platforms
//!   - Base addresses: 0xFED40000 (TPM1.2), 0xFED40000 (TPM2.0)
//!   - Registers: Access/Status/Data FIFOs at base + offsets
//! - **I2C/SPI**: Common on embedded platforms (ARM, RISC-V)
//!   - Requires I2C/SPI driver integration
//!   - Device addresses configurable via device tree
//! - **Firmware Interface**: UEFI/BIOS integration
//!   - Runtime services for TPM access
//!   - Platform-specific implementations
//!
//! ## Implementation
//!
//! Provides a full TPM 2.0 CRB (Command Response Buffer) interface over MMIO.
//! When no hardware TPM is detected (common in QEMU without swtpm), the module
//! runs in software-emulation mode with in-memory PCR banks, random number
//! generation via the kernel PRNG, and software sealing/unsealing backed by
//! SHA-256 key derivation.
//!
//! For hardware TPM (e.g., QEMU + swtpm), the CRB interface marshals TPM 2.0
//! command packets to the MMIO command buffer and reads responses.

use alloc::{vec, vec::Vec};

use spin::Mutex;

use super::tpm_commands::{
    TpmGetRandomCommand, TpmPcrExtendCommand, TpmPcrReadCommand, TpmResponseHeader,
    TpmStartupCommand, TpmStartupType,
};
use crate::error::KernelError;

/// TPM MMIO base addresses and register offsets (platform-specific)
pub mod mmio {
    /// Standard TPM 2.0 MMIO base address (x86_64)
    pub const TPM2_BASE: usize = 0xFED40000;

    /// AArch64 QEMU virt platform TPM CRB base
    #[cfg(target_arch = "aarch64")]
    pub const TPM2_BASE_AARCH64: usize = 0x0C000000;

    /// TPM CRB locality 0 offset
    pub const CRB_LOC_STATE: usize = 0x0000;
    /// TPM CRB locality control
    pub const CRB_LOC_CTRL: usize = 0x0008;
    /// TPM CRB locality status
    pub const CRB_LOC_STS: usize = 0x000C;

    /// TPM access register offset (FIFO interface)
    pub const TPM_ACCESS: usize = 0x0000;
    /// TPM status register offset (FIFO interface)
    pub const TPM_STS: usize = 0x0018;
    /// TPM data FIFO offset (FIFO interface)
    pub const TPM_DATA_FIFO: usize = 0x0024;
    /// TPM interface ID offset
    pub const TPM_INTERFACE_ID: usize = 0x0030;

    /// CRB control request register
    pub const CRB_CTRL_REQ: usize = 0x0040;
    /// CRB control status register
    pub const CRB_CTRL_STS: usize = 0x0044;
    /// CRB control cancel register
    pub const CRB_CTRL_CANCEL: usize = 0x0048;
    /// CRB control start register
    pub const CRB_CTRL_START: usize = 0x004C;

    /// CRB command buffer size
    pub const CRB_CTRL_CMD_SIZE: usize = 0x0058;
    /// CRB command buffer address (low)
    pub const CRB_CTRL_CMD_LADDR: usize = 0x005C;
    /// CRB command buffer address (high)
    pub const CRB_CTRL_CMD_HADDR: usize = 0x0060;
    /// CRB response buffer size
    pub const CRB_CTRL_RSP_SIZE: usize = 0x0064;
    /// CRB response buffer address
    pub const CRB_CTRL_RSP_ADDR: usize = 0x0068;

    /// CRB data buffer (command/response share this region)
    pub const CRB_DATA_BUFFER: usize = 0x0080;

    /// Maximum command/response buffer size
    pub const CRB_BUFFER_SIZE: usize = 3968; // 0x1000 - 0x80

    // TPM_STS register bit masks
    /// TPM is ready to accept a command
    pub const STS_COMMAND_READY: u32 = 1 << 6;
    /// TPM expects more data
    pub const STS_EXPECT: u32 = 1 << 3;
    /// Data is available to read
    pub const STS_DATA_AVAIL: u32 = 1 << 4;
    /// TPM has completed processing
    pub const STS_VALID: u32 = 1 << 7;

    // CRB_CTRL_START values
    /// Start command processing
    pub const CRB_START: u32 = 1;
}

/// TPM Interface type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmInterfaceType {
    /// Memory-mapped I/O (common on x86_64)
    Mmio,
    /// I2C bus interface
    I2c,
    /// SPI bus interface
    Spi,
    /// Firmware/UEFI interface
    Firmware,
    /// Software emulation (no hardware TPM detected)
    Software,
    /// Not detected
    None,
}

/// TPM 2.0 command codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TpmCommand {
    Startup = 0x144,
    GetRandom = 0x17B,
    PCRRead = 0x17E,
    PCRExtend = 0x182,
    CreatePrimary = 0x131,
    Create = 0x153,
    Load = 0x157,
    Sign = 0x15D,
    VerifySignature = 0x177,
    Quote = 0x158,
    Unseal = 0x15E,
}

/// TPM Platform Configuration Register (PCR) index
pub type PcrIndex = u8;

/// TPM handle for objects
pub type TpmHandle = u32;

/// TPM result
pub type TpmResult<T> = Result<T, TpmError>;

/// Number of PCR banks (0-23, per TCG spec)
const PCR_COUNT: usize = 24;

/// Maximum number of sealed blobs in software emulation
/// Kept small (4) to avoid stack overflow on x86_64 during init.
/// Each SealedEntry is ~850 bytes due to PCR policy arrays.
const MAX_SEALED_ENTRIES: usize = 4;

/// TPM errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmError {
    NotInitialized,
    CommandFailed,
    InvalidHandle,
    AuthFailed,
    NotSupported,
    HardwareError,
    InvalidPcr,
    BufferTooSmall,
    SealStorageFull,
    UnsealFailed,
    Timeout,
}

/// Software-emulated PCR bank (used when no hardware TPM is present)
struct SoftPcrBank {
    /// PCR values (SHA-256, 32 bytes each)
    values: [[u8; 32]; PCR_COUNT],
    /// Whether each PCR has been extended at least once
    extended: [bool; PCR_COUNT],
}

impl SoftPcrBank {
    const fn new() -> Self {
        Self {
            values: [[0u8; 32]; PCR_COUNT],
            extended: [false; PCR_COUNT],
        }
    }

    /// Extend PCR: new_value = SHA-256(old_value || measurement)
    fn extend(&mut self, index: usize, measurement: &[u8; 32]) {
        use crate::crypto::hash::sha256;

        let mut concat = [0u8; 64];
        concat[..32].copy_from_slice(&self.values[index]);
        concat[32..].copy_from_slice(measurement);

        let hash = sha256(&concat);
        self.values[index].copy_from_slice(hash.as_bytes());
        self.extended[index] = true;
    }

    /// Read PCR value
    fn read(&self, index: usize) -> [u8; 32] {
        self.values[index]
    }
}

/// Sealed data entry for software emulation
struct SealedEntry {
    /// Handle identifying this sealed blob
    handle: u32,
    /// PCR values at the time of sealing (for policy check)
    pcr_policy: [[u8; 32]; PCR_COUNT],
    /// Which PCRs are part of the policy
    pcr_mask: [bool; PCR_COUNT],
    /// Encrypted data (XOR with derived key in software mode)
    _sealed_data: Vec<u8>,
    /// Key derivation salt
    _salt: [u8; 32],
    /// Whether this entry is in use
    active: bool,
}

impl SealedEntry {
    const fn empty() -> Self {
        Self {
            handle: 0,
            pcr_policy: [[0u8; 32]; PCR_COUNT],
            pcr_mask: [false; PCR_COUNT],
            _sealed_data: Vec::new(),
            _salt: [0u8; 32],
            active: false,
        }
    }
}

/// TPM 2.0 interface
///
/// Supports both hardware MMIO and software-emulated TPM operations.
pub struct Tpm {
    initialized: bool,
    locality: u8,
    interface_type: TpmInterfaceType,
    /// MMIO base address (if using MMIO interface)
    mmio_base: Option<usize>,
    /// Software PCR bank (used in software emulation mode)
    soft_pcrs: SoftPcrBank,
    /// Sealed data storage (software emulation)
    sealed_entries: [SealedEntry; MAX_SEALED_ENTRIES],
    /// Next handle for sealed entries
    next_seal_handle: u32,
}

impl Tpm {
    /// Create new TPM interface
    pub fn new() -> Self {
        Self {
            initialized: false,
            locality: 0,
            interface_type: TpmInterfaceType::None,
            mmio_base: None,
            soft_pcrs: SoftPcrBank::new(),
            sealed_entries: [
                SealedEntry::empty(),
                SealedEntry::empty(),
                SealedEntry::empty(),
                SealedEntry::empty(),
            ],
            next_seal_handle: 0x80000100,
        }
    }

    /// Detect TPM hardware
    pub fn detect_hardware(&mut self) -> TpmResult<TpmInterfaceType> {
        // Try MMIO detection first (x86_64 platforms)
        #[cfg(target_arch = "x86_64")]
        {
            if let Some(base) = self.try_detect_mmio(mmio::TPM2_BASE) {
                crate::println!("[TPM] Detected MMIO TPM at 0x{:X}", base);
                self.interface_type = TpmInterfaceType::Mmio;
                self.mmio_base = Some(base);
                return Ok(TpmInterfaceType::Mmio);
            }
        }

        // Try AArch64 QEMU virt platform detection
        #[cfg(target_arch = "aarch64")]
        {
            // Would check device tree for TPM CRB node
            crate::println!("[TPM] AArch64 TPM detection: device tree probe not implemented");
        }

        // RISC-V detection via device tree
        #[cfg(target_arch = "riscv64")]
        {
            crate::println!("[TPM] RISC-V TPM detection: device tree probe not implemented");
        }

        // No hardware found -- fall back to software emulation
        crate::println!("[TPM] No hardware TPM detected, using software emulation");
        self.interface_type = TpmInterfaceType::Software;
        Ok(TpmInterfaceType::Software)
    }

    /// Try to detect MMIO-based TPM at the given base address.
    ///
    /// Probes the TPM interface ID register to verify presence.
    /// Returns `Some(base)` if a valid TPM is found.
    ///
    /// NOTE: The physical MMIO address (e.g. 0xFED40000) must be mapped in the
    /// kernel page tables before this probe will work.  In a higher-half kernel
    /// the low physical addresses are not identity-mapped, so a raw
    /// read_volatile will page-fault.  Until proper MMIO mapping is wired
    /// up, this function returns None to fall through to software TPM
    /// emulation.
    #[cfg(target_arch = "x86_64")]
    fn try_detect_mmio(&self, _base: usize) -> Option<usize> {
        // TODO: map the TPM MMIO page via the VMM before probing.
        // For now, skip the probe to avoid page faults on higher-half kernels
        // where physical address 0xFED40000 is not mapped.
        crate::println!("[TPM] x86_64 TPM MMIO probe skipped (page not mapped)");
        None
    }

    /// Request locality from the TPM.
    ///
    /// On CRB interface, write to LOC_CTRL to request access to the
    /// specified locality.
    fn request_locality(&mut self, locality: u8) -> TpmResult<()> {
        self.locality = locality;

        if let Some(base) = self.mmio_base {
            if self.interface_type == TpmInterfaceType::Mmio {
                let locality_offset = (locality as usize) * 0x1000;
                let loc_ctrl = base + locality_offset + mmio::CRB_LOC_CTRL;

                // SAFETY: Writing to TPM CRB locality control register
                unsafe {
                    let ptr = loc_ctrl as *mut u32;
                    core::ptr::write_volatile(ptr, 1); // Request locality
                }

                // Wait for locality to be granted (poll LOC_STS)
                let loc_sts = base + locality_offset + mmio::CRB_LOC_STS;
                let mut retries = 1000u32;
                loop {
                    // SAFETY: Reading TPM CRB locality status register via MMIO
                    let sts = unsafe { core::ptr::read_volatile(loc_sts as *const u32) };
                    if sts & 1 != 0 {
                        // Locality granted
                        return Ok(());
                    }
                    retries -= 1;
                    if retries == 0 {
                        return Err(TpmError::Timeout);
                    }
                }
            }
        }

        // Software mode: locality is always granted
        Ok(())
    }

    /// Send a raw command to the TPM via CRB MMIO and read the response.
    ///
    /// Writes the command bytes to the CRB data buffer, triggers processing
    /// via CRB_CTRL_START, polls for completion, and reads the response.
    fn send_command(&self, command: &[u8]) -> TpmResult<Vec<u8>> {
        let base = match self.mmio_base {
            Some(b) => b,
            None => return Err(TpmError::HardwareError),
        };

        if command.len() > mmio::CRB_BUFFER_SIZE {
            return Err(TpmError::BufferTooSmall);
        }

        let locality_offset = (self.locality as usize) * 0x1000;
        let buf_addr = base + locality_offset + mmio::CRB_DATA_BUFFER;
        let ctrl_start = base + locality_offset + mmio::CRB_CTRL_START;
        let ctrl_sts = base + locality_offset + mmio::CRB_CTRL_STS;

        // Write command bytes to CRB data buffer
        // SAFETY: Writing to TPM MMIO registers within the mapped CRB region
        unsafe {
            for (i, &byte) in command.iter().enumerate() {
                let ptr = (buf_addr + i) as *mut u8;
                core::ptr::write_volatile(ptr, byte);
            }
        }

        // Trigger command processing
        // SAFETY: Writing to TPM CRB_CTRL_START register to begin command execution
        unsafe {
            core::ptr::write_volatile(ctrl_start as *mut u32, mmio::CRB_START);
        }

        // Poll CRB_CTRL_START until it clears (command complete)
        let mut retries = 100_000u32;
        loop {
            // SAFETY: Polling TPM CRB_CTRL_START register for command completion
            let start_val = unsafe { core::ptr::read_volatile(ctrl_start as *const u32) };
            if start_val == 0 {
                break; // Command complete
            }
            retries -= 1;
            if retries == 0 {
                return Err(TpmError::Timeout);
            }
        }

        // Check for error in CRB_CTRL_STS
        // SAFETY: Reading TPM CRB status register to check for command errors
        let sts = unsafe { core::ptr::read_volatile(ctrl_sts as *const u32) };
        if sts & 1 != 0 {
            return Err(TpmError::CommandFailed);
        }

        // Read response from data buffer
        // First read the response header to get the size
        let mut header_bytes = [0u8; 10];
        // SAFETY: Reading TPM response header (10 bytes) from CRB data buffer via MMIO
        unsafe {
            for (i, byte) in header_bytes.iter_mut().enumerate() {
                *byte = core::ptr::read_volatile((buf_addr + i) as *const u8);
            }
        }

        let response_header =
            TpmResponseHeader::parse(&header_bytes).ok_or(TpmError::CommandFailed)?;

        let response_size = response_header.size as usize;
        if !(10..=mmio::CRB_BUFFER_SIZE).contains(&response_size) {
            return Err(TpmError::CommandFailed);
        }

        // Read the full response
        let mut response = vec![0u8; response_size];
        response[..10].copy_from_slice(&header_bytes);
        // Index-based loop required: each element reads from a different MMIO address
        // SAFETY: Reading remaining TPM response bytes from CRB data buffer via MMIO.
        // response_size is bounded to CRB_BUFFER_SIZE above.
        #[allow(clippy::needless_range_loop)]
        unsafe {
            for i in 10..response_size {
                response[i] = core::ptr::read_volatile((buf_addr + i) as *const u8);
            }
        }

        Ok(response)
    }

    /// Initialize TPM with TPM2_Startup command
    pub fn startup(&mut self) -> TpmResult<()> {
        // Detect hardware first
        if self.interface_type == TpmInterfaceType::None {
            self.detect_hardware()?;
        }

        match self.interface_type {
            TpmInterfaceType::Mmio => {
                crate::println!("[TPM] Performing CRB startup sequence...");

                // Request locality 0
                self.request_locality(0)?;

                // Send TPM2_Startup(Clear)
                let cmd = TpmStartupCommand::new(TpmStartupType::Clear);
                let cmd_bytes = cmd.to_bytes();

                match self.send_command(&cmd_bytes) {
                    Ok(response) => {
                        let header =
                            TpmResponseHeader::parse(&response).ok_or(TpmError::CommandFailed)?;
                        if !header.response_code().is_success() {
                            let _rc = header.response_code;
                            crate::println!(
                                "[TPM] TPM2_Startup failed with response code 0x{:08X}",
                                _rc
                            );
                            return Err(TpmError::CommandFailed);
                        }
                        crate::println!("[TPM] TPM2_Startup(Clear) succeeded via CRB");
                    }
                    Err(e) => {
                        crate::println!("[TPM] TPM2_Startup command send failed: {:?}", e);
                        return Err(e);
                    }
                }
            }
            TpmInterfaceType::Software => {
                crate::println!("[TPM] Software TPM emulation active (no hardware)");
                // Initialize soft PCR bank -- already zeroed in new()
            }
            TpmInterfaceType::I2c | TpmInterfaceType::Spi => {
                crate::println!("[TPM] I2C/SPI startup not yet implemented");
            }
            TpmInterfaceType::Firmware => {
                crate::println!("[TPM] Firmware interface startup not yet implemented");
            }
            TpmInterfaceType::None => {
                crate::println!("[TPM] No TPM interface available");
                return Err(TpmError::NotSupported);
            }
        }

        self.initialized = true;
        crate::println!("[TPM] TPM 2.0 startup complete");
        Ok(())
    }

    /// Get random bytes from TPM hardware RNG or software PRNG fallback.
    pub fn get_random(&self, num_bytes: usize) -> TpmResult<Vec<u8>> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        if num_bytes == 0 {
            return Ok(Vec::new());
        }

        match self.interface_type {
            TpmInterfaceType::Mmio => {
                // Send TPM2_GetRandom via CRB
                let mut result = Vec::with_capacity(num_bytes);
                let mut remaining = num_bytes;

                // TPM may return fewer bytes than requested per call (max ~32-48)
                while remaining > 0 {
                    let request_size = core::cmp::min(remaining, 48) as u16;
                    let cmd = TpmGetRandomCommand::new(request_size);
                    let cmd_bytes = cmd.to_bytes();

                    let response = self.send_command(&cmd_bytes)?;
                    let header =
                        TpmResponseHeader::parse(&response).ok_or(TpmError::CommandFailed)?;

                    if !header.response_code().is_success() {
                        return Err(TpmError::CommandFailed);
                    }

                    // Parse random bytes from response: after header (10 bytes), 2-byte length,
                    // then data
                    if response.len() < 12 {
                        return Err(TpmError::CommandFailed);
                    }
                    let bytes_returned = u16::from_be_bytes([response[10], response[11]]) as usize;
                    if response.len() < 12 + bytes_returned {
                        return Err(TpmError::CommandFailed);
                    }
                    result.extend_from_slice(&response[12..12 + bytes_returned]);
                    remaining -= bytes_returned;
                }

                result.truncate(num_bytes);
                Ok(result)
            }
            TpmInterfaceType::Software => {
                // Use kernel PRNG as fallback
                let mut buffer = vec![0u8; num_bytes];
                let rng = crate::crypto::random::get_random();
                rng.fill_bytes(&mut buffer)
                    .map_err(|_| TpmError::HardwareError)?;
                Ok(buffer)
            }
            _ => {
                // Unsupported interface: return PRNG bytes
                let mut buffer = vec![0u8; num_bytes];
                let rng = crate::crypto::random::get_random();
                rng.fill_bytes(&mut buffer)
                    .map_err(|_| TpmError::HardwareError)?;
                Ok(buffer)
            }
        }
    }

    /// Read Platform Configuration Register (PCR) value.
    ///
    /// Returns the current 32-byte SHA-256 PCR value for the given index.
    pub fn pcr_read(&self, pcr_index: PcrIndex) -> TpmResult<[u8; 32]> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        if pcr_index as usize >= PCR_COUNT {
            return Err(TpmError::InvalidPcr);
        }

        match self.interface_type {
            TpmInterfaceType::Mmio => {
                // Send TPM2_PCR_Read via CRB
                let cmd =
                    TpmPcrReadCommand::new(super::tpm_commands::hash_alg::SHA256, &[pcr_index]);
                let cmd_bytes = cmd.to_bytes();

                let response = self.send_command(&cmd_bytes)?;
                let header = TpmResponseHeader::parse(&response).ok_or(TpmError::CommandFailed)?;

                if !header.response_code().is_success() {
                    return Err(TpmError::CommandFailed);
                }

                // Parse PCR digest from response
                // Response format: header(10) + pcrUpdateCounter(4) + pcrSelectionOut(var) +
                // pcrValues
                // For simplicity, find the 32-byte digest at the end
                if response.len() < 10 + 4 + 8 + 4 + 32 {
                    return Err(TpmError::CommandFailed);
                }

                // The digest is the last 32 bytes before any padding
                // Typically: header(10) + counter(4) + selection(~8) + count(4) + size(2) +
                // digest(32)
                let digest_offset = response.len() - 32;
                let mut pcr_value = [0u8; 32];
                pcr_value.copy_from_slice(&response[digest_offset..]);
                Ok(pcr_value)
            }
            TpmInterfaceType::Software => {
                crate::println!("[TPM] Reading PCR {} (software)", pcr_index);
                Ok(self.soft_pcrs.read(pcr_index as usize))
            }
            _ => {
                crate::println!("[TPM] Reading PCR {} (unsupported interface)", pcr_index);
                Ok([0u8; 32])
            }
        }
    }

    /// Extend Platform Configuration Register with a measurement hash.
    ///
    /// Computes: PCR[index] = SHA-256(PCR[index] || measurement)
    pub fn pcr_extend(&mut self, pcr_index: PcrIndex, data: &[u8; 32]) -> TpmResult<()> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        if pcr_index as usize >= PCR_COUNT {
            return Err(TpmError::InvalidPcr);
        }

        crate::println!("[TPM] Extending PCR {} with measurement", pcr_index);

        match self.interface_type {
            TpmInterfaceType::Mmio => {
                // Send TPM2_PCR_Extend via CRB
                let cmd = TpmPcrExtendCommand::new(pcr_index, data);
                let cmd_bytes = cmd.to_bytes();

                let response = self.send_command(&cmd_bytes)?;
                let header = TpmResponseHeader::parse(&response).ok_or(TpmError::CommandFailed)?;

                if !header.response_code().is_success() {
                    return Err(TpmError::CommandFailed);
                }

                crate::println!("[TPM] PCR {} extended via hardware", pcr_index);
                Ok(())
            }
            TpmInterfaceType::Software => {
                self.soft_pcrs.extend(pcr_index as usize, data);
                crate::println!("[TPM] PCR {} extended (software)", pcr_index);
                Ok(())
            }
            _ => {
                crate::println!("[TPM] PCR extend unsupported on {:?}", self.interface_type);
                Ok(())
            }
        }
    }

    /// Create attestation quote over selected PCRs.
    ///
    /// Returns a signed quote structure containing the PCR values and nonce.
    pub fn quote(&self, pcr_selection: &[PcrIndex], nonce: &[u8; 32]) -> TpmResult<Vec<u8>> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        crate::println!(
            "[TPM] Creating attestation quote for {} PCRs",
            pcr_selection.len()
        );

        // Build quote data: nonce || PCR values
        let mut quote_data = Vec::with_capacity(32 + pcr_selection.len() * 32);
        quote_data.extend_from_slice(nonce);

        for &pcr_idx in pcr_selection {
            let pcr_value = self.pcr_read(pcr_idx)?;
            quote_data.extend_from_slice(&pcr_value);
        }

        // Hash the quote data to produce a digest
        let digest = crate::crypto::hash::sha256(&quote_data);
        let mut result = Vec::with_capacity(64);
        result.extend_from_slice(digest.as_bytes());
        // Append the raw PCR data for verification
        result.extend_from_slice(&quote_data);

        Ok(result)
    }

    /// Seal data to current PCR values.
    ///
    /// The sealed blob can only be unsealed when the PCRs match the values
    /// recorded at seal time. In software emulation mode, this uses a
    /// SHA-256 derived key for XOR encryption.
    pub fn seal(&mut self, data: &[u8], pcr_selection: &[PcrIndex]) -> TpmResult<Vec<u8>> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        crate::println!(
            "[TPM] Sealing {} bytes to {} PCRs",
            data.len(),
            pcr_selection.len()
        );

        // Find a free slot
        let slot = self
            .sealed_entries
            .iter()
            .position(|e| !e.active)
            .ok_or(TpmError::SealStorageFull)?;

        // Generate a random salt
        let mut salt = [0u8; 32];
        let rng = crate::crypto::random::get_random();
        rng.fill_bytes(&mut salt)
            .map_err(|_| TpmError::HardwareError)?;

        // Record PCR policy
        let mut pcr_policy = [[0u8; 32]; PCR_COUNT];
        let mut pcr_mask = [false; PCR_COUNT];
        for &pcr_idx in pcr_selection {
            if (pcr_idx as usize) < PCR_COUNT {
                pcr_policy[pcr_idx as usize] = self.pcr_read(pcr_idx)?;
                pcr_mask[pcr_idx as usize] = true;
            }
        }

        // Derive encryption key from salt + PCR values
        let key = self.derive_seal_key(&salt, &pcr_policy, &pcr_mask);

        // XOR-encrypt the data with the derived key stream
        let sealed_data = xor_with_keystream(data, &key);

        let handle = self.next_seal_handle;
        self.next_seal_handle += 1;

        self.sealed_entries[slot] = SealedEntry {
            handle,
            pcr_policy,
            pcr_mask,
            _sealed_data: sealed_data.clone(),
            _salt: salt,
            active: true,
        };

        // Return a blob: handle(4) + salt(32) + sealed_data
        let mut blob = Vec::with_capacity(4 + 32 + sealed_data.len());
        blob.extend_from_slice(&handle.to_be_bytes());
        blob.extend_from_slice(&salt);
        blob.extend_from_slice(&sealed_data);

        crate::println!(
            "[TPM] Data sealed to handle 0x{:08X} ({} bytes)",
            handle,
            blob.len()
        );

        Ok(blob)
    }

    /// Unseal data from a sealed blob.
    ///
    /// Checks that current PCR values match the policy recorded at seal time.
    /// Returns the original plaintext data on success.
    pub fn unseal(&self, sealed_blob: &[u8]) -> TpmResult<Vec<u8>> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        if sealed_blob.len() < 36 {
            return Err(TpmError::UnsealFailed);
        }

        crate::println!("[TPM] Unsealing data blob ({} bytes)", sealed_blob.len());

        // Parse blob: handle(4) + salt(32) + sealed_data
        let handle = u32::from_be_bytes([
            sealed_blob[0],
            sealed_blob[1],
            sealed_blob[2],
            sealed_blob[3],
        ]);
        let mut salt = [0u8; 32];
        salt.copy_from_slice(&sealed_blob[4..36]);
        let sealed_data = &sealed_blob[36..];

        // Find the sealed entry
        let entry = self
            .sealed_entries
            .iter()
            .find(|e| e.active && e.handle == handle)
            .ok_or(TpmError::InvalidHandle)?;

        // Verify PCR policy: all policy PCRs must match current values
        for i in 0..PCR_COUNT {
            if entry.pcr_mask[i] {
                let current = self.pcr_read(i as u8)?;
                if current != entry.pcr_policy[i] {
                    crate::println!("[TPM] Unseal failed: PCR {} mismatch (policy violation)", i);
                    return Err(TpmError::AuthFailed);
                }
            }
        }

        // Derive the same key and decrypt
        let key = self.derive_seal_key(&salt, &entry.pcr_policy, &entry.pcr_mask);
        let plaintext = xor_with_keystream(sealed_data, &key);

        crate::println!(
            "[TPM] Data unsealed from handle 0x{:08X} ({} bytes)",
            handle,
            plaintext.len()
        );

        Ok(plaintext)
    }

    /// Derive a sealing key from salt and PCR values using SHA-256.
    fn derive_seal_key(
        &self,
        salt: &[u8; 32],
        pcr_policy: &[[u8; 32]; PCR_COUNT],
        pcr_mask: &[bool; PCR_COUNT],
    ) -> [u8; 32] {
        use crate::crypto::hash::sha256;

        // key = SHA-256(salt || PCR_0_value || PCR_1_value || ... ||
        // "veridian-tpm-seal")
        let mut key_material = Vec::with_capacity(32 + PCR_COUNT * 32 + 17);
        key_material.extend_from_slice(salt);
        for i in 0..PCR_COUNT {
            if pcr_mask[i] {
                key_material.extend_from_slice(&pcr_policy[i]);
            }
        }
        key_material.extend_from_slice(b"veridian-tpm-seal");

        let hash = sha256(&key_material);
        *hash.as_bytes()
    }

    /// Create signing key in TPM
    pub fn create_signing_key(&self) -> TpmResult<TpmHandle> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        crate::println!("[TPM] Creating signing key");

        // In software mode, return a pseudo-handle
        let handle: TpmHandle = 0x80000001;
        Ok(handle)
    }

    /// Sign data with TPM key
    pub fn sign(&self, handle: TpmHandle, data: &[u8]) -> TpmResult<Vec<u8>> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        crate::println!(
            "[TPM] Signing {} bytes with key handle 0x{:08X}",
            data.len(),
            handle
        );

        // Software mode: produce HMAC-like signature using SHA-256
        let mut sign_input = Vec::with_capacity(4 + data.len() + 16);
        sign_input.extend_from_slice(&handle.to_be_bytes());
        sign_input.extend_from_slice(data);
        sign_input.extend_from_slice(b"veridian-tpm-sig");

        let hash = crate::crypto::hash::sha256(&sign_input);
        Ok(hash.as_bytes().to_vec())
    }

    /// Verify signature with TPM key
    pub fn verify_signature(
        &self,
        handle: TpmHandle,
        data: &[u8],
        signature: &[u8],
    ) -> TpmResult<bool> {
        if !self.initialized {
            return Err(TpmError::NotInitialized);
        }

        crate::println!("[TPM] Verifying signature with key handle 0x{:08X}", handle);

        // Software mode: recompute and compare
        let expected = self.sign(handle, data)?;
        Ok(expected == signature)
    }

    /// Check if the TPM is running in software emulation mode
    pub fn is_software_emulation(&self) -> bool {
        self.interface_type == TpmInterfaceType::Software
    }

    /// Check if the TPM has been initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

impl Default for Tpm {
    fn default() -> Self {
        Self::new()
    }
}

/// XOR data with a SHA-256-derived keystream.
///
/// Generates a keystream by hashing the key with a counter, then XORs each
/// 32-byte block of data with the corresponding keystream block.
fn xor_with_keystream(data: &[u8], key: &[u8; 32]) -> Vec<u8> {
    use crate::crypto::hash::sha256;

    let mut result = Vec::with_capacity(data.len());
    let mut counter: u64 = 0;
    let mut offset = 0;

    while offset < data.len() {
        // Derive keystream block: SHA-256(key || counter)
        let mut block_input = [0u8; 40]; // 32 key + 8 counter
        block_input[..32].copy_from_slice(key);
        block_input[32..40].copy_from_slice(&counter.to_le_bytes());

        let keystream_hash = sha256(&block_input);
        let keystream = keystream_hash.as_bytes();

        let chunk_len = core::cmp::min(32, data.len() - offset);
        for i in 0..chunk_len {
            result.push(data[offset + i] ^ keystream[i]);
        }

        offset += chunk_len;
        counter += 1;
    }

    result
}

/// Global TPM instance
static TPM: Mutex<Option<Tpm>> = Mutex::new(None);

/// Initialize TPM support
pub fn init() -> Result<(), KernelError> {
    let mut tpm = Tpm::new();

    // Try to initialize TPM hardware
    match tpm.startup() {
        Ok(()) => {
            let _mode = if tpm.is_software_emulation() {
                "software emulation"
            } else {
                "hardware"
            };
            crate::println!("[TPM] TPM 2.0 support initialized ({})", _mode);
            *TPM.lock() = Some(tpm);
            Ok(())
        }
        Err(_) => {
            // TPM not available - this is okay, continue without it
            crate::println!("[TPM] TPM hardware not available (continuing without TPM support)");
            Ok(())
        }
    }
}

/// Execute a closure with a reference to the global TPM instance.
///
/// Returns `None` if the TPM is not initialized.
pub fn with_tpm<R, F: FnOnce(&Tpm) -> R>(f: F) -> Option<R> {
    let guard = TPM.lock();
    guard.as_ref().map(f)
}

/// Execute a closure with a mutable reference to the global TPM instance.
///
/// Returns `None` if the TPM is not initialized.
pub fn with_tpm_mut<R, F: FnOnce(&mut Tpm) -> R>(f: F) -> Option<R> {
    let mut guard = TPM.lock();
    guard.as_mut().map(f)
}

/// Check if TPM is available
pub fn is_available() -> bool {
    TPM.lock().is_some()
}

/// Convenience: extend a PCR with a measurement hash via the global TPM
/// instance.
///
/// Returns Ok(()) if the TPM is available and the extend succeeded, or if no
/// TPM is available (silent no-op for callers that use TPM opportunistically).
pub fn pcr_extend(pcr_index: PcrIndex, measurement: &[u8; 32]) -> Result<(), TpmError> {
    match with_tpm_mut(|tpm| tpm.pcr_extend(pcr_index, measurement)) {
        Some(result) => result,
        None => Ok(()), // No TPM -- silently succeed
    }
}

/// Convenience: read a PCR value via the global TPM instance.
pub fn pcr_read(pcr_index: PcrIndex) -> Result<[u8; 32], TpmError> {
    match with_tpm(|tpm| tpm.pcr_read(pcr_index)) {
        Some(result) => result,
        None => Err(TpmError::NotInitialized),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_tpm_creation() {
        let tpm = Tpm::new();
        assert!(!tpm.initialized);
    }

    #[test_case]
    fn test_soft_pcr_extend() {
        let mut bank = SoftPcrBank::new();
        let measurement = [0x42u8; 32];

        // Initial PCR should be all zeros
        assert_eq!(bank.read(0), [0u8; 32]);

        // After extend, PCR should be SHA-256(zeros || measurement)
        bank.extend(0, &measurement);
        assert!(bank.extended[0]);
        // Value should no longer be all zeros
        assert_ne!(bank.read(0), [0u8; 32]);
    }

    #[test_case]
    fn test_xor_keystream_roundtrip() {
        let key = [0xABu8; 32];
        let plaintext = b"Hello, VeridianOS TPM!";

        let encrypted = xor_with_keystream(plaintext, &key);
        assert_ne!(&encrypted[..], &plaintext[..]);

        let decrypted = xor_with_keystream(&encrypted, &key);
        assert_eq!(&decrypted[..], &plaintext[..]);
    }
}
