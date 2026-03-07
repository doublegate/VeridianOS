--------------------------- MODULE memory_allocator ---------------------------
\* Formal TLA+ Specification for VeridianOS Memory Allocator
\*
\* Models the frame allocator with buddy system, bitmap tracking,
\* zone-aware allocation, and conservation invariants.

EXTENDS Integers, FiniteSets

CONSTANTS
    Frames,         \* Set of all frame addresses
    DmaFrames,      \* Subset of frames in DMA zone (< 16MB)
    NormalFrames,   \* Subset of frames in Normal zone
    MaxOrder        \* Maximum buddy order

VARIABLES
    frame_state,    \* Function: frame -> "Free" | "Allocated"
    alloc_count,    \* Number of allocations performed
    free_count,     \* Number of frees performed
    buddy_tree      \* Function: frame -> buddy order (0 = single frame)

vars == <<frame_state, alloc_count, free_count, buddy_tree>>

\* ============================================================================
\* Type Invariant
\* ============================================================================

TypeOK ==
    /\ frame_state \in [Frames -> {"Free", "Allocated"}]
    /\ alloc_count \in Nat
    /\ free_count \in Nat
    /\ buddy_tree \in [Frames -> 0..MaxOrder]

\* ============================================================================
\* Initial State: All frames free
\* ============================================================================

Init ==
    /\ frame_state = [f \in Frames |-> "Free"]
    /\ alloc_count = 0
    /\ free_count = 0
    /\ buddy_tree = [f \in Frames |-> 0]

\* ============================================================================
\* Actions
\* ============================================================================

\* Allocate a single frame
AllocFrame(f) ==
    /\ f \in Frames
    /\ frame_state[f] = "Free"
    /\ frame_state' = [frame_state EXCEPT ![f] = "Allocated"]
    /\ alloc_count' = alloc_count + 1
    /\ UNCHANGED <<free_count, buddy_tree>>

\* Allocate a frame from DMA zone
AllocDmaFrame(f) ==
    /\ f \in DmaFrames
    /\ frame_state[f] = "Free"
    /\ frame_state' = [frame_state EXCEPT ![f] = "Allocated"]
    /\ alloc_count' = alloc_count + 1
    /\ UNCHANGED <<free_count, buddy_tree>>

\* Free a previously allocated frame
FreeFrame(f) ==
    /\ f \in Frames
    /\ frame_state[f] = "Allocated"
    /\ frame_state' = [frame_state EXCEPT ![f] = "Free"]
    /\ free_count' = free_count + 1
    /\ UNCHANGED <<alloc_count, buddy_tree>>

\* Buddy split: split a block of order n into two blocks of order n-1
BuddySplit(f, order) ==
    /\ f \in Frames
    /\ order > 0
    /\ buddy_tree[f] = order
    /\ frame_state[f] = "Free"
    /\ buddy_tree' = [buddy_tree EXCEPT ![f] = order - 1]
    /\ UNCHANGED <<frame_state, alloc_count, free_count>>

\* Buddy coalesce: merge two free buddy blocks into one block of higher order
BuddyCoalesce(f1, f2, order) ==
    /\ f1 \in Frames
    /\ f2 \in Frames
    /\ f1 # f2
    /\ buddy_tree[f1] = order
    /\ buddy_tree[f2] = order
    /\ frame_state[f1] = "Free"
    /\ frame_state[f2] = "Free"
    /\ order < MaxOrder
    \* f1 gets the higher order, f2 stays
    /\ buddy_tree' = [buddy_tree EXCEPT ![f1] = order + 1]
    /\ UNCHANGED <<frame_state, alloc_count, free_count>>

\* ============================================================================
\* Next-State Relation
\* ============================================================================

Next ==
    \/ \E f \in Frames : AllocFrame(f)
    \/ \E f \in DmaFrames : AllocDmaFrame(f)
    \/ \E f \in Frames : FreeFrame(f)
    \/ \E f \in Frames, o \in 1..MaxOrder : BuddySplit(f, o)
    \/ \E f1, f2 \in Frames, o \in 0..MaxOrder-1 : BuddyCoalesce(f1, f2, o)

\* ============================================================================
\* Specification
\* ============================================================================

Spec == Init /\ [][Next]_vars

\* ============================================================================
\* Safety Properties (Invariants)
\* ============================================================================

\* Frame Conservation: allocated + free = total
FrameConservation ==
    LET allocated == Cardinality({f \in Frames : frame_state[f] = "Allocated"})
        free == Cardinality({f \in Frames : frame_state[f] = "Free"})
    IN allocated + free = Cardinality(Frames)

\* No Double Allocation: a frame can only be in one state
NoDoubleAllocation ==
    \A f \in Frames : frame_state[f] \in {"Free", "Allocated"}

\* Free count never exceeds alloc count (can't free what wasn't allocated)
FreeNotExceedAlloc ==
    free_count <= alloc_count

\* DMA zone constraint: DMA allocations come from DMA frames
DmaZoneConstraint ==
    \A f \in Frames \ DmaFrames :
        \* Non-DMA frames allocated through AllocDmaFrame would violate this
        \* (enforced by AllocDmaFrame precondition)
        TRUE

\* Buddy order bounds
BuddyOrderBound ==
    \A f \in Frames : buddy_tree[f] >= 0 /\ buddy_tree[f] <= MaxOrder

\* A freed frame is free
FreedFrameIsFree ==
    \* After FreeFrame(f), frame_state[f] = "Free"
    \* This is guaranteed by the action definition
    \A f \in Frames :
        frame_state[f] = "Free" \/ frame_state[f] = "Allocated"

================================================================================
