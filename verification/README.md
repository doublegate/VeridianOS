# VeridianOS Formal Verification

Formal verification infrastructure for VeridianOS, combining Rust-level
property-based proofs (Kani) with system-level model checking (TLA+).

## Overview

| Method | Scope | Tool | Count |
|--------|-------|------|-------|
| Kani proofs | Rust data structures, algorithms | [Kani](https://model-checking.github.io/kani/) | 38 harnesses |
| TLA+ specs | System protocols, invariants | [TLC](https://github.com/tlaplus/tlaplus) | 6 specifications |

## Directory Structure

```
verification/
    kani/           Kani proof documentation (proofs live in kernel/src/verification/)
    tla+/           TLA+ specifications and TLC configuration files
```

## Quick Start

Run all available verifications:

```bash
./scripts/verify.sh
```

The script auto-detects installed tools and runs what's available.

## Kani (Rust Model Checking)

38 proof harnesses covering IPC, capabilities, memory allocation, and boot chain.
Proofs are colocated with kernel source in `kernel/src/verification/`.

See [kani/README.md](kani/README.md) for details.

## TLA+ (Protocol Model Checking)

6 specifications with TLC configuration files for automated model checking.
Covers boot chain integrity, capability security, IPC correctness, deadlock
freedom, memory conservation, and information flow control.

See [tla+/README.md](tla+/README.md) for details.

## Tool Installation

### Kani

```bash
cargo install --locked kani-verifier
cargo kani setup
```

### TLA+ (TLC)

Download `tla2tools.jar` from the [TLA+ releases](https://github.com/tlaplus/tlaplus/releases):

```bash
wget https://github.com/tlaplus/tlaplus/releases/latest/download/tla2tools.jar
```

Or install the [TLA+ Toolbox IDE](https://lamport.azurewebsites.net/tla/toolbox.html).
