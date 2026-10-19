# Capability System Design

> **Authoritative specification**: [docs/design/CAPABILITY-SYSTEM-DESIGN.md](https://github.com/doublegate/VeridianOS/blob/main/docs/design/CAPABILITY-SYSTEM-DESIGN.md)
>
> **Implementation Status (v0.27.0)**: 64-bit packed tokens with generation counters, a per-process hash table (O(1) average; the two-level table was replaced in v0.26.0, CAP-PERF-01), hierarchical delegation and cascading revocation. `cap_lookup` 29 ns (release kernel, KVM). Revocation does not yet follow every grant path (N-89).

See the authoritative specification linked above for the full design document including token format, delegation trees, revocation algorithms, and integration with IPC and system calls.
