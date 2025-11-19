//! Block Device Abstraction
//!
//! Provides a common interface for block-level storage devices.

use alloc::vec::Vec;

use crate::error::KernelError;

/// Block device trait
pub trait BlockDevice: Send + Sync {
    /// Get device name
    fn name(&self) -> &str;

    /// Get block size in bytes
    fn block_size(&self) -> usize;

    /// Get total number of blocks
    fn block_count(&self) -> u64;

    /// Read blocks from device
    fn read_blocks(&self, start_block: u64, buffer: &mut [u8]) -> Result<(), KernelError>;

    /// Write blocks to device
    fn write_blocks(&mut self, start_block: u64, buffer: &[u8]) -> Result<(), KernelError>;

    /// Flush any cached writes
    fn flush(&mut self) -> Result<(), KernelError> {
        Ok(()) // Default: no-op
    }
}

/// RAM-backed block device (for testing/ramdisk)
pub struct RamBlockDevice {
    name: alloc::string::String,
    block_size: usize,
    data: Vec<u8>,
}

impl RamBlockDevice {
    /// Create a new RAM block device
    pub fn new(name: alloc::string::String, block_size: usize, block_count: u64) -> Self {
        let size = block_size * block_count as usize;
        Self {
            name,
            block_size,
            data: alloc::vec![0u8; size],
        }
    }

    /// Get total size in bytes
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

impl BlockDevice for RamBlockDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        (self.data.len() / self.block_size) as u64
    }

    fn read_blocks(&self, start_block: u64, buffer: &mut [u8]) -> Result<(), KernelError> {
        let start_byte = start_block as usize * self.block_size;
        let end_byte = start_byte + buffer.len();

        if end_byte > self.data.len() {
            return Err(KernelError::InvalidArgument {
                name: "block_range",
                value: "out_of_bounds",
            });
        }

        buffer.copy_from_slice(&self.data[start_byte..end_byte]);
        Ok(())
    }

    fn write_blocks(&mut self, start_block: u64, buffer: &[u8]) -> Result<(), KernelError> {
        let start_byte = start_block as usize * self.block_size;
        let end_byte = start_byte + buffer.len();

        if end_byte > self.data.len() {
            return Err(KernelError::InvalidArgument {
                name: "block_range",
                value: "out_of_bounds",
            });
        }

        self.data[start_byte..end_byte].copy_from_slice(buffer);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::*;

    #[test_case]
    fn test_ram_block_device() {
        let mut dev = RamBlockDevice::new(String::from("test"), 512, 100);

        assert_eq!(dev.block_size(), 512);
        assert_eq!(dev.block_count(), 100);

        // Write some data
        let write_data = [0x42u8; 512];
        dev.write_blocks(0, &write_data).unwrap();

        // Read it back
        let mut read_data = [0u8; 512];
        dev.read_blocks(0, &mut read_data).unwrap();

        assert_eq!(read_data, write_data);
    }
}
