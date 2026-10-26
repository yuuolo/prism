//! Opt-in: validates generated configs with real core binaries.
//!   SINGBOX=path/to/sing-box.exe MIHOMO=path/to/mihomo.exe cargo test -p prism-core --test singbox_check

use prism_core::config::{self, clash, clash::Overrides, singbox, AppRoute, Mode, UserRule};

/// The GUI's domain / IP rules, one of each kind.
fn user_rules() -> Vec<UserRule> {
    [
        ("DOMAIN-SUFFIX", "openai.com", "PROXY"),
        ("DOMAIN", "www.example.com", "DIRECT"),
        ("DOMAIN-KEYWORD", "ads", "REJECT"),
        ("IP-CIDR", "192.168.50.0/24", "DIRECT"),
        ("IP-CIDR", "2001:db8::1", "PROXY"),
        ("GEOSITE", "netflix", "PROXY"),
        ("GEOIP", "telegram", "PROXY"),
    ]
    .into_iter()
    .map(|(k, v, t)| UserRule { kind: k.into(), value: v.into(), target: t.into(), resolve: false })
    .collect()
}

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
    clash::relays_to_dialer_proxy(&mut doc);
    // GEO rules would make mihomo download geodata for a check.
    let user: Vec<UserRule> = user_rules().into_iter().filter(|r| !r.kind.starts_with("GEO")).collect();
    assert!(clash::add_user_rules(&mut doc, &user).is_empty());
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
  - { name: tu, type: tuic, server: tu.example.com, port: 443, uuid: 0b6a0b4e-1111-4222-8333-444455556666, password: pw, congestion-controller: bbr, udp-relay-mode: native, alpn: [h3] }
  - { name: wg, type: wireguard, server: 162.159.192.1, port: 2408, private-key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", public-key: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=", ip: 172.16.0.2, ipv6: "fd01::2", reserved: [1, 2, 3], mtu: 1280, udp: true, dialer-proxy: ss1 }
  - { name: vm, type: vmess, server: v.example.com, port: 443, uuid: 0b6a0b4e-1111-4222-8333-444455556666, alterId: 0, cipher: auto, tls: true, servername: v.example.com, network: grpc, grpc-opts: { grpc-service-name: g } }
  - { name: vu, type: vmess, server: v.example.com, port: 80, uuid: 0b6a0b4e-1111-4222-8333-444455556666, alterId: 0, cipher: chacha20-poly1305, network: ws, ws-opts: { path: /u, headers: { Host: v.example.com }, v2ray-http-upgrade: true } }
proxy-groups:
  - { name: PROXY, type: select, proxies: [auto, "东京", ss1, tj, vr, hy, tu, wg, vm, vu, chain, DIRECT, REJECT] }
  - { name: chain, type: relay, proxies: [auto, ss1, tu] }
  - { name: auto, type: url-test, proxies: ["东京", ss1], url: "https://www.gstatic.com/generate_204", interval: 300 }
rules:
  - DOMAIN-SUFFIX,ads.example,REJECT
  - DOMAIN,a.example.com,DIRECT
  - DOMAIN-KEYWORD,google,PROXY
  - IP-CIDR,10.0.0.0/8,DIRECT,no-resolve
  - DST-PORT,22,DIRECT
  - AND,((NETWORK,udp),(DST-PORT,443)),REJECT
  - OR,((DOMAIN,x.example.com),(DST-PORT,8443)),chain
  - NOT,((GEOSITE,cn)),PROXY
  - SRC-GEOIP,lan,DIRECT
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
    // The profile's own fake-ip DNS, carried over for TUN.
    let clash_dns = format!(
        "{CLASH}dns:\n  enable: true\n  enhanced-mode: fake-ip\n  fake-ip-range: 198.18.0.1/16\n  fake-ip-filter: ['+.lan', 'time.windows.com']\n  \
         default-nameserver: [119.29.29.29]\n  nameserver: [223.5.5.5, 'https://doh.pub/dns-query', 'tls://dns.google#PROXY', 'dhcp://en0']\n"
    );
    for (name, raw, tun) in [
        ("clash", CLASH, false),
        ("clash-tun", CLASH, true),
        ("uri", uri, false),
        ("uri-tun", uri, true),
        ("clash-dns-tun", &clash_dns, true),
    ] {
        o.tun = tun;
        let p = config::parse_any(&raw).unwrap();
        let mut cfg = singbox::emit(&p, &o);
        // App rules point at groups that exist in the CLASH profile.
        if name.starts_with("clash") {
            assert!(singbox::add_user_rules(&mut cfg, &user_rules()).is_empty());
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
