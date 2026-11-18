//! The service's pipe server, run in-process with a temporary layout: the
//! owner-only pipe, both identity checks, data files, config sanitising and
//! a real prism-core (without TUN, so no administrator rights needed).
//! Needs target/debug/prism-core(.exe) (`cargo build -p prism-core`).
//! On macOS the same runs over a unix socket.
#![cfg(any(windows, target_os = "macos"))]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use prism_service::client::{current_user_sid, request_at};
use prism_service::{server, sha256_file, Layout, Owner, Request};

#[cfg(windows)]
fn test_pipe(tag: &str) -> String {
    format!(r"\\.\pipe\prism-service-test-{tag}-{}", std::process::id())
}

/// Short: unix socket paths are limited to about 100 bytes.
#[cfg(not(windows))]
fn test_pipe(tag: &str) -> String {
    format!("/tmp/prism-svc-{tag}-{}.sock", std::process::id())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn layout(tag: &str, gui: PathBuf) -> Layout {
    let root = std::env::temp_dir().join(format!("prism-service-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let l = Layout {
        program: root.join("program"),
        data: root.join("data"),
        pipe: test_pipe(tag),
        service_exe: std::env::current_exe().unwrap(),
    };
    std::fs::create_dir_all(l.cores_dir()).unwrap();
    std::fs::create_dir_all(l.run_dir()).unwrap();
    let owner = Owner { user_sid: current_user_sid().unwrap(), gui_path: gui };
    std::fs::write(l.owner_file(), serde_json::to_vec(&owner).unwrap()).unwrap();
    l
}

async fn wait_ready(l: &Layout) {
    let mut last = String::new();
    for _ in 0..50 {
        match request_at(l, &Request::Status).await {
            Ok(_) => return,
            Err(e) => last = format!("{e:#}"),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("pipe server did not come up: {last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_a_core_for_its_owner_only() {
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let core = target.join("debug").join(format!("prism-core{}", prism_service::EXE));
    if !core.exists() {
        eprintln!("skipped: build prism-core first ({})", core.display());
        return;
    }
    let me = std::env::current_exe().unwrap();
    let l = layout("owner", me.clone());
    std::fs::copy(&core, l.cores_dir().join(format!("prism-core{}", prism_service::EXE))).unwrap();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let srv = tokio::spawn(server::serve(l.clone(), stop_rx));
    wait_ready(&l).await;

    let st = request_at(&l, &Request::Status).await.unwrap().status.unwrap();
    assert!(!st.running);
    assert_eq!(st.cores.get("prism"), sha256_file(&core).as_ref());
    assert!(!st.cores.contains_key("mihomo"));
    #[cfg(windows)] {
        assert!(st.kill_switch_apps_supported);
        let r = request_at(&l, &Request::SetKillSwitchApps { paths: vec![me.to_string_lossy().into_owned()] }).await.unwrap();
        assert!(r.ok);
        assert_eq!(r.status.unwrap().kill_switch_apps.len(), 1);
        let r = request_at(&l, &Request::SetKillSwitchApps { paths: vec!["*.exe".into()] }).await.unwrap();
        assert!(!r.ok);
        let st = request_at(&l, &Request::Status).await.unwrap().status.unwrap();
        assert_eq!(st.kill_switch_apps.len(), 1, "invalid updates must preserve the previous policy");
        assert!(request_at(&l, &Request::SetKillSwitchApps { paths: vec![] }).await.unwrap().ok);
    }

    // Files outside the allowed list are refused before anything starts.
    let b64 = |s: &[u8]| base64::engine::general_purpose::STANDARD.encode(s);
    let evil = BTreeMap::from([("../evil.dll".to_string(), b64(b"x"))]);
    let r = request_at(&l, &Request::Start { core: "prism".into(), config: "{}".into(), files: evil, kill_switch: false, tailscale: false, kill_switch_apps: vec![], recovery: false }).await.unwrap();
    assert!(!r.ok && r.error.unwrap().contains("not allowed"));
    let r = request_at(&l, &Request::Start { core: "mihomo".into(), config: "{}".into(), files: BTreeMap::new(), kill_switch: false, tailscale: false, kill_switch_apps: vec![], recovery: false }).await.unwrap();
    assert!(!r.ok, "mihomo isn't installed");

    let (mixed, api) = (free_port(), free_port());
    let config = format!(
        "mixed-port: {mixed}\nexternal-controller: 127.0.0.1:{api}\nsecret: svc-test\n\
         proxy-providers:\n  p: {{ type: http, url: 'http://127.0.0.1:1/x', path: 'C:\\Windows\\win.ini', interval: 0 }}\n\
         proxies:\n  - {{name: n, type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: x}}\n\
         rules:\n  - MATCH,DIRECT\n"
    );
    let files = BTreeMap::from([("geosite.dat".to_string(), b64(b"not really geodata"))]);
    let r = request_at(&l, &Request::Start { core: "prism".into(), config, files, kill_switch: false, tailscale: false, kill_switch_apps: vec![], recovery: false }).await.unwrap();
    assert!(r.ok, "{:?}", r.error);
    let st = r.status.unwrap();
    assert!(st.running && st.core.as_deref() == Some("prism") && st.pid.is_some());
    assert!(st.files.contains_key("geosite.dat"));
    let written = std::fs::read_to_string(l.run_dir().join("config.yaml")).unwrap();
    assert!(written.contains("./providers/proxies/p") && !written.contains("win.ini"), "{written}");

    // The core really runs: its API answers.
    let mut up = false;
    for _ in 0..50 {
        let r = tokio::net::TcpStream::connect(("127.0.0.1", api)).await;
        if r.is_ok() {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(up, "core API on {api} never came up");

    let lines = request_at(&l, &Request::Logs { since: 0 }).await.unwrap().lines;
    assert!(lines.iter().any(|(_, s)| s.contains("[service]")), "{lines:?}");
    let last = lines.last().unwrap().0;
    assert!(request_at(&l, &Request::Logs { since: last }).await.unwrap().lines.iter().all(|(n, _)| *n > last));

    assert!(request_at(&l, &Request::Stop).await.unwrap().ok);
    let st = request_at(&l, &Request::Status).await.unwrap().status.unwrap();
    assert!(!st.running);

    stop_tx.send(true).unwrap();
    srv.await.unwrap().unwrap();
    let _ = std::fs::remove_dir_all(l.data.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_other_programs() {
    // Registered for a different Prism: this test binary is turned away.
    let other = if cfg!(windows) { r"C:\Program Files\Prism\prism.exe" } else { "/Applications/Prism.app/Contents/MacOS/prism" };
    let l = layout("stranger", PathBuf::from(other));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let srv = tokio::spawn(server::serve(l.clone(), stop_rx));
    wait_ready(&l).await; // answers, if only with a refusal
    let r = request_at(&l, &Request::Status).await.unwrap();
    assert!(!r.ok && r.error.as_deref().unwrap_or("").contains("登记"), "{r:?}");
    stop_tx.send(true).unwrap();
    srv.await.unwrap().unwrap();

    // A client expecting the real service exe refuses a server that isn't it
    // (Windows; on macOS the client requires a root peer instead).
    #[cfg(windows)]
    {
        let l2 = layout("impostor", std::env::current_exe().unwrap());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let srv = tokio::spawn(server::serve(l2.clone(), stop_rx));
        wait_ready(&l2).await;
        let expecting_real = Layout { service_exe: prism_service::service_exe(), ..l2.clone() };
        let err = request_at(&expecting_real, &Request::Status).await.unwrap_err();
        assert!(format!("{err:#}").contains("不是由 Prism 服务创建的"), "{err:#}");
        stop_tx.send(true).unwrap();
        srv.await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(l2.data.parent().unwrap());
    }
    let _ = std::fs::remove_dir_all(l.data.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_recovery_preserves_reload_and_selection_and_manual_stop_wins() {
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let source = target.join("debug").join(format!("prism-core{}", prism_service::EXE));
    assert!(source.exists(), "build prism-core before this test");
    let me = std::env::current_exe().unwrap(); let l = layout("recovery", me);
    let executable = l.cores_dir().join(format!("prism-core{}", prism_service::EXE));
    std::fs::copy(source, &executable).unwrap();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let service = tokio::spawn(server::serve(l.clone(), stop_rx)); wait_ready(&l).await;
    let ctl = free_port();
    let config = format!("mixed-port: {}\nexternal-controller: 127.0.0.1:{ctl}\nsecret: recovery-test\nproxies: [{{name: local, type: socks5, server: 127.0.0.1, port: 9}}]\nproxy-groups: [{{name: choice, type: select, proxies: [local, DIRECT]}}]\nrules: ['MATCH,choice']\n", free_port());
    let started = request_at(&l, &Request::Start {core:"prism".into(),config:config.clone(),files:BTreeMap::new(),kill_switch:false,tailscale:false,kill_switch_apps:vec![],recovery:true}).await.unwrap();
    assert!(started.ok, "{:?}", started.error);
    let original = started.status.unwrap(); assert!(original.recovery.enabled); let pid = original.pid.unwrap();
    for (method, path, body) in [
        ("PUT", "/proxies/choice", serde_json::json!({"name":"DIRECT"})),
        ("PATCH", "/configs", serde_json::json!({"mode":"global"})),
    ] {
        let result = request_at(&l, &Request::CoreMutation {method:method.into(),path:path.into(),body}).await.unwrap();
        assert!(result.ok, "{:?}", result.error);
    }
    let updated = format!("{config}\nhosts: {{reload-check.invalid: 203.0.113.77}}\n");
    let result = request_at(&l, &Request::Reload {config:updated,files:BTreeMap::new()}).await.unwrap();
    assert!(result.ok, "{:?}", result.error);
    assert!(std::fs::read_to_string(l.run_dir().join("config.yaml")).unwrap().contains("reload-check.invalid"));
    let bad = request_at(&l, &Request::Reload {config:"not a config".into(),files:BTreeMap::new()}).await.unwrap();
    assert!(!bad.ok);
    // Terminate only the exact isolated child whose image matches this fixture.
    assert!(prism_service::client::same_path(&prism_service::client::process_image(pid).unwrap(), &executable));
    #[cfg(windows)] unsafe {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE}};
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid); assert!(!handle.is_null());
        assert_ne!(TerminateProcess(handle, 77), 0); CloseHandle(handle);
    }
    #[cfg(not(windows))] unsafe { assert_eq!(libc::kill(pid as i32, libc::SIGKILL), 0); }
    let mut recovered = None;
    for _ in 0..300 {
        let state = request_at(&l, &Request::Status).await.unwrap().status.unwrap();
        if state.running && state.pid != Some(pid) && state.recovery.restarts == 1 && state.recovery.phase == "monitoring" { recovered = Some(state); break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let state = recovered.expect("service did not independently restore the child");
    assert_eq!(state.recovery.last_exit_pid, Some(pid)); assert!(state.recovery.incident.is_some());
    let http = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap();
    let cfg: serde_json::Value = http.get(format!("http://127.0.0.1:{ctl}/configs")).bearer_auth("recovery-test").send().await.unwrap().json().await.unwrap();
    assert_eq!(cfg["mode"], "global");
    let choice: serde_json::Value = http.get(format!("http://127.0.0.1:{ctl}/proxies/choice")).bearer_auth("recovery-test").send().await.unwrap().json().await.unwrap();
    assert_eq!(choice["now"], "DIRECT");
    assert!(!serde_json::to_string(&state.recovery).unwrap().contains("recovery-test"));
    assert!(request_at(&l, &Request::Stop).await.unwrap().ok);
    tokio::time::sleep(Duration::from_millis(2300)).await;
    let stopped = request_at(&l, &Request::Status).await.unwrap().status.unwrap();
    assert!(!stopped.running && !stopped.recovery.enabled);
    stop_tx.send(true).unwrap(); service.await.unwrap().unwrap();
    std::fs::remove_dir_all(l.data.parent().unwrap()).unwrap();
}
