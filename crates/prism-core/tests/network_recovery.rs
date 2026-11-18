//! Recovery uses the same Hub reload path as the Windows network monitor.
use std::sync::Arc;
use prism_core::{config, hub::{Hub, Source}};

#[tokio::test]
async fn repeated_recovery_preserves_routing_and_fakeip_but_replaces_sessions() {
    let dir = std::env::temp_dir().join(format!("prism-recovery-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    let raw = "proxies:\n  - {name: local, type: socks5, server: 127.0.0.1, port: 9}\nproxy-groups:\n  - {name: 节点选择, type: select, proxies: [DIRECT, local]}\nrules: ['MATCH,节点选择']\ndns:\n  enable: true\n  enhanced-mode: fake-ip\n  fake-ip-range: 198.18.0.1/16\n  nameserver: [127.0.0.1]\n";
    std::fs::write(&path, raw).unwrap();
    let hub = Hub::new(Source { path: Some(path.clone()), ..Default::default() }, dir.clone(), config::parse_any(raw).unwrap()).await.unwrap();
    let original = hub.core();
    original.tunnel.reg.get("节点选择").unwrap().as_group().unwrap().select("local").unwrap();
    *original.tunnel.mode.write().unwrap() = config::Mode::Global;
    for _ in 0..20 {
        hub.reload(None).await.unwrap();
        let current = hub.core();
        assert!(!Arc::ptr_eq(&original.tunnel.reg, &current.tunnel.reg));
        assert!(Arc::ptr_eq(&original.tunnel.stats, &current.tunnel.stats));
        assert!(Arc::ptr_eq(original.dns.as_ref().unwrap().fake.as_ref().unwrap(), current.dns.as_ref().unwrap().fake.as_ref().unwrap()));
        assert_eq!(current.tunnel.reg.get("节点选择").unwrap().as_group().unwrap().now(), "local");
        assert_eq!(current.tunnel.mode(), config::Mode::Global);
    }
    let working = hub.core();
    std::fs::write(&path, "not a valid config").unwrap();
    assert!(hub.reload(None).await.is_err());
    assert!(Arc::ptr_eq(&working, &hub.core()), "failed recovery must retain the working core");
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn automatic_repair_keeps_the_last_successful_payload_and_releases_old_core() {
    let dir = std::env::temp_dir().join(format!("prism-repair-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.yaml");
    let raw = "proxies: []\nrules: ['MATCH,DIRECT']\n";
    std::fs::write(&path, raw).unwrap();
    let hub = Hub::new(Source {path:Some(path.clone()),..Default::default()}, dir.clone(), config::parse_any(raw).unwrap()).await.unwrap();
    let updated = "proxies: [{name: fresh, type: socks5, server: 127.0.0.1, port: 9}]\nproxy-groups: [{name: choice, type: select, proxies: [DIRECT, fresh]}]\nrules: ['MATCH,choice']\n";
    hub.reload(Some(updated.into())).await.unwrap();
    hub.core().tunnel.reg.get("choice").unwrap().as_group().unwrap().select("fresh").unwrap();
    *hub.core().tunnel.mode.write().unwrap() = config::Mode::Global;
    // Disk is deliberately stale, then broken: neither can replace live state
    // during a network repair. Explicit reloads still validate disk contents.
    for disk in [raw, "not a valid config"] {
        std::fs::write(&path, disk).unwrap();
        let old = Arc::downgrade(&hub.core());
        hub.repair().await.unwrap();
        assert!(old.upgrade().is_none(), "unused old Core must be released after repair");
        let current = hub.core();
        assert_eq!(current.tunnel.mode(), config::Mode::Global);
        assert_eq!(current.tunnel.reg.get("choice").unwrap().as_group().unwrap().now(), "fresh");
    }
    assert!(hub.reload(None).await.is_err());
    assert!(hub.reload(Some("broken payload".into())).await.is_err());
    hub.repair().await.unwrap();
    assert!(hub.core().tunnel.reg.get("fresh").is_some(), "failed reload must not poison cached profile");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn repair_api_works_without_a_config_file_and_requires_authentication() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let raw = "secret: repair-test\nproxies: []\nrules: ['MATCH,DIRECT']\n";
    let hub = Hub::new(Source::default(), std::env::temp_dir(), config::parse_any(raw).unwrap()).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = prism_core::api::router(hub.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    for authenticated in [false, true] {
        let previous = hub.core();
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        let auth = if authenticated { "Authorization: Bearer repair-test\r\n" } else { "" };
        let body = r#"{"repair":true}"#;
        socket.write_all(format!("PUT /configs HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        let mut response = Vec::new(); socket.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with(if authenticated { "HTTP/1.1 204" } else { "HTTP/1.1 401" }), "{response}");
        assert_eq!(Arc::ptr_eq(&previous, &hub.core()), !authenticated);
    }
    server.abort(); let _ = server.await;
}
