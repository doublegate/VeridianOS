# Security Policy

## Supported Versions

VeridianOS is at v0.27.0, a research system that has not run on real hardware yet. Security fixes
go into the main branch and the next release; earlier releases are not patched.

| Version | Supported          |
| ------- | ------------------ |
| 0.27.x (latest) | :white_check_mark: |
| main branch | :white_check_mark: |
| < 0.27  | :x:                |

Once we reach 1.0, we will maintain security updates for the current major version and one previous major version.

## Reporting a Vulnerability

We take the security of VeridianOS seriously. If you believe you have found a security vulnerability, please report it to us as described below.

### Please do NOT:

- Open a public issue
- Post to public forums or social media
- Exploit the vulnerability

### Please DO:

1. Report it privately through GitHub: open the repository's **Security** tab and choose
   **Report a vulnerability** (<https://github.com/doublegate/VeridianOS/security/advisories/new>).
   Only the maintainers can see the report.
2. Include the following information:
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
- Minimize unsafe code (3 justified `static mut` remaining: early serial, boot info, per-CPU data)
- Document every safety invariant (every `unsafe` block carries a `// SAFETY:` comment)
- Fuzz parsers of untrusted input (planned with sprint G; see below)

## Security Features

What is enforced today and what exists only as code is listed below; open items carry the IDs used
in [`docs/KNOWN-LIMITATIONS.md`](docs/KNOWN-LIMITATIONS.md), which is the authoritative list.

1. **Capability-based access control**
   - Unforgeable 64-bit capability tokens with generation counters and O(1) lookup
   - Hierarchical delegation and cascading revocation; capabilities guard IPC, memory sharing and
     process creation, while files, sockets and devices use Unix permissions
   - Cascading revocation does not yet follow every grant path (N-89)

2. **Memory protection**
   - NX and W^X for user memory (`mmap`/`mprotect`; W+X only for programs with the opt-in, ADR 0017)
   - A W^X kernel image and guarded kernel stacks on all three architectures
   - Checked, fault-tolerant user-memory access; BTI for AArch64 user programs
   - KPTI, KASLR, stack canaries, SMEP/SMAP and retpoline exist as code but are not active
     (N-145, N-146, N-15)

3. **Cryptography**
   - ChaCha20-Poly1305, Ed25519, X25519, SHA-256
   - ML-DSA-65 signature verification per FIPS 204, checked against NIST ACVP vectors; no trusted
     post-quantum package key is provisioned yet, so a policy that requires one rejects every package
   - An experimental, non-standard Kyber KEM (N-57), not ML-KEM
   - RDRAND-backed CSPRNG where CPUID reports it

4. **Access control and audit**
   - Credentials with real, effective, saved and filesystem IDs, supplementary groups and Linux's
     permission rules; RLIMIT_* limits
   - Salted password hashes compared in constant time, with a password history (as
     pam_pwhistory's `remember`)
   - Multi-factor authentication is not usable yet: TOTP does not follow RFC 6238, and pam_veridian
     cannot check a second factor, so an account with MFA enabled fails every PAM login. In
     `/etc/veridian/mfa`, a line that names an account but has a bad secret leaves that account
     needing a factor no token gives, and an unreadable file does that for every account; a line
     that names no account (no `:`) is only reported as malformed and protects no account.
   - A MAC policy parser with an RBAC/MLS model and audit logging; enforcement is path-blind and
     partial (N-152)

5. **Hardware security**
   - TPM 2.0 and secure-boot verification code
   - IOMMU tables are parsed, but DMA isolation is not enabled (N-77)

6. **Network security**
   - Firewall code with conntrack and NAT, TLS 1.3, SSH and WireGuard implementations
   - User-space INET sockets do not work yet (N-64 to N-76); loopback and Unix sockets do

## Development Security

### Threat Model

Our threat model considers:
- Malicious applications
- Network attackers
- Physical access attacks
- Supply chain attacks
- Side-channel attacks

### Security Testing

- Static analysis: clippy with `-D warnings` on every target, and `cargo audit` (RustSec) in CI
- Host unit tests, QEMU boot tests and in-guest runtime suites on every change
  (`scripts/dev/gate.sh --boot --rootfs`)
- An in-kernel mutation fuzzer (`kernel/src/security/fuzzing.rs`); coverage-guided fuzzing of the
  parsers is planned with sprint G
- Audits: `docs/audit/` tracks every finding (N-xx) with its status

### Incident Response

In case of a security incident:
1. Immediate patch development
2. Security advisory publication
3. Coordinated disclosure
4. Post-mortem analysis
5. Process improvement

## Contact

- Private reports: <https://github.com/doublegate/VeridianOS/security/advisories/new>
- Published advisories: <https://github.com/doublegate/VeridianOS/security/advisories>

Thank you for helping keep VeridianOS secure!
