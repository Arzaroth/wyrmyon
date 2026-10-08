use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Nonce, XSalsa20Poly1305};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;

#[derive(Clone, PartialEq, Eq)]
pub struct Key(pub(crate) [u8; KEY_LEN]);

impl Key {
    #[must_use]
    pub(crate) fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    #[must_use]
    pub fn derive(&self, purpose: &[u8]) -> Key {
        Key(hkdf(&self.0, purpose))
    }

    #[must_use]
    pub fn derive_phase(&self, side: &str, phase: &str) -> Key {
        let mut purpose = b"wormhole:phase:".to_vec();
        purpose.extend_from_slice(&Sha256::digest(side.as_bytes()));
        purpose.extend_from_slice(&Sha256::digest(phase.as_bytes()));
        self.derive(&purpose)
    }

    #[must_use]
    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce);
        self.encrypt_with_nonce(&nonce, plaintext)
    }

    #[must_use]
    pub fn encrypt_with_nonce(&self, nonce: &[u8; NONCE_LEN], plaintext: &[u8]) -> Vec<u8> {
        let ciphertext = XSalsa20Poly1305::new(&self.0.into())
            .encrypt(Nonce::from_slice(nonce), plaintext)
            .expect("secretbox encryption cannot fail");
        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(nonce);
        out.extend_from_slice(&ciphertext);
        out
    }

    #[must_use]
    pub fn decrypt(&self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() < NONCE_LEN {
            return None;
        }
        let (nonce, ciphertext) = data.split_at(NONCE_LEN);
        XSalsa20Poly1305::new(&self.0.into())
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .ok()
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

#[must_use]
pub(crate) fn hkdf(key: &[u8], info: &[u8]) -> [u8; KEY_LEN] {
    let mut out = [0u8; KEY_LEN];
    Hkdf::<Sha256>::new(None, key)
        .expand(info, &mut out)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Key {
        Key(std::array::from_fn(|i| u8::try_from(i).unwrap()))
    }

    #[test]
    fn derivations_match_the_python_client() {
        assert_eq!(
            hex::encode(key().derive(b"wormhole:verifier").0),
            "116c6e41d0faf2886a5b488079748585db2c4d6d151cca6c580055e1bd176459"
        );
        assert_eq!(
            hex::encode(key().derive_phase("0123456789", "version").0),
            "65187c8822d10970289e2eaa1eb623d7c90d0ff2af2507fcdac4f1a08bf13fa9"
        );
    }

    #[test]
    fn decrypt_reverses_encrypt_and_rejects_tampering() {
        let k = key();
        let mut sealed = k.encrypt(b"hello");
        assert_eq!(k.decrypt(&sealed).as_deref(), Some(&b"hello"[..]));
        *sealed.last_mut().unwrap() ^= 1;
        assert_eq!(k.decrypt(&sealed), None);
        assert_eq!(k.decrypt(&[0; 4]), None);
    }
}
