# Security Architecture

VeridianOS implements defense-in-depth with multiple layers of security, from hardware features to application sandboxing. This document describes the security architecture and threat model.

## Security Principles

### 1. Principle of Least Privilege
Every component runs with minimal required permissions:
- Microkernel has minimal privileged code
- Drivers run in user space
- Applications start with no capabilities

### 2. Complete Mediation
All resource access goes through capability checks:
- No bypass mechanisms
- No superuser/root
- Uniform security model

### 3. Fail Secure
Security failures result in denial of access:
- Invalid capabilities are rejected
- Errors don't grant permissions
- Safe defaults everywhere

### 4. Defense in Depth
Multiple security layers:
- Hardware security features
- Capability-based access control
- Mandatory access control
- Application sandboxing

## Threat Model

### In-Scope Threats

#### 1. Malicious Applications
- **Threat**: Applications attempting privilege escalation
- **Mitigation**: Capability confinement, no ambient authority
- **Example**: Application can't access files without file capability

#### 2. Compromised Drivers
- **Threat**: Driver exploits affecting system
- **Mitigation**: User-space isolation, IOMMU protection
- **Example**: Network driver compromise can't access disk

#### 3. Network Attacks
- **Threat**: Remote code execution, DoS
- **Mitigation**: Memory safety (Rust), rate limiting
- **Example**: Buffer overflows prevented by language

#### 4. Side-Channel Attacks
- **Threat**: Spectre, Meltdown variants
- **Mitigation**: Hardware mitigations, kernel page table isolation
- **Example**: Speculation barriers in critical paths

### Out-of-Scope Threats

1. **Hardware Attacks**: Physical access, hardware implants
2. **Supply Chain**: Compromised hardware/firmware
3. **Cryptanalysis**: Breaking cryptographic primitives

## Capability Security

### Capability Properties

```rust
pub struct Capability {
    // Unforgeable token
    cap_type: CapabilityType,
    object_id: ObjectId,
    rights: Rights,
    generation: u16,
    
    // Optional restrictions
    #[cfg(feature = "capability_expiry")]
    expiry: Option<Instant>,
    
    #[cfg(feature = "capability_quotas")]
    usage_quota: Option<u32>,
}
```

### Capability Isolation

Each process has a capability table:

```rust
pub struct Process {
    // Isolated capability space
    cap_table: CapabilityTable,
    
    // No global namespace access
    // No file paths, only capabilities
}

impl Process {
    pub fn access_file(&self, cap: FileCapability) -> Result<File, Error> {
        // Validate capability
        self.cap_table.validate(&cap)?;
        
        // Access granted only with valid capability
        File::open_with_cap(cap)
    }
}
```

### Capability Delegation

Safe capability sharing:

```rust
// Parent creates restricted capability
let read_only = file_cap.derive(Rights::READ)?;

// Pass to child process
child.grant_capability(read_only)?;

// Child cannot escalate privileges
// Cannot derive WRITE from READ
```

## Memory Protection

### Address Space Layout

```
Process Address Space:
┌─────────────────────┐ 0xFFFFFFFFFFFFFFFF
│ Kernel (Invisible)  │ Not mapped in user mode
├─────────────────────┤ 0x0000800000000000
│ Stack (Guard Page)  │ 
├─────────────────────┤
│ Heap (ASLR)        │ Randomized base
├─────────────────────┤
│ Libraries (ASLR)    │ Randomized
├─────────────────────┤
│ Code (ASLR)        │ W^X enforced
├─────────────────────┤
│ Guard Page         │ 
└─────────────────────┘ 0x0000000000000000
```

### Memory Safety Features

1. **W^X Enforcement**: Pages either writable or executable, never both
2. **ASLR**: Address space randomization
3. **Guard Pages**: Detect stack/heap overflows
4. **NX Bit**: Non-executable data pages
5. **SMEP/SMAP**: Supervisor mode execution/access prevention

## Hardware Security Integration

### Intel TDX Support

```rust
#[cfg(feature = "intel_tdx")]
pub mod tdx {
    pub fn create_td_guest() -> Result<TdGuest, Error> {
        // Create trusted domain
        let td = TdGuest::new()?;
        
        // Attestation
        let report = td.generate_attestation_report()?;
        
        Ok(td)
    }
}
```

### ARM CCA Support

```rust
#[cfg(feature = "arm_cca")]
pub mod cca {
    pub fn create_realm() -> Result<Realm, Error> {
        // Create confidential compute realm
        let realm = Realm::new()?;
        
        // Measurement and attestation
        let measurement = realm.measure()?;
        
        Ok(realm)
    }
}
```

## Secure Boot

### Boot Chain Verification

```
┌──────────────┐
│ Hardware RoT │ Immutable root of trust
└──────┬───────┘
       ↓ Measures
┌──────────────┐
│ UEFI Secure  │ Verifies signature
│    Boot      │
└──────┬───────┘
       ↓ Loads
┌──────────────┐
│ VeridianOS   │ Verifies kernel
│ Bootloader   │
└──────┬───────┘
       ↓ Loads
┌──────────────┐
│   Kernel     │ Verifies drivers
└──────────────┘
```

