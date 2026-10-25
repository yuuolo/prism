import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";

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
  auto_update_check: boolean;
  /** When started with Windows, stay in the tray. */
  silent_start: boolean;
  theme: "system" | "light" | "dark";
  lang: "zh" | "en";
  /** Global hotkeys: action -> accelerator ("Ctrl+Alt+P"). */
  hotkeys: Record<string, string>;
  /** password is "saved" when one is stored (never sent to the UI). */
  webdav: { url: string; username: string; password: string };
  tun: boolean;
  unified_delay: boolean;
  proxy_bypass: string;
  active_profile: string | null;
  mihomo_path: string | null;
  singbox_path: string | null;
  test_url: string;
  selections: Record<string, Record<string, string>>;
  app_rules: AppRule[];
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
  /** Minutes between automatic updates (remote profiles); 0 = never. */
  interval: number;
}

export interface AppRule {
  id: string;
  enabled: boolean;
  /** Match the full path instead of the file name. */
  by_path: boolean;
  value: string;
  target: string;
}

export interface ProcessInfo {
  pid: number;
  name: string;
  path: string;
}

export interface Outbounds {
  groups: string[];
  proxies: string[];
}

export interface Snapshot {
  settings: Settings;
  profiles: ProfileMeta[];
  running: CoreKind | null;
  system_proxy: boolean;
  elevated: boolean;
  core_paths: Record<CoreKind, { path: string; exists: boolean }>;
  data_dir: string;
  /** Prism app version. */
  version: string;
  /** Windows starts Prism at logon. */
  autostart: boolean;
  /** clash:// links open Prism. */
  clash_links: boolean;
  /** File name of the program that opens clash:// links now. */
  clash_handler: string | null;
  /** TUN service (runs TUN cores without admin rights for Prism). */
  tun_service: "missing" | "stopped" | "running";
}

/** From a prism:// or clash:// install-config link. */
export interface ImportRequest {
  url: string;
  name: string | null;
}

/** A link waiting for confirmation, or why it couldn't be used. */
export interface PendingLink {
  request: ImportRequest | null;
  error: string | null;
}

export interface RemoteBackup {
  name: string;
  modified: string;
  size: number;
}

export interface UnlockCheck {
  service: string;
  verdict: "yes" | "partial" | "no" | "unknown";
  region: string | null;
  note: string | null;
}

/** 覆写: Merge (YAML) and Script (JS), global or for one profile. */
export interface Patches {
  merge_enabled: boolean;
  merge: string;
  script_enabled: boolean;
  script: string;
}

export interface TrafficDay {
  /** Local date, YYYY-MM-DD. */
  date: string;
  upload: number;
  download: number;
}

export interface UpdateInfo {
  current: string;
  version: string;
  notes: string | null;
  date: string | null;
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
    processPath?: string;
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
  listProcesses: () => invoke<ProcessInfo[]>("list_processes"),
  profileOutbounds: () => invoke<Outbounds>("profile_outbounds"),
  setAppRules: (rules: AppRule[]) => invoke<void>("set_app_rules", { rules }),
  setProfileInterval: (id: string, minutes: number) => invoke<void>("set_profile_interval", { id, minutes }),
  setAutostart: (enable: boolean) => invoke<void>("set_autostart", { enable }),
  setClashLinks: (enable: boolean) => invoke<void>("set_clash_links", { enable }),
  takeImportRequests: () => invoke<PendingLink[]>("take_import_requests"),
  updateTray: () => invoke<void>("update_tray"),
  trafficHistory: () => invoke<TrafficDay[]>("traffic_history"),
  unlockCheck: () => invoke<UnlockCheck[]>("unlock_check"),
  /** `profile` = null: the global set. */
  getPatches: (profile: string | null) => invoke<Patches>("get_patches", { profile }),
  savePatches: (patches: Patches, profile: string | null) => invoke<void>("save_patches", { patches, profile }),
  patchedProfiles: () => invoke<string[]>("patched_profiles"),
  webdavSave: (url: string, username: string, password: string | null) => invoke<void>("webdav_save", { url, username, password }),
  webdavTest: () => invoke<void>("webdav_test"),
  backupUpload: () => invoke<string>("backup_upload"),
  backupList: () => invoke<RemoteBackup[]>("backup_list"),
  backupDelete: (name: string) => invoke<void>("backup_delete", { name }),
  backupRestore: (name: string) => invoke<void>("backup_restore", { name }),
  backupImport: (data: string) => invoke<void>("backup_import", { data }),
  backupExport: () => invoke<string>("backup_export"),
  installTunService: () => invoke<void>("install_tun_service"),
  uninstallTunService: () => invoke<void>("uninstall_tun_service"),
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
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
  if (s < 60) return t("刚刚");
  if (s < 3600) return t("{n} 分钟前", { n: Math.floor(s / 60) });
  if (s < 86400) return t("{n} 小时前", { n: Math.floor(s / 3600) });
  return t("{n} 天前", { n: Math.floor(s / 86400) });
}

/** 90 -> "1.5 小时", 1440 -> "1 天". */
export function fmtInterval(min: number): string {
  if (min >= 1440 && min % 1440 === 0) return t("{n} 天", { n: min / 1440 });
  if (min >= 60) return t("{n} 小时", { n: +(min / 60).toFixed(1) });
  return t("{n} 分钟", { n: min });
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
