//! SHA-256 digesting and Ed25519 signature checking for module binaries.
//!
//! Scheme: `signature = Ed25519.sign(sk, SHA-256(binary))`. The `.sig` file
//! next to a module holds the signature either as 64 raw bytes or as 128 hex
//! characters. Trusted public keys live in `<root>/trust/*.pub` (64 hex chars).

use crate::error::{MandalaError, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use std::path::Path;

pub type Digest32 = [u8; 32];

pub fn sha256(bytes: &[u8]) -> Digest32 {
    Sha256::digest(bytes).into()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha256(bytes))
}

/// Set of public keys the host trusts. A signature from any one is accepted.
#[derive(Clone, Default)]
pub struct TrustStore {
    keys: Vec<VerifyingKey>,
}

impl TrustStore {
    pub fn new(keys: Vec<VerifyingKey>) -> Self {
        Self { keys }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn add_hex(&mut self, text: &str) -> Result<()> {
        self.keys.push(parse_public_key(text)?);
        Ok(())
    }

    /// Load every `*.pub` file in `dir`. A missing directory yields an empty
    /// store (which rejects all modules).
    pub fn load_dir(dir: &Path) -> Result<Self> {
        let mut store = Self::default();
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(store),
            Err(e) => return Err(MandalaError::io(dir, e)),
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("pub") {
                let text = std::fs::read_to_string(&p).map_err(|e| MandalaError::io(&p, e))?;
                store.add_hex(&text)?;
            }
        }
        Ok(store)
    }

    /// Verify `digest` against `sig_bytes` using any trusted key.
    pub fn verify(&self, digest: &Digest32, sig_bytes: &[u8]) -> Result<()> {
        if self.keys.is_empty() {
            return Err(MandalaError::Crypto("no trusted public keys installed".into()));
        }
        let sig = parse_signature(sig_bytes)?;
        if self.keys.iter().any(|k| k.verify_strict(digest, &sig).is_ok()) {
            Ok(())
        } else {
            Err(MandalaError::Crypto("signature does not match any trusted key".into()))
        }
    }
}

pub fn parse_public_key(text: &str) -> Result<VerifyingKey> {
    let raw = hex::decode(text.trim()).map_err(|e| MandalaError::Crypto(format!("public key hex: {e}")))?;
    let arr: [u8; 32] = raw
        .try_into()
        .map_err(|_| MandalaError::Crypto("public key must be 32 bytes".into()))?;
    VerifyingKey::from_bytes(&arr).map_err(|e| MandalaError::Crypto(format!("public key: {e}")))
}

pub fn parse_signing_key(text: &str) -> Result<SigningKey> {
    let raw = hex::decode(text.trim()).map_err(|e| MandalaError::Crypto(format!("secret key hex: {e}")))?;
    let arr: [u8; 32] = raw
        .try_into()
        .map_err(|_| MandalaError::Crypto("secret key must be 32 bytes".into()))?;
    Ok(SigningKey::from_bytes(&arr))
}

/// Accepts 64 raw bytes or 128 hex chars (whitespace tolerated).
pub fn parse_signature(raw: &[u8]) -> Result<Signature> {
    let bytes: Vec<u8> = if raw.len() == 64 {
        raw.to_vec()
    } else {
        let text = std::str::from_utf8(raw).map_err(|_| MandalaError::Crypto("signature is neither 64 raw bytes nor hex".into()))?;
        hex::decode(text.trim()).map_err(|e| MandalaError::Crypto(format!("signature hex: {e}")))?
    };
    let arr: [u8; 64] = bytes
        .try_into()
        .map_err(|_| MandalaError::Crypto("signature must be 64 bytes".into()))?;
    Ok(Signature::from_bytes(&arr))
}

/// Sign a binary: returns the signature over its SHA-256 digest.
pub fn sign_binary(key: &SigningKey, binary: &[u8]) -> Signature {
    key.sign(&sha256(binary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    fn keypair() -> (SigningKey, TrustStore) {
        let sk = SigningKey::generate(&mut OsRng);
        let ts = TrustStore::new(vec![sk.verifying_key()]);
        (sk, ts)
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let (sk, ts) = keypair();
        let bin = b"\x7fELF fake module";
        let sig = sign_binary(&sk, bin);
        ts.verify(&sha256(bin), &sig.to_bytes()).unwrap();
        // hex form works too
        ts.verify(&sha256(bin), hex::encode(sig.to_bytes()).as_bytes()).unwrap();
    }

    #[test]
    fn tampered_binary_is_rejected() {
        let (sk, ts) = keypair();
        let sig = sign_binary(&sk, b"original");
        assert!(ts.verify(&sha256(b"tampered"), &sig.to_bytes()).is_err());
    }

    #[test]
    fn foreign_key_is_rejected() {
        let (sk, _) = keypair();
        let (_, other) = keypair();
        let sig = sign_binary(&sk, b"x");
        assert!(other.verify(&sha256(b"x"), &sig.to_bytes()).is_err());
    }

    #[test]
    fn empty_store_rejects_everything() {
        let (sk, _) = keypair();
        let sig = sign_binary(&sk, b"x");
        assert!(TrustStore::default().verify(&sha256(b"x"), &sig.to_bytes()).is_err());
    }

    #[test]
    fn garbage_signature_is_rejected() {
        let (_, ts) = keypair();
        assert!(ts.verify(&sha256(b"x"), b"nope").is_err());
        assert!(ts.verify(&sha256(b"x"), &[0u8; 64]).is_err());
    }

    #[test]
    fn key_hex_roundtrip() {
        let sk = SigningKey::generate(&mut OsRng);
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        assert_eq!(parse_public_key(&pk_hex).unwrap(), sk.verifying_key());
        let sk_hex = hex::encode(sk.to_bytes());
        assert_eq!(parse_signing_key(&sk_hex).unwrap().to_bytes(), sk.to_bytes());
    }
}
