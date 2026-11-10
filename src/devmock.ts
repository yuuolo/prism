// Dev-only UI preview: when the Vite dev server is opened in a normal
// browser (no Tauri backend), answer the app's commands with sample data so
// pages can be checked visually. Never included in production builds.

import type { RealIpRule } from "./api";

type Handler = (args: Record<string, unknown>) => unknown;

export function installDevMock() {
  const w = window as unknown as Record<string, unknown>;
  if (w.__TAURI_INTERNALS__) return;

  const settings = {
    core: "prism",
    mixed_port: 7899,
    tray_speed: false,
    controller_port: 9097,
    secret: "preview",
    mode: "rule",
    allow_lan: false,
    log_level: "info",
    system_proxy: true,
    auto_update_check: true,
    silent_start: true,
    theme: "system",
    lang: "zh",
    hotkeys: { toggle_window: "Ctrl+Alt+P" } as Record<string, string>,
    webdav: { url: "https://dav.jianguoyun.com/dav/", username: "me@example.com", password: "saved" },
    tun: false,
    unified_delay: true,
    proxy_bypass: "localhost;127.*;<local>",
    active_profile: "demo",
    merge_profiles: [] as string[],
    mihomo_path: null,
    singbox_path: null,
    test_url: "https://www.gstatic.com/generate_204",
    selections: {},
    app_rules: [
      { id: "1", enabled: true, by_path: false, value: "Telegram.exe", target: "节点选择" },
      { id: "2", enabled: true, by_path: true, value: "D:\\Games\\Steam\\steam.exe", target: "DIRECT" },
      { id: "3", enabled: false, by_path: false, value: "updater.exe", target: "REJECT" },
    ],
    real_ip_rules: [
      { id: "r1", enabled: true, kind: "domain", value: "corp.example", subdomains: true, dns: "10.1.1.1" },
      { id: "r2", enabled: true, kind: "network", value: "10.0.0.0/8", subdomains: false, dns: "" },
    ] as RealIpRule[],
    geosite_url: "",
    geoip_url: "",
    geosite_srs_url: "",
    geoip_srs_url: "",
    kill_switch: false,
    chains: [{ id: "c1", enabled: true, name: "日本落地", hops: ["香港", "东京"] }],
    route_rules: [
      { id: "r1", enabled: true, kind: "DOMAIN-SUFFIX", value: "openai.com", target: "节点选择", resolve: false },
      { id: "r2", enabled: true, kind: "IP-CIDR", value: "192.168.50.0/24", target: "DIRECT", resolve: false },
      { id: "r3", enabled: false, kind: "GEOSITE", value: "category-ads-all", target: "REJECT", resolve: false },
    ] as import("./api").RouteRule[],
  };
  const profiles = [
    { id: "demo", name: "sbm 订阅", url: "https://sub.example.com/api/sub?token=demo", format: "clash", nodes: 2, updated_at: Date.now() / 1000 - 7200, usage: null, interval: 1440 },
    { id: "local", name: "本地配置", url: null, format: "uri", nodes: 3, updated_at: Date.now() / 1000 - 86400 * 3, usage: null, interval: 1440 },
  ];
  const sys = { autostart: false, clash_links: false, tun_service: "missing" as "missing" | "stopped" | "running", kill_switch_blocking: false };
  // `?import` shows the confirmation dialog for a clash:// link once.
  let pendingLinks = location.search.includes("import")
    ? [
        { request: { url: "https://panel.example.com/api/v1/client/subscribe?token=8f3c2a91d4&flag=clash", name: "我的机场" }, error: null },
        { request: { url: "https://sub.example.com/api/sub?token=demo", name: null }, error: null },
      ]
    : [];
  const processes = [
    { pid: 1200, name: "chrome.exe", path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe" },
    { pid: 3400, name: "Telegram.exe", path: "C:\\Users\\me\\AppData\\Roaming\\Telegram Desktop\\Telegram.exe" },
    { pid: 5100, name: "steam.exe", path: "D:\\Games\\Steam\\steam.exe" },
    { pid: 6100, name: "WeChat.exe", path: "C:\\Program Files\\Tencent\\WeChat\\WeChat.exe" },
    { pid: 7000, name: "Code.exe", path: "C:\\Users\\me\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe" },
  ];
  const conn = (id: string, host: string, proc: number, rule: string, payload: string, chains: string[], network = "tcp") => ({
    id,
    metadata: {
      network,
      type: "HTTPS",
      sourceIP: "127.0.0.1",
      sourcePort: "50" + id,
      destinationIP: "",
      destinationPort: "443",
      host,
      process: processes[proc].name,
      processPath: processes[proc].path,
    },
    upload: 12000 * Number(id),
    download: 340000 * Number(id),
    start: new Date(Date.now() - 60000 * Number(id)).toISOString(),
    chains,
    rule,
    rulePayload: payload,
  });
  const nodeNames = ["香港 01", "香港 02", "日本 东京", "日本 大阪", "新加坡", "美国 洛杉矶", "台湾", "韩国 首尔"];
  const node = (name: string, delay?: number) => ({
    name,
    type: "AnyTLS",
    udp: true,
    history: delay === undefined ? [] : [{ time: new Date().toISOString(), delay }],
  });
  const proxies: Record<string, Record<string, unknown>> = {
    GLOBAL: { name: "GLOBAL", type: "Selector", now: "节点选择", all: ["节点选择", "自动选择", "流媒体"], history: [] },
    节点选择: { name: "节点选择", type: "Selector", now: "日本 东京", all: ["自动选择", ...nodeNames, "DIRECT"], history: [] },
    自动选择: { name: "自动选择", type: "URLTest", now: "香港 01", all: nodeNames, history: [] },
    流媒体: { name: "流媒体", type: "Selector", now: "新加坡", all: ["节点选择", "新加坡", "美国 洛杉矶", "台湾"], history: [] },
    日本落地: { name: "日本落地", type: "Relay", all: ["香港", "东京"], now: "东京", history: [] },
    DIRECT: { name: "DIRECT", type: "Direct", history: [] },
    ...Object.fromEntries(nodeNames.map((n, i) => [n, node(n, i === 4 ? 0 : i < 3 ? 80 + i * 70 : undefined)])),
  };
  const api: Record<string, unknown> = {
    "/proxies": { proxies },
    "/version": { version: "prism-preview" },
    "/connections": {
      uploadTotal: 3_000_000,
      downloadTotal: 52_000_000,
      connections: [
        conn("1", "www.google.com", 0, "GeoSite", "geolocation-!cn", ["东京", "自动选择", "节点选择"]),
        conn("2", "api.telegram.org", 1, "ProcessName", "Telegram.exe", ["东京", "节点选择"]),
        conn("3", "steamcdn-a.akamaihd.net", 2, "ProcessPath", "D:\\Games\\Steam\\steam.exe", ["DIRECT"]),
        conn("4", "weixin.qq.com", 3, "GeoSite", "cn", ["DIRECT"], "udp"),
        conn("5", "chatgpt.com", 0, "DomainSuffix", "chatgpt.com", ["美国 洛杉矶", "节点选择"]),
        conn("6", "www.netflix.com", 0, "GeoSite", "netflix", ["新加坡", "流媒体"]),
        conn("7", "github.com", 4, "GeoSite", "github", ["日本 东京", "节点选择"]),
        conn("8", "www.bbc.co.uk", 0, "Match", "", ["香港 01", "自动选择", "节点选择"]),
        conn("9", "www.bilibili.com", 0, "GeoSite", "cn", ["DIRECT"]),
        conn("10", "api.deepl.com", 4, "Match", "", ["日本 东京", "节点选择"]),
      ],
    },
  };
  const handlers: Record<string, Handler> = {
    get_state: () => ({
      settings,
      profiles,
      running: "prism",
      uptime_secs: 3661,
      system_proxy: true,
      elevated: false,
      core_paths: { prism: { path: "", exists: true }, mihomo: { path: "", exists: true }, singbox: { path: "", exists: false } },
      data_dir: "%APPDATA%\\dev.prism.client",
      version: "0.1.1",
      ...sys,
      clash_handler: sys.clash_links ? "prism.exe" : "FlClash.exe",
      // ?platform=macos previews the macOS wording.
      platform: new URLSearchParams(location.search).get("platform") ?? "windows",
    }),
    install_tun_service: async () => {
      await new Promise((r) => setTimeout(r, 800));
      sys.tun_service = "running";
    },
    uninstall_tun_service: () => {
      sys.tun_service = "missing";
    },
    set_autostart: (a) => {
      sys.autostart = a.enable as boolean;
    },
    set_clash_links: (a) => {
      sys.clash_links = a.enable as boolean;
    },
    take_import_requests: () => {
      const l = pendingLinks;
      pendingLinks = [];
      return l;
    },
    security_events: () => [
      `${Math.floor(Date.now() / 1000) - 120} [prism] 启动 Prism 内核`,
      `${Math.floor(Date.now() / 1000) - 60} [prism] 断网保护已触发：内核已退出，网络被拦截`,
      `${Math.floor(Date.now() / 1000)} [TCP] example.com match Domain using REJECT failed: rejected by rule`,
    ],
    add_single_node: (a) => {
      if (!String(a.content).trim()) throw "请输入一个有效节点";
      const p = { id: `node-${Date.now()}`, name: String(a.name).trim() || "本地节点", url: null, format: "clash", nodes: 1, updated_at: Date.now() / 1000, usage: null, interval: 0 };
      profiles.push(p);
      settings.merge_profiles.push(p.id);
      return p;
    },
    import_profile: (a) => {
      const args = a.args as { url: string; name?: string };
      const p = { id: `p${profiles.length}`, name: args.name ?? "新订阅", url: args.url, format: "clash", nodes: 5, updated_at: Date.now() / 1000, usage: null, interval: 720 };
      profiles.push(p);
      return p;
    },
    update_profile: (a) => {
      const p = profiles.find((x) => x.id === a.id)!;
      p.updated_at = Date.now() / 1000;
      return p;
    },
    check_update: () => ({
      current: "0.1.1",
      version: "0.1.2",
      notes: "- 应用分流：指定程序走固定出口\n- 软件更新：自动从 GitHub 获取签名更新包\n- UDP / DNS / providers",
      date: "2026-10-24 08:00:00.0 +00:00:00",
    }),
    install_update: () => {
      throw "预览模式不会安装更新";
    },
    core_versions: () => ({ prism: "prism-core preview", mihomo: null, singbox: null }),
    profile_outbounds: () => ({ groups: ["节点选择", "自动选择"], chains: settings.chains.filter((c) => c.enabled).map((c) => c.name), proxies: ["东京", "香港"] }),
    list_processes: () => processes,
    speed_test: async (a) => {
      await new Promise((r) => setTimeout(r, 1200));
      return (a.names as string[]).map((name, i) =>
        i === 4
          ? { name, speed: null, error: "connect timeout", done: true }
          : { name, speed: { bytes: 1e8, secs: 8, mbps: [212, 95.4, 48.2, 160, 0, 33.7, 87, 140][i % 8], ttfb: 180 + i * 40 }, error: null, done: true },
      );
    },
    lan_addresses: () => [
      { ip: "192.168.1.157", iface: "以太网" },
      { ip: "192.168.31.155", iface: "WLAN" },
    ],
    set_allow_lan: (a) => {
      settings.allow_lan = a.enable as boolean;
    },
    app_traffic: (a) => {
      const k = Number(a.days);
      return [
        { name: "chrome.exe", path: "C:\Program Files\Google\Chrome\chrome.exe", upload: 42e6 * k, download: 1.9e9 * k, exits: [["日本 东京", 1.5e9 * k], ["DIRECT", 3e8 * k], ["美国 洛杉矶", 1e8 * k]] },
        { name: "steam.exe", path: "D:\Games\Steam\steam.exe", upload: 5e6 * k, download: 1.1e9 * k, exits: [["DIRECT", 1.1e9 * k]] },
        { name: "Telegram.exe", path: "", upload: 30e6 * k, download: 210e6 * k, exits: [["香港 01", 2.4e8 * k]] },
        { name: "Code.exe", path: "", upload: 4e6 * k, download: 60e6 * k, exits: [["日本 东京", 4e7 * k], ["DIRECT", 2.4e7 * k]] },
        { name: "", path: "", upload: 1e6 * k, download: 9e6 * k, exits: [["DIRECT", 1e7 * k]] },
      ];
    },
    node_servers: () => Object.fromEntries(nodeNames.map((n) => [n, `${n}.example`])),
    geo_locate: (a) => {
      const where: Record<string, string> = {
        "www.google.com": "US", "api.telegram.org": "NL", "steamcdn-a.akamaihd.net": "CN", "weixin.qq.com": "CN", "chatgpt.com": "US",
        "www.netflix.com": "SG", "github.com": "US", "www.bbc.co.uk": "GB", "www.bilibili.com": "CN", "api.deepl.com": "DE",
        "香港 01.example": "HK", "香港 02.example": "HK", "日本 东京.example": "JP", "日本 大阪.example": "JP", "新加坡.example": "SG",
        "美国 洛杉矶.example": "US", "台湾.example": "TW", "韩国 首尔.example": "KR",
      };
      return Object.fromEntries((a.hosts as string[]).map((h) => [h, where[h] ?? null]));
    },
    node_quality: () => {
      const now = Math.floor(Date.now() / 1000);
      const series = (base: number, spread: number, fail: number, seed: number) => {
        const out: [number, number][] = [];
        let r = seed;
        for (let ts = now - 24 * 3600; ts < now; ts += 300) {
          r = (r * 9301 + 49297) % 233280;
          const v = r / 233280;
          out.push([ts, v < fail ? 0 : Math.round(base + spread * Math.sin(ts / 3000 + seed) + spread * v)]);
        }
        return out;
      };
      return nodeNames.map((name, i) => ({
        name,
        samples: series(50 + i * 25, 10 + ((i * 37) % 70), [0, 0.03, 0.15, 0, 0.01, 0.3, 0, 0.05][i], i + 1),
        avg: null,
        jitter: null,
        loss: 0,
        score: null,
      }));
    },
    test_rule: (a) => {
      const host = String(a.target).replace(/^\w+:\/\//, "").split(/[/:?]/)[0].toLowerCase();
      const google = host.endsWith("google.com");
      return {
        host,
        port: 443,
        ip: google ? null : "104.18.32.47",
        hit: google
          ? { index: 3, kind: "DOMAIN-SUFFIX", payload: "google.com", target: "节点选择", line: "DOMAIN-SUFFIX,google.com,节点选择", resolved: false }
          : { index: 41, kind: "GEOIP", payload: "CN", target: "DIRECT", line: "GEOIP,CN,DIRECT", resolved: true },
        total: 128,
        warnings: [],
      };
    },
    set_kill_switch: (a) => {
      settings.kill_switch = !!a.enable;
    },
    release_kill_switch: () => {},
    set_real_ip_rules: (a) => {
      settings.real_ip_rules = a.rules as RealIpRule[];
    },
    test_real_ip: (a) => (String(a.host).endsWith("corp.example") ? { ips: ["10.2.3.4"], fake: false } : { ips: ["198.18.0.9"], fake: true }),
    set_chains: (a) => {
      settings.chains = a.chains as typeof settings.chains;
    },
    set_route_rules: (a) => {
      settings.route_rules = a.rules as typeof settings.route_rules;
    },
    set_merge_profiles: (a) => {
      settings.merge_profiles = a.ids as string[];
    },
    set_app_rules: (a) => {
      settings.app_rules = a.rules as typeof settings.app_rules;
    },
    get_logs: () => [],
    update_tray: () => null,
    get_patches: () => ({
      merge_enabled: true,
      merge: "prepend-rules:\n  - DOMAIN-SUFFIX,example.com,DIRECT\n",
      script_enabled: false,
      script: "function main(config, profileName) {\n  return config;\n}\n",
    }),
    save_patches: () => null,
    patched_profiles: () => ["demo"],
    export_table: (a) => {
      console.log("export_table", a);
      return `C:\\Users\\me\\Downloads\\prism-${a.what}-20261025-120000.${a.format}`;
    },
    unlock_check: async () => {
      await new Promise((r) => setTimeout(r, 900));
      return [
        { service: "Netflix", verdict: "partial", region: "JP", note: "仅自制剧" },
        { service: "YouTube Premium", verdict: "yes", region: "JP", note: null },
        { service: "Prime Video", verdict: "yes", region: "JP", note: null },
        { service: "TikTok", verdict: "unknown", region: null, note: "HTTP 403" },
        { service: "ChatGPT", verdict: "yes", region: "JP", note: null },
        { service: "Claude", verdict: "yes", region: null, note: null },
        { service: "Gemini", verdict: "no", region: null, note: null },
      ];
    },
    webdav_test: () => null,
    webdav_save: (a) => {
      settings.webdav = { url: a.url as string, username: a.username as string, password: "saved" };
    },
    backup_upload: () => "prism-DESKTOP-20261025-120000.zip",
    backup_list: () => [
      { name: "prism-DESKTOP-20261025-120000.zip", modified: "Sat, 25 Oct 2026 04:00:00 GMT", size: 18342 },
      { name: "prism-LAPTOP-20261020-090000.zip", modified: "Mon, 20 Oct 2026 01:00:00 GMT", size: 17210 },
    ],
    backup_export: () => "C:\\Users\\me\\Downloads\\prism-DESKTOP-20261025-120000.zip",
    traffic_history: () =>
      Array.from({ length: 24 }, (_, i) => {
        const d = new Date();
        d.setDate(d.getDate() - i);
        const p = (n: number) => String(n).padStart(2, "0");
        return { date: `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`, upload: (i % 5) * 4e7 + 1e7, download: ((i * 7) % 11) * 2.2e8 + 3e7 };
      }).reverse(),
    save_settings: (a) => {
      Object.assign(settings, a.settings as object);
    },
    select_proxy: (a) => {
      proxies[a.group as string].now = a.name;
    },
    core_api: async (a) => {
      const delay = /^\/proxies\/([^/]+)\/delay/.exec(String(a.path));
      if (delay) {
        const n = decodeURIComponent(delay[1]);
        await new Promise((r) => setTimeout(r, 150 + Math.random() * 400));
        const d = n === "台湾" ? 0 : Math.round(40 + Math.random() * 500);
        proxies[n].history = [{ time: new Date().toISOString(), delay: d }];
        return { delay: d };
      }
      const r = api[String(a.path)];
      if (r === undefined) throw `preview: no sample data for ${a.path}`;
      return r;
    },
    "plugin:event|listen": () => 1,
    "plugin:event|unlisten": () => null,
  };
  let cb = 0;
  w.__TAURI_INTERNALS__ = {
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      const h = handlers[cmd];
      if (!h) throw `preview: ${cmd} is not available`;
      return h(args);
    },
    transformCallback: () => ++cb,
  };
}