### TPM Integration

```rust
pub struct TpmSealing {
    pcr_mask: u32,
    policy: TpmPolicy,
}

impl TpmSealing {
    pub fn seal_key(&self, key: &[u8]) -> Result<SealedKey, Error> {
        // Seal to current PCR values
        let sealed = tpm2_seal(key, self.pcr_mask)?;
        Ok(sealed)
    }
    
    pub fn unseal_key(&self, sealed: &SealedKey) -> Result<Vec<u8>, Error> {
        // Only unseals if PCRs match
        tpm2_unseal(sealed, self.pcr_mask)
    }
}
```

## Cryptography

### Algorithm Selection

Post-quantum ready algorithms:

```rust
pub enum CryptoAlgorithm {
    // Classical
    AesGcm256,
    ChaCha20Poly1305,
    Sha3_256,
    
    // Post-quantum
    MlKem768,       // Key encapsulation
    MlDsa65,        // Digital signatures
    
    // Hybrid
    HybridKem(ClassicalKem, PostQuantumKem),
}
```

### Key Management

```rust
pub struct KeyManager {
    // Hardware key storage
    hsm: Option<HardwareSecurityModule>,
    
    // Software key storage
    keyring: EncryptedKeyring,
}

impl KeyManager {
    pub fn generate_key(&mut self, algorithm: CryptoAlgorithm) -> Result<KeyId, Error> {
        let key = match &self.hsm {
            Some(hsm) => hsm.generate_key(algorithm)?,
            None => software_generate_key(algorithm)?,
        };
        
        self.keyring.store(key)
    }
}
```

## Mandatory Access Control

### Security Contexts

```rust
pub struct SecurityContext {
    // Type enforcement
    domain: Domain,
    
    // Multi-level security
    level: SecurityLevel,
    categories: BitSet,
}

pub struct Domain {
    name: String,
    allowed_transitions: Vec<Domain>,
    allowed_operations: HashMap<ObjectType, Operations>,
}
```

### Policy Enforcement

```rust
pub fn check_access(
    subject: &SecurityContext,
    object: &SecurityContext,
    operation: Operation,
) -> Result<(), AccessDenied> {
    // Type enforcement
    if !subject.domain.allows_operation(&object.domain, operation) {
        return Err(AccessDenied::TypeEnforcement);
    }
    
    // MLS constraints
    match operation {
        Operation::Read => {
            // No read up
            if object.level > subject.level {
                return Err(AccessDenied::NoReadUp);
            }
        }
        Operation::Write => {
            // No write down
            if object.level < subject.level {
                return Err(AccessDenied::NoWriteDown);
            }
        }
    }
    
    Ok(())
}
```

## Audit System

### Security Events

```rust
pub enum SecurityEvent {
    // Authentication
    LoginAttempt { user: UserId, success: bool },
    
    // Authorization  
    CapabilityCheck { cap: Capability, result: bool },
    AccessDenied { subject: ProcessId, object: ObjectId },
    
    // Integrity
    FileModified { file: FileId, hash: Hash },
    
    // Accountability
    ProcessCreated { parent: ProcessId, child: ProcessId },
}
```

### Audit Trail

```rust
pub struct AuditLog {
    // Tamper-evident log
    entries: MerkleTree<AuditEntry>,
    
    // Real-time analysis
    analyzer: SecurityAnalyzer,
}

impl AuditLog {
    pub fn log_event(&mut self, event: SecurityEvent) {
        let entry = AuditEntry {
            timestamp: Instant::now(),
            event,
            subject: current_process(),
        };
        
        // Append to tamper-evident log
        self.entries.append(entry);
        
        // Real-time analysis
        if self.analyzer.is_suspicious(&entry) {
            self.raise_alert(entry);
        }
    }
}
```

## Sandboxing

### Application Isolation

```rust
pub struct Sandbox {
    // Capability whitelist
    allowed_caps: Vec<Capability>,
    
    // System call filter
    syscall_filter: SeccompFilter,
    
    // Resource limits
    limits: ResourceLimits,
}

impl Sandbox {
    pub fn execute(&self, binary: &[u8]) -> Result<Process, Error> {
        let process = Process::create()?;
        
        // Apply restrictions
        process.set_capabilities(&self.allowed_caps)?;
        process.set_syscall_filter(&self.syscall_filter)?;
        process.set_resource_limits(&self.limits)?;
        
        // Load and execute
        process.load(binary)?;
        process.start()
    }
}
```

## Security Best Practices

1. **Never trust user input**: Validate everything
2. **Fail closed**: Deny by default
3. **Minimize attack surface**: Disable unused features
4. **Log security events**: Enable forensics
5. **Regular updates**: Patch vulnerabilities quickly
6. **Security testing**: Fuzz, pen test, code review