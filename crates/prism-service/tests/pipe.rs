//! The service's pipe server, run in-process with a temporary layout: the
//! owner-only pipe, both identity checks, data files, config sanitising and
//! a real prism-core (without TUN, so no administrator rights needed).
//! Needs target/debug/prism-core.exe (`cargo build -p prism-core`).
#![cfg(windows)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use prism_service::client::{current_user_sid, request_at};
use prism_service::{server, sha256_file, Layout, Owner, Request};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn layout(tag: &str, gui: PathBuf) -> Layout {
    let root = std::env::temp_dir().join(format!("prism-service-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let l = Layout {
        program: root.join("program"),
        data: root.join("data"),
        pipe: format!(r"\\.\pipe\prism-service-test-{tag}-{}", std::process::id()),
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
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/prism-core.exe");
    if !core.exists() {
        eprintln!("skipped: build prism-core first ({})", core.display());
        return;
    }
    let me = std::env::current_exe().unwrap();
    let l = layout("owner", me.clone());
    std::fs::copy(&core, l.cores_dir().join("prism-core.exe")).unwrap();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let srv = tokio::spawn(server::serve(l.clone(), stop_rx));
    wait_ready(&l).await;

    let st = request_at(&l, &Request::Status).await.unwrap().status.unwrap();
    assert!(!st.running);
    assert_eq!(st.cores.get("prism"), sha256_file(&core).as_ref());
    assert!(!st.cores.contains_key("mihomo"));

    // Files outside the allowed list are refused before anything starts.
    let b64 = |s: &[u8]| base64::engine::general_purpose::STANDARD.encode(s);
    let evil = BTreeMap::from([("../evil.dll".to_string(), b64(b"x"))]);
    let r = request_at(&l, &Request::Start { core: "prism".into(), config: "{}".into(), files: evil }).await.unwrap();
    assert!(!r.ok && r.error.unwrap().contains("not allowed"));
    let r = request_at(&l, &Request::Start { core: "mihomo".into(), config: "{}".into(), files: BTreeMap::new() }).await.unwrap();
    assert!(!r.ok, "mihomo isn't installed");

    let (mixed, api) = (free_port(), free_port());
    let config = format!(
        "mixed-port: {mixed}\nexternal-controller: 127.0.0.1:{api}\nsecret: svc-test\n\
         proxy-providers:\n  p: {{ type: http, url: 'http://127.0.0.1:1/x', path: 'C:\\Windows\\win.ini', interval: 0 }}\n\
         proxies:\n  - {{name: n, type: ss, server: 127.0.0.1, port: 1, cipher: aes-128-gcm, password: x}}\n\
         rules:\n  - MATCH,DIRECT\n"
    );
    let files = BTreeMap::from([("geosite.dat".to_string(), b64(b"not really geodata"))]);
    let r = request_at(&l, &Request::Start { core: "prism".into(), config, files }).await.unwrap();
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
    let l = layout("stranger", PathBuf::from(r"C:\Program Files\Prism\prism.exe"));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let srv = tokio::spawn(server::serve(l.clone(), stop_rx));
    wait_ready(&l).await; // answers, if only with a refusal
    let r = request_at(&l, &Request::Status).await.unwrap();
    assert!(!r.ok && r.error.as_deref().unwrap_or("").contains("登记"), "{r:?}");
    stop_tx.send(true).unwrap();
    srv.await.unwrap().unwrap();

    // A client expecting the real service exe refuses a server that isn't it.
    let l2 = layout("impostor", std::env::current_exe().unwrap());
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let srv = tokio::spawn(server::serve(l2.clone(), stop_rx));
    wait_ready(&l2).await;
    let expecting_real = Layout { service_exe: prism_service::service_exe(), ..l2.clone() };
    let err = request_at(&expecting_real, &Request::Status).await.unwrap_err();
    assert!(format!("{err:#}").contains("不是由 Prism 服务创建的"), "{err:#}");
    stop_tx.send(true).unwrap();
    srv.await.unwrap().unwrap();
    for l in [l, l2] {
        let _ = std::fs::remove_dir_all(l.data.parent().unwrap());
    }
}
