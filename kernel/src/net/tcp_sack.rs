//! TCP Selective Acknowledgment (SACK) implementation
//!
//! Implements RFC 2018 TCP SACK option parsing, scoreboard tracking,
//! and selective retransmission hole detection. Enables efficient
//! recovery from multiple segment losses without retransmitting
//! already-received data.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// ============================================================================
// Sequence Number Arithmetic (wrapping-safe)
// ============================================================================

/// Compare sequence numbers using signed 32-bit difference.
/// Returns true if `a` is strictly less than `b` in sequence space.
#[inline]
pub fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

/// Returns true if `a <= b` in sequence space.
#[inline]
pub fn seq_le(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) <= 0
}

/// Returns true if `a > b` in sequence space.
#[inline]
pub fn seq_gt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// Returns true if `a >= b` in sequence space.
#[inline]
pub fn seq_ge(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

// ============================================================================
// SACK Option Constants
// ============================================================================

/// TCP option kind for SACK-Permitted (sent in SYN/SYN-ACK)
pub const TCP_OPT_SACK_PERMITTED: u8 = 4;

/// TCP option length for SACK-Permitted
pub const TCP_OPT_SACK_PERMITTED_LEN: u8 = 2;

/// TCP option kind for SACK blocks
pub const TCP_OPT_SACK: u8 = 5;

/// Maximum number of SACK blocks in a single TCP option (limited by option
/// space)
pub const MAX_SACK_BLOCKS: usize = 4;

/// Size of a single SACK block (left_edge + right_edge = 4 + 4 bytes)
const SACK_BLOCK_SIZE: usize = 8;

/// TCP option End of Option List
const TCP_OPT_EOL: u8 = 0;

/// TCP option No-Operation (padding)
const TCP_OPT_NOP: u8 = 1;

// ============================================================================
// SACK Block
// ============================================================================

/// A single SACK block representing a contiguous range of received bytes.
///
/// `left_edge` is the first sequence number in the block (inclusive).
/// `right_edge` is one past the last sequence number (exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SackBlock {
    pub left_edge: u32,
    pub right_edge: u32,
}

impl SackBlock {
    /// Create a new SACK block.
    pub const fn new(left_edge: u32, right_edge: u32) -> Self {
        Self {
            left_edge,
            right_edge,
        }
    }

    /// Returns true if this block contains the given sequence number.
    pub fn contains(&self, seq: u32) -> bool {
        seq_ge(seq, self.left_edge) && seq_lt(seq, self.right_edge)
    }

    /// Returns the length of this block in sequence space.
    pub fn len(&self) -> u32 {
        self.right_edge.wrapping_sub(self.left_edge)
    }

    /// Returns true if the block is empty (zero length).
    pub fn is_empty(&self) -> bool {
        self.left_edge == self.right_edge
    }

    /// Returns true if this block overlaps or is adjacent to `other`.
    pub fn overlaps_or_adjacent(&self, other: &SackBlock) -> bool {
        // Two blocks [a, b) and [c, d) overlap or are adjacent if
        // a <= d AND c <= b (in sequence space).
        seq_le(self.left_edge, other.right_edge) && seq_le(other.left_edge, self.right_edge)
    }

    /// Merge another block into this one (union of ranges).
    /// Only valid if the blocks overlap or are adjacent.
    pub fn merge(&mut self, other: &SackBlock) {
        if seq_lt(other.left_edge, self.left_edge) {
            self.left_edge = other.left_edge;
        }
        if seq_gt(other.right_edge, self.right_edge) {
            self.right_edge = other.right_edge;
        }
    }
}

// ============================================================================
// SACK Option Parsing and Serialization
// ============================================================================

