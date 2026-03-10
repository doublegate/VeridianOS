# Security Policy

## Supported Versions

VeridianOS is at v0.25.1 with all phases (0-12) complete. Security updates are provided for:

| Version | Supported          |
| ------- | ------------------ |
| 0.25.x (latest) | :white_check_mark: |
| main branch | :white_check_mark: |
| < 0.25  | :x:                |

Once we reach 1.0, we will maintain security updates for the current major version and one previous major version.

## Reporting a Vulnerability

We take the security of VeridianOS seriously. If you believe you have found a security vulnerability, please report it to us as described below.

### Please do NOT:

- Open a public issue
- Post to public forums or social media
- Exploit the vulnerability

### Please DO:

1. Email your findings to security@veridian-os.org
2. Encrypt your message using our PGP key (available at https://veridian-os.org/security-key.asc)
3. Include the following information:
   - Type of vulnerability
   - Full paths of source file(s) related to the issue
   - Location of affected code (tag/branch/commit or direct URL)
   - Step-by-step instructions to reproduce
   - Proof-of-concept or exploit code (if possible)
   - Impact assessment

### What to expect:

- **Acknowledgment**: Within 48 hours
- **Initial Assessment**: Within 1 week
- **Status Updates**: Every 2 weeks
- **Resolution Timeline**: Depends on severity
  - Critical: 1-7 days
  - High: 1-2 weeks
  - Medium: 2-4 weeks
  - Low: 1-2 months

### Recognition:

We maintain a Hall of Fame for security researchers who have responsibly disclosed vulnerabilities. With your permission, we will:

- Add your name to our Security Hall of Fame
- Acknowledge your contribution in release notes
- Provide a letter of recognition if requested

## Security Best Practices

When contributing to VeridianOS:

### Code Review

- All changes undergo security review
- Use static analysis tools
- Follow secure coding guidelines

### Dependencies

- Minimize external dependencies
- Audit all dependencies
- Keep dependencies updated
- Use `cargo audit` regularly

### Cryptography

- Never implement custom cryptography
- Use well-established libraries
- Follow current best practices

### Memory Safety

- Leverage Rust's memory safety
- Minimize unsafe code (7 justified `static mut` remaining)
- Document all safety invariants (99%+ SAFETY comment coverage)
- Use fuzzing for testing

## Security Features

VeridianOS implements multiple layers of security (all complete as of v0.25.1):

1. **Capability-based access control**
   - Unforgeable 64-bit capability tokens with generation counters
   - Fine-grained permissions with O(1) lookup
   - Hierarchical inheritance and cascading revocation
   - Per-CPU capability cache

2. **Memory protection**
   - W^X enforcement
   - KASLR (Kernel Address Space Layout Randomization)
   - Stack canaries and guards
   - Heap isolation
   - SMEP/SMAP enforcement

3. **Cryptographic services**
   - ChaCha20-Poly1305, Ed25519, X25519, SHA-256
   - Post-quantum cryptography: ML-KEM (Kyber), ML-DSA (Dilithium)
   - Hardware CSPRNG (RDRAND with CPUID check)
   - TLS 1.3, SSH, WireGuard VPN

4. **Mandatory access control**
   - MAC policy parser with RBAC and MLS enforcement
   - Audit logging framework
   - Secure boot chain verification

5. **Hardware security**
   - TPM integration
   - Intel TDX, AMD SEV-SNP, ARM CCA support
   - IOMMU for DMA protection
   - Retpoline for Spectre mitigation

6. **Network security**
   - Stateful firewall with NAT/conntrack
   - Mandatory TLS for system services
   - Certificate pinning
   - Network isolation

7. **Kernel hardening**
   - Speculative execution mitigations (retpoline)
   - Checked arithmetic in critical paths
   - Password history with salted hashes and constant-time comparison
   - Capability cache invalidation before revocation

## Development Security

### Threat Model

Our threat model considers:
- Malicious applications
- Network attackers
- Physical access attacks
- Supply chain attacks
- Side-channel attacks

### Security Testing

- Fuzzing with AFL++ and libFuzzer
- Static analysis with clippy and cargo-audit
- Dynamic analysis with sanitizers
- Security scan completed (v0.20.2): 7 findings remediated

### Incident Response

In case of a security incident:
1. Immediate patch development
2. Security advisory publication
3. Coordinated disclosure
4. Post-mortem analysis
5. Process improvement

## Contact

- Security Team Email: security@veridian-os.org
- PGP Key: https://veridian-os.org/security-key.asc
- Security Advisory Feed: https://veridian-os.org/security/advisories.atom

Thank you for helping keep VeridianOS secure!
