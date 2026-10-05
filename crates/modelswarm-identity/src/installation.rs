//! Per-installation Ed25519 identity (ADR-004, msp-v1 §2.1).

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::{CryptoRngCore, OsRng};
use sha2::{Digest, Sha256};

/// An installation's Ed25519 signing identity.
///
/// One keypair per installation. The public identity label is
/// `installation_id()` = base58(SHA-256(public key bytes)). The libp2p
/// `PeerId` introduced by `modelswarm-transport` derives from the same key
/// and carries the same base58(SHA-256(pubkey)) label; until that crate
/// lands, [`InstallationIdentity::peer_id_label`] returns the identical
/// string so callers can already bind records to the future PeerId form.
#[derive(Debug, Clone)]
pub struct InstallationIdentity {
    signing_key: SigningKey,
}

impl InstallationIdentity {
    /// Generates a fresh keypair from the operating system CSPRNG.
    pub fn generate() -> Self {
        Self::generate_with(&mut OsRng)
    }

    /// Generates a fresh keypair from a caller-supplied RNG (tests,
    /// deterministic simulations).
    pub fn generate_with(rng: &mut impl CryptoRngCore) -> Self {
        Self {
            signing_key: SigningKey::generate(rng),
        }
    }

    /// Restores an identity from its 32 seed bytes.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(bytes),
        }
    }

    /// The 32 seed bytes (SECRET — never log, transmit, or persist unencrypted).
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// The 32-byte Ed25519 public key.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// The Ed25519 verifying key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// `installationId` = base58(SHA-256(public key bytes)) (msp-v1 §2.1).
    pub fn installation_id(&self) -> String {
        installation_id_for(&self.public_key_bytes())
    }

    /// The libp2p PeerId label for this key: identical to
    /// [`Self::installation_id`] by construction (same key, same derivation).
    /// The typed `PeerId` wrapper arrives with `modelswarm-transport`.
    pub fn peer_id_label(&self) -> String {
        self.installation_id()
    }

    /// Signs an arbitrary message (used by the envelope and lease modules).
    pub(crate) fn sign_raw(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }
}

/// base58(SHA-256(public key bytes)) — the public identity label derivation.
pub fn installation_id_for(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    bs58::encode(digest).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_id_is_deterministic_and_key_derived() {
        let seed = [7u8; 32];
        let a = InstallationIdentity::from_bytes(&seed);
        let b = InstallationIdentity::from_bytes(&seed);
        assert_eq!(a.installation_id(), b.installation_id());
        assert_eq!(a.peer_id_label(), a.installation_id());
        assert_eq!(
            a.installation_id(),
            installation_id_for(&a.public_key_bytes())
        );

        // Cross-check the derivation against a hand-computed value over the
        // PUBLIC key (not the seed).
        let digest = Sha256::digest(a.public_key_bytes());
        assert_eq!(a.installation_id(), bs58::encode(digest).into_string());
    }

    #[test]
    fn distinct_keys_have_distinct_ids() {
        let a = InstallationIdentity::generate();
        let b = InstallationIdentity::generate();
        assert_ne!(a.installation_id(), b.installation_id());
        assert!(!a.installation_id().is_empty());
    }

    #[test]
    fn from_bytes_round_trips() {
        let id = InstallationIdentity::generate();
        let restored = InstallationIdentity::from_bytes(&id.to_bytes());
        assert_eq!(restored.installation_id(), id.installation_id());
    }
}
