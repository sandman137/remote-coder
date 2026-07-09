//! Pairing round-trip (DESIGN.md §8.3 acceptance): device keystore →
//! enroll over the one-time token channel → authorized_keys gains the
//! forced-command line → host key pinned on the device → revocation
//! removes access. Host side = broker::enroll, client side =
//! engine::security::pairing — this test pins their wire format together.

use std::net::TcpListener;
use std::time::Duration;

use engine::security::pairing::{enroll, PairPayload};
use engine::{FileKeyStore, KeyStore};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("helm-pair-{name}-{}", std::process::id()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_round_trip_and_revocation() {
    let dir = tmp("rt");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let authorized_keys = dir.join("authorized_keys");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let enroll_port = listener.local_addr().unwrap().port();
    let token = broker::enroll::generate_token();

    // Host side: serve one enrollment in a thread (std, blocking).
    let host_ak = authorized_keys.clone();
    let host_token = token.clone();
    let server = std::thread::spawn(move || {
        broker::enroll::serve_enroll_once(
            listener,
            &host_token,
            Duration::from_secs(20),
            &host_ak,
            "/opt/helm/broker --session agents",
        )
    });

    // Client side: keystore + enroll.
    let keystore = FileKeyStore::new(dir.join("keys")).unwrap();
    let payload = PairPayload {
        v: 1,
        host: "127.0.0.1".into(),
        port: 22,
        user: "dev".into(),
        hostkey_fp: "SHA256:pinnedvalue".into(),
        enroll: token.clone(),
        enroll_port,
        ttl: 600,
    };

    let pubkey = enroll(&payload, &keystore, "pixel8").await.unwrap();
    let device = server.join().unwrap().unwrap();
    assert_eq!(device, "pixel8");

    // authorized_keys carries the forced command + restrictions + our key.
    let content = std::fs::read_to_string(&authorized_keys).unwrap();
    assert!(content.contains("command=\"/opt/helm/broker --session agents\""));
    assert!(content.contains("no-pty"));
    let key_material = pubkey.split_whitespace().nth(1).unwrap();
    assert!(content.contains(key_material));
    assert!(content.trim_end().ends_with("helm:pixel8"));

    // Host key pinned on the device.
    assert_eq!(
        keystore.pinned_hostkey("127.0.0.1").unwrap().as_deref(),
        Some("SHA256:pinnedvalue")
    );

    // Revocation removes the device line.
    assert!(broker::enroll::revoke(&authorized_keys, "pixel8").unwrap());
    let content = std::fs::read_to_string(&authorized_keys).unwrap();
    assert!(!content.contains("helm:pixel8"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_token_is_rejected_and_does_not_consume() {
    let dir = tmp("bad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let authorized_keys = dir.join("authorized_keys");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let enroll_port = listener.local_addr().unwrap().port();
    let token = broker::enroll::generate_token();

    let host_ak = authorized_keys.clone();
    let host_token = token.clone();
    let server = std::thread::spawn(move || {
        broker::enroll::serve_enroll_once(
            listener,
            &host_token,
            Duration::from_secs(20),
            &host_ak,
            "/opt/helm/broker",
        )
    });

    let keystore = FileKeyStore::new(dir.join("keys")).unwrap();
    let mut payload = PairPayload {
        v: 1,
        host: "127.0.0.1".into(),
        port: 22,
        user: "dev".into(),
        hostkey_fp: "SHA256:x".into(),
        enroll: "wrong-token".into(),
        enroll_port,
        ttl: 600,
    };

    // Wrong token: client errors, nothing appended, nothing pinned.
    let err = enroll(&payload, &keystore, "mallory").await.unwrap_err();
    assert!(err.to_string().contains("bad token"), "{err}");
    assert!(!std::fs::read_to_string(&authorized_keys)
        .unwrap_or_default()
        .contains("helm:mallory"));
    assert_eq!(keystore.pinned_hostkey("127.0.0.1").unwrap(), None);

    // The token wasn't consumed: the real device still enrolls.
    payload.enroll = token;
    enroll(&payload, &keystore, "pixel8").await.unwrap();
    assert_eq!(server.join().unwrap().unwrap(), "pixel8");
    assert!(std::fs::read_to_string(&authorized_keys)
        .unwrap()
        .contains("helm:pixel8"));

    std::fs::remove_dir_all(&dir).ok();
}
