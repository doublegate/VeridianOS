//! Security Feature Tests
//!
//! Comprehensive tests for authentication, ASLR, post-quantum crypto, and TPM.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use veridian_kernel::security::auth::{UserManager, UserId};
use veridian_kernel::security::memory_protection::{Aslr, StackCanary, RegionType};
use veridian_kernel::crypto::post_quantum::{
    DilithiumSigningKey, DilithiumLevel, KyberSecretKey, KyberLevel, HybridKeyExchange
};
use veridian_kernel::security::tpm::{Tpm, TpmInterfaceType};
use veridian_kernel::{serial_print, serial_println};
use alloc::string::String;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

// ============================================================================
// Authentication Tests
// ============================================================================

#[test_case]
fn test_user_creation_and_authentication() {
    serial_print!("test_user_creation_and_authentication... ");

    let user_mgr = UserManager::new();

    // Create user
    let user_id = user_mgr.create_user(String::from("alice"), "password123")
        .expect("Failed to create user");

    // Authenticate with correct password
    assert!(user_mgr.authenticate("alice", "password123").is_ok());

    // Authenticate with wrong password should fail
    assert!(user_mgr.authenticate("alice", "wrongpassword").is_err());

    serial_println!("[ok]");
}

#[test_case]
fn test_account_locking_after_failed_attempts() {
    serial_print!("test_account_locking_after_failed_attempts... ");

    let user_mgr = UserManager::new();
    user_mgr.create_user(String::from("bob"), "correct").expect("Failed to create user");

    // Try 5 failed attempts
    for _ in 0..5 {
        let _ = user_mgr.authenticate("bob", "wrong");
    }

    // Account should be locked even with correct password
    assert!(user_mgr.authenticate("bob", "correct").is_err());

    serial_println!("[ok]");
}

#[test_case]
fn test_mfa_enablement() {
    serial_print!("test_mfa_enablement... ");

    let user_mgr = UserManager::new();
    user_mgr.create_user(String::from("charlie"), "pass").expect("Failed to create user");

    // Enable MFA
    let mfa_secret = user_mgr.enable_mfa("charlie").expect("Failed to enable MFA");

    // Secret should be 32 bytes
    assert_eq!(mfa_secret.len(), 32);

    serial_println!("[ok]");
}

// ============================================================================
// ASLR Tests
// ============================================================================

#[test_case]
fn test_aslr_randomization_uniqueness() {
    serial_print!("test_aslr_randomization_uniqueness... ");

    let aslr = Aslr::new().expect("Failed to create ASLR");
    let base = 0x400000;

    // Generate 10 randomized addresses
    let mut addresses = [0usize; 10];
    for addr in addresses.iter_mut() {
        *addr = aslr.randomize_address(base, RegionType::Stack);
    }

    // At least 8 out of 10 should be different (allowing some collision)
    let mut unique_count = 0;
    for i in 0..10 {
        let mut is_unique = true;
        for j in 0..i {
            if addresses[i] == addresses[j] {
                is_unique = false;
                break;
            }
        }
        if is_unique {
            unique_count += 1;
        }
    }

    assert!(unique_count >= 8, "ASLR not producing enough entropy");

    serial_println!("[ok]");
}

#[test_case]
fn test_aslr_page_alignment() {
    serial_print!("test_aslr_page_alignment... ");

    let aslr = Aslr::new().expect("Failed to create ASLR");

    for _ in 0..20 {
        let addr = aslr.randomize_address(0x400000, RegionType::Heap);
        // Check 4KB alignment
        assert_eq!(addr & 0xFFF, 0, "Address not page-aligned: 0x{:X}", addr);
    }

    serial_println!("[ok]");
}

#[test_case]
fn test_stack_canary_uniqueness() {
    serial_print!("test_stack_canary_uniqueness... ");

    let canary1 = StackCanary::new();
    let canary2 = StackCanary::new();

    // Canaries should be different
    assert_ne!(canary1.value(), canary2.value());

    serial_println!("[ok]");
}

#[test_case]
fn test_stack_canary_verification() {
    serial_print!("test_stack_canary_verification... ");

    let canary = StackCanary::new();
    let value = canary.value();

    // Correct value should verify
    assert!(canary.verify(value));

    // Modified value should not verify
    assert!(!canary.verify(value ^ 1));

    serial_println!("[ok]");
}

