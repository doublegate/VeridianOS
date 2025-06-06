# Phase 3: Security Hardening (Months 16-21)

## Overview

Phase 3 focuses on comprehensive security hardening of VeridianOS, implementing defense-in-depth strategies, mandatory access controls, secure boot, and advanced security features. This phase transforms the functional OS into a security-focused system suitable for high-assurance environments.

## Objectives

1. **Mandatory Access Control**: SELinux-style policy enforcement
2. **Secure Boot**: Full chain of trust from firmware to applications
3. **Cryptographic Services**: System-wide encryption and key management
4. **Security Monitoring**: Audit system and intrusion detection
5. **Sandboxing**: Application isolation and containment
6. **Hardware Security**: Integration with TPM, HSM, and TEE

## Architecture Components

### 1. Mandatory Access Control (MAC)

#### 1.1 Security Server

**services/security/src/main.rs**
```rust
use alloc::collections::{BTreeMap, BTreeSet};
use veridian_abi::{SecurityContext, SecurityId, Permission};

/// Security policy engine
pub struct SecurityServer {
    /// Security policy
    policy: Policy,
    /// Subject security contexts
    subjects: BTreeMap<SubjectId, SecurityContext>,
    /// Object security contexts
    objects: BTreeMap<ObjectId, SecurityContext>,
    /// Access vector cache
    avc: AccessVectorCache,
    /// Audit subsystem
    audit: AuditSystem,
    /// Policy version
    policy_version: u64,
}

/// Security policy
struct Policy {
    /// Type enforcement rules
    type_enforcement: TypeEnforcement,
    /// Role-based access control
    rbac: RoleBasedAccessControl,
    /// Multi-level security
    mls: MultiLevelSecurity,
    /// Constraints
    constraints: Vec<Constraint>,
}

/// Type enforcement
struct TypeEnforcement {
    /// Type definitions
    types: BTreeMap<String, TypeId>,
    /// Type attributes
    attributes: BTreeMap<String, BTreeSet<TypeId>>,
    /// Access vector rules
    rules: Vec<TERule>,
}

#[derive(Debug, Clone)]
struct TERule {
    /// Source type/attribute
    source: TypeSpec,
    /// Target type/attribute
    target: TypeSpec,
    /// Object class
    class: ObjectClass,
    /// Permissions
    permissions: Permissions,
    /// Rule type (allow, deny, auditallow, dontaudit)
    rule_type: RuleType,
}

impl SecurityServer {
    /// Check access permission
    pub fn check_access(
        &mut self,
        subject: SubjectId,
        object: ObjectId,
        class: ObjectClass,
        permission: Permission,
    ) -> Result<(), SecurityError> {
        // Get security contexts
        let subject_ctx = self.subjects.get(&subject)
            .ok_or(SecurityError::InvalidSubject)?;
        let object_ctx = self.objects.get(&object)
            .ok_or(SecurityError::InvalidObject)?;
        
        // Check cache first
        let cache_key = AVCKey {
            source_type: subject_ctx.type_id,
            target_type: object_ctx.type_id,
            class,
            permission,
        };
        
        if let Some(decision) = self.avc.lookup(&cache_key) {
            return self.enforce_decision(decision, subject, object, permission);
        }
        
        // Compute access decision
        let decision = self.compute_access_decision(
            subject_ctx,
            object_ctx,
            class,
            permission,
        )?;
        
        // Cache decision
        self.avc.insert(cache_key, decision);
        
        // Enforce decision
        self.enforce_decision(decision, subject, object, permission)
    }
    
    /// Compute access decision
    fn compute_access_decision(
        &self,
        subject_ctx: &SecurityContext,
        object_ctx: &SecurityContext,
        class: ObjectClass,
        permission: Permission,
    ) -> Result<AccessDecision, SecurityError> {
        let mut decision = AccessDecision::default();
        
        // Check type enforcement
        if let Some(te_result) = self.check_type_enforcement(
            subject_ctx.type_id,
            object_ctx.type_id,
            class,
            permission,
        ) {
            decision.allowed = te_result.allowed;
            decision.audit = te_result.audit;
        }
        
        // Check role-based access control
        if decision.allowed {
            decision.allowed = self.check_rbac(
                subject_ctx.role,
                object_ctx.type_id,
                class,
                permission,
            )?;
        }
        
        // Check multi-level security
        if decision.allowed {
            decision.allowed = self.check_mls(
                &subject_ctx.mls_range,
                &object_ctx.mls_level,
                class,
                permission,
            )?;
        }
        
        // Check constraints
        if decision.allowed {
            for constraint in &self.policy.constraints {
                if !self.evaluate_constraint(constraint, subject_ctx, object_ctx)? {
                    decision.allowed = false;
                    decision.constraint_violation = Some(constraint.name.clone());
                    break;
                }
            }
        }
        
        Ok(decision)
    }
    
    /// Transition security context
    pub fn transition_context(
        &mut self,
        old_ctx: &SecurityContext,
        new_type: TypeId,
        exec_type: Option<TypeId>,
    ) -> Result<SecurityContext, SecurityError> {
        // Check for type transition rule
        if let Some(exec_type) = exec_type {
            for rule in &self.policy.type_enforcement.transitions {
                if rule.matches(old_ctx.type_id, exec_type) {
                    return Ok(SecurityContext {
                        user: old_ctx.user,
                        role: self.transition_role(old_ctx.role, new_type)?,
                        type_id: rule.new_type,
                        mls_range: old_ctx.mls_range.clone(),
                    });
                }
            }
        }
        
        // Default transition
        Ok(SecurityContext {
            user: old_ctx.user,
            role: old_ctx.role,
            type_id: new_type,
            mls_range: old_ctx.mls_range.clone(),
        })
    }
    
    /// Label new object
    pub fn label_object(
        &mut self,
        creator_ctx: &SecurityContext,
        parent_ctx: Option<&SecurityContext>,
        class: ObjectClass,
    ) -> Result<SecurityContext, SecurityError> {
        // Check for type transition on object creation
        let target_type = parent_ctx
            .map(|p| p.type_id)
            .unwrap_or(creator_ctx.type_id);
            
        for rule in &self.policy.type_enforcement.type_transitions {
            if rule.matches_create(creator_ctx.type_id, target_type, class) {
                return Ok(SecurityContext {
                    user: creator_ctx.user,
                    role: ObjectRole::Object,
                    type_id: rule.new_type,
                    mls_range: MlsRange::single(creator_ctx.mls_range.low()),
                });
            }
        }
        
        // Inherit from parent or creator
        if let Some(parent) = parent_ctx {
            Ok(parent.clone())
        } else {
            Ok(SecurityContext {
                user: creator_ctx.user,
                role: ObjectRole::Object,
                type_id: creator_ctx.type_id,
                mls_range: MlsRange::single(creator_ctx.mls_range.low()),
            })
        }
    }
}

/// Access Vector Cache
struct AccessVectorCache {
    /// Cache entries
    cache: BTreeMap<AVCKey, AccessDecision>,
    /// Maximum cache size
    max_size: usize,
    /// Statistics
    stats: AVCStats,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AVCKey {
    source_type: TypeId,
    target_type: TypeId,
    class: ObjectClass,
    permission: Permission,
}

#[derive(Debug, Clone, Default)]
struct AccessDecision {
    allowed: bool,
    audit: bool,
    constraint_violation: Option<String>,
}

/// Multi-Level Security
#[derive(Debug, Clone)]
struct MlsLevel {
    sensitivity: SensitivityLevel,
    categories: BTreeSet<Category>,
}

#[derive(Debug, Clone)]
struct MlsRange {
    low: MlsLevel,
    high: MlsLevel,
}

impl MlsRange {
    /// Check dominance for read (subject must dominate object)
    fn dominates(&self, other: &MlsLevel) -> bool {
        self.low.sensitivity >= other.sensitivity &&
        self.low.categories.is_superset(&other.categories)
    }
    
    /// Check write permission (subject range must contain object level)
    fn contains(&self, other: &MlsLevel) -> bool {
        other.sensitivity >= self.low.sensitivity &&
        other.sensitivity <= self.high.sensitivity &&
        other.categories.is_subset(&self.high.categories) &&
        other.categories.is_superset(&self.low.categories)
    }
}
```

