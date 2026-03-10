# Capability System Design

> **Authoritative specification**: [docs/design/CAPABILITY-SYSTEM-DESIGN.md](https://github.com/doublegate/VeridianOS/blob/main/docs/design/CAPABILITY-SYSTEM-DESIGN.md)
>
> **Implementation Status**: Complete as of v0.25.1. 64-bit packed tokens, two-level O(1) lookup, per-CPU cache, hierarchical inheritance, cascading revocation. Benchmarked at 57ns cap_validate.

See the authoritative specification linked above for the full design document including token format, delegation trees, revocation algorithms, and integration with IPC and system calls.
