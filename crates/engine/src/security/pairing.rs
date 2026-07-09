//! Client side of pairing (DESIGN.md §8.3). The QR payload tells the device
//! where the host is and carries a one-time enroll token + host key
//! fingerprint; the device generates its key in the keystore (private key
//! never leaves), submits the public key over the enroll channel, and pins
//! the host key. Wire format: one JSON line each way (host side lives in
//! `broker::enroll`).

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::KeyStore;
use crate::error::{EngineError, Result};
use crate::transport::SshParams;

/// The QR payload shown by `helm pair` (§8.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairPayload {
    pub v: u8,
    pub host: String,
    /// sshd port for the eventual transport connection.
    pub port: u16,
    pub user: String,
    /// Host key fingerprint to pin ("SHA256:…").
    pub hostkey_fp: String,
    /// One-time enroll token.
    pub enroll: String,
    /// TCP port of the transient enroll listener on `host`.
    pub enroll_port: u16,
    /// Token lifetime, seconds (informational for the client).
    pub ttl: u64,
}

impl PairPayload {
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s.trim())
            .map_err(|e| EngineError::Parse(format!("pairing payload: {e}")))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| EngineError::Parse(format!("pairing payload: {e}")))
    }
}

#[derive(Serialize)]
struct EnrollRequest<'a> {
    v: u8,
    token: &'a str,
    device: &'a str,
    pubkey: &'a str,
}

#[derive(Deserialize)]
struct EnrollResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Enroll this device: generate/reuse the device key, submit it with the
/// one-time token, pin the host key. Returns the public key that was
/// enrolled.
pub async fn enroll(
    payload: &PairPayload,
    keystore: &dyn KeyStore,
    device: &str,
) -> Result<String> {
    if payload.v != 1 {
        return Err(EngineError::Parse(format!(
            "unsupported pairing payload version {}",
            payload.v
        )));
    }
    let pubkey = keystore.generate_device_key(device)?;

    let stream = tokio::net::TcpStream::connect((payload.host.as_str(), payload.enroll_port))
        .await
        .map_err(|e| {
            EngineError::Parse(format!(
                "enroll connect {}:{}: {e}",
                payload.host, payload.enroll_port
            ))
        })?;
    let mut stream = BufReader::new(stream);

    let request = serde_json::to_string(&EnrollRequest {
        v: payload.v,
        token: &payload.enroll,
        device,
        pubkey: pubkey.trim(),
    })
    .map_err(|e| EngineError::Parse(format!("encode enroll request: {e}")))?;
    stream
        .get_mut()
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|e| EngineError::Parse(format!("enroll send: {e}")))?;

    let mut line = String::new();
    stream
        .read_line(&mut line)
        .await
        .map_err(|e| EngineError::Parse(format!("enroll read: {e}")))?;
    let response: EnrollResponse = serde_json::from_str(line.trim())
        .map_err(|e| EngineError::Parse(format!("enroll response {line:?}: {e}")))?;
    if !response.ok {
        return Err(EngineError::Parse(format!(
            "enrollment rejected: {}",
            response.error.unwrap_or_else(|| "unknown".into())
        )));
    }

    keystore.pin_hostkey(&payload.host, &payload.hostkey_fp)?;
    Ok(pubkey)
}

/// SSH connection parameters for a paired host (file keystore variant —
/// the key path comes from the store).
pub fn ssh_params_for(
    payload: &PairPayload,
    keystore: &super::FileKeyStore,
    device: &str,
) -> SshParams {
    let mut params = SshParams::new(
        payload.host.clone(),
        payload.port,
        payload.user.clone(),
        keystore.key_path(device).display().to_string(),
    );
    params.hostkey_fp = Some(payload.hostkey_fp.clone());
    params
}
