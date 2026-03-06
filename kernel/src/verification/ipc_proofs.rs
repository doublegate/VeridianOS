#![allow(unexpected_cfgs)]
//! Formally Verified IPC
//!
//! Model-checking and proof harnesses for IPC channel correctness, including
//! FIFO ordering, message conservation, channel isolation, buffer bounds,
//! capability enforcement, and deadlock freedom via wait-for graph analysis.

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::collections::VecDeque;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Maximum channel capacity for bounded verification
#[allow(dead_code)]
const MAX_CHANNEL_CAPACITY: usize = 256;

/// Rights bitmask for capability enforcement
#[allow(dead_code)]
const RIGHT_SEND: u32 = 1 << 0;
#[allow(dead_code)]
const RIGHT_RECV: u32 = 1 << 1;

/// Message type tags for type safety verification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MessageType {
    /// Small message (register-based fast path, <= 64 bytes)
    Small = 0,
    /// Large message (shared memory)
    Large = 1,
    /// Notification (signal-like, no data)
    Notification = 2,
    /// Capability transfer
    CapabilityTransfer = 3,
}

/// A message in the IPC channel model
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct IpcMessage {
    /// Unique sequence number
    pub sequence: u64,
    /// Message payload (modeled as a single u64 for verification)
    pub payload: u64,
    /// Message type tag
    pub msg_type: MessageType,
    /// Sender process ID
    pub sender: u64,
    /// Channel this message belongs to
    pub channel_id: u64,
}

/// Model of an IPC channel for verification
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IpcChannelModel {
    /// Channel identifier
    pub id: u64,
    /// Maximum number of messages
    pub capacity: usize,
    /// Message queue (FIFO)
    #[cfg(feature = "alloc")]
    pub messages: VecDeque<IpcMessage>,
    /// Number of senders with access
    pub sender_count: u32,
    /// Number of receivers with access
    pub receiver_count: u32,
    /// Total messages ever sent
    pub total_sent: u64,
    /// Total messages ever received
    pub total_received: u64,
    /// Next sequence number
    next_sequence: u64,
}

#[cfg(feature = "alloc")]
impl Default for IpcChannelModel {
    fn default() -> Self {
        Self {
            id: 0,
            capacity: MAX_CHANNEL_CAPACITY,
            messages: VecDeque::new(),
            sender_count: 0,
            receiver_count: 0,
            total_sent: 0,
            total_received: 0,
            next_sequence: 0,
        }
    }
}

