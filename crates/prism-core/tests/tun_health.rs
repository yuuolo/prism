//! Fault injection without installing adapters or changing host routing.
use std::{sync::Arc, time::Duration};
use prism_core::{config, hub::{Hub, Source}, tun_health::{Health, Pump}};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn port() -> u16 { std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }
async fn request(port: u16, authorized: bool) -> String {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let auth = if authorized { "Authorization: Bearer never-export-token\r\n" } else { "" };
    s.write_all(format!("GET /prism/health HTTP/1.1\r\nHost: localhost\r\n{auth}Connection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut bytes = Vec::new(); s.read_to_end(&mut bytes).await.unwrap(); String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn failed_pump_exits_core_with_sanitized_snapshot_and_health_is_authenticated() {
    let folder = std::env::temp_dir().join(format!("prism-health-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).unwrap(); let ctl = port();
    let raw = format!("mixed-port: 0\nexternal-controller: 127.0.0.1:{ctl}\nsecret: never-export-token\nproxies: []\nrules: ['MATCH,DIRECT']\n");
    let hub = Hub::new(Source::default(), folder.clone(), config::parse_any(&raw).unwrap()).await.unwrap();
    let health = hub.tun_health.clone(); health.enable(); let guard = health.guard(Pump::Receive);
    let run = tokio::spawn(prism_core::run(hub));
    for _ in 0..100 { if tokio::net::TcpStream::connect(("127.0.0.1", ctl)).await.is_ok() { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
    assert!(request(ctl, false).await.starts_with("HTTP/1.1 401"));
    assert!(request(ctl, true).await.starts_with("HTTP/1.1 200"));
    health.received(100); health.sent(80); drop(guard);
    let result = tokio::time::timeout(Duration::from_secs(2), run).await.unwrap().unwrap();
    assert!(result.is_err(), "dead TUN worker must not leave a healthy-looking core running");
    let bytes = std::fs::read(folder.join("tun-health.json")).unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(snapshot["fault"]["stage"], "adapter_rx"); assert_eq!(snapshot["rx_packets"], 1);
    assert!(!String::from_utf8(bytes).unwrap().contains("never-export-token"));
    std::fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
async fn worker_panic_is_reported_without_network_side_effects() {
    let health = Arc::new(Health::default()); let guard = health.guard(Pump::FromStack);
    let task = tokio::spawn(async move { let _guard = guard; panic!("injected worker panic"); });
    assert!(task.await.is_err());
    assert!(health.failed.is_cancelled()); assert_eq!(health.snapshot().fault.unwrap().stage, "from_stack");
}

#[tokio::test]
async fn saved_mode_and_exit_are_applied_before_starting_listeners() {
    let raw = "proxies: [{name: local, type: socks5, server: 127.0.0.1, port: 9}]\nproxy-groups: [{name: choice, type: select, proxies: [DIRECT, local]}]\nrules: ['MATCH,choice']\n";
    let hub = Hub::new(Source::default(), std::env::temp_dir(), config::parse_any(raw).unwrap()).await.unwrap();
    hub.restore_state(serde_json::json!({"mode":"global","selections":{"choice":"local"}})).unwrap();
    assert_eq!(hub.core().tunnel.mode(), config::Mode::Global);
    assert_eq!(hub.core().tunnel.reg.get("choice").unwrap().as_group().unwrap().now(), "local");
    assert!(hub.restore_state(serde_json::json!({"mode":"rule","selections":{"choice":"missing"}})).is_err());
}
