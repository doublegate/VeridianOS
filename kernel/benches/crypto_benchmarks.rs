//! Cryptographic Performance Benchmarks
//!
//! Benchmarks for hash functions, encryption, signatures, and post-quantum crypto.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::bench_runner)]
#![reexport_test_harness_main = "bench_main"]

extern crate alloc;

use veridian_kernel::crypto::hash::{sha256, sha512, blake3};
use veridian_kernel::crypto::symmetric::{Aes256Gcm, ChaCha20Poly1305, SymmetricCipher};
use veridian_kernel::crypto::asymmetric::{SigningKey, key_exchange::SecretKey};
use veridian_kernel::crypto::post_quantum::{
    DilithiumSigningKey, DilithiumLevel, KyberSecretKey, KyberLevel
};
use veridian_kernel::{serial_print, serial_println, bench_function};
use alloc::vec;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    bench_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

// ============================================================================
// Hash Function Benchmarks
// ============================================================================

#[bench_case]
fn bench_sha256_1kb() {
    serial_print!("bench_sha256_1kb... ");

    let data = vec![0x42u8; 1024];

    let (duration_ns, _) = bench_function(|| {
        sha256(&data)
    });

    let throughput_mbps = (1024.0 * 1000.0) / duration_ns as f64;
    serial_println!("{} ns ({:.2} MB/s)", duration_ns, throughput_mbps);
}

#[bench_case]
fn bench_sha512_1kb() {
    serial_print!("bench_sha512_1kb... ");

    let data = vec![0x42u8; 1024];

    let (duration_ns, _) = bench_function(|| {
        sha512(&data)
    });

    let throughput_mbps = (1024.0 * 1000.0) / duration_ns as f64;
    serial_println!("{} ns ({:.2} MB/s)", duration_ns, throughput_mbps);
}

#[bench_case]
fn bench_blake3_1kb() {
    serial_print!("bench_blake3_1kb... ");

    let data = vec![0x42u8; 1024];

    let (duration_ns, _) = bench_function(|| {
        blake3(&data)
    });

    let throughput_mbps = (1024.0 * 1000.0) / duration_ns as f64;
    serial_println!("{} ns ({:.2} MB/s)", duration_ns, throughput_mbps);
}

// ============================================================================
// Symmetric Encryption Benchmarks
// ============================================================================

#[bench_case]
fn bench_aes256gcm_encrypt_1kb() {
    serial_print!("bench_aes256gcm_encrypt_1kb... ");

    let key = [0x42u8; 32];
    let nonce = [0x01u8; 12];
    let data = vec![0xAAu8; 1024];

    let cipher = Aes256Gcm::new(&key).expect("Failed to create cipher");

    let (duration_ns, _) = bench_function(|| {
        cipher.encrypt(&data, &nonce).expect("Encryption failed")
    });

    let throughput_mbps = (1024.0 * 1000.0) / duration_ns as f64;
    serial_println!("{} ns ({:.2} MB/s)", duration_ns, throughput_mbps);
}

#[bench_case]
fn bench_chacha20poly1305_encrypt_1kb() {
    serial_print!("bench_chacha20poly1305_encrypt_1kb... ");

    let key = [0x42u8; 32];
    let nonce = [0x01u8; 12];
    let data = vec![0xAAu8; 1024];

    let cipher = ChaCha20Poly1305::new(&key).expect("Failed to create cipher");

    let (duration_ns, _) = bench_function(|| {
        cipher.encrypt(&data, &nonce).expect("Encryption failed")
    });

    let throughput_mbps = (1024.0 * 1000.0) / duration_ns as f64;
    serial_println!("{} ns ({:.2} MB/s)", duration_ns, throughput_mbps);
}

// ============================================================================
// Asymmetric Cryptography Benchmarks
// ============================================================================

#[bench_case]
fn bench_ed25519_sign() {
    serial_print!("bench_ed25519_sign... ");

    let signing_key = SigningKey::generate().expect("Failed to generate key");
    let message = b"Benchmark message for Ed25519 signing";

    let (duration_ns, _) = bench_function(|| {
        signing_key.sign(message).expect("Signing failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_ed25519_verify() {
    serial_print!("bench_ed25519_verify... ");

    let signing_key = SigningKey::generate().expect("Failed to generate key");
    let verifying_key = signing_key.verifying_key();
    let message = b"Benchmark message for Ed25519 verification";
    let signature = signing_key.sign(message).expect("Signing failed");

    let (duration_ns, _) = bench_function(|| {
        verifying_key.verify(message, &signature).expect("Verification failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_x25519_key_exchange() {
    serial_print!("bench_x25519_key_exchange... ");

    let alice_secret = SecretKey::generate().expect("Failed to generate key");
    let bob_secret = SecretKey::generate().expect("Failed to generate key");
    let bob_public = bob_secret.public_key();

    let (duration_ns, _) = bench_function(|| {
        alice_secret.exchange(&bob_public).expect("Key exchange failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

// ============================================================================
// Post-Quantum Cryptography Benchmarks
// ============================================================================

#[bench_case]
fn bench_dilithium2_keygen() {
    serial_print!("bench_dilithium2_keygen... ");

    let (duration_ns, _) = bench_function(|| {
        DilithiumSigningKey::generate(DilithiumLevel::Level2).expect("Keygen failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_dilithium2_sign() {
    serial_print!("bench_dilithium2_sign... ");

    let signing_key = DilithiumSigningKey::generate(DilithiumLevel::Level2)
        .expect("Keygen failed");
    let message = b"Benchmark message for Dilithium signing";

    let (duration_ns, _) = bench_function(|| {
        signing_key.sign(message).expect("Signing failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_dilithium2_verify() {
    serial_print!("bench_dilithium2_verify... ");

    let signing_key = DilithiumSigningKey::generate(DilithiumLevel::Level2)
        .expect("Keygen failed");
    let verifying_key = signing_key.verifying_key();
    let message = b"Benchmark message for Dilithium verification";
    let signature = signing_key.sign(message).expect("Signing failed");

    let (duration_ns, _) = bench_function(|| {
        verifying_key.verify(message, &signature).expect("Verification failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_kyber768_keygen() {
    serial_print!("bench_kyber768_keygen... ");

    let (duration_ns, _) = bench_function(|| {
        KyberSecretKey::generate(KyberLevel::Kyber768).expect("Keygen failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_kyber768_encapsulate() {
    serial_print!("bench_kyber768_encapsulate... ");

    let secret_key = KyberSecretKey::generate(KyberLevel::Kyber768)
        .expect("Keygen failed");
    let public_key = secret_key.public_key();

    let (duration_ns, _) = bench_function(|| {
        public_key.encapsulate().expect("Encapsulation failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_kyber768_decapsulate() {
    serial_print!("bench_kyber768_decapsulate... ");

    let secret_key = KyberSecretKey::generate(KyberLevel::Kyber768)
        .expect("Keygen failed");
    let public_key = secret_key.public_key();
    let (ciphertext, _) = public_key.encapsulate().expect("Encapsulation failed");

    let (duration_ns, _) = bench_function(|| {
        secret_key.decapsulate(&ciphertext).expect("Decapsulation failed")
    });

    serial_println!("{} ns ({:.2} ops/sec)", duration_ns, 1_000_000_000.0 / duration_ns as f64);
}

// ============================================================================
// Security Feature Benchmarks
// ============================================================================

#[bench_case]
fn bench_aslr_address_randomization() {
    serial_print!("bench_aslr_address_randomization... ");

    use veridian_kernel::security::memory_protection::{Aslr, RegionType};

    let aslr = Aslr::new().expect("Failed to create ASLR");

    let (duration_ns, _) = bench_function(|| {
        aslr.randomize_address(0x400000, RegionType::Stack)
    });

    serial_println!("{} ns ({:.2} M ops/sec)", duration_ns, 1000.0 / duration_ns as f64);
}

#[bench_case]
fn bench_stack_canary_generation() {
    serial_print!("bench_stack_canary_generation... ");

    use veridian_kernel::security::memory_protection::StackCanary;

    let (duration_ns, _) = bench_function(|| {
        StackCanary::new()
    });

    serial_println!("{} ns ({:.2} M ops/sec)", duration_ns, 1000.0 / duration_ns as f64);
}