// ============================================================================
// Post-Quantum Cryptography Tests
// ============================================================================

#[test_case]
fn test_dilithium_sign_verify_level2() {
    serial_print!("test_dilithium_sign_verify_level2... ");

    let signing_key = DilithiumSigningKey::generate(DilithiumLevel::Level2)
        .expect("Failed to generate signing key");
    let verifying_key = signing_key.verifying_key();

    let message = b"Test message for Dilithium signature";
    let signature = signing_key.sign(message).expect("Failed to sign");

    // Signature should verify
    assert!(verifying_key.verify(message, &signature).expect("Verification failed"));

    serial_println!("[ok]");
}

#[test_case]
fn test_dilithium_all_security_levels() {
    serial_print!("test_dilithium_all_security_levels... ");

    for level in [DilithiumLevel::Level2, DilithiumLevel::Level3, DilithiumLevel::Level5] {
        let key = DilithiumSigningKey::generate(level).expect("Failed to generate key");
        let vkey = key.verifying_key();
        let sig = key.sign(b"test").expect("Failed to sign");
        assert!(vkey.verify(b"test", &sig).expect("Failed to verify"));
    }

    serial_println!("[ok]");
}

#[test_case]
fn test_kyber_kem_encapsulation() {
    serial_print!("test_kyber_kem_encapsulation... ");

    let secret_key = KyberSecretKey::generate(KyberLevel::Kyber768)
        .expect("Failed to generate secret key");
    let public_key = secret_key.public_key();

    // Encapsulate to get ciphertext and shared secret
    let (ciphertext, shared_secret1) = public_key.encapsulate()
        .expect("Failed to encapsulate");

    // Decapsulate to recover shared secret
    let shared_secret2 = secret_key.decapsulate(&ciphertext)
        .expect("Failed to decapsulate");

    // Shared secrets should match
    assert_eq!(shared_secret1.as_bytes(), shared_secret2.as_bytes());

    serial_println!("[ok]");
}

#[test_case]
fn test_hybrid_key_exchange() {
    serial_print!("test_hybrid_key_exchange... ");

    let alice = HybridKeyExchange::generate(KyberLevel::Kyber768)
        .expect("Failed to generate Alice's keys");
    let bob = HybridKeyExchange::generate(KyberLevel::Kyber768)
        .expect("Failed to generate Bob's keys");

    let (alice_classical, alice_pq) = alice.public_keys();
    let (bob_classical, bob_pq) = bob.public_keys();

    // Alice encapsulates to Bob
    let (bob_ct, _) = alice_pq.encapsulate().expect("Failed to encapsulate");

    // Alice performs key exchange
    let alice_shared = alice.exchange(&bob_classical, &bob_ct)
        .expect("Failed to exchange");

    // Shared secret should be 32 bytes
    assert_eq!(alice_shared.len(), 32);

    serial_println!("[ok]");
}

// ============================================================================
// TPM Tests
// ============================================================================

#[test_case]
fn test_tpm_detection() {
    serial_print!("test_tpm_detection... ");

    let mut tpm = Tpm::new();

    // Detect hardware (will return None in virtual environment)
    let interface = tpm.detect_hardware().expect("Detection failed");

    // Should detect None in QEMU
    assert_eq!(interface, TpmInterfaceType::None);

    serial_println!("[ok]");
}

#[test_case]
fn test_tpm_startup_stub() {
    serial_print!("test_tpm_startup_stub... ");

    let mut tpm = Tpm::new();

    // Startup should succeed even without hardware (stub mode)
    assert!(tpm.startup().is_ok());

    serial_println!("[ok]");
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test_case]
fn test_security_stack_integration() {
    serial_print!("test_security_stack_integration... ");

    // Test that all security components can be initialized together
    let _user_mgr = UserManager::new();
    let _aslr = Aslr::new().expect("Failed to create ASLR");
    let _canary = StackCanary::new();
    let _tpm = Tpm::new();

    // Test crypto operations
    let _dil_key = DilithiumSigningKey::generate(DilithiumLevel::Level2)
        .expect("Failed to generate Dilithium key");
    let _kyber_key = KyberSecretKey::generate(KyberLevel::Kyber512)
        .expect("Failed to generate Kyber key");

    serial_println!("[ok]");
}
