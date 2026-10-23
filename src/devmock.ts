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
  const api: Record<string, unknown> = {
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
      profiles: [{ id: "demo", name: "sbm 订阅", url: null, format: "clash", nodes: 2, updated_at: Date.now() / 1000, usage: null }],
      running: "prism",
      system_proxy: true,
      elevated: false,
      core_paths: { prism: { path: "", exists: true }, mihomo: { path: "", exists: true }, singbox: { path: "", exists: false } },
      data_dir: "%APPDATA%\\dev.prism.client",
      version: "0.1.1",
    }),
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
    core_api: (a) => {
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