#### 1.2 Policy Compiler

**tools/sepolicy/src/compiler.rs**
```rust
use std::collections::{HashMap, HashSet};

/// SELinux-style policy language compiler
pub struct PolicyCompiler {
    /// Type definitions
    types: HashMap<String, TypeDef>,
    /// Attributes
    attributes: HashMap<String, HashSet<String>>,
    /// Rules
    rules: Vec<ParsedRule>,
    /// Roles
    roles: HashMap<String, RoleDef>,
    /// Users
    users: HashMap<String, UserDef>,
}

impl PolicyCompiler {
    /// Compile policy from source
    pub fn compile(source: &str) -> Result<CompiledPolicy, CompileError> {
        let mut compiler = Self::new();
        
        // Parse policy
        compiler.parse(source)?;
        
        // Validate policy
        compiler.validate()?;
        
        // Generate binary policy
        compiler.generate()
    }
    
    /// Parse policy source
    fn parse(&mut self, source: &str) -> Result<(), CompileError> {
        let tokens = tokenize(source)?;
        let mut parser = Parser::new(tokens);
        
        while !parser.is_at_end() {
            match parser.peek()? {
                Token::Type => self.parse_type(&mut parser)?,
                Token::Attribute => self.parse_attribute(&mut parser)?,
                Token::TypeAttribute => self.parse_type_attribute(&mut parser)?,
                Token::Allow => self.parse_allow_rule(&mut parser)?,
                Token::TypeTransition => self.parse_type_transition(&mut parser)?,
                Token::Role => self.parse_role(&mut parser)?,
                Token::User => self.parse_user(&mut parser)?,
                Token::Constrain => self.parse_constraint(&mut parser)?,
                _ => return Err(CompileError::UnexpectedToken),
            }
        }
        
        Ok(())
    }
    
    /// Parse type definition
    /// Example: type init_t;
    fn parse_type(&mut self, parser: &mut Parser) -> Result<(), CompileError> {
        parser.expect(Token::Type)?;
        let name = parser.expect_identifier()?;
        
        let mut attributes = Vec::new();
        if parser.match_token(Token::Comma) {
            attributes = parser.parse_list()?;
        }
        
        parser.expect(Token::Semicolon)?;
        
        self.types.insert(name.clone(), TypeDef {
            name: name.clone(),
            attributes,
        });
        
        Ok(())
    }
    
    /// Parse allow rule
    /// Example: allow init_t self:process { fork sigchld };
    fn parse_allow_rule(&mut self, parser: &mut Parser) -> Result<(), CompileError> {
        parser.expect(Token::Allow)?;
        
        let source = parser.parse_type_spec()?;
        let target = parser.parse_type_spec()?;
        
        parser.expect(Token::Colon)?;
        let class = parser.expect_identifier()?;
        
        let permissions = parser.parse_permission_set()?;
        parser.expect(Token::Semicolon)?;
        
        self.rules.push(ParsedRule::Allow {
            source,
            target,
            class,
            permissions,
        });
        
        Ok(())
    }
    
    /// Generate binary policy
    fn generate(&self) -> Result<CompiledPolicy, CompileError> {
        let mut policy = CompiledPolicy::new();
        
        // Assign type IDs
        let mut type_map = HashMap::new();
        let mut next_type_id = 1;
        
        for type_name in self.types.keys() {
            type_map.insert(type_name.clone(), TypeId(next_type_id));
            next_type_id += 1;
        }
        
        // Compile type enforcement rules
        for rule in &self.rules {
            match rule {
                ParsedRule::Allow { source, target, class, permissions } => {
                    let compiled_rule = TERule {
                        source: self.compile_type_spec(source, &type_map)?,
                        target: self.compile_type_spec(target, &type_map)?,
                        class: self.compile_class(class)?,
                        permissions: self.compile_permissions(permissions)?,
                        rule_type: RuleType::Allow,
                    };
                    policy.te_rules.push(compiled_rule);
                }
                _ => {}
            }
        }
        
        Ok(policy)
    }
}

/// Example policy file
const EXAMPLE_POLICY: &str = r#"
# Core system types
type kernel_t;
type init_t;
type init_exec_t;
type user_t;
type user_home_t;
type device_t;
type console_device_t, device_t;

# Attributes
attribute domain;
attribute file_type;
attribute exec_type;

# Type attributes
typeattribute init_t domain;
typeattribute init_exec_t exec_type, file_type;
typeattribute user_t domain;

# Allow rules
allow init_t self:process { fork sigchld };
allow init_t init_exec_t:file { read execute };
allow init_t console_device_t:chr_file { read write };
allow init_t user_t:process transition;

# Type transitions
type_transition init_t init_exec_t:process init_t;
type_transition init_t user_exec_t:process user_t;

# Roles
role system_r;
role user_r;

role system_r types { init_t kernel_t };
role user_r types { user_t };

# Users
user system_u roles { system_r } level s0 range s0-s15:c0.c1023;
user user_u roles { user_r } level s0 range s0;

# Constraints
constrain process transition (u1 == u2 or t1 == can_change_user);
"#;
```

