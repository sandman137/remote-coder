//! Security primitives (DESIGN.md §8.3/§8.4): the `KeyStore` trait hiding
//! platform key storage, the desktop/file implementation used for dev and
//! tests, and the client side of the pairing flow.
//!
//! Platform impls: Android Keystore/StrongBox and iOS Secure Enclave arrive
//! with their apps (keys non-exportable, biometric-gated). `FileKeyStore`
//! stands in on desktop so the whole pairing/pinning/signing path is
//! exercised on Linux.

pub mod pairing;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::{HashAlg, LineEnding, PrivateKey};

use crate::error::{EngineError, Result};

pub trait KeyStore: Send + Sync {
    /// Create (or return the existing) device key; returns the OpenSSH
    /// public key line. The private key never leaves the store.
    fn generate_device_key(&self, alias: &str) -> Result<String>;

    /// OpenSSH public key line for `alias`, if the key exists.
    fn public_key(&self, alias: &str) -> Result<Option<String>>;

    /// Sign `data` with the device key (SSHSIG, PEM bytes). Biometric-gated
    /// on mobile.
    fn sign(&self, alias: &str, data: &[u8]) -> Result<Vec<u8>>;

    fn pinned_hostkey(&self, host: &str) -> Result<Option<String>>;
    fn pin_hostkey(&self, host: &str, fp: &str) -> Result<()>;
}

/// File-backed keystore: `<dir>/<alias>` (OpenSSH private key, 0600),
/// `<dir>/<alias>.pub`, and `<dir>/pins.json`.
pub struct FileKeyStore {
    dir: PathBuf,
}

impl FileKeyStore {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| EngineError::Parse(format!("keystore dir {}: {e}", dir.display())))?;
        Ok(FileKeyStore { dir })
    }

    /// Default location: `$XDG_CONFIG_HOME/helm/keys` (or ~/.config/helm/keys).
    pub fn default_dir() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|base| base.join("helm/keys"))
    }

    /// Path to the private key file — what `SshParams::key_path` wants.
    /// (File-impl specific: hardware stores have no extractable path; the
    /// mobile transports sign through the trait instead.)
    pub fn key_path(&self, alias: &str) -> PathBuf {
        self.dir.join(alias)
    }

    fn pins_path(&self) -> PathBuf {
        self.dir.join("pins.json")
    }

    fn load_pins(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.pins_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn store_pins(&self, pins: &BTreeMap<String, String>) -> Result<()> {
        let json = serde_json::to_string_pretty(pins)
            .map_err(|e| EngineError::Parse(format!("pins: {e}")))?;
        write_600(&self.pins_path(), json.as_bytes())
    }

    fn load_key(&self, alias: &str) -> Result<Option<PrivateKey>> {
        let path = self.key_path(alias);
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| EngineError::Parse(format!("read {}: {e}", path.display())))?;
        PrivateKey::from_openssh(&text)
            .map(Some)
            .map_err(|e| EngineError::Parse(format!("parse key {alias}: {e}")))
    }
}

fn write_600(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, bytes)
        .map_err(|e| EngineError::Parse(format!("write {}: {e}", path.display())))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| EngineError::Parse(format!("chmod {}: {e}", path.display())))?;
    Ok(())
}

impl KeyStore for FileKeyStore {
    fn generate_device_key(&self, alias: &str) -> Result<String> {
        if let Some(existing) = self.public_key(alias)? {
            return Ok(existing);
        }
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| EngineError::Parse(format!("os rng: {e}")))?;
        let keypair = Ed25519Keypair::from_seed(&seed);
        let mut key = PrivateKey::from(keypair);
        key.set_comment(format!("helm-device-{alias}"));

        let openssh = key
            .to_openssh(LineEnding::LF)
            .map_err(|e| EngineError::Parse(format!("encode key: {e}")))?;
        write_600(&self.key_path(alias), openssh.as_bytes())?;

        let public = key
            .public_key()
            .to_openssh()
            .map_err(|e| EngineError::Parse(format!("encode pubkey: {e}")))?;
        write_600(&self.dir.join(format!("{alias}.pub")), public.as_bytes())?;
        Ok(public)
    }

    fn public_key(&self, alias: &str) -> Result<Option<String>> {
        Ok(self
            .load_key(alias)?
            .map(|k| k.public_key().to_openssh().unwrap_or_default()))
    }

    fn sign(&self, alias: &str, data: &[u8]) -> Result<Vec<u8>> {
        let key = self
            .load_key(alias)?
            .ok_or_else(|| EngineError::NotFound(format!("device key {alias}")))?;
        let sig = key
            .sign("helm", HashAlg::Sha256, data)
            .map_err(|e| EngineError::Parse(format!("sign: {e}")))?;
        let pem = sig
            .to_pem(LineEnding::LF)
            .map_err(|e| EngineError::Parse(format!("encode sig: {e}")))?;
        Ok(pem.into_bytes())
    }

    fn pinned_hostkey(&self, host: &str) -> Result<Option<String>> {
        Ok(self.load_pins().get(host).cloned())
    }

    fn pin_hostkey(&self, host: &str, fp: &str) -> Result<()> {
        let mut pins = self.load_pins();
        pins.insert(host.to_string(), fp.to_string());
        self.store_pins(&pins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> FileKeyStore {
        let dir = std::env::temp_dir().join(format!("helm-ks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        FileKeyStore::new(dir).unwrap()
    }

    #[test]
    fn generate_is_idempotent_and_loadable_by_russh() {
        let ks = store("gen");
        let pk1 = ks.generate_device_key("phone").unwrap();
        let pk2 = ks.generate_device_key("phone").unwrap();
        assert_eq!(pk1, pk2, "second call must return the same key");
        assert!(pk1.starts_with("ssh-ed25519 "));

        // The stored file is a valid OpenSSH identity (what SshParams needs).
        let loaded = russh::keys::load_secret_key(ks.key_path("phone"), None).unwrap();
        assert_eq!(
            loaded.public_key().to_openssh().unwrap(),
            pk1.trim_end().to_string()
        );
    }

    #[test]
    fn sign_produces_sshsig_pem() {
        let ks = store("sign");
        ks.generate_device_key("phone").unwrap();
        let sig = ks.sign("phone", b"challenge-bytes").unwrap();
        let text = String::from_utf8(sig).unwrap();
        assert!(text.starts_with("-----BEGIN SSH SIGNATURE-----"));
        assert!(ks.sign("missing", b"x").is_err());
    }

    #[test]
    fn pins_roundtrip() {
        let ks = store("pins");
        assert_eq!(ks.pinned_hostkey("100.1.2.3").unwrap(), None);
        ks.pin_hostkey("100.1.2.3", "SHA256:abc").unwrap();
        ks.pin_hostkey("100.9.9.9", "SHA256:def").unwrap();
        assert_eq!(
            ks.pinned_hostkey("100.1.2.3").unwrap().as_deref(),
            Some("SHA256:abc")
        );
        // Overwrite on re-pair.
        ks.pin_hostkey("100.1.2.3", "SHA256:new").unwrap();
        assert_eq!(
            ks.pinned_hostkey("100.1.2.3").unwrap().as_deref(),
            Some("SHA256:new")
        );
    }
}
