# Security Policy

The authoritative security policy is maintained in the repository root:

**[SECURITY.md](https://github.com/doublegate/VeridianOS/blob/main/SECURITY.md)**

This page summarizes it; where they differ, SECURITY.md and
[Known Limitations](https://github.com/doublegate/VeridianOS/blob/main/docs/KNOWN-LIMITATIONS.md)
are correct.

## Reporting Vulnerabilities

- Report privately through GitHub: the repository's **Security** tab, **Report a vulnerability**
  (<https://github.com/doublegate/VeridianOS/security/advisories/new>)
- **Do NOT** open public issues for security vulnerabilities
- **Response time**: within 48 hours for acknowledgment

## Security Features

This section lists what is in effect. Features that exist as code but are not active yet
(KPTI, KASLR, stack canaries, SMEP/SMAP, retpoline and other speculation mitigations, IOMMU
isolation, seccomp, TLS certificate verification) are tracked in Known Limitations.

### Capability-Based Security
- 64-bit capability tokens with generation counters
- O(1) capability lookup in a per-process table
- Hierarchical inheritance with cascading revocation (not yet along every grant path, N-89)
- Capability checks on IPC, shared memory and exec (filtered capability space)

### Cryptographic Services
- ChaCha20-Poly1305, Ed25519, X25519, SHA-256, AES; the in-tree implementations are not
  constant-time and have no known-answer tests (N-148, N-154)
- Post-quantum: ML-DSA-65 signature verification (FIPS 204, RustCrypto `ml-dsa`, NIST ACVP known-answer tests); an experimental Kyber KEM that is not FIPS 203 ML-KEM (N-57)
- TLS 1.3, SSH and WireGuard protocol code (the TLS client does not yet verify certificate
  signatures, N-150; WireGuard is not bound to the network stack, NET-INC-02)
- CSPRNG seeded from RDRAND where the CPU has it (timer jitter only elsewhere, N-149)

### Kernel Hardening
- NX and W^X: page permissions follow `mmap`/`mprotect`/`brk` protection flags; W+X only for
  programs with the opt-in (ADR 0017); a W^X kernel image
- Guard pages around kernel stacks on all three architectures, and a bounded user stack with a
  guard gap
- Fault-tolerant user-memory copies (EFAULT instead of a kernel fault)
- BTI for AArch64 user programs
- Checked arithmetic in critical paths

### Accounts and Access Control
- Credentials with real, effective, saved and filesystem IDs and supplementary groups (`/etc/group`
  is not applied at login, N-153)
- Salted password hashes compared in constant time, with a password history
- The kernel console has no login, and `su`/`sudo` there are stubs (N-131); MFA cannot be
  completed through PAM yet (planned for v0.28)
- MAC policy parser with an RBAC/MLS model; enforcement is path-blind and partial (N-152)
- Audit logging framework

### Hardware Security
- TPM 2.0 and secure-boot verification code
- IOMMU tables are parsed, but DMA isolation is not enabled (N-77)

### Memory Safety
- Written in Rust (memory safety by default)
- 3 justified `static mut` remaining (early serial, boot info, per-CPU data)
- Every `unsafe` block carries a `// SAFETY:` comment (enforced by clippy)

### Network Security
- Firewall code with conntrack and NAT
- User-space INET sockets do not work yet (N-64 to N-76); loopback and Unix sockets do

Do not expose a VeridianOS system to untrusted users or networks.

## Security Scan History

- **v0.20.2**: 7 findings remediated (2 medium, 2 low, 2 info, 1 doc)
  - Password history: salted hashes with constant-time comparison
  - Capability revocation: cache invalidation before revoke
  - Compositor bounds checking
  - ACPI checked arithmetic
- **v0.26.0 and v0.27.0**: audit remediation; every finding is tracked as N-xx in `docs/audit/`
  and, while open, in Known Limitations

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.27.x (latest) | Yes |
| main branch | Yes |
| < 0.27 | No |
