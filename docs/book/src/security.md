# Security Policy

The authoritative security policy is maintained in the repository root:

**[SECURITY.md](https://github.com/doublegate/VeridianOS/blob/main/SECURITY.md)**

## Reporting Vulnerabilities

- **Email**: security@veridian-os.org
- **Do NOT** open public issues for security vulnerabilities
- **Response time**: Within 48 hours for acknowledgment

## Security Features

This section lists what is in effect. Features that exist as code but are not active yet
(KPTI, KASLR, stack canaries, SMEP/SMAP, retpoline and other speculation mitigations, IOMMU
isolation, seccomp, TLS certificate verification) are tracked in
[Known Limitations](https://github.com/doublegate/VeridianOS/blob/main/docs/KNOWN-LIMITATIONS.md).

### Capability-Based Security
- 64-bit capability tokens with generation counters
- O(1) capability lookup in a per-process table
- Hierarchical inheritance with cascading revocation (not yet along every grant path, N-89)
- Capability checks on IPC, shared memory and exec (filtered capability space)

### Cryptographic Services
- ChaCha20-Poly1305, Ed25519, X25519, SHA-256
- Post-quantum: ML-DSA-65 signature verification (FIPS 204, RustCrypto `ml-dsa`, NIST ACVP known-answer tests); an experimental Kyber KEM that is not FIPS 203 ML-KEM (N-57)
- TLS 1.3, SSH and WireGuard protocol code (the TLS client does not yet verify certificate
  signatures, N-150; WireGuard is not bound to the network stack, NET-INC-02)
- CSPRNG seeded from RDRAND where the CPU has it (timer jitter only elsewhere, N-149)

### Kernel Hardening
- NX and W^X: page permissions follow `mmap`/`mprotect`/`brk` protection flags
- Guard pages around x86_64 kernel stacks and a bounded user stack with a guard gap
- Fault-tolerant user-memory copies (EFAULT instead of a kernel fault)
- Checked arithmetic in critical paths

### Mandatory Access Control
- MAC policy parser with an RBAC/MLS model; enforcement is path-blind and partial (N-152)
- Audit logging framework
- Secure boot chain verification code

### Hardware Security
- TPM integration code
- Intel TDX, AMD SEV-SNP and ARM CCA interfaces (not exercised on hardware)

### Memory Safety
- Written in Rust (memory safety by default)
- 7 justified `static mut` remaining (early boot, per-CPU, heap)
- 99%+ SAFETY comment coverage on all unsafe blocks
- 0 soundness bugs

### Network Security
- Stateful firewall with NAT/conntrack
- Certificate pinning
- Network isolation

## Security Scan History

- **v0.20.2**: 7 findings remediated (2 medium, 2 low, 2 info, 1 doc)
  - Password history: salted hashes with constant-time comparison
  - Capability revocation: cache invalidation before revoke
  - Compositor bounds checking
  - ACPI checked arithmetic

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.25.x (latest) | Yes |
| main branch | Yes |
| < 0.25 | No |