### 2. Secure Boot

#### 2.1 Boot Chain Verification

**bootloader/src/secure_boot.rs**
```rust
use uefi::{Handle, Status, proto::loaded_image::LoadedImage};
use x509_cert::{Certificate, TbsCertificate};
use rsa::{RsaPublicKey, PaddingScheme, PublicKey};
use sha2::{Sha256, Digest};

/// Secure boot manager
pub struct SecureBootManager {
    /// UEFI secure boot database
    db: SecureBootDb,
    /// Forbidden signatures database
    dbx: SecureBootDbx,
    /// Key enrollment key
    kek: Vec<Certificate>,
    /// Platform key
    pk: Option<Certificate>,
    /// Measurement log for TPM
    measurements: Vec<Measurement>,
}

/// Boot measurement for TPM PCR extension
struct Measurement {
    pcr_index: u8,
    digest: [u8; 32],
    description: String,
}

impl SecureBootManager {
    /// Verify and load kernel
    pub fn load_kernel(
        &mut self,
        kernel_path: &str,
        initrd_path: Option<&str>,
    ) -> Result<LoadedKernel, SecureBootError> {
        // Read kernel image
        let kernel_data = self.read_file(kernel_path)?;
        
        // Verify kernel signature
        let signature = self.extract_signature(&kernel_data)?;
        self.verify_signature(&kernel_data, &signature)?;
        
        // Measure kernel
        let kernel_hash = self.measure_component(&kernel_data, "kernel");
        self.extend_pcr(PCR_KERNEL, &kernel_hash)?;
        
        // Load and verify initrd if provided
        let initrd_data = if let Some(path) = initrd_path {
            let data = self.read_file(path)?;
            let sig = self.extract_signature(&data)?;
            self.verify_signature(&data, &sig)?;
            
            let initrd_hash = self.measure_component(&data, "initrd");
            self.extend_pcr(PCR_INITRD, &initrd_hash)?;
            
            Some(data)
        } else {
            None
        };
        
        // Verify kernel headers
        let kernel_header = self.parse_kernel_header(&kernel_data)?;
        if !kernel_header.is_valid() {
            return Err(SecureBootError::InvalidKernelFormat);
        }
        
        // Lock down boot variables
        self.lock_boot_variables()?;
        
        Ok(LoadedKernel {
            entry_point: kernel_header.entry_point,
            kernel_data,
            initrd_data,
            command_line: self.get_verified_cmdline()?,
        })
    }
    
    /// Verify signature against secure boot database
    fn verify_signature(
        &self,
        data: &[u8],
        signature: &Signature,
    ) -> Result<(), SecureBootError> {
        // Check if signature is in forbidden database
        if self.dbx.contains(signature) {
            return Err(SecureBootError::ForbiddenSignature);
        }
        
        // Calculate data hash
        let mut hasher = Sha256::new();
        hasher.update(data);
        let digest = hasher.finalize();
        
        // Find matching certificate in DB
        let cert = self.db.find_cert_for_signature(signature)
            .ok_or(SecureBootError::NoMatchingCertificate)?;
        
        // Verify certificate chain
        self.verify_cert_chain(&cert)?;
        
        // Extract public key and verify signature
        let public_key = self.extract_public_key(&cert)?;
        public_key.verify(
            PaddingScheme::new_pkcs1v15_sign(Some(Hash::SHA256)),
            &digest,
            &signature.data,
        )?;
        
        Ok(())
    }
    
    /// Extend TPM PCR with measurement
    fn extend_pcr(&mut self, pcr: u8, digest: &[u8]) -> Result<(), SecureBootError> {
        // Get TPM protocol
        let tpm = self.get_tpm_protocol()?;
        
        // Extend PCR
        tpm.pcr_extend(pcr, digest)?;
        
        // Log measurement
        self.measurements.push(Measurement {
            pcr_index: pcr,
            digest: digest.try_into()?,
            description: format!("PCR{} extended", pcr),
        });
        
        Ok(())
    }
    
    /// Lock boot variables to prevent runtime modification
    fn lock_boot_variables(&mut self) -> Result<(), SecureBootError> {
        // Set BootNext and BootOrder to read-only
        self.set_variable_lock("BootNext")?;
        self.set_variable_lock("BootOrder")?;
        
        // Lock secure boot databases
        self.set_variable_lock("db")?;
        self.set_variable_lock("dbx")?;
        self.set_variable_lock("KEK")?;
        self.set_variable_lock("PK")?;
        
        Ok(())
    }
}

/// Kernel secure boot header
#[repr(C)]
struct SecureKernelHeader {
    magic: [u8; 8],
    version: u32,
    header_size: u32,
    signature_offset: u32,
    signature_size: u32,
    certificate_offset: u32,
    certificate_size: u32,
    entry_point: u64,
    load_address: u64,
    kernel_size: u64,
    flags: u32,
    reserved: [u32; 8],
}

impl SecureKernelHeader {
    const MAGIC: &'static [u8; 8] = b"VERIDIAN";
    
    fn is_valid(&self) -> bool {
        self.magic == Self::MAGIC &&
        self.version >= MIN_SUPPORTED_VERSION &&
        self.header_size >= size_of::<Self>() as u32
    }
}

/// Platform Configuration Registers
const PCR_PLATFORM: u8 = 0;  // Platform firmware
const PCR_CONFIG: u8 = 1;    // Platform configuration  
const PCR_KERNEL: u8 = 4;    // Kernel
const PCR_INITRD: u8 = 5;    // Initial ramdisk
const PCR_CMDLINE: u8 = 8;   // Command line
```

