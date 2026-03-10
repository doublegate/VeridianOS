# Security Policy

The authoritative security policy is maintained in the repository root:

**[SECURITY.md](https://github.com/doublegate/VeridianOS/blob/main/SECURITY.md)**

## Reporting Vulnerabilities

- **Email**: security@veridian-os.org
- **Do NOT** open public issues for security vulnerabilities
- **Response time**: Within 48 hours for acknowledgment

## Security Features (All Complete as of v0.25.1)

### Capability-Based Security
- Unforgeable 64-bit capability tokens with generation counters
- Two-level O(1) capability lookup with per-CPU cache
- Hierarchical inheritance with cascading revocation
- System call capability enforcement

### Cryptographic Services
- ChaCha20-Poly1305, Ed25519, X25519, SHA-256
- Post-quantum: ML-KEM (Kyber), ML-DSA (Dilithium)
- TLS 1.3, SSH, WireGuard VPN
- Hardware CSPRNG (RDRAND with CPUID check)

### Kernel Hardening
- KASLR (Kernel Address Space Layout Randomization)
- Stack canaries and guards
- SMEP/SMAP enforcement
- Retpoline for Spectre mitigation
- W^X enforcement
- Checked arithmetic in critical paths

### Mandatory Access Control
- MAC policy parser with RBAC and MLS enforcement
- Audit logging framework
- Secure boot chain verification

### Hardware Security
- TPM integration
- Intel TDX, AMD SEV-SNP, ARM CCA
- IOMMU for DMA protection

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
