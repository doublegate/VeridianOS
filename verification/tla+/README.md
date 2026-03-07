# TLA+ Specifications

6 TLA+ specifications modeling VeridianOS system protocols and invariants.
Each spec has a `.cfg` file for automated TLC model checking.

## Installation

Download `tla2tools.jar`:

```bash
wget https://github.com/tlaplus/tlaplus/releases/latest/download/tla2tools.jar
```

Or use a package manager:

```bash
# Arch Linux
yay -S tla-toolbox

# macOS
brew install --cask tla-plus-toolbox
```

## Running

```bash
# Check a single spec
java -jar tla2tools.jar -config boot_chain.cfg boot_chain.tla

# Check all specs (use scripts/verify.sh)
../../scripts/verify.sh
```

## Specification Inventory

### boot_chain.tla -- Verified Boot Chain

Models PCR extension, measurement logging, and boot policy decisions through
6 stages (Firmware, Bootloader, Kernel, Init, Drivers, UserSpace).

| Invariant | Property |
|-----------|----------|
| `TypeOK` | All variables well-typed |
| `PcrMonotonicity` | PCR extend counts only increase |
| `MeasurementCompleteness` | All stages measured before approval |
| `NoPcrReset` | No PCR resets after extension |

### capability_model.tla -- Capability System

Models 64-bit capability tokens with rights lattice, derivation chains,
and cascading revocation.

| Invariant | Property |
|-----------|----------|
| `TypeOK` | All variables well-typed |
| `NonForgery` | Capabilities created only through API |
| `RightsMonotonicity` | Child rights subset of parent |
| `RevocationCompleteness` | Parent revocation cascades to children |
| `GenerationIntegrity` | Revocation bumps generation counter |
| `NoOrphans` | No valid capability has an invalid parent |

### ipc_protocol.tla -- IPC Protocol

Models FIFO channels with bounded capacity, message ordering,
and capability-based authorization.

| Invariant | Property |
|-----------|----------|
| `TypeOK` | All variables well-typed |
| `FifoInvariant` | Messages dequeued in send order |
| `CapacityBound` | Buffer size never exceeds capacity |
| `MessageConservation` | sent - received = pending |
| `GlobalConservation` | No messages created or destroyed |
| `ChannelIsolation` | Messages tagged to correct channel |
| `AuthorizedSenders` | Only authorized processes send |

### ipc_deadlock.tla -- Deadlock Freedom

Models wait-for graph with cycle detection to prove system-wide
deadlock freedom.

| Invariant | Property |
|-----------|----------|
| `TypeOK` | All variables well-typed |
| `DeadlockFreedom` | No cycles in wait-for graph |
| `NoSelfWait` | No process waits for itself |
| `BoundedWaiting` | Wait sets bounded by process count |

### memory_allocator.tla -- Frame Allocator

Models hybrid bitmap + buddy system with DMA zone awareness
and frame conservation.

| Invariant | Property |
|-----------|----------|
| `TypeOK` | All variables well-typed |
| `FrameConservation` | allocated + free = total |
| `NoDoubleAllocation` | Frame in exactly one state |
| `FreeNotExceedAlloc` | Cannot free more than allocated |
| `BuddyOrderBound` | Orders within 0..MaxOrder |

### information_flow.tla -- Non-Interference

Models multi-level security domains with lattice ordering
and capability-based declassification.

| Invariant | Property |
|-----------|----------|
| `TypeOK` | All variables well-typed |
| `NonInterference` | No unauthorized information flow |
| `NoDownwardFlow` | High-to-low requires declassification |

## Configuration Notes

The `.cfg` files use small constant domains (3-5 elements) to keep model
checking feasible. Larger constants provide stronger guarantees but
exponentially increase state space. Adjust as needed for deeper analysis.