#### 2.2 Verified Boot Policy

**bootloader/src/policy.rs**
```rust
/// Verified boot policy engine
pub struct BootPolicy {
    /// Minimum security version
    min_security_version: u32,
    /// Required capabilities
    required_capabilities: BootCapabilities,
    /// Trusted boot configurations
    trusted_configs: Vec<TrustedConfig>,
    /// Rollback protection
    rollback_index: u64,
}

bitflags! {
    struct BootCapabilities: u32 {
        const SECURE_BOOT = 1 << 0;
        const MEASURED_BOOT = 1 << 1;
        const ENCRYPTED_STORAGE = 1 << 2;
        const ATTESTATION = 1 << 3;
        const ROLLBACK_PROTECTION = 1 << 4;
    }
}

impl BootPolicy {
    /// Evaluate boot policy
    pub fn evaluate(
        &self,
        measurements: &[Measurement],
        capabilities: BootCapabilities,
    ) -> Result<BootDecision, PolicyError> {
        // Check minimum capabilities
        if !capabilities.contains(self.required_capabilities) {
            return Err(PolicyError::InsufficientCapabilities);
        }
        
        // Verify measurements against trusted configurations
        let config_match = self.trusted_configs.iter()
            .any(|config| config.matches_measurements(measurements));
            
        if !config_match {
            return Err(PolicyError::UntrustedConfiguration);
        }
        
        // Check rollback protection
        let current_version = self.extract_version(measurements)?;
        if current_version < self.rollback_index {
            return Err(PolicyError::RollbackAttempt);
        }
        
        Ok(BootDecision {
            allowed: true,
            attestation_quote: self.generate_attestation(measurements)?,
            sealed_keys: self.unseal_keys(measurements)?,
        })
    }
}
```

### 3. Cryptographic Services

#### 3.1 Key Management Service

