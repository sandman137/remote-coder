//! Host-side `rcoder pair` / `rcoder revoke` / `rcoder devices` and the client
//! `rcoder enroll` dev command (DESIGN.md §8.3).

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use engine::security::pairing::{enroll, ssh_params_for, PairPayload};
use engine::FileKeyStore;

fn default_authorized_keys() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("$HOME not set")?;
    Ok(PathBuf::from(home).join(".ssh/authorized_keys"))
}

fn host_fingerprint(hostkey_pub: &PathBuf) -> Result<String> {
    let out = Command::new("ssh-keygen")
        .args(["-l", "-f"])
        .arg(hostkey_pub)
        .output()
        .context("run ssh-keygen -l")?;
    if !out.status.success() {
        bail!(
            "ssh-keygen -lf {} failed: {}",
            hostkey_pub.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_string)
        .context("no fingerprint in ssh-keygen output")
}

#[allow(clippy::too_many_arguments)]
pub fn pair(
    host: String,
    ssh_port: u16,
    user: Option<String>,
    authorized_keys: Option<PathBuf>,
    broker_path: String,
    session: String,
    enroll_port: u16,
    ttl_secs: u64,
    hostkey_pub: Option<PathBuf>,
) -> Result<()> {
    let user = user
        .or_else(|| std::env::var("USER").ok())
        .context("--user required")?;
    let authorized_keys = match authorized_keys {
        Some(p) => p,
        None => default_authorized_keys()?,
    };
    let hostkey_pub =
        hostkey_pub.unwrap_or_else(|| PathBuf::from("/etc/ssh/ssh_host_ed25519_key.pub"));
    let hostkey_fp = host_fingerprint(&hostkey_pub)?;

    let listener = TcpListener::bind(("0.0.0.0", enroll_port))
        .with_context(|| format!("bind enroll listener on :{enroll_port}"))?;
    let token = broker::enroll::generate_token();

    let payload = PairPayload {
        v: 1,
        host: host.clone(),
        port: ssh_port,
        user,
        hostkey_fp,
        enroll: token.clone(),
        enroll_port,
        ttl: ttl_secs,
    };
    let json = serde_json::to_string(&payload)?;

    // QR for the phone; JSON for the desktop `rcoder enroll` dev flow.
    let code = qrcode::QrCode::new(json.as_bytes()).context("QR encode")?;
    let art = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .build();
    println!("{art}\n");
    println!("Scan with the Remote Coder app, or on another machine:\n");
    println!("  rcoder enroll --device <name> --json '{json}'\n");
    println!(
        "Waiting for one device on :{enroll_port} (token valid {ttl_secs}s, Ctrl-C to abort)…"
    );

    let broker_cmd = format!("{broker_path} --session {session}");
    let device = broker::enroll::serve_enroll_once(
        listener,
        &token,
        Duration::from_secs(ttl_secs),
        &authorized_keys,
        &broker_cmd,
    )
    .context("enrollment")?;

    println!(
        "✔ enrolled device {device:?} in {}",
        authorized_keys.display()
    );
    println!("  revoke anytime with: rcoder revoke {device}");
    Ok(())
}

pub async fn enroll_cmd(json: String, device: String, keys_dir: Option<PathBuf>) -> Result<()> {
    let payload: PairPayload = serde_json::from_str(json.trim())
        .context("parse pairing payload JSON (from `rcoder pair`)")?;
    let dir = match keys_dir {
        Some(d) => d,
        None => FileKeyStore::default_dir().context("cannot determine key dir")?,
    };
    let keystore = FileKeyStore::new(dir)?;
    enroll(&payload, &keystore, &device).await?;

    let params = ssh_params_for(&payload, &keystore, &device);
    println!(
        "✔ enrolled as {device:?}; host key pinned ({})",
        payload.hostkey_fp
    );
    println!("\nConnect with:");
    println!(
        "  rcoder --transport ssh --host {} --port {} --user {} --key {} --hostkey-fp '{}' tui",
        params.host, params.port, params.user, params.key_path, payload.hostkey_fp
    );
    Ok(())
}

pub fn revoke(device: String, authorized_keys: Option<PathBuf>) -> Result<()> {
    let path = match authorized_keys {
        Some(p) => p,
        None => default_authorized_keys()?,
    };
    if broker::enroll::revoke(&path, &device)? {
        println!("✔ revoked {device:?}");
    } else {
        println!("no key found for device {device:?} in {}", path.display());
    }
    Ok(())
}

pub fn devices(authorized_keys: Option<PathBuf>) -> Result<()> {
    let path = match authorized_keys {
        Some(p) => p,
        None => default_authorized_keys()?,
    };
    let devices = broker::enroll::list_devices(&path);
    if devices.is_empty() {
        println!("(no paired devices)");
    }
    for d in devices {
        println!("{d}");
    }
    Ok(())
}
