import { invoke } from "@tauri-apps/api/core";

export type CoreKind = "prism" | "mihomo" | "singbox";
export type Mode = "rule" | "global" | "direct";

export interface Settings {
  core: CoreKind;
  mixed_port: number;
  controller_port: number;
  secret: string;
  mode: Mode;
  allow_lan: boolean;
  log_level: string;
  system_proxy: boolean;
  tun: boolean;
  proxy_bypass: string;
  active_profile: string | null;
  mihomo_path: string | null;
  singbox_path: string | null;
  test_url: string;
  selections: Record<string, Record<string, string>>;
}

export interface Usage {
  upload: number;
  download: number;
  total: number;
  expire: number;
}

export interface ProfileMeta {
  id: string;
  name: string;
  url: string | null;
  format: "clash" | "singbox" | "uri";
  nodes: number;
  updated_at: number;
  usage: Usage | null;
}

export interface Snapshot {
  settings: Settings;
  profiles: ProfileMeta[];
  running: CoreKind | null;
  system_proxy: boolean;
  elevated: boolean;
  core_paths: Record<CoreKind, { path: string; exists: boolean }>;
  data_dir: string;
}

export interface DelayRecord {
  time: string;
  delay: number;
}

export interface ProxyItem {
  name: string;
  type: string;
  udp?: boolean;
  history: DelayRecord[];
  all?: string[];
  now?: string;
  testUrl?: string;
  hidden?: boolean;
}

export interface Connection {
  id: string;
  metadata: {
    network: string;
    type: string;
    sourceIP: string;
    sourcePort: string;
    destinationIP: string;
    destinationPort: string;
    host: string;
    process?: string;
  };
  upload: number;
  download: number;
  start: string;
  chains: string[];
  rule: string;
  rulePayload: string;
}

export interface ConnectionsResp {
  downloadTotal: number;
  uploadTotal: number;
  connections: Connection[] | null;
}

export interface RuleItem {
  type: string;
  payload: string;
  proxy: string;
}

export const api = {
  state: () => invoke<Snapshot>("get_state"),
  versions: () => invoke<Record<CoreKind, string | null>>("core_versions"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  importProfile: (args: { url?: string; content?: string; name?: string }) =>
    invoke<ProfileMeta>("import_profile", { args }),
  updateProfile: (id: string) => invoke<ProfileMeta>("update_profile", { id }),
  saveProfileContent: (id: string, content: string) => invoke<ProfileMeta>("save_profile_content", { id, content }),
  renameProfile: (id: string, name: string) => invoke<void>("rename_profile", { id, name }),
  readProfile: (id: string) => invoke<string>("read_profile", { id }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  activateProfile: (id: string) => invoke<void>("activate_profile", { id }),
  start: () => invoke<void>("start_core"),
  stop: () => invoke<void>("stop_core"),
  selectProxy: (group: string, name: string) => invoke<void>("select_proxy", { group, name }),
  setMode: (mode: Mode) => invoke<void>("set_mode", { mode }),
  setSystemProxy: (enable: boolean) => invoke<void>("set_system_proxy", { enable }),
  setTun: (enable: boolean) => invoke<void>("set_tun", { enable }),
  restartAsAdmin: () => invoke<void>("restart_as_admin"),
  logs: () => invoke<string[]>("get_logs"),
  clearLogs: () => invoke<void>("clear_logs"),
  downloadCore: (kind: CoreKind) => invoke<string>("download_core", { kind }),
  updateGeodata: () => invoke<void>("update_geodata"),
  previewConfig: () => invoke<string>("preview_config"),
  openDataDir: () => invoke<void>("open_data_dir"),
  core: <T = unknown>(method: string, path: string, body?: unknown) =>
    invoke<T>("core_api", { method, path, body: body ?? null }),
};

export const enc = encodeURIComponent;

export function fmtBytes(n: number): string {
  if (!n) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), u.length - 1);
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
}

export function fmtSpeed(n: number): string {
  return `${fmtBytes(n)}/s`;
}

export function fmtAgo(ts: number): string {
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return "刚刚";
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}

export function lastDelay(p?: ProxyItem): number | undefined {
  const h = p?.history;
  return h && h.length ? h[h.length - 1].delay : undefined;
}

export function delayClass(d?: number): string {
  if (d === undefined) return "d-none";
  if (d === 0) return "d-bad";
  if (d < 300) return "d-good";
  if (d < 800) return "d-mid";
  return "d-slow";
}

export const CORE_NAMES: Record<CoreKind, string> = {
  prism: "Prism 内核",
  mihomo: "Mihomo",
  singbox: "sing-box",
};

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