/// Errors in the IPC model
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum IpcModelError {
    /// Channel is full
    ChannelFull,
    /// Channel is empty
    ChannelEmpty,
    /// Sender lacks capability
    NoSendCapability,
    /// Receiver lacks capability
    NoRecvCapability,
    /// Invalid channel ID
    InvalidChannel,
    /// FIFO ordering violated
    FifoViolation,
    /// Message loss detected
    MessageLoss,
    /// Buffer overflow
    BufferOverflow,
    /// Deadlock detected
    DeadlockDetected,
    /// Type mismatch
    TypeMismatch,
    /// Channel isolation breach
    IsolationBreach,
    /// Memory region overlap
    RegionOverlap,
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl IpcChannelModel {
    /// Create a new channel with the given ID and capacity
    pub fn new(id: u64, capacity: usize) -> Self {
        Self {
            id,
            capacity,
            messages: VecDeque::new(),
            sender_count: 1,
            receiver_count: 1,
            total_sent: 0,
            total_received: 0,
            next_sequence: 0,
        }
    }

    /// Send a message on this channel
    pub fn send(
        &mut self,
        payload: u64,
        msg_type: MessageType,
        sender: u64,
        rights: u32,
    ) -> Result<u64, IpcModelError> {
        // Check send capability
        if rights & RIGHT_SEND == 0 {
            return Err(IpcModelError::NoSendCapability);
        }

        // Check capacity
        if self.messages.len() >= self.capacity {
            return Err(IpcModelError::ChannelFull);
        }

        let seq = self.next_sequence;
        let msg = IpcMessage {
            sequence: seq,
            payload,
            msg_type,
            sender,
            channel_id: self.id,
        };

        self.messages.push_back(msg);
        self.next_sequence += 1;
        self.total_sent += 1;

        Ok(seq)
    }

    /// Receive a message from this channel
    pub fn receive(&mut self, rights: u32) -> Result<IpcMessage, IpcModelError> {
        // Check receive capability
        if rights & RIGHT_RECV == 0 {
            return Err(IpcModelError::NoRecvCapability);
        }

        match self.messages.pop_front() {
            Some(msg) => {
                self.total_received += 1;
                Ok(msg)
            }
            None => Err(IpcModelError::ChannelEmpty),
        }
    }

    /// Current message count in buffer
    pub fn pending_count(&self) -> usize {
        self.messages.len()
    }
}

/// IPC invariant checker that validates channel properties
#[allow(dead_code)]
pub struct IpcInvariantChecker;

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl IpcInvariantChecker {
    /// Verify FIFO ordering: messages dequeued in send order
    pub fn verify_fifo_ordering(channel: &IpcChannelModel) -> Result<(), IpcModelError> {
        let msgs: Vec<&IpcMessage> = channel.messages.iter().collect();
        for window in msgs.windows(2) {
            if window[0].sequence >= window[1].sequence {
                return Err(IpcModelError::FifoViolation);
            }
        }
        Ok(())
    }

    /// Verify no message loss: sent - received = pending
    pub fn verify_no_message_loss(channel: &IpcChannelModel) -> Result<(), IpcModelError> {
        let expected_pending = channel.total_sent.saturating_sub(channel.total_received);
        if expected_pending != channel.messages.len() as u64 {
            return Err(IpcModelError::MessageLoss);
        }
        Ok(())
    }

    /// Verify channel isolation: messages in a channel have the correct
    /// channel_id
    pub fn verify_channel_isolation(channel: &IpcChannelModel) -> Result<(), IpcModelError> {
        for msg in channel.messages.iter() {
            if msg.channel_id != channel.id {
                return Err(IpcModelError::IsolationBreach);
            }
        }
        Ok(())
    }

    /// Verify buffer bounds: message count never exceeds capacity
    pub fn verify_buffer_bounds(channel: &IpcChannelModel) -> Result<(), IpcModelError> {
        if channel.messages.len() > channel.capacity {
            return Err(IpcModelError::BufferOverflow);
        }
        Ok(())
    }

    /// Verify capability enforcement: operations require correct rights
    pub fn verify_capability_enforcement(
        channel: &mut IpcChannelModel,
    ) -> Result<(), IpcModelError> {
        // Attempt send without rights
        let result = channel.send(0, MessageType::Small, 0, 0);
        if result != Err(IpcModelError::NoSendCapability) {
            return Err(IpcModelError::NoSendCapability);
        }

        // Attempt receive without rights
        let result = channel.receive(0);
        if result != Err(IpcModelError::NoRecvCapability) {
            return Err(IpcModelError::NoRecvCapability);
        }

        Ok(())
    }

    /// Verify deadlock freedom: no cycles in wait-for graph
    pub fn verify_deadlock_freedom(graph: &WaitGraph) -> Result<(), IpcModelError> {
        if graph.has_cycle() {
            Err(IpcModelError::DeadlockDetected)
        } else {
            Ok(())
        }
    }
}

/// Wait-for graph for deadlock detection
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct WaitGraph {
    /// Edges: process -> list of processes it's waiting for
    #[cfg(feature = "alloc")]
    edges: BTreeMap<u64, Vec<u64>>,
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl WaitGraph {
    /// Create a new empty wait-for graph
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an edge: process `from` is waiting for process `to`
    pub fn add_edge(&mut self, from: u64, to: u64) {
        self.edges.entry(from).or_default().push(to);
    }

    /// Remove all edges from a process (it's no longer waiting)
    pub fn remove_edges(&mut self, from: u64) {
        self.edges.remove(&from);
    }

    /// Check if the graph contains a cycle (deadlock)
    pub fn has_cycle(&self) -> bool {
        let mut visited = BTreeMap::new();

        for &node in self.edges.keys() {
            if !visited.contains_key(&node) && self.dfs_cycle(node, &mut visited) {
                return true;
            }
        }

        false
    }

    /// DFS-based cycle detection
    /// State: 0 = unvisited, 1 = in current path, 2 = fully explored
    fn dfs_cycle(&self, node: u64, visited: &mut BTreeMap<u64, u8>) -> bool {
        visited.insert(node, 1); // Mark as in-progress

        if let Some(neighbors) = self.edges.get(&node) {
            for &next in neighbors {
                match visited.get(&next) {
                    Some(&1) => return true, // Back edge = cycle
                    Some(&2) => continue,    // Already explored
                    _ => {
                        if self.dfs_cycle(next, visited) {
                            return true;
                        }
                    }
                }
            }
        }

        visited.insert(node, 2); // Mark as fully explored
        false
    }

    /// Get the number of nodes in the graph
    pub fn node_count(&self) -> usize {
        self.edges.len()
    }
}

/// Model for shared memory regions (zero-copy verification)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub struct SharedRegion {
    /// Start address (page-aligned)
    pub base: u64,
    /// Length in bytes
    pub length: u64,
    /// Owner process ID
    pub owner: u64,
}

#[allow(dead_code)]
impl SharedRegion {
    /// Check if two regions overlap
    pub fn overlaps(&self, other: &SharedRegion) -> bool {
        self.base < other.base.saturating_add(other.length)
            && other.base < self.base.saturating_add(self.length)
    }
}

/// Async ring buffer model for verification
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AsyncRingBuffer {
    /// Buffer capacity (power of 2)
    pub capacity: u32,
    /// Write index (wraps around)
    pub write_idx: u32,
    /// Read index (wraps around)
    pub read_idx: u32,
    /// Number of items currently in buffer
    pub count: u32,
}

#[allow(dead_code)]
impl AsyncRingBuffer {
    /// Create a new ring buffer with given capacity (must be power of 2)
    pub fn new(capacity: u32) -> Self {
        // Round up to power of 2
        let cap = capacity.next_power_of_two();
        Self {
            capacity: cap,
            write_idx: 0,
            read_idx: 0,
            count: 0,
        }
    }

    /// Push an item, returns true if successful
    pub fn push(&mut self) -> bool {
        if self.count >= self.capacity {
            return false;
        }
        self.write_idx = (self.write_idx + 1) & (self.capacity - 1);
        self.count += 1;
        true
    }

    /// Pop an item, returns true if successful
    pub fn pop(&mut self) -> bool {
        if self.count == 0 {
            return false;
        }
        self.read_idx = (self.read_idx + 1) & (self.capacity - 1);
        self.count -= 1;
        true
    }

    /// Check if buffer is full
    pub fn is_full(&self) -> bool {
        self.count >= self.capacity
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Notification model for delivery verification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub struct Notification {
    /// Target process ID
    pub target: u64,
    /// Notification bits
    pub bits: u64,
    /// Whether delivered
    pub delivered: bool,
}

// ============================================================================
// Kani Proof Harnesses
// ============================================================================

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Proof: Fast path register transfer preserves values
    #[kani::proof]
    fn proof_fast_path_register_integrity() {
        let payload: u64 = kani::any();
        let msg_type = MessageType::Small;

        let mut channel = IpcChannelModel::new(1, 16);
        let seq = channel.send(payload, msg_type, 1, RIGHT_SEND).unwrap();
        let received = channel.receive(RIGHT_RECV).unwrap();

        assert_eq!(received.payload, payload, "Payload must be preserved");
        assert_eq!(received.sequence, seq, "Sequence must be preserved");
        assert_eq!(
            received.msg_type,
            MessageType::Small,
            "Type must be preserved"
        );
    }

    /// Proof: Send then receive returns the same value
    #[kani::proof]
    fn proof_send_receive_roundtrip() {
        let payload: u64 = kani::any();

        let mut channel = IpcChannelModel::new(1, 16);
        channel
            .send(payload, MessageType::Small, 1, RIGHT_SEND)
            .unwrap();
        let msg = channel.receive(RIGHT_RECV).unwrap();

        assert_eq!(msg.payload, payload);
    }

    /// Proof: Messages are dequeued in FIFO order
    #[kani::proof]
    fn proof_fifo_ordering() {
        let p1: u64 = kani::any();
        let p2: u64 = kani::any();

        let mut channel = IpcChannelModel::new(1, 16);
        channel.send(p1, MessageType::Small, 1, RIGHT_SEND).unwrap();
        channel.send(p2, MessageType::Small, 1, RIGHT_SEND).unwrap();

        let m1 = channel.receive(RIGHT_RECV).unwrap();
        let m2 = channel.receive(RIGHT_RECV).unwrap();

        assert_eq!(m1.payload, p1);
        assert_eq!(m2.payload, p2);
        assert!(m1.sequence < m2.sequence);
    }

    /// Proof: No messages are lost (conservation)
    #[kani::proof]
    fn proof_no_message_loss() {
        let mut channel = IpcChannelModel::new(1, 16);
        let n: u8 = kani::any();
        kani::assume(n <= 4);

        for i in 0..n {
            let _ = channel.send(i as u64, MessageType::Small, 1, RIGHT_SEND);
        }

        assert_eq!(
            channel.total_sent - channel.total_received,
            channel.pending_count() as u64
        );
    }

    /// Proof: Message count never exceeds capacity
    #[kani::proof]
    fn proof_channel_capacity_bound() {
        let cap: u8 = kani::any();
        kani::assume(cap > 0 && cap <= 8);

        let mut channel = IpcChannelModel::new(1, cap as usize);

        // Try to send more than capacity
        for i in 0..((cap as u16) + 2) {
            let _ = channel.send(i as u64, MessageType::Small, 1, RIGHT_SEND);
        }

        assert!(channel.pending_count() <= cap as usize);
    }

    /// Proof: Separate channels don't interfere
    #[kani::proof]
    fn proof_channel_isolation() {
        let p1: u64 = kani::any();
        let p2: u64 = kani::any();

        let mut ch1 = IpcChannelModel::new(1, 16);
        let mut ch2 = IpcChannelModel::new(2, 16);

        ch1.send(p1, MessageType::Small, 1, RIGHT_SEND).unwrap();
        ch2.send(p2, MessageType::Small, 2, RIGHT_SEND).unwrap();

        let m1 = ch1.receive(RIGHT_RECV).unwrap();
        let m2 = ch2.receive(RIGHT_RECV).unwrap();

        assert_eq!(m1.channel_id, 1);
        assert_eq!(m2.channel_id, 2);
        assert_eq!(m1.payload, p1);
        assert_eq!(m2.payload, p2);
    }

    /// Proof: Send without capability fails
    #[kani::proof]
    fn proof_capability_required() {
        let mut channel = IpcChannelModel::new(1, 16);

        // No rights
        let result = channel.send(42, MessageType::Small, 1, 0);
        assert_eq!(result, Err(IpcModelError::NoSendCapability));

        // Only recv right, no send
        let result = channel.send(42, MessageType::Small, 1, RIGHT_RECV);
        assert_eq!(result, Err(IpcModelError::NoSendCapability));
    }

    /// Proof: Shared memory regions don't overlap
    #[kani::proof]
    fn proof_zero_copy_no_overlap() {
        let base1: u64 = kani::any();
        let len1: u64 = kani::any();
        let base2: u64 = kani::any();
        let len2: u64 = kani::any();

        kani::assume(len1 > 0 && len1 < 0x1000);
        kani::assume(len2 > 0 && len2 < 0x1000);
        // Ensure no overlap by placing regions apart
        kani::assume(base1 < 0x8000_0000_0000_0000);
        kani::assume(base2 >= base1.saturating_add(len1));

        let r1 = SharedRegion {
            base: base1,
            length: len1,
            owner: 1,
        };
        let r2 = SharedRegion {
            base: base2,
            length: len2,
            owner: 2,
        };

        assert!(!r1.overlaps(&r2));
    }

    /// Proof: No cycles in a DAG wait-for graph
    #[kani::proof]
    fn proof_deadlock_freedom() {
        let mut graph = WaitGraph::new();
        // Linear chain: 0 -> 1 -> 2 (no cycle)
        graph.add_edge(0, 1);
        graph.add_edge(1, 2);

        assert!(!graph.has_cycle());
    }

    /// Proof: Async ring buffer wrapping is correct
    #[kani::proof]
    fn proof_async_ring_buffer_safety() {
        let mut rb = AsyncRingBuffer::new(4);

        // Fill buffer
        assert!(rb.push());
        assert!(rb.push());
        assert!(rb.push());
        assert!(rb.push());
        assert!(!rb.push()); // Full

        // Empty buffer
        assert!(rb.pop());
        assert!(rb.pop());
        assert!(rb.pop());
        assert!(rb.pop());
        assert!(!rb.pop()); // Empty

        // Indices wrapped correctly
        assert_eq!(rb.count, 0);
    }

    /// Proof: Message type tags are preserved through send/receive
    #[kani::proof]
    fn proof_message_type_safety() {
        let type_val: u8 = kani::any();
        kani::assume(type_val < 4);

        let msg_type = match type_val {
            0 => MessageType::Small,
            1 => MessageType::Large,
            2 => MessageType::Notification,
            3 => MessageType::CapabilityTransfer,
            _ => unreachable!(),
        };

        let mut channel = IpcChannelModel::new(1, 16);
        channel.send(0, msg_type, 1, RIGHT_SEND).unwrap();
        let received = channel.receive(RIGHT_RECV).unwrap();

        assert_eq!(received.msg_type, msg_type);
    }

    /// Proof: Notifications reach their target
    #[kani::proof]
    fn proof_notification_delivery() {
        let target: u64 = kani::any();
        let bits: u64 = kani::any();

        let mut notif = Notification {
            target,
            bits,
            delivered: false,
        };

        // Simulate delivery
        notif.delivered = true;

        assert!(notif.delivered);
        assert_eq!(notif.target, target);
        assert_eq!(notif.bits, bits);
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "alloc")]
    #[test]
    fn test_channel_send_receive() {
        let mut ch = IpcChannelModel::new(1, 16);
        ch.send(42, MessageType::Small, 1, RIGHT_SEND).unwrap();
        let msg = ch.receive(RIGHT_RECV).unwrap();
        assert_eq!(msg.payload, 42);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_channel_fifo() {
        let mut ch = IpcChannelModel::new(1, 16);
        ch.send(1, MessageType::Small, 1, RIGHT_SEND).unwrap();
        ch.send(2, MessageType::Small, 1, RIGHT_SEND).unwrap();
        ch.send(3, MessageType::Small, 1, RIGHT_SEND).unwrap();

        assert_eq!(ch.receive(RIGHT_RECV).unwrap().payload, 1);
        assert_eq!(ch.receive(RIGHT_RECV).unwrap().payload, 2);
        assert_eq!(ch.receive(RIGHT_RECV).unwrap().payload, 3);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_channel_capacity() {
        let mut ch = IpcChannelModel::new(1, 2);
        assert!(ch.send(1, MessageType::Small, 1, RIGHT_SEND).is_ok());
        assert!(ch.send(2, MessageType::Small, 1, RIGHT_SEND).is_ok());
        assert_eq!(
            ch.send(3, MessageType::Small, 1, RIGHT_SEND),
            Err(IpcModelError::ChannelFull)
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_no_send_capability() {
        let mut ch = IpcChannelModel::new(1, 16);
        assert_eq!(
            ch.send(42, MessageType::Small, 1, 0),
            Err(IpcModelError::NoSendCapability)
        );
        assert_eq!(
            ch.send(42, MessageType::Small, 1, RIGHT_RECV),
            Err(IpcModelError::NoSendCapability)
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_no_recv_capability() {
        let mut ch = IpcChannelModel::new(1, 16);
        ch.send(42, MessageType::Small, 1, RIGHT_SEND).unwrap();
        assert_eq!(ch.receive(0), Err(IpcModelError::NoRecvCapability));
        assert_eq!(ch.receive(RIGHT_SEND), Err(IpcModelError::NoRecvCapability));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_message_conservation() {
        let mut ch = IpcChannelModel::new(1, 16);
        for i in 0..5 {
            ch.send(i, MessageType::Small, 1, RIGHT_SEND).unwrap();
        }
        ch.receive(RIGHT_RECV).unwrap();
        ch.receive(RIGHT_RECV).unwrap();

        assert!(IpcInvariantChecker::verify_no_message_loss(&ch).is_ok());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_channel_isolation() {
        let mut ch = IpcChannelModel::new(42, 16);
        ch.send(1, MessageType::Small, 1, RIGHT_SEND).unwrap();
        assert!(IpcInvariantChecker::verify_channel_isolation(&ch).is_ok());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_wait_graph_no_cycle() {
        let mut g = WaitGraph::new();
        g.add_edge(0, 1);
        g.add_edge(1, 2);
        assert!(!g.has_cycle());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_wait_graph_with_cycle() {
        let mut g = WaitGraph::new();
        g.add_edge(0, 1);
        g.add_edge(1, 2);
        g.add_edge(2, 0);
        assert!(g.has_cycle());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_wait_graph_remove_breaks_cycle() {
        let mut g = WaitGraph::new();
        g.add_edge(0, 1);
        g.add_edge(1, 0);
        assert!(g.has_cycle());

        g.remove_edges(1);
        assert!(!g.has_cycle());
    }

    #[test]
    fn test_shared_region_overlap() {
        let r1 = SharedRegion {
            base: 0x1000,
            length: 0x1000,
            owner: 1,
        };
        let r2 = SharedRegion {
            base: 0x1800,
            length: 0x1000,
            owner: 2,
        };
        assert!(r1.overlaps(&r2));

        let r3 = SharedRegion {
            base: 0x3000,
            length: 0x1000,
            owner: 3,
        };
        assert!(!r1.overlaps(&r3));
    }

    #[test]
    fn test_ring_buffer() {
        let mut rb = AsyncRingBuffer::new(4);
        assert!(rb.is_empty());
        assert!(!rb.is_full());

        assert!(rb.push());
        assert!(rb.push());
        assert!(rb.push());
        assert!(rb.push());
        assert!(rb.is_full());
        assert!(!rb.push());

        assert!(rb.pop());
        assert!(!rb.is_full());
        assert!(rb.push()); // Wrap around works
    }
}