**services/crypto/src/kms.rs**
```rust
use ring::{aead, rand, signature};
use zeroize::Zeroize;

/// Key management service
pub struct KeyManagementService {
    /// Master key encryption key (in TPM)
    master_kek: TpmHandle,
    /// Key hierarchy
    key_hierarchy: KeyHierarchy,
    /// Key store
    key_store: EncryptedKeyStore,
    /// HSM interface (if available)
    hsm: Option<HsmInterface>,
    /// Audit log
    audit: AuditLog,
}

/// Key hierarchy for derivation
struct KeyHierarchy {
    /// Root key (in TPM/HSM)
    root: KeyHandle,
    /// Domain keys
    domains: BTreeMap<DomainId, DomainKey>,
    /// Service keys
    services: BTreeMap<ServiceId, ServiceKey>,
}

/// Encrypted key storage
struct EncryptedKeyStore {
    /// Storage backend
    backend: Box<dyn KeyStorage>,
    /// Encryption algorithm
    aead: aead::Algorithm,
    /// Key cache
    cache: LruCache<KeyId, CachedKey>,
}

#[derive(Zeroize)]
#[zeroize(drop)]
struct CachedKey {
    key_material: Vec<u8>,
    attributes: KeyAttributes,
    expiry: Instant,
}

impl KeyManagementService {
    /// Generate new key
    pub fn generate_key(
        &mut self,
        request: KeyGenRequest,
    ) -> Result<KeyHandle, KmsError> {
        // Validate request
        self.validate_key_request(&request)?;
        
        // Check caller permissions
        let caller = self.get_caller_context()?;
        self.check_permission(&caller, Permission::GenerateKey)?;
        
        // Generate key material
        let key_material = match request.algorithm {
            KeyAlgorithm::Aes256 => self.generate_aes_key(256)?,
            KeyAlgorithm::RsA4096 => self.generate_rsa_key(4096)?,
            KeyAlgorithm::EcdsaP256 => self.generate_ecdsa_key(Curve::P256)?,
            KeyAlgorithm::Ed25519 => self.generate_ed25519_key()?,
        };
        
        // Derive wrapping key
        let wrap_key = self.derive_wrapping_key(&request.domain)?;
        
        // Wrap key material
        let wrapped_key = self.wrap_key(&key_material, &wrap_key)?;
        
        // Store wrapped key
        let key_id = self.key_store.store(wrapped_key, request.attributes)?;
        
        // Audit
        self.audit.log_key_generation(caller, key_id, request)?;
        
        // Clear sensitive material
        key_material.zeroize();
        
        Ok(KeyHandle { id: key_id, version: 1 })
    }
    
    /// Use key for cryptographic operation
    pub fn use_key(
        &mut self,
        handle: KeyHandle,
        operation: CryptoOperation,
        data: &[u8],
    ) -> Result<Vec<u8>, KmsError> {
        // Get caller context
        let caller = self.get_caller_context()?;
        
        // Check permissions
        let required_perm = match operation {
            CryptoOperation::Encrypt => Permission::Encrypt,
            CryptoOperation::Decrypt => Permission::Decrypt,
            CryptoOperation::Sign => Permission::Sign,
            CryptoOperation::Verify => Permission::Verify,
        };
        self.check_key_permission(&caller, handle, required_perm)?;
        
        // Get key from cache or storage
        let key = self.get_key(handle)?;
        
        // Perform operation
        let result = match operation {
            CryptoOperation::Encrypt => {
                self.encrypt_with_key(&key, data)?
            }
            CryptoOperation::Decrypt => {
                self.decrypt_with_key(&key, data)?
            }
            CryptoOperation::Sign => {
                self.sign_with_key(&key, data)?
            }
            CryptoOperation::Verify => {
                // Verify operations return boolean as vec
                let valid = self.verify_with_key(&key, data)?;
                vec![valid as u8]
            }
        };
        
        // Update key usage statistics
        self.update_key_usage(handle)?;
        
        // Audit
        self.audit.log_key_usage(caller, handle, operation)?;
        
        Ok(result)
    }
    
    /// Seal data to platform state
    pub fn seal_to_pcr(
        &mut self,
        data: &[u8],
        pcr_policy: PcrPolicy,
    ) -> Result<SealedData, KmsError> {
        // Get TPM handle
        let tpm = self.get_tpm()?;
        
        // Create policy session
        let policy_session = tpm.start_policy_session()?;
        
        // Set PCR policy
        tpm.policy_pcr(&policy_session, &pcr_policy)?;
        
        // Seal data
        let sealed = tpm.seal(
            self.master_kek,
            data,
            &policy_session,
        )?;
        
        Ok(SealedData {
            blob: sealed,
            policy: pcr_policy,
        })
    }
    
    /// Unseal data with platform state
    pub fn unseal_from_pcr(
        &mut self,
        sealed: &SealedData,
    ) -> Result<Vec<u8>, KmsError> {
        // Get TPM handle
        let tpm = self.get_tpm()?;
        
        // Create policy session
        let policy_session = tpm.start_policy_session()?;
        
        // Set PCR policy (will fail if PCRs don't match)
        tpm.policy_pcr(&policy_session, &sealed.policy)?;
        
        // Unseal data
        let data = tpm.unseal(
            self.master_kek,
            &sealed.blob,
            &policy_session,
        )?;
        
        Ok(data)
    }
    
    /// Derive key using KDF
    fn derive_key(
        &self,
        master: &[u8],
        context: &[u8],
        length: usize,
    ) -> Result<Vec<u8>, KmsError> {
        use hkdf::Hkdf;
        use sha2::Sha256;
        
        let hkdf = Hkdf::<Sha256>::new(None, master);
        let mut derived = vec![0u8; length];
        
        hkdf.expand(context, &mut derived)
            .map_err(|_| KmsError::DerivationFailed)?;
            
        Ok(derived)
    }
}

/// Post-quantum key exchange
pub struct PostQuantumKeyExchange {
    /// ML-KEM instance
    mlkem: MlKem,
    /// Classic key exchange for hybrid
    classic: X25519,
}

impl PostQuantumKeyExchange {
    /// Generate key pair
    pub fn generate_keypair(&mut self) -> Result<(PublicKey, PrivateKey), Error> {
        // Generate ML-KEM key pair
        let (mlkem_pk, mlkem_sk) = self.mlkem.generate_keypair()?;
        
        // Generate X25519 key pair for hybrid
        let (x25519_pk, x25519_sk) = self.classic.generate_keypair()?;
        
        // Combine into hybrid keys
        let public_key = PublicKey::Hybrid {
            mlkem: mlkem_pk,
            classic: x25519_pk,
        };
        
        let private_key = PrivateKey::Hybrid {
            mlkem: mlkem_sk,
            classic: x25519_sk,
        };
        
        Ok((public_key, private_key))
    }
    
    /// Encapsulate shared secret
    pub fn encapsulate(
        &self,
        public_key: &PublicKey,
    ) -> Result<(SharedSecret, Ciphertext), Error> {
        match public_key {
            PublicKey::Hybrid { mlkem, classic } => {
                // ML-KEM encapsulation
                let (mlkem_ss, mlkem_ct) = self.mlkem.encapsulate(mlkem)?;
                
                // X25519 key exchange
                let (x25519_ss, x25519_pk) = self.classic.generate_and_exchange(classic)?;
                
                // Combine shared secrets
                let combined_ss = self.combine_shared_secrets(&mlkem_ss, &x25519_ss)?;
                
                let ciphertext = Ciphertext::Hybrid {
                    mlkem: mlkem_ct,
                    classic: x25519_pk,
                };
                
                Ok((combined_ss, ciphertext))
            }
            _ => Err(Error::UnsupportedKeyType),
        }
    }
}
```

