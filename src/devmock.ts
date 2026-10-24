// Dev-only UI preview: when the Vite dev server is opened in a normal
// browser (no Tauri backend), answer the app's commands with sample data so
// pages can be checked visually. Never included in production builds.

type Handler = (args: Record<string, unknown>) => unknown;

export function installDevMock() {
  const w = window as unknown as Record<string, unknown>;
  if (w.__TAURI_INTERNALS__) return;

  const settings = {
    core: "prism",
    mixed_port: 7899,
    controller_port: 9097,
    secret: "preview",
    mode: "rule",
    allow_lan: false,
    log_level: "info",
    system_proxy: true,
    auto_update_check: true,
    silent_start: true,
    tun: false,
    unified_delay: true,
    proxy_bypass: "localhost;127.*;<local>",
    active_profile: "demo",
    mihomo_path: null,
    singbox_path: null,
    test_url: "https://www.gstatic.com/generate_204",
    selections: {},
    app_rules: [
      { id: "1", enabled: true, by_path: false, value: "Telegram.exe", target: "节点选择" },
      { id: "2", enabled: true, by_path: true, value: "D:\\Games\\Steam\\steam.exe", target: "DIRECT" },
      { id: "3", enabled: false, by_path: false, value: "updater.exe", target: "REJECT" },
    ],
  };
  const profiles = [
    { id: "demo", name: "sbm 订阅", url: "https://sub.example.com/api/sub?token=demo", format: "clash", nodes: 2, updated_at: Date.now() / 1000 - 7200, usage: null, interval: 1440 },
    { id: "local", name: "本地配置", url: null, format: "uri", nodes: 3, updated_at: Date.now() / 1000 - 86400 * 3, usage: null, interval: 1440 },
  ];
  const sys = { autostart: false, clash_links: false, tun_service: "missing" as "missing" | "stopped" | "running" };
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
      ],
    },
  };
  const handlers: Record<string, Handler> = {
    get_state: () => ({
      settings,
      profiles,
      running: "prism",
      system_proxy: true,
      elevated: false,
      core_paths: { prism: { path: "", exists: true }, mihomo: { path: "", exists: true }, singbox: { path: "", exists: false } },
      data_dir: "%APPDATA%\\dev.prism.client",
      version: "0.1.1",
      ...sys,
      clash_handler: sys.clash_links ? "prism.exe" : "FlClash.exe",
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
    profile_outbounds: () => ({ groups: ["节点选择", "自动选择"], proxies: ["东京", "香港"] }),
    list_processes: () => processes,
    set_app_rules: (a) => {
      settings.app_rules = a.rules as typeof settings.app_rules;
    },
    get_logs: () => [],
    update_tray: () => null,
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