/// Parse SACK blocks from TCP options bytes.
///
/// Scans the options field for kind=5 (SACK) and extracts up to 4 blocks.
/// Returns an empty Vec if no SACK option is found.
#[cfg(feature = "alloc")]
pub fn parse_sack_blocks(options: &[u8]) -> Vec<SackBlock> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < options.len() {
        match options[i] {
            TCP_OPT_EOL => break,
            TCP_OPT_NOP => {
                i += 1;
            }
            TCP_OPT_SACK => {
                // Kind = 5, next byte is length
                if i + 1 >= options.len() {
                    break;
                }
                let opt_len = options[i + 1] as usize;
                if opt_len < 2 || i + opt_len > options.len() {
                    break;
                }
                // Number of blocks = (length - 2) / 8
                let num_blocks = (opt_len - 2) / SACK_BLOCK_SIZE;
                let mut offset = i + 2;
                for _ in 0..num_blocks.min(MAX_SACK_BLOCKS) {
                    if offset + SACK_BLOCK_SIZE > i + opt_len {
                        break;
                    }
                    let left = u32::from_be_bytes([
                        options[offset],
                        options[offset + 1],
                        options[offset + 2],
                        options[offset + 3],
                    ]);
                    let right = u32::from_be_bytes([
                        options[offset + 4],
                        options[offset + 5],
                        options[offset + 6],
                        options[offset + 7],
                    ]);
                    blocks.push(SackBlock::new(left, right));
                    offset += SACK_BLOCK_SIZE;
                }
                i += opt_len;
            }
            _ => {
                // Variable-length option: kind + length + data
                if i + 1 >= options.len() {
                    break;
                }
                let opt_len = options[i + 1] as usize;
                if opt_len < 2 {
                    break;
                }
                i += opt_len;
            }
        }
    }

    blocks
}

/// Check if SACK-Permitted option is present in TCP options.
pub fn has_sack_permitted(options: &[u8]) -> bool {
    let mut i = 0;

    while i < options.len() {
        match options[i] {
            TCP_OPT_EOL => break,
            TCP_OPT_NOP => {
                i += 1;
            }
            TCP_OPT_SACK_PERMITTED => {
                if i + 1 < options.len() && options[i + 1] == TCP_OPT_SACK_PERMITTED_LEN {
                    return true;
                }
                // Malformed, skip
                if i + 1 < options.len() {
                    let opt_len = options[i + 1] as usize;
                    i += if opt_len >= 2 { opt_len } else { 2 };
                } else {
                    break;
                }
            }
            _ => {
                if i + 1 >= options.len() {
                    break;
                }
                let opt_len = options[i + 1] as usize;
                if opt_len < 2 {
                    break;
                }
                i += opt_len;
            }
        }
    }

    false
}

/// Serialize SACK-Permitted option (2 bytes, for SYN/SYN-ACK).
#[cfg(feature = "alloc")]
pub fn serialize_sack_permitted() -> Vec<u8> {
    alloc::vec![TCP_OPT_SACK_PERMITTED, TCP_OPT_SACK_PERMITTED_LEN]
}

/// Serialize SACK blocks into TCP option bytes.
///
/// Produces kind(1) + length(1) + N * 8 bytes of block data.
/// At most `MAX_SACK_BLOCKS` (4) blocks are serialized.
#[cfg(feature = "alloc")]
pub fn serialize_sack_blocks(blocks: &[SackBlock]) -> Vec<u8> {
    if blocks.is_empty() {
        return Vec::new();
    }

    let count = blocks.len().min(MAX_SACK_BLOCKS);
    let opt_len = 2 + count * SACK_BLOCK_SIZE;
    let mut out = Vec::with_capacity(opt_len);

    out.push(TCP_OPT_SACK);
    out.push(opt_len as u8);

    for block in blocks.iter().take(count) {
        out.extend_from_slice(&block.left_edge.to_be_bytes());
        out.extend_from_slice(&block.right_edge.to_be_bytes());
    }

    out
}

// ============================================================================
// SACK Scoreboard
// ============================================================================

/// Tracks selectively acknowledged ranges for a TCP connection.
///
/// Maintains a sorted, non-overlapping list of SACK blocks representing
/// data the remote receiver has confirmed receiving out of order.
#[cfg(feature = "alloc")]
pub struct SackScoreboard {
    /// Sorted list of non-overlapping SACK blocks (by left_edge in sequence
    /// space).
    blocks: Vec<SackBlock>,
}