### 4. Security Monitoring

#### 4.1 Audit System

**services/audit/src/main.rs**
```rust
/// Security audit daemon
pub struct AuditDaemon {
    /// Audit rules
    rules: AuditRules,
    /// Event queue
    event_queue: RingBuffer<AuditEvent>,
    /// Log writers
    log_writers: Vec<Box<dyn LogWriter>>,
    /// Alert handlers
    alert_handlers: Vec<Box<dyn AlertHandler>>,
    /// Statistics
    stats: AuditStats,
}

/// Audit event
#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    /// Event ID
    pub id: u64,
    /// Timestamp
    pub timestamp: u64,
    /// Event type
    pub event_type: AuditEventType,
    /// Subject (who)
    pub subject: Subject,
    /// Object (what)
    pub object: Option<Object>,
    /// Action
    pub action: Action,
    /// Result
    pub result: ActionResult,
    /// Additional fields
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub enum AuditEventType {
    SystemCall,
    FileAccess,
    NetworkConnection,
    ProcessExecution,
    Authentication,
    Authorization,
    ConfigChange,
    AnomalyDetected,
}

impl AuditDaemon {
    /// Process kernel audit event
    pub fn process_event(&mut self, event: RawAuditEvent) {
        // Parse raw event
        let audit_event = match self.parse_event(event) {
            Ok(event) => event,
            Err(e) => {
                self.stats.parse_errors += 1;
                return;
            }
        };
        
        // Check audit rules
        if !self.should_log(&audit_event) {
            self.stats.filtered += 1;
            return;
        }
        
        // Detect anomalies
        if let Some(anomaly) = self.detect_anomaly(&audit_event) {
            self.handle_anomaly(anomaly);
        }
        
        // Add to event queue
        self.event_queue.push(audit_event.clone());
        
        // Write to logs
        for writer in &mut self.log_writers {
            if let Err(e) = writer.write(&audit_event) {
                eprintln!("Failed to write audit log: {}", e);
            }
        }
        
        // Check for alerts
        if let Some(alert) = self.check_alerts(&audit_event) {
            self.trigger_alert(alert);
        }
        
        self.stats.processed += 1;
    }
    
    /// Anomaly detection using ML
    fn detect_anomaly(&self, event: &AuditEvent) -> Option<Anomaly> {
        // Extract features
        let features = self.extract_features(event);
        
        // Run through anomaly detection model
        let score = self.anomaly_model.score(&features);
        
        if score > ANOMALY_THRESHOLD {
            Some(Anomaly {
                event_id: event.id,
                score,
                anomaly_type: self.classify_anomaly(&features),
                description: self.describe_anomaly(event, score),
            })
        } else {
            None
        }
    }
    
    /// Real-time alerting
    fn trigger_alert(&mut self, alert: Alert) {
        // Log alert
        self.log_alert(&alert);
        
        // Execute alert handlers
        for handler in &mut self.alert_handlers {
            if handler.should_handle(&alert) {
                if let Err(e) = handler.handle(alert.clone()) {
                    eprintln!("Alert handler failed: {}", e);
                }
            }
        }
        
        // Update statistics
        self.stats.alerts_triggered += 1;
    }
}

/// Intrusion detection system
pub struct IntrusionDetection {
    /// Network IDS
    network_ids: NetworkIDS,
    /// Host IDS
    host_ids: HostIDS,
    /// Correlation engine
    correlation: CorrelationEngine,
    /// Threat intelligence
    threat_intel: ThreatIntelligence,
}

impl IntrusionDetection {
    /// Analyze network traffic
    pub fn analyze_network(&mut self, packet: &NetworkPacket) {
        // Check against signatures
        if let Some(signature) = self.network_ids.match_signature(packet) {
            self.handle_network_alert(signature, packet);
        }
        
        // Behavioral analysis
        if let Some(anomaly) = self.network_ids.detect_behavior_anomaly(packet) {
            self.handle_network_anomaly(anomaly, packet);
        }
        
        // Update connection tracking
        self.network_ids.update_connection_state(packet);
    }
    
    /// Analyze system behavior
    pub fn analyze_host(&mut self, event: &SystemEvent) {
        // File integrity monitoring
        if let SystemEvent::FileModified { path, hash, .. } = event {
            if self.host_ids.is_monitored_file(path) {
                if !self.host_ids.verify_file_integrity(path, hash) {
                    self.handle_integrity_violation(path);
                }
            }
        }
        
        // Process behavior monitoring
        if let SystemEvent::ProcessCreated { pid, path, .. } = event {
            if let Some(threat) = self.host_ids.analyze_process_behavior(pid, path) {
                self.handle_process_threat(threat);
            }
        }
        
        // Correlate events
        self.correlation.add_event(event);
        if let Some(incident) = self.correlation.detect_incident() {
            self.handle_incident(incident);
        }
    }
}
```

### 5. Application Sandboxing

#### 5.1 Container Runtime

