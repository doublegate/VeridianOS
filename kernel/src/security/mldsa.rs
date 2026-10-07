//! ML-DSA-65 signature verification (FIPS 204).
//!
//! A thin wrapper over the RustCrypto `ml-dsa` crate (pure Rust, `no_std`,
//! `forbid(unsafe_code)`), checked here against NIST ACVP known-answer
//! vectors. It replaces `security/dilithium.rs`, which claimed FIPS 204 but
//! was not a lattice verifier at all: it never used the public key and
//! accepted any signature with non-zero bytes (N-55). Only verification is
//! needed in the kernel; signing happens off-system.

use ml_dsa::{EncodedSignature, EncodedVerifyingKey, MlDsa65, Signature, VerifyingKey};

/// Encoded ML-DSA-65 public key size in bytes (FIPS 204, Table 2).
pub const PUBLIC_KEY_SIZE: usize = 1952;

/// Encoded ML-DSA-65 signature size in bytes (FIPS 204, Table 2).
pub const SIGNATURE_SIZE: usize = 3309;

/// Longest context string FIPS 204 allows (Algorithm 2/3).
pub const MAX_CONTEXT_LEN: usize = 255;

/// Verify an ML-DSA-65 signature over `message` with context string
/// `context` (FIPS 204 Algorithm 3, ML-DSA.Verify, pure variant).
///
/// Returns `false` for anything that is not a valid signature: a key or
/// signature of the wrong length, an encoding that does not decode
/// (including a malformed hint), an over-long context, or a signature that
/// does not verify. There is no other outcome and no fallback.
pub fn verify(public_key: &[u8], message: &[u8], context: &[u8], signature: &[u8]) -> bool {
    if public_key.len() != PUBLIC_KEY_SIZE
        || signature.len() != SIGNATURE_SIZE
        || context.len() > MAX_CONTEXT_LEN
    {
        return false;
    }
    let Ok(pk) = EncodedVerifyingKey::<MlDsa65>::try_from(public_key) else {
        return false;
    };
    let Ok(sig) = EncodedSignature::<MlDsa65>::try_from(signature) else {
        return false;
    };
    let Some(sig) = Signature::<MlDsa65>::decode(&sig) else {
        return false;
    };
    VerifyingKey::<MlDsa65>::decode(&pk).verify_with_context(message, context, &sig)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;
    use crate::security::mldsa_vectors as vectors;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn acvp_sigver_ml_dsa_65() {
        for v in vectors::SIG_VER {
            let got = verify(
                &hex(v.pk),
                &hex(v.message),
                &hex(v.context),
                &hex(v.signature),
            );
            assert_eq!(got, v.expected, "tcId {} ({})", v.tc_id, v.reason);
        }
    }

    #[test]
    fn acvp_keygen_ml_dsa_65_matches_the_public_key() {
        let seed = hex(vectors::KEYGEN_SEED);
        let sk = ml_dsa::SigningKey::<MlDsa65>::from_seed(seed.as_slice().try_into().unwrap());
        assert_eq!(
            sk.expanded_key().verifying_key().encode().as_slice(),
            hex(vectors::KEYGEN_PK).as_slice()
        );
    }

    #[test]
    fn sign_verify_round_trip_and_tamper() {
        let sk = ml_dsa::SigningKey::<MlDsa65>::from_seed(&[7u8; 32].into());
        let pk = sk.expanded_key().verifying_key().encode();
        let sig = sk
            .expanded_key()
            .sign_deterministic(b"package payload", b"veridian-pkg")
            .unwrap()
            .encode();
        assert!(verify(&pk, b"package payload", b"veridian-pkg", &sig));
        // Message, context, signature and key are each bound.
        assert!(!verify(&pk, b"package payloaD", b"veridian-pkg", &sig));
        assert!(!verify(&pk, b"package payload", b"other-context", &sig));
        let mut bad = sig.to_vec();
        bad[100] ^= 1;
        assert!(!verify(&pk, b"package payload", b"veridian-pkg", &bad));
        let other = ml_dsa::SigningKey::<MlDsa65>::from_seed(&[8u8; 32].into());
        assert!(!verify(
            &other.expanded_key().verifying_key().encode(),
            b"package payload",
            b"veridian-pkg",
            &sig
        ));
    }

    #[test]
    fn wrong_sizes_and_garbage_are_rejected() {
        // The old structural "verifier" accepted exactly these.
        assert!(!verify(
            &[1u8; PUBLIC_KEY_SIZE],
            b"m",
            b"",
            &[1u8; SIGNATURE_SIZE]
        ));
        assert!(!verify(&[0u8; 32], b"m", b"", &[0xAB; 64]));
        assert!(!verify(&[1u8; PUBLIC_KEY_SIZE], b"m", b"", &[1u8; 3293]));
        assert!(!verify(
            &[1u8; PUBLIC_KEY_SIZE],
            b"m",
            &[0u8; 256],
            &[1u8; SIGNATURE_SIZE]
        ));
    }
}
