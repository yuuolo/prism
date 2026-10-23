//! Opt-in: update verification with the real updater plugin and the real
//! updater config (public key, requireSignedVersion) from tauri.conf.json,
//! against a local HTTP server. Needs a signed build first:
//!   npm run bundle
//!   PRISM_UPDATE_TEST=1 cargo test -p prism --test updater

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri_plugin_updater::UpdaterExt;

/// Serves fixed bodies by path on `l`; one request per connection.
fn serve(l: TcpListener, routes: HashMap<String, Vec<u8>>) {
    let routes = Arc::new(routes);
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) <= 2 {
                        break;
                    }
                }
                let mut s = s;
                match routes.get(&path) {
                    Some(body) => {
                        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = s.write_all(body);
                    }
                    None => {
                        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
        }
    });
}

#[test]
fn verifies_signed_updates() {
    if std::env::var("PRISM_UPDATE_TEST").is_err() {
        eprintln!("PRISM_UPDATE_TEST not set, skipping");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let conf: Value = serde_json::from_str(&std::fs::read_to_string(root.join("tauri.conf.json")).unwrap()).unwrap();
    let nsis = root.join("../target/release/bundle/nsis");
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(nsis.join("latest.json")).unwrap()).unwrap();
    let version = manifest["version"].as_str().unwrap().to_string();
    let installer = std::fs::read(nsis.join(format!("Prism_{version}_x64-setup.exe"))).unwrap();
    let signature = manifest["platforms"]["windows-x86_64"]["signature"].as_str().unwrap().to_string();
    let mut tampered = installer.clone();
    let mid = tampered.len() / 2;
    tampered[mid] ^= 0x01;

    // Manifests point back at this server, so bind before building them.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let manifest_for = |v: &str, file: &str| {
        serde_json::to_vec(&json!({
            "version": v,
            "notes": "test",
            "pub_date": "2026-10-24T00:00:00Z",
            "platforms": { "windows-x86_64": { "signature": signature, "url": format!("http://127.0.0.1:{port}/{file}") } }
        }))
        .unwrap()
    };
    let mut routes = HashMap::new();
    routes.insert("/setup.exe".to_string(), installer.clone());
    routes.insert("/tampered.exe".to_string(), tampered);
    routes.insert("/good.json".to_string(), manifest_for(&version, "setup.exe"));
    routes.insert("/tampered.json".to_string(), manifest_for(&version, "tampered.exe"));
    // A newer version number paired with an older signed artifact.
    routes.insert("/replay.json".to_string(), manifest_for("9.9.9", "setup.exe"));
    routes.insert("/old.json".to_string(), manifest_for("0.0.1", "setup.exe"));
    serve(listener, routes);

    // Mock app (version 0.1.0) with the real updater config.
    let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
    ctx.config_mut().plugins.0.insert("updater".into(), conf["plugins"]["updater"].clone());
    let app = tauri::test::mock_builder().plugin(tauri_plugin_updater::Builder::new().build()).build(ctx).unwrap();
    let handle = app.handle().clone();

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async move {
        let check = |path: &str| {
            let url: tauri::Url = format!("http://127.0.0.1:{port}{path}").parse().unwrap();
            let b = handle.updater_builder().no_proxy().endpoints(vec![url]).unwrap().build().unwrap();
            async move { b.check().await }
        };

        let good = check("/good.json").await.unwrap().expect("update offered");
        assert_eq!(good.version, version);
        let bytes = good.download(|_, _| {}, || {}).await.expect("genuine signed installer verifies");
        assert_eq!(bytes, installer);
        println!("ok: signed v{version} installer downloaded and verified ({} bytes)", bytes.len());

        let bad = check("/tampered.json").await.unwrap().expect("update offered");
        let err = bad.download(|_, _| {}, || {}).await.expect_err("tampered installer must be rejected");
        println!("ok: tampered installer rejected: {err}");

        let replay = check("/replay.json").await.unwrap().expect("update offered");
        let err = replay.download(|_, _| {}, || {}).await.expect_err("version mismatch must be rejected");
        println!("ok: 9.9.9 manifest pointing at the v{version} artifact rejected: {err}");

        assert!(check("/old.json").await.unwrap().is_none());
        println!("ok: older version is not offered");
    });
}
