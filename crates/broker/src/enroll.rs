//! Host-side pairing (DESIGN.md §8.3): one-time enroll tokens, an
//! authorized_keys manager (append with forced command / revoke by device),
//! and a tiny single-shot TCP enroll listener the `helm pair` command runs
//! inside the tailnet. std-only — the broker stays lean.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

/// Marker comment identifying HELM-managed authorized_keys lines.
fn marker(device: &str) -> String {
    format!("helm:{device}")
}

pub fn generate_token() -> String {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).expect("os rng");
    // URL-safe hex; no external deps.
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Append a device key restricted to the broker. `broker_cmd` is the full
/// forced command (binary + args, already quoted as needed).
pub fn add_authorized_key(
    path: &Path,
    device: &str,
    pubkey_line: &str,
    broker_cmd: &str,
) -> std::io::Result<()> {
    let pubkey = pubkey_line.trim();
    // Key material only — strip any client-supplied comment.
    let mut fields = pubkey.split_whitespace();
    let (algo, key) = (
        fields.next().unwrap_or_default(),
        fields.next().unwrap_or_default(),
    );
    if algo.is_empty() || key.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "malformed public key",
        ));
    }
    let line = format!(
        "command=\"{broker_cmd}\",no-port-forwarding,no-x11-forwarding,no-agent-forwarding,no-pty {algo} {key} {}\n",
        marker(device)
    );

    // Revoke any previous key for this device, then append.
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut kept: String = existing
        .lines()
        .filter(|l| !l.trim_end().ends_with(&marker(device)))
        .map(|l| format!("{l}\n"))
        .collect();
    kept.push_str(&line);
    write_600(path, &kept)
}

/// Remove a device's line. Returns whether anything was removed.
pub fn revoke(path: &Path, device: &str) -> std::io::Result<bool> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let kept: String = existing
        .lines()
        .filter(|l| !l.trim_end().ends_with(&marker(device)))
        .map(|l| format!("{l}\n"))
        .collect();
    let removed = kept != existing;
    if removed {
        write_600(path, &kept)?;
    }
    Ok(removed)
}

/// Devices currently authorized via HELM-managed lines.
pub fn list_devices(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            l.rsplit_once(" helm:")
                .map(|(_, device)| device.trim().to_string())
        })
        .collect()
}

fn write_600(path: &Path, content: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// One line of JSON each way. Wire format shared with
/// `engine::security::pairing` (duplicated by hand — the broker takes no
/// engine dependency; the pairing round-trip test pins them together).
#[derive(Debug)]
pub struct EnrollRequest {
    pub token: String,
    pub device: String,
    pub pubkey: String,
}

/// Serve exactly one successful enrollment, then return the device name.
/// Invalid attempts are answered with an error and do not consume the token.
pub fn serve_enroll_once(
    listener: TcpListener,
    token: &str,
    ttl: Duration,
    authorized_keys: &Path,
    broker_cmd: &str,
) -> std::io::Result<String> {
    let deadline = Instant::now() + ttl;
    listener.set_nonblocking(false)?;

    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "enroll ttl expired")
            })?;
        // Accept with a coarse timeout by polling the listener.
        listener.set_nonblocking(true)?;
        let conn = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "enroll ttl expired",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e),
            }
        };
        listener.set_nonblocking(false)?;
        conn.set_read_timeout(Some(remaining.min(Duration::from_secs(10))))?;

        let mut reader = BufReader::new(conn);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            continue;
        }
        let mut conn = reader.into_inner();

        match parse_request(&line) {
            Ok(req) if req.token == token => {
                add_authorized_key(authorized_keys, &req.device, &req.pubkey, broker_cmd)?;
                let _ = conn.write_all(b"{\"ok\":true}\n");
                return Ok(req.device);
            }
            Ok(_) => {
                let _ = conn.write_all(b"{\"ok\":false,\"error\":\"bad token\"}\n");
            }
            Err(e) => {
                let _ = conn.write_all(format!("{{\"ok\":false,\"error\":\"{e}\"}}\n").as_bytes());
            }
        }
    }
}

/// Minimal JSON field extraction (flat object of string fields) — avoids a
/// serde dependency in the broker. Escapes inside values are not supported;
/// tokens/devices/pubkeys are plain [A-Za-z0-9 +/=@.:_-].
fn parse_request(line: &str) -> Result<EnrollRequest, String> {
    let field = |name: &str| -> Result<String, String> {
        let needle = format!("\"{name}\"");
        let at = line
            .find(&needle)
            .ok_or_else(|| format!("missing {name}"))?;
        let rest = &line[at + needle.len()..];
        let colon = rest.find(':').ok_or("malformed json")?;
        let rest = rest[colon + 1..].trim_start();
        let rest = rest.strip_prefix('"').ok_or("expected string value")?;
        let end = rest.find('"').ok_or("unterminated string")?;
        Ok(rest[..end].to_string())
    };
    Ok(EnrollRequest {
        token: field("token")?,
        device: field("device")?,
        pubkey: field("pubkey")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("helm-enroll-{name}-{}", std::process::id()))
    }

    #[test]
    fn add_revoke_list_roundtrip() {
        let path = tmp("ak");
        let _ = std::fs::remove_file(&path);

        add_authorized_key(
            &path,
            "pixel8",
            "ssh-ed25519 AAAATESTKEY client-comment",
            "/opt/helm/broker --session agents",
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("command=\"/opt/helm/broker --session agents\""));
        assert!(content.contains("no-pty"));
        assert!(content.contains("ssh-ed25519 AAAATESTKEY helm:pixel8"));
        assert!(
            !content.contains("client-comment"),
            "client comment must be stripped"
        );

        // Re-enrolling the same device replaces its key.
        add_authorized_key(&path, "pixel8", "ssh-ed25519 NEWKEY x", "/opt/helm/broker").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches("helm:pixel8").count(), 1);
        assert!(content.contains("NEWKEY"));

        assert_eq!(list_devices(&path), vec!["pixel8"]);
        assert!(revoke(&path, "pixel8").unwrap());
        assert!(!revoke(&path, "pixel8").unwrap());
        assert_eq!(list_devices(&path), Vec::<String>::new());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn foreign_lines_survive_helm_management() {
        let path = tmp("foreign");
        std::fs::write(&path, "ssh-rsa USERKEY user@laptop\n").unwrap();
        add_authorized_key(&path, "d1", "ssh-ed25519 K1 c", "/b").unwrap();
        revoke(&path, "d1").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("USERKEY"));
        assert!(!content.contains("helm:"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parses_enroll_request() {
        let req = parse_request(
            r#"{"v":1,"token":"abc123","device":"pixel8","pubkey":"ssh-ed25519 AAAA dev"}"#,
        )
        .unwrap();
        assert_eq!(req.token, "abc123");
        assert_eq!(req.device, "pixel8");
        assert!(req.pubkey.starts_with("ssh-ed25519"));
        assert!(parse_request("{}").is_err());
    }

    #[test]
    fn tokens_are_long_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
    }
}