**services/container/src/runtime.rs**
```rust
/// Secure container runtime
pub struct ContainerRuntime {
    /// Container instances
    containers: BTreeMap<ContainerId, Container>,
    /// Security policies
    policies: SecurityPolicies,
    /// Resource limits
    cgroups: CgroupManager,
    /// Network isolation
    network: NetworkNamespace,
    /// Storage
    storage: ContainerStorage,
}

/// Container instance
struct Container {
    id: ContainerId,
    config: ContainerConfig,
    rootfs: PathBuf,
    namespaces: Namespaces,
    capabilities: CapabilitySet,
    seccomp: SeccompFilter,
    apparmor_profile: Option<String>,
    state: ContainerState,
}

/// Container namespaces
struct Namespaces {
    pid: NamespaceHandle,
    net: NamespaceHandle,
    mnt: NamespaceHandle,
    ipc: NamespaceHandle,
    uts: NamespaceHandle,
    user: NamespaceHandle,
}

impl ContainerRuntime {
    /// Create new container
    pub fn create_container(
        &mut self,
        config: ContainerConfig,
    ) -> Result<ContainerId, Error> {
        // Validate configuration
        self.validate_config(&config)?;
        
        // Check security policy
        self.policies.check_container_creation(&config)?;
        
        // Set up root filesystem
        let rootfs = self.storage.prepare_rootfs(&config.image)?;
        
        // Create namespaces
        let namespaces = self.create_namespaces(&config)?;
        
        // Set up cgroups
        let cgroup = self.cgroups.create_cgroup(&config.resources)?;
        
        // Create container
        let container = Container {
            id: ContainerId::generate(),
            config: config.clone(),
            rootfs,
            namespaces,
            capabilities: self.compute_capabilities(&config),
            seccomp: self.create_seccomp_filter(&config)?,
            apparmor_profile: config.apparmor_profile.clone(),
            state: ContainerState::Created,
        };
        
        let id = container.id;
        self.containers.insert(id, container);
        
        Ok(id)
    }
    
    /// Start container
    pub fn start_container(&mut self, id: ContainerId) -> Result<(), Error> {
        let container = self.containers.get_mut(&id)
            .ok_or(Error::ContainerNotFound)?;
            
        // Clone for new process
        let pid = unsafe {
            libc::clone(
                container_init,
                container.stack.as_mut_ptr().add(STACK_SIZE),
                CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWNET | 
                CLONE_NEWIPC | CLONE_NEWUTS | CLONE_NEWUSER,
                &container as *const _ as *mut libc::c_void,
            )
        };
        
        if pid < 0 {
            return Err(Error::CloneFailed);
        }
        
        container.state = ContainerState::Running(pid);
        
        Ok(())
    }
    
    /// Container init process
    extern "C" fn container_init(arg: *mut libc::c_void) -> i32 {
        let container = unsafe { &*(arg as *const Container) };
        
        // Set hostname
        if let Err(e) = sethostname(&container.config.hostname) {
            eprintln!("Failed to set hostname: {}", e);
            return 1;
        }
        
        // Mount filesystems
        if let Err(e) = setup_mounts(&container.rootfs) {
            eprintln!("Failed to setup mounts: {}", e);
            return 1;
        }
        
        // Apply resource limits
        if let Err(e) = apply_rlimits(&container.config.limits) {
            eprintln!("Failed to apply limits: {}", e);
            return 1;
        }
        
        // Drop capabilities
        if let Err(e) = drop_capabilities(&container.capabilities) {
            eprintln!("Failed to drop capabilities: {}", e);
            return 1;
        }
        
        // Apply seccomp filter
        if let Err(e) = container.seccomp.apply() {
            eprintln!("Failed to apply seccomp: {}", e);
            return 1;
        }
        
        // Change root
        if let Err(e) = chroot(&container.rootfs) {
            eprintln!("Failed to chroot: {}", e);
            return 1;
        }
        
        // Execute container process
        let result = Command::new(&container.config.entrypoint)
            .args(&container.config.args)
            .envs(&container.config.env)
            .exec();
            
        eprintln!("Failed to exec: {:?}", result);
        1
    }
    
    /// Create seccomp filter
    fn create_seccomp_filter(&self, config: &ContainerConfig) -> Result<SeccompFilter, Error> {
        let mut filter = SeccompFilter::new(SeccompAction::Kill);
        
        // Allow basic syscalls
        for syscall in &ALLOWED_SYSCALLS {
            filter.add_rule(SeccompAction::Allow, *syscall)?;
        }
        
        // Add custom rules from config
        for rule in &config.seccomp_rules {
            filter.add_rule(rule.action, rule.syscall)?;
        }
        
        filter.compile()
    }
}

/// Default allowed syscalls for containers
const ALLOWED_SYSCALLS: &[Syscall] = &[
    Syscall::Read,
    Syscall::Write,
    Syscall::Open,
    Syscall::Close,
    Syscall::Stat,
    Syscall::Fstat,
    Syscall::Lseek,
    Syscall::Mmap,
    Syscall::Mprotect,
    Syscall::Munmap,
    Syscall::Brk,
    Syscall::Sigaction,
    Syscall::Sigprocmask,
    Syscall::Ioctl,
    Syscall::Access,
    Syscall::Execve,
    Syscall::Exit,
    Syscall::ExitGroup,
    Syscall::Getpid,
    Syscall::Gettid,
    // ... more essential syscalls
];
```

### 6. Hardware Security Integration

#### 6.1 TPM Integration