#[cfg(feature = "alloc")]
impl Default for SackScoreboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl SackScoreboard {
    /// Create an empty scoreboard.
    pub fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Returns the number of tracked SACK blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Returns a slice of all current SACK blocks.
    pub fn blocks(&self) -> &[SackBlock] {
        &self.blocks
    }

    /// Returns true if the given sequence number falls within a SACKed range.
    pub fn is_sacked(&self, seq: u32) -> bool {
        self.blocks.iter().any(|b| b.contains(seq))
    }

    /// Mark a range [left, right) as selectively acknowledged.
    ///
    /// Merges with any overlapping or adjacent existing blocks to maintain
    /// the invariant that blocks are sorted and non-overlapping.
    pub fn mark_sacked(&mut self, left: u32, right: u32) {
        if seq_ge(left, right) {
            return; // Empty or invalid range
        }

        let mut new_block = SackBlock::new(left, right);

        // Collect indices of blocks that overlap or are adjacent
        let mut merge_indices = Vec::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if new_block.overlaps_or_adjacent(block) {
                merge_indices.push(i);
            }
        }

        // Merge all overlapping blocks into new_block
        for &i in merge_indices.iter() {
            new_block.merge(&self.blocks[i]);
        }

        // Remove merged blocks in reverse order to preserve indices
        for &i in merge_indices.iter().rev() {
            self.blocks.remove(i);
        }

        // Insert new_block in sorted order (by left_edge in sequence space)
        let insert_pos = self
            .blocks
            .iter()
            .position(|b| seq_gt(b.left_edge, new_block.left_edge))
            .unwrap_or(self.blocks.len());

        self.blocks.insert(insert_pos, new_block);
    }

    /// Remove all blocks (or portions) below the cumulative ACK.
    ///
    /// Data below `ack` has been cumulatively acknowledged, so SACK
    /// blocks covering that range are no longer needed.
    pub fn clear_below(&mut self, ack: u32) {
        self.blocks.retain_mut(|block| {
            if seq_le(block.right_edge, ack) {
                // Entire block is below ack -- remove it
                false
            } else if seq_lt(block.left_edge, ack) {
                // Partial overlap -- trim the left side
                block.left_edge = ack;
                true
            } else {
                true
            }
        });
    }

    /// Returns the highest SACKed sequence number, or None if empty.
    pub fn highest_sacked(&self) -> Option<u32> {
        self.blocks.last().map(|b| b.right_edge)
    }

    /// Returns the next hole (gap) that needs retransmission.
    ///
    /// Starting from `snd_una` (send unacknowledged), finds the first
    /// gap between SACK blocks. Returns `(start_seq, length)` of the hole.
    pub fn next_retransmit(&self, snd_una: u32) -> Option<(u32, u32)> {
        if self.blocks.is_empty() {
            return None;
        }

        // Check for hole between snd_una and first SACK block
        let first = &self.blocks[0];
        if seq_lt(snd_una, first.left_edge) {
            let hole_len = first.left_edge.wrapping_sub(snd_una);
            return Some((snd_una, hole_len));
        }

        // Check for holes between consecutive SACK blocks
        for window in self.blocks.windows(2) {
            let gap_start = window[0].right_edge;
            let gap_end = window[1].left_edge;
            if seq_lt(gap_start, gap_end) {
                let hole_len = gap_end.wrapping_sub(gap_start);
                return Some((gap_start, hole_len));
            }
        }

        None
    }

    /// Returns all holes (gaps) between `snd_una` and the highest SACKed byte.
    ///
    /// Each hole is represented as `(start_seq, length)`.
    pub fn holes(&self, snd_una: u32) -> Vec<(u32, u32)> {
        let mut result = Vec::new();

        if self.blocks.is_empty() {
            return result;
        }

        // Hole before first block
        let first = &self.blocks[0];
        if seq_lt(snd_una, first.left_edge) {
            let hole_len = first.left_edge.wrapping_sub(snd_una);
            result.push((snd_una, hole_len));
        }

        // Holes between consecutive blocks
        for window in self.blocks.windows(2) {
            let gap_start = window[0].right_edge;
            let gap_end = window[1].left_edge;
            if seq_lt(gap_start, gap_end) {
                let hole_len = gap_end.wrapping_sub(gap_start);
                result.push((gap_start, hole_len));
            }
        }

        result
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // -- Sequence number arithmetic --

    #[test]
    fn test_seq_lt_normal() {
        assert!(seq_lt(100, 200));
        assert!(!seq_lt(200, 100));
        assert!(!seq_lt(100, 100));
    }

    #[test]
    fn test_seq_lt_wrapping() {
        // Near the wraparound point
        let a = u32::MAX - 10;
        let b = 10u32; // b is "after" a in sequence space
        assert!(seq_lt(a, b));
        assert!(!seq_lt(b, a));
    }

    #[test]
    fn test_seq_le_ge() {
        assert!(seq_le(100, 100));
        assert!(seq_le(100, 200));
        assert!(seq_ge(200, 100));
        assert!(seq_ge(100, 100));
        assert!(!seq_ge(100, 200));
    }

    #[test]
    fn test_seq_gt_wrapping() {
        let a = 5u32;
        let b = u32::MAX - 5;
        assert!(seq_gt(a, b)); // a is "after" b across the wrap
    }

    // -- SACK block --

    #[test]
    fn test_sack_block_contains() {
        let block = SackBlock::new(1000, 2000);
        assert!(block.contains(1000));
        assert!(block.contains(1500));
        assert!(block.contains(1999));
        assert!(!block.contains(2000));
        assert!(!block.contains(999));
    }

    #[test]
    fn test_sack_block_len() {
        let block = SackBlock::new(1000, 2000);
        assert_eq!(block.len(), 1000);

        let empty = SackBlock::new(500, 500);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_sack_block_overlap_and_merge() {
        let mut a = SackBlock::new(100, 200);
        let b = SackBlock::new(150, 300);
        assert!(a.overlaps_or_adjacent(&b));
        a.merge(&b);
        assert_eq!(a.left_edge, 100);
        assert_eq!(a.right_edge, 300);
    }

    // -- Option parsing --

    #[test]
    fn test_parse_sack_blocks() {
        // Build a SACK option with 2 blocks: [1000, 2000) and [3000, 4000)
        let mut opts = vec![TCP_OPT_SACK, 18]; // kind=5, length=2+8*2=18
        opts.extend_from_slice(&1000u32.to_be_bytes());
        opts.extend_from_slice(&2000u32.to_be_bytes());
        opts.extend_from_slice(&3000u32.to_be_bytes());
        opts.extend_from_slice(&4000u32.to_be_bytes());

        let blocks = parse_sack_blocks(&opts);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], SackBlock::new(1000, 2000));
        assert_eq!(blocks[1], SackBlock::new(3000, 4000));
    }

    #[test]
    fn test_has_sack_permitted() {
        let opts = vec![
            TCP_OPT_NOP,
            TCP_OPT_SACK_PERMITTED,
            TCP_OPT_SACK_PERMITTED_LEN,
        ];
        assert!(has_sack_permitted(&opts));

        let no_sack = vec![TCP_OPT_NOP, TCP_OPT_EOL];
        assert!(!has_sack_permitted(&no_sack));
    }

    #[test]
    fn test_serialize_sack_blocks() {
        let blocks = vec![SackBlock::new(1000, 2000), SackBlock::new(3000, 4000)];
        let serialized = serialize_sack_blocks(&blocks);
        assert_eq!(serialized[0], TCP_OPT_SACK);
        assert_eq!(serialized[1], 18); // 2 + 2*8
        let parsed = parse_sack_blocks(&serialized);
        assert_eq!(parsed, blocks);
    }

    #[test]
    fn test_serialize_sack_permitted() {
        let opt = serialize_sack_permitted();
        assert_eq!(opt.len(), 2);
        assert!(has_sack_permitted(&opt));
    }

    // -- Scoreboard --

    #[test]
    fn test_scoreboard_mark_and_query() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(1000, 2000);
        sb.mark_sacked(3000, 4000);

        assert!(sb.is_sacked(1500));
        assert!(sb.is_sacked(3500));
        assert!(!sb.is_sacked(2500));
        assert_eq!(sb.block_count(), 2);
    }

    #[test]
    fn test_scoreboard_merge_overlapping() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(1000, 2000);
        sb.mark_sacked(1500, 3000);

        assert_eq!(sb.block_count(), 1);
        assert_eq!(sb.blocks()[0], SackBlock::new(1000, 3000));
    }

    #[test]
    fn test_scoreboard_merge_adjacent() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(1000, 2000);
        sb.mark_sacked(2000, 3000);

        assert_eq!(sb.block_count(), 1);
        assert_eq!(sb.blocks()[0], SackBlock::new(1000, 3000));
    }

    #[test]
    fn test_scoreboard_merge_multiple() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(1000, 2000);
        sb.mark_sacked(3000, 4000);
        sb.mark_sacked(5000, 6000);
        // Now merge all three by inserting a spanning range
        sb.mark_sacked(1500, 5500);

        assert_eq!(sb.block_count(), 1);
        assert_eq!(sb.blocks()[0], SackBlock::new(1000, 6000));
    }

    #[test]
    fn test_scoreboard_clear_below() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(1000, 2000);
        sb.mark_sacked(3000, 4000);

        // Cumulative ACK advances to 1500 -- trims first block
        sb.clear_below(1500);
        assert_eq!(sb.block_count(), 2);
        assert_eq!(sb.blocks()[0].left_edge, 1500);

        // Cumulative ACK advances to 2500 -- removes first block entirely
        sb.clear_below(2500);
        assert_eq!(sb.block_count(), 1);
        assert_eq!(sb.blocks()[0], SackBlock::new(3000, 4000));
    }

    #[test]
    fn test_scoreboard_highest_sacked() {
        let mut sb = SackScoreboard::new();
        assert_eq!(sb.highest_sacked(), None);

        sb.mark_sacked(1000, 2000);
        assert_eq!(sb.highest_sacked(), Some(2000));

        sb.mark_sacked(5000, 6000);
        assert_eq!(sb.highest_sacked(), Some(6000));
    }

    #[test]
    fn test_scoreboard_holes() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(2000, 3000);
        sb.mark_sacked(4000, 5000);

        let holes = sb.holes(1000);
        assert_eq!(holes.len(), 2);
        assert_eq!(holes[0], (1000, 1000)); // [1000, 2000)
        assert_eq!(holes[1], (3000, 1000)); // [3000, 4000)
    }

    #[test]
    fn test_scoreboard_next_retransmit() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(2000, 3000);
        sb.mark_sacked(4000, 5000);

        // First hole: [1000, 2000)
        let next = sb.next_retransmit(1000);
        assert_eq!(next, Some((1000, 1000)));

        // If snd_una is 2000, first hole is between blocks: [3000, 4000)
        let next = sb.next_retransmit(3000);
        assert_eq!(next, Some((3000, 1000)));
    }

    #[test]
    fn test_scoreboard_no_holes() {
        let mut sb = SackScoreboard::new();
        sb.mark_sacked(1000, 3000);

        // snd_una is at start of the single block -- no hole before it
        let next = sb.next_retransmit(1000);
        assert_eq!(next, None);

        let holes = sb.holes(1000);
        assert!(holes.is_empty());
    }

    #[test]
    fn test_scoreboard_wrapping_sequence() {
        let mut sb = SackScoreboard::new();
        // Blocks near the u32 wraparound
        let near_max = u32::MAX - 500;
        let past_wrap = 500u32;

        sb.mark_sacked(near_max, u32::MAX);
        sb.mark_sacked(0, past_wrap);

        // These are adjacent (MAX wraps to 0) -- should merge
        // Actually u32::MAX and 0 differ by 1, so wrapping sub = 1
        // But our overlaps_or_adjacent check: seq_le(near_max, past_wrap) && seq_le(0,
        // u32::MAX) near_max..u32::MAX is [MAX-500, MAX), 0..500 is [0, 500)
        // They are NOT adjacent because MAX != 0 in wrapping (gap of 1)
        // However the gap is only 1 byte, so they won't auto-merge
        assert_eq!(sb.block_count(), 2);

        // Verify sequence arithmetic across the boundary
        assert!(sb.is_sacked(near_max + 100));
        assert!(sb.is_sacked(200));
    }
}
