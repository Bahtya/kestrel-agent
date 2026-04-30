//! AES-256-GCM encryption/decryption with Argon2 key derivation.

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use argon2::Argon2;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// Derive a 32-byte AES key from a password and salt using Argon2id.
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let argon2 = Argon2::default();
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow::anyhow!("Argon2 key derivation failed: {e}"))?;
    Ok(key)
}

/// Encrypt plaintext using AES-256-GCM with a password-derived key.
///
/// Binary format: `salt (16 bytes) || nonce (12 bytes) || ciphertext+tag`.
pub fn encrypt(plaintext: &str, password: &str) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);

    let key = derive_key(password, &salt)?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("AES key init failed: {e}"))?;

    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {e}"))?;

    let mut out = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt ciphertext produced by [`encrypt`].
///
/// Expects binary format: `salt (16 bytes) || nonce (12 bytes) || ciphertext+tag`.
pub fn decrypt(data: &[u8], password: &str) -> Result<String> {
    if data.len() < SALT_LEN + NONCE_LEN + 1 {
        anyhow::bail!("Encrypted data too short");
    }

    let (salt, rest) = data.split_at(SALT_LEN);
    let (nonce_bytes, ciphertext) = rest.split_at(NONCE_LEN);

    let key = derive_key(password, salt)?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("AES key init failed: {e}"))?;

    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed — wrong password or corrupted data"))?;

    String::from_utf8(plaintext).context("Decrypted data is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let msg = "hello kestrel config";
        let encrypted = encrypt(msg, "hunter2").unwrap();
        let decrypted = decrypt(&encrypted, "hunter2").unwrap();
        assert_eq!(decrypted, msg);
    }

    #[test]
    fn wrong_password_fails() {
        let encrypted = encrypt("secret", "correct").unwrap();
        assert!(decrypt(&encrypted, "wrong").is_err());
    }

    #[test]
    fn truncated_data_fails() {
        assert!(decrypt(&[0u8; 10], "pw").is_err());
    }
}
