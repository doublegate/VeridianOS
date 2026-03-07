# Kani Proof Harnesses

38 Kani proof harnesses verifying core kernel data structures and algorithms.
Source files are in `kernel/src/verification/` (colocated with kernel code).

## Installation

```bash
cargo install --locked kani-verifier
cargo kani setup
```

## Running

```bash
# Run all proofs
cd kernel && cargo kani

# Run a specific harness
cd kernel && cargo kani --harness proof_no_double_allocation
```

## Proof Inventory

### IPC Proofs (`ipc_proofs.rs`) -- 12 harnesses

| Harness | Property |
|---------|----------|
| `proof_fast_path_register_integrity` | Register-based IPC preserves message data |
| `proof_send_receive_roundtrip` | Send followed by receive returns original message |
| `proof_fifo_ordering` | Messages dequeued in send order |
| `proof_no_message_loss` | No messages lost in transit |
| `proof_channel_capacity_bound` | Buffer never exceeds declared capacity |
| `proof_channel_isolation` | Messages stay within their channel |
| `proof_capability_required` | Send requires valid capability |
| `proof_zero_copy_no_overlap` | Shared memory regions don't overlap |
| `proof_deadlock_freedom` | Wait-for graph has no cycles |
| `proof_async_ring_buffer_safety` | Ring buffer indices stay in bounds |
| `proof_message_type_safety` | Message type tags match payload |
| `proof_notification_delivery` | Notifications reach all waiters |

### Capability Proofs (`cap_proofs.rs`) -- 8 harnesses

| Harness | Property |
|---------|----------|
| `proof_token_encoding_roundtrip` | Token encode/decode is bijective |
| `proof_no_forgery` | Random tokens fail validation |
| `proof_derivation_subset` | Derived rights are subset of parent |
| `proof_cascading_revocation` | Revoking parent revokes all children |
| `proof_generation_invalidation` | Generation bump invalidates old tokens |
| `proof_rights_mask_operations` | Bitwise rights operations are correct |
| `proof_capability_isolation` | Process A cannot access process B's capabilities |
| `proof_revocation_completeness` | No orphaned capabilities after revocation |

### Boot Chain Proofs (`boot_chain.rs`) -- 8 harnesses

| Harness | Property |
|---------|----------|
| `proof_pcr_extend_monotonic` | PCR values only increase |
| `proof_pcr_extend_deterministic` | Same inputs produce same PCR state |
| `proof_measurement_log_ordered` | Log entries in chronological order |
| `proof_boot_status_transitions` | Status follows valid transition graph |
| `proof_policy_decision_complete` | All boot states get a policy decision |
| `proof_hash_chain_integrity` | Replaying log reproduces PCR values |
| `proof_pcr_no_reset` | PCRs cannot be reset after extension |
| `proof_measurement_count_matches` | Log length matches measurement count |

### Allocator Proofs (`alloc_proofs.rs`) -- 10 harnesses

| Harness | Property |
|---------|----------|
| `proof_no_double_allocation` | A frame cannot be allocated twice |
| `proof_dealloc_makes_available` | Freed frames become available |
| `proof_buddy_split_correct` | Splitting produces valid sub-blocks |
| `proof_buddy_coalesce_correct` | Coalescing produces valid parent block |
| `proof_bitmap_buddy_threshold` | Correct allocator selected by size |
| `proof_frame_conservation` | Total frames = allocated + free |
| `proof_zone_dma_range` | DMA frames within physical range |
| `proof_alignment_preserved` | Allocated frames are page-aligned |
| `proof_no_overlap` | No two allocations share a frame |
| `proof_free_idempotent` | Double-free is detected and rejected |
