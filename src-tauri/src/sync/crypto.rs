// SPDX-FileCopyrightText: Copyright (c) 2026 kanri-sync contributors
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Authenticated encryption for everything sent over the LAN.
//!
//! All paired devices share a 256-bit group key. Every request and response
//! body is sealed with XChaCha20-Poly1305 under that key, with the HTTP path
//! as associated data, so a device on the same Wi-Fi that was never paired
//! can neither read boards nor inject changes. Pairing uses a one-time token
//! (carried in the QR code) to deliver the group key the same way.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Key = [u8; 32];

pub fn random_key() -> Key {
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    k
}

pub fn random_id(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn encode_key(k: &Key) -> String {
    B64.encode(k)
}

pub fn decode_key(s: &str) -> Result<Key, String> {
    let v = B64.decode(s).map_err(|e| format!("invalid key encoding: {e}"))?;
    v.try_into().map_err(|_| "invalid key length".to_string())
}

/// Key used during pairing, derived from the one-time token in the QR code.
pub fn pairing_key(token: &str) -> Key {
    let mut h = Sha256::new();
    h.update(b"kanri-sync/pair/v1");
    h.update(token.as_bytes());
    h.finalize().into()
}

/// Short public tag of a group, advertised over mDNS so devices can find
/// members of their own group without revealing the group id itself.
pub fn group_tag(group_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"kanri-sync/group-tag/v1");
    h.update(group_id.as_bytes());
    hex::encode(&h.finalize()[..8])
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    v: u8,
    n: String,
    c: String,
}

pub fn seal<T: Serialize>(key: &Key, aad: &str, msg: &T) -> Result<Vec<u8>, String> {
    let plain = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plain, aad: aad.as_bytes() })
        .map_err(|_| "encryption failed".to_string())?;
    serde_json::to_vec(&Envelope { v: 1, n: B64.encode(nonce), c: B64.encode(ct) }).map_err(|e| e.to_string())
}

pub fn open<T: DeserializeOwned>(key: &Key, aad: &str, body: &[u8]) -> Result<T, String> {
    let env: Envelope = serde_json::from_slice(body).map_err(|_| "malformed message".to_string())?;
    if env.v != 1 {
        return Err(format!("unsupported protocol version {}", env.v));
    }
    let nonce = B64.decode(env.n).map_err(|_| "malformed nonce".to_string())?;
    if nonce.len() != 24 {
        return Err("malformed nonce".into());
    }
    let ct = B64.decode(env.c).map_err(|_| "malformed ciphertext".to_string())?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let plain = cipher
        .decrypt(XNonce::from_slice(&nonce), Payload { msg: &ct, aad: aad.as_bytes() })
        .map_err(|_| "could not decrypt (device not paired with this group, or key mismatch)".to_string())?;
    serde_json::from_slice(&plain).map_err(|e| format!("invalid payload: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_tamper_detection() {
        let k = random_key();
        let sealed = seal(&k, "/sync", &serde_json::json!({"x": 1})).unwrap();
        let v: serde_json::Value = open(&k, "/sync", &sealed).unwrap();
        assert_eq!(v["x"], 1);
        assert!(open::<serde_json::Value>(&k, "/pair", &sealed).is_err());
        assert!(open::<serde_json::Value>(&random_key(), "/sync", &sealed).is_err());
    }
}