**kernel/src/security/tpm.rs**
```rust
/// TPM 2.0 driver
pub struct Tpm2Driver {
    /// TPM device
    device: TpmDevice,
    /// Active sessions
    sessions: BTreeMap<SessionHandle, Session>,
    /// Loaded keys
    loaded_keys: BTreeMap<KeyHandle, LoadedKey>,
    /// Event log
    event_log: EventLog,
}

impl Tpm2Driver {
    /// Initialize TPM
    pub fn init(&mut self) -> Result<(), TpmError> {
        // Start up TPM
        self.device.startup(StartupType::Clear)?;
        
        // Self test
        self.device.self_test()?;
        
        // Get TPM properties
        let props = self.device.get_capability(Capability::TpmProperties)?;
        println!("TPM manufacturer: {:?}", props.manufacturer);
        println!("TPM version: {:?}", props.version);
        
        // Initialize platform hierarchy
        self.init_platform_hierarchy()?;
        
        // Create SRK (Storage Root Key)
        self.create_srk()?;
        
        Ok(())
    }
    
    /// Extend PCR
    pub fn extend_pcr(
        &mut self,
        pcr_index: u8,
        digest: &[u8],
        event: &str,
    ) -> Result<(), TpmError> {
        // Validate PCR index
        if pcr_index >= 24 {
            return Err(TpmError::InvalidPcr);
        }
        
        // Extend PCR
        self.device.pcr_extend(pcr_index, digest)?;
        
        // Log event
        self.event_log.add_event(Event {
            pcr_index,
            event_type: EventType::Action,
            digest: digest.to_vec(),
            event_data: event.as_bytes().to_vec(),
        });
        
        Ok(())
    }
    
    /// Create attestation quote
    pub fn create_quote(
        &mut self,
        pcr_selection: &[u8],
        nonce: &[u8],
        signing_key: KeyHandle,
    ) -> Result<Quote, TpmError> {
        // Start auth session
        let session = self.start_auth_session(SessionType::Hmac)?;
        
        // Create quote
        let quote_info = self.device.quote(
            signing_key,
            pcr_selection,
            nonce,
            session,
        )?;
        
        // Get PCR values
        let pcr_values = self.device.pcr_read(pcr_selection)?;
        
        Ok(Quote {
            quoted: quote_info,
            signature: quote_info.signature,
            pcr_digest: pcr_values.digest(),
            pcr_values,
        })
    }
    
    /// Seal data to PCR state
    pub fn seal(
        &mut self,
        data: &[u8],
        pcr_policy: &PcrPolicy,
        auth: &[u8],
    ) -> Result<SealedBlob, TpmError> {
        // Create sealing key
        let key_handle = self.create_sealing_key(pcr_policy)?;
        
        // Create auth session
        let session = self.start_auth_session(SessionType::Policy)?;
        
        // Apply PCR policy
        self.device.policy_pcr(session, pcr_policy)?;
        
        // Seal data
        let sealed = self.device.create_sealed(
            key_handle,
            data,
            auth,
            session,
        )?;
        
        Ok(sealed)
    }
}

/// Intel TDX attestation
pub struct TdxAttestation {
    /// TDX module handle
    tdx_module: TdxModule,
    /// Quote generation enclave
    qe: QuoteEnclave,
}

impl TdxAttestation {
    /// Generate TDX attestation report
    pub fn generate_report(
        &self,
        user_data: &[u8; 64],
    ) -> Result<TdxReport, Error> {
        // Get TD info
        let td_info = self.tdx_module.get_td_info()?;
        
        // Create report struct
        let report_data = ReportData {
            user_data: *user_data,
            td_info,
        };
        
        // Generate report
        let report = self.tdx_module.create_report(&report_data)?;
        
        Ok(report)
    }
    
    /// Get signed quote
    pub fn get_quote(
        &self,
        report: &TdxReport,
        nonce: &[u8],
    ) -> Result<SignedQuote, Error> {
        // Send report to quoting enclave
        let quote_req = QuoteRequest {
            report: report.clone(),
            nonce: nonce.to_vec(),
            quote_type: QuoteType::EcdsaP256,
        };
        
        let signed_quote = self.qe.generate_quote(&quote_req)?;
        
        Ok(signed_quote)
    }
}
```

## Implementation Timeline

### Month 16-17: Mandatory Access Control
- Week 1-2: Security server implementation
- Week 3-4: Policy compiler and tools
- Week 5-6: Kernel enforcement hooks
- Week 7-8: Testing and validation

### Month 18: Secure Boot
- Week 1-2: UEFI secure boot integration
- Week 3-4: Boot chain verification

### Month 19: Cryptographic Services
- Week 1-2: Key management service
- Week 3-4: Post-quantum crypto integration

### Month 20: Security Monitoring
- Week 1-2: Audit system
- Week 3-4: Intrusion detection

### Month 21: Sandboxing & Hardware Security
- Week 1-2: Container runtime
- Week 3-4: TPM/HSM integration

## Testing Strategy

### Security Testing
- Penetration testing
- Fuzzing all interfaces
- Policy validation
- Cryptographic verification

### Compliance Testing
- Common Criteria requirements
- FIPS 140-3 validation
- Security Technical Implementation Guides (STIGs)

### Performance Testing
- Crypto operation benchmarks
- Policy decision caching
- Audit system overhead

## Success Criteria

1. **MAC System**: < 1μs policy decision with caching
2. **Secure Boot**: Complete chain of trust verification
3. **Crypto Performance**: Hardware acceleration where available
4. **Audit System**: < 5% overhead for normal operations
5. **Container Security**: Process isolation with minimal overhead
6. **Hardware Security**: Full TPM 2.0 and HSM support

## Dependencies for Phase 4

- Hardened kernel with security hooks
- Verified boot chain
- Cryptographic infrastructure
- Security policy management tools
- Audit and monitoring framework