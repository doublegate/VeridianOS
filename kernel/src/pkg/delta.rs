//! Binary Delta Updates
//!
//! Provides block-matching binary diff and patch operations for incremental
//! package updates. Uses 256-byte fixed blocks with FNV-1a hashing to detect
//! matching regions, producing a compact delta representation.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::KernelError;

/// Block size for delta computation (bytes).
const BLOCK_SIZE: usize = 256;

/// A single operation in a binary delta.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub enum DeltaOp {
    /// Copy `len` bytes from the source at `offset`.
    Copy { offset: usize, len: usize },
    /// Insert new data not present in the source.
    Insert { data: Vec<u8> },
}

/// A binary delta between two versions of a file.
///
/// Applying the delta to the source produces the target.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BinaryDelta {
    /// SHA-256 hash of the source file
    pub source_hash: [u8; 32],
    /// SHA-256 hash of the target file
    pub target_hash: [u8; 32],
    /// Ordered list of delta operations
    pub operations: Vec<DeltaOp>,
    /// Compressed size of the delta (for statistics)
    pub compressed_size: usize,
}

/// Metadata describing a delta update between two versions.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct DeltaMetadata {
    /// Version this delta upgrades FROM
    pub version_from: super::Version,
    /// Version this delta upgrades TO
    pub version_to: super::Version,
    /// Size of the delta in bytes
    pub delta_size: usize,
    /// Size of the full package in bytes (for comparison)
    pub full_size: usize,
}

#[cfg(feature = "alloc")]
impl DeltaMetadata {
    /// Return the space savings ratio (0.0 = no savings, 1.0 = 100% savings).
    pub fn savings_ratio(&self) -> f64 {
        if self.full_size == 0 {
            return 0.0;
        }
        1.0 - (self.delta_size as f64 / self.full_size as f64)
    }
}

/// Compute a binary delta between `old` and `new` data.
///
/// Uses a fixed 256-byte block matching algorithm:
/// 1. Hash all blocks in `old` with FNV-1a
/// 2. Slide through `new`, checking for matching blocks
/// 3. Matching regions become `Copy` ops, non-matching become `Insert` ops
#[cfg(feature = "alloc")]
pub fn compute_delta(old: &[u8], new: &[u8]) -> BinaryDelta {
    use alloc::collections::BTreeMap;

    let source_hash = *crate::crypto::hash::sha256(old).as_bytes();
    let target_hash = *crate::crypto::hash::sha256(new).as_bytes();

    // Build a hash table of all blocks in the old data
    let mut block_map: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    let mut offset = 0;
    while offset + BLOCK_SIZE <= old.len() {
        let hash = super::manifest::fnv1a_hash(&old[offset..offset + BLOCK_SIZE]);
        block_map.entry(hash).or_insert_with(Vec::new).push(offset);
        offset += BLOCK_SIZE;
    }

    let mut operations: Vec<DeltaOp> = Vec::new();
    let mut pos = 0;
    let mut pending_insert: Vec<u8> = Vec::new();

    while pos < new.len() {
        let remaining = new.len() - pos;

        if remaining >= BLOCK_SIZE {
            let block_hash = super::manifest::fnv1a_hash(&new[pos..pos + BLOCK_SIZE]);

            // Check if this block exists in the old data
            let found = block_map.get(&block_hash).and_then(|offsets| {
                offsets.iter().find(|&&old_offset| {
                    old_offset + BLOCK_SIZE <= old.len()
                        && old[old_offset..old_offset + BLOCK_SIZE] == new[pos..pos + BLOCK_SIZE]
                })
            });

            if let Some(&old_offset) = found {
                // Flush any pending insert data
                if !pending_insert.is_empty() {
                    operations.push(DeltaOp::Insert {
                        data: core::mem::take(&mut pending_insert),
                    });
                }

                // Extend the copy region as far as possible
                let mut copy_len = BLOCK_SIZE;
                while pos + copy_len < new.len()
                    && old_offset + copy_len < old.len()
                    && new[pos + copy_len] == old[old_offset + copy_len]
                {
                    copy_len += 1;
                }

                operations.push(DeltaOp::Copy {
                    offset: old_offset,
                    len: copy_len,
                });
                pos += copy_len;
                continue;
            }
        }

        // No match found, add to pending insert
        pending_insert.push(new[pos]);
        pos += 1;
    }

    // Flush remaining insert data
    if !pending_insert.is_empty() {
        operations.push(DeltaOp::Insert {
            data: pending_insert,
        });
    }

    // Compute compressed size (approximate: ops metadata + insert data)
    let compressed_size = operations.iter().fold(0usize, |acc, op| match op {
        DeltaOp::Copy { .. } => acc + 12, // 4 bytes tag + 4 bytes offset + 4 bytes len
        DeltaOp::Insert { data } => acc + 4 + data.len(), // 4 bytes tag + data
    });

    BinaryDelta {
        source_hash,
        target_hash,
        operations,
        compressed_size,
    }
}

/// Apply a binary delta to the source data, producing the target.
///
/// Returns an error if a Copy operation references out-of-bounds source data.
#[cfg(feature = "alloc")]
pub fn apply_delta(old: &[u8], delta: &BinaryDelta) -> Result<Vec<u8>, KernelError> {
    let mut result = Vec::new();

    for op in &delta.operations {
        match op {
            DeltaOp::Copy { offset, len } => {
                if *offset + *len > old.len() {
                    return Err(KernelError::InvalidArgument {
                        name: "delta_copy",
                        value: "source_out_of_bounds",
                    });
                }
                result.extend_from_slice(&old[*offset..*offset + *len]);
            }
            DeltaOp::Insert { data } => {
                result.extend_from_slice(data);
            }
        }
    }

    Ok(result)
}

/// Verify that a delta result matches the expected hash.
#[cfg(feature = "alloc")]
pub fn verify_delta_result(result: &[u8], expected_hash: &[u8; 32]) -> bool {
    crate::crypto::hash::sha256(result).as_bytes() == expected_hash
}

/// Serialize a BinaryDelta to bytes for storage/transmission.
///
/// Format:
/// ```text
/// source_hash:  [u8; 32]
/// target_hash:  [u8; 32]
/// op_count:     u32 (little-endian)
/// for each op:
///   tag:        u8 (0 = Copy, 1 = Insert)
///   Copy:       offset: u32 LE, len: u32 LE
///   Insert:     len: u32 LE, data: [u8; len]
/// ```
#[cfg(feature = "alloc")]
pub fn serialize_delta(delta: &BinaryDelta) -> Vec<u8> {
    let mut buf = Vec::new();

    buf.extend_from_slice(&delta.source_hash);
    buf.extend_from_slice(&delta.target_hash);
    buf.extend_from_slice(&(delta.operations.len() as u32).to_le_bytes());

    for op in &delta.operations {
        match op {
            DeltaOp::Copy { offset, len } => {
                buf.push(0); // tag
                buf.extend_from_slice(&(*offset as u32).to_le_bytes());
                buf.extend_from_slice(&(*len as u32).to_le_bytes());
            }
            DeltaOp::Insert { data } => {
                buf.push(1); // tag
                buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
                buf.extend_from_slice(data);
            }
        }
    }

    buf
}

/// Deserialize a BinaryDelta from bytes.
#[cfg(feature = "alloc")]
pub fn deserialize_delta(data: &[u8]) -> Result<BinaryDelta, KernelError> {
    // Minimum size: 32 (source hash) + 32 (target hash) + 4 (op count)
    if data.len() < 68 {
        return Err(KernelError::InvalidArgument {
            name: "delta_data",
            value: "too_short",
        });
    }

    let mut source_hash = [0u8; 32];
    let mut target_hash = [0u8; 32];
    source_hash.copy_from_slice(&data[0..32]);
    target_hash.copy_from_slice(&data[32..64]);

    let op_count = u32::from_le_bytes([data[64], data[65], data[66], data[67]]) as usize;

    let mut operations = Vec::with_capacity(op_count);
    let mut pos = 68;

    for _ in 0..op_count {
        if pos >= data.len() {
            return Err(KernelError::InvalidArgument {
                name: "delta_data",
                value: "truncated_ops",
            });
        }

        let tag = data[pos];
        pos += 1;

        match tag {
            0 => {
                // Copy
                if pos + 8 > data.len() {
                    return Err(KernelError::InvalidArgument {
                        name: "delta_data",
                        value: "truncated_copy",
                    });
                }
                let offset =
                    u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                        as usize;
                pos += 4;
                let len =
                    u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                        as usize;
                pos += 4;
                operations.push(DeltaOp::Copy { offset, len });
            }
            1 => {
                // Insert
                if pos + 4 > data.len() {
                    return Err(KernelError::InvalidArgument {
                        name: "delta_data",
                        value: "truncated_insert_len",
                    });
                }
                let len =
                    u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                        as usize;
                pos += 4;
                if pos + len > data.len() {
                    return Err(KernelError::InvalidArgument {
                        name: "delta_data",
                        value: "truncated_insert_data",
                    });
                }
                let insert_data = data[pos..pos + len].to_vec();
                pos += len;
                operations.push(DeltaOp::Insert { data: insert_data });
            }
            _ => {
                return Err(KernelError::InvalidArgument {
                    name: "delta_tag",
                    value: "unknown_op_type",
                });
            }
        }
    }

    Ok(BinaryDelta {
        source_hash,
        target_hash,
        operations,
        compressed_size: data.len(),
    })
}
