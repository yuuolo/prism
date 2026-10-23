//! Opt-in: validates generated configs with real core binaries.
//!   SINGBOX=path/to/sing-box.exe MIHOMO=path/to/mihomo.exe cargo test -p prism-core --test singbox_check

use prism_core::config::{self, clash, clash::Overrides, singbox, AppRoute, Mode};

fn app_routes() -> Vec<AppRoute> {
    vec![
        AppRoute { by_path: false, value: "chrome".into(), target: "PROXY".into() },
        AppRoute { by_path: true, value: "C:\\Program Files\\Steam\\steam.exe".into(), target: "DIRECT".into() },
        AppRoute { by_path: false, value: "ads.exe".into(), target: "REJECT".into() },
    ]
}

/// The YAML the GUI writes for mihomo (overrides + app rules) loads in mihomo.
#[test]
fn generated_mihomo_config_passes_check() {
    let Ok(bin) = std::env::var("MIHOMO") else {
        eprintln!("MIHOMO not set, skipping");
        return;
    };
    let o = Overrides {
        mixed_port: 17892,
        controller: "127.0.0.1:19099".into(),
        secret: "s".into(),
        mode: Mode::Rule,
        allow_lan: false,
        log_level: "info".into(),
        tun: false,
        unified_delay: true,
    };
    let dir = std::env::temp_dir().join(format!("prism-mhcheck-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Drop GEOSITE/GEOIP rules so mihomo doesn't download geodata for a check.
    let raw: String = CLASH.lines().filter(|l| !l.contains("GEO")).collect::<Vec<_>>().join("\n");
    let mut doc: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
    clash::apply_overrides(&mut doc, &o);
    assert!(clash::add_app_rules(&mut doc, &app_routes()).is_empty());
    let path = dir.join("config.yaml");
    std::fs::write(&path, serde_yaml::to_string(&doc).unwrap()).unwrap();
    let out = std::process::Command::new(&bin).args(["-t", "-f"]).arg(&path).arg("-d").arg(&dir).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && text.contains("test is successful"), "mihomo -t failed:\n{text}");
}

const CLASH: &str = r#"
mixed-port: 7890
proxies:
  - { name: "东京", type: anytls, server: jp.example.com, port: 443, password: "pw", sni: jp.example.com, client-fingerprint: chrome, udp: true }
  - { name: ss1, type: ss, server: 1.2.3.4, port: 8388, cipher: aes-256-gcm, password: pw }
  - { name: tj, type: trojan, server: t.example.com, port: 443, password: pw, network: ws, ws-opts: { path: /ws, headers: { Host: t.example.com } } }
  - { name: vr, type: vless, server: r.example.com, port: 443, uuid: 0b6a0b4e-1111-4222-8333-444455556666, tls: true, flow: xtls-rprx-vision, servername: www.microsoft.com, reality-opts: { public-key: "Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw", short-id: "6ba85179e30d4fc2" }, client-fingerprint: chrome }
  - { name: hy, type: hysteria2, server: h.example.com, port: 443, password: pw, sni: h.example.com }
proxy-groups:
  - { name: PROXY, type: select, proxies: [auto, "东京", ss1, tj, vr, hy, DIRECT, REJECT] }
  - { name: auto, type: url-test, proxies: ["东京", ss1], url: "https://www.gstatic.com/generate_204", interval: 300 }
rules:
  - DOMAIN-SUFFIX,ads.example,REJECT
  - DOMAIN,a.example.com,DIRECT
  - DOMAIN-KEYWORD,google,PROXY
  - IP-CIDR,10.0.0.0/8,DIRECT,no-resolve
  - DST-PORT,22,DIRECT
  - GEOSITE,cn,DIRECT
  - GEOIP,private,DIRECT,no-resolve
  - GEOIP,CN,DIRECT
  - MATCH,PROXY
"#;

#[test]
fn generated_singbox_config_passes_check() {
    let Ok(bin) = std::env::var("SINGBOX") else {
        eprintln!("SINGBOX not set, skipping");
        return;
    };
    let mut o = Overrides {
        mixed_port: 17891,
        controller: "127.0.0.1:19098".into(),
        secret: "s".into(),
        mode: Mode::Rule,
        allow_lan: false,
        log_level: "info".into(),
        tun: false,
        unified_delay: false,
    };
    let dir = std::env::temp_dir().join(format!("prism-sbcheck-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let uri = "anytls://pw@jp.example.com:443/?sni=jp.example.com#jp\nss://YWVzLTI1Ni1nY206cHc@1.2.3.4:8388#ss";
    for (name, raw, tun) in [
        ("clash", CLASH, false),
        ("clash-tun", CLASH, true),
        ("uri", uri, false),
        ("uri-tun", uri, true),
    ] {
        o.tun = tun;
        let p = config::parse_any(&raw).unwrap();
        let mut cfg = singbox::emit(&p, &o);
        // App rules point at groups that exist in the CLASH profile.
        if name.starts_with("clash") {
            assert!(singbox::add_app_rules(&mut cfg, &app_routes()).is_empty());
        }
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
        let out = std::process::Command::new(&bin).args(["check", "-c"]).arg(&path).output().unwrap();
        assert!(
            out.status.success(),
            "{name}: sing-box check failed:\n{}\n{}",
            String::from_utf8_lossy(&out.stderr),
            serde_json::to_string_pretty(&cfg).unwrap()
        );
    }
}
