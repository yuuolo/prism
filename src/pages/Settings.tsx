import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, CORE_NAMES, errText, fmtBytes, type CoreKind, type RemoteBackup, type Settings } from "../api";
import { useApp } from "../state";
import { t } from "../i18n";

/** 软件更新: check GitHub releases and install signed updates. */
function UpdateCard({ form, set }: { form: Settings; set: <K extends keyof Settings>(k: K, v: Settings[K]) => void }) {
  const { snap, toast, update, setUpdate } = useApp();
  const [state, setState] = useState<"idle" | "checking" | "latest" | "installing">("idle");
  const [progress, setProgress] = useState<{ downloaded: number; total: number | null } | null>(null);

  useEffect(() => {
    const un = listen<{ downloaded: number; total: number | null }>("update-progress", (e) => setProgress(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const check = async () => {
    setState("checking");
    try {
      const u = await api.checkUpdate();
      setUpdate(u);
      setState(u ? "idle" : "latest");
    } catch (e) {
      toast(errText(e), "err");
      setState("idle");
    }
  };

  const install = async () => {
    setState("installing");
    setProgress(null);
    try {
      // On success Prism exits and the installer starts the new version.
      await api.installUpdate();
    } catch (e) {
      toast(errText(e), "err");
      setState("idle");
    }
  };

  const pct = progress?.total ? Math.min(100, (progress.downloaded / progress.total) * 100) : null;
  return (
    <div className="card span-2">
      <h3>{t("软件更新")}</h3>
      <div className="row wrap">
        <span>
          {t("当前版本")} <b>v{snap?.version}</b>
        </span>
        <button className="btn" disabled={state === "checking" || state === "installing"} onClick={check}>
          {state === "checking" ? t("检查中…") : t("检查更新")}
        </button>
        {state === "latest" && !update && <span className="small muted">{t("已是最新版本")}</span>}
        <span className="spacer" />
        <span className="small muted">{t("启动时自动检查")}</span>
        <button className={`switch ${form.auto_update_check ? "on" : ""}`} onClick={() => set("auto_update_check", !form.auto_update_check)} />
      </div>
      {update && (
        <div className="update-box">
          <div className="row">
            <b>{t("新版本 v{v}", { v: update.version })}</b>
            {update.date && <span className="small muted">{update.date.slice(0, 10)}</span>}
            <span className="spacer" />
            <button className="btn primary" disabled={state === "installing"} onClick={install}>
              {state === "installing" ? t("安装中…") : t("下载并安装")}
            </button>
          </div>
          {update.notes && <pre className="update-notes">{update.notes}</pre>}
          {state === "installing" && (
            <>
              <div className="bar">
                <div style={{ width: `${pct ?? 5}%` }} />
              </div>
              <div className="small muted">
                {progress ? t("已下载 {size}", { size: fmtBytes(progress.downloaded) + (progress.total ? ` / ${fmtBytes(progress.total)}` : "") }) : t("开始下载…")}
                {pct === 100 && t(" · 校验签名并启动安装程序，Prism 将自动重启")}
              </div>
            </>
          )}
        </div>
      )}
      <div className="small muted" style={{ marginTop: 10 }}>
        {t("更新包来自 GitHub Releases，安装前会校验发布签名；内核运行中时经代理下载。安装时会先停止内核。")}
      </div>
    </div>
  );
}

const HOTKEY_ACTIONS: [string, string][] = [
  ["toggle_window", "显示 / 隐藏主界面"],
  ["sysproxy", "开关系统代理"],
  ["tun", "开关 TUN 模式"],
  ["mode_rule", "切换到规则模式"],
  ["mode_global", "切换到全局模式"],
  ["mode_direct", "切换到直连模式"],
];

/** Turns a key press into an accelerator such as "Ctrl+Alt+P". */
function accelerator(e: React.KeyboardEvent): string | null {
  const code = e.code;
  if (/^(Control|Shift|Alt|Meta)(Left|Right)$/.test(code)) return null;
  let key = code;
  if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
  else if (/^Digit\d$/.test(code)) key = code.slice(5);
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  // Without a modifier only function keys make sense as global hotkeys.
  if (!mods.length && !/^F\d{1,2}$/.test(key)) return null;
  return [...mods, key].join("+");
}

function HotkeyInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [recording, setRecording] = useState(false);
  return (
    <input
      className="input mono hotkey"
      readOnly
      value={recording ? t("请按下组合键…") : value || t("未设置")}
      onFocus={() => setRecording(true)}
      onBlur={() => setRecording(false)}
      onKeyDown={(e) => {
        e.preventDefault();
        if (e.key === "Backspace" || e.key === "Delete" || e.key === "Escape") {
          onChange("");
          (e.target as HTMLInputElement).blur();
          return;
        }
        const acc = accelerator(e);
        if (acc) {
          onChange(acc);
          (e.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}

/** 外观、语言与全局快捷键. */
function AppearanceCard({ form, set }: { form: Settings; set: <K extends keyof Settings>(k: K, v: Settings[K]) => void }) {
  return (
    <div className="card span-2">
      <h3>{t("外观与快捷键")}</h3>
      <div className="field">
        <label>{t("主题")}</label>
        <div className="seg">
          {(
            [
              ["system", t("跟随系统")],
              ["light", t("浅色")],
              ["dark", t("深色")],
            ] as const
          ).map(([k, label]) => (
            <button key={k} className={form.theme === k ? "on" : ""} onClick={() => set("theme", k)}>
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className="field">
        <label>{t("界面语言")}</label>
        <div className="seg">
          {(
            [
              ["zh", t("中文")],
              ["en", "English"],
            ] as const
          ).map(([k, label]) => (
            <button key={k} className={form.lang === k ? "on" : ""} onClick={() => set("lang", k)}>
              {label}
            </button>
          ))}
        </div>
      </div>
      {HOTKEY_ACTIONS.map(([action, label]) => (
        <div className="field" key={action}>
          <label>{t(label)}</label>
          <HotkeyInput value={form.hotkeys?.[action] ?? ""} onChange={(v) => set("hotkeys", { ...form.hotkeys, [action]: v })} />
        </div>
      ))}
      <div className="small muted" style={{ marginTop: 8 }}>
        {t("点击输入框后按下组合键（如 Ctrl+Alt+P），按 Backspace 清除。快捷键在 Prism 最小化到托盘时也有效；保存后生效。")}
      </div>
    </div>
  );
}

/** 备份与同步: WebDAV and local zip backups of settings and profiles. */
function BackupCard() {
  const { snap, run, toast } = useApp();
  const saved = snap?.settings.webdav;
  const [url, setUrl] = useState(saved?.url ?? "");
  const [user, setUser] = useState(saved?.username ?? "");
  const [pass, setPass] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [list, setList] = useState<RemoteBackup[] | null>(null);
  if (!snap || !saved) return null;
  const configured = !!saved.url;
  const changed = url !== saved.url || user !== saved.username || pass !== "";

  const act = async (key: string, fn: () => Promise<unknown>, ok?: string) => {
    setBusy(key);
    const r = await run(fn, ok);
    setBusy(null);
    return r;
  };
  const saveLogin = () => act("save", () => api.webdavSave(url, user, pass === "" ? null : pass), t("WebDAV 设置已保存")).then(() => setPass(""));
  const refreshList = () => act("list", async () => setList(await api.backupList()));
  const restore = (name: string) =>
    confirm(t("用备份「{name}」覆盖当前的设置和订阅？\n本机的内核路径、系统代理和 TUN 开关、WebDAV 登录信息会保留。", { name })) &&
    act(name, () => api.backupRestore(name), t("已恢复，内核已按备份重新启动"));
  const importFile = (f: File) => {
    const reader = new FileReader();
    reader.onload = () => {
      const data = String(reader.result).split(",")[1] ?? "";
      if (confirm(t("用「{name}」覆盖当前的设置和订阅？", { name: f.name }))) act("import", () => api.backupImport(data), t("已从文件恢复"));
    };
    reader.readAsDataURL(f);
  };

  return (
    <div className="card span-2">
      <h3>{t("备份与同步")}</h3>
      <div className="field">
        <label>{t("WebDAV 地址")}</label>
        <input className="input" placeholder="https://dav.jianguoyun.com/dav/" value={url} onChange={(e) => setUrl(e.target.value)} />
      </div>
      <div className="field">
        <label>{t("用户名")}</label>
        <input className="input" value={user} onChange={(e) => setUser(e.target.value)} />
      </div>
      <div className="field">
        <label>{t("密码")}</label>
        <input
          className="input"
          type="password"
          placeholder={saved.password ? t("已保存（不显示），留空则不修改") : t("坚果云等请使用「应用密码」")}
          value={pass}
          onChange={(e) => setPass(e.target.value)}
        />
      </div>
      <div className="row wrap" style={{ marginTop: 10 }}>
        <button className="btn" disabled={!changed || !!busy} onClick={saveLogin}>
          {t("保存")}
        </button>
        <button className="btn" disabled={!configured || changed || !!busy} onClick={() => act("test", api.webdavTest, t("WebDAV 连接正常"))}>
          {busy === "test" ? t("连接中…") : t("测试连接")}
        </button>
        <button
          className="btn primary"
          disabled={!configured || changed || !!busy}
          onClick={() => act("upload", async () => toast(t("已上传 {name}", { name: await api.backupUpload() }))).then(() => list && refreshList())}
        >
          {busy === "upload" ? t("上传中…") : t("立即备份")}
        </button>
        <button className="btn" disabled={!configured || changed || !!busy} onClick={refreshList}>
          {busy === "list" ? t("读取中…") : t("查看云端备份")}
        </button>
        <span className="spacer" />
        <button className="btn" disabled={!!busy} onClick={() => act("export", async () => toast(t("已导出到 {path}", { path: await api.backupExport() })))}>
          {t("导出到文件")}
        </button>
        <label className="btn">
          {t("从文件恢复")}
          <input type="file" accept=".zip" hidden onChange={(e) => e.target.files?.[0] && importFile(e.target.files[0])} />
        </label>
      </div>
      {list && (
        <div className="backup-list">
          {list.length === 0 && <div className="small muted">{t("云端还没有备份")}</div>}
          {list.map((b) => (
            <div className="row backup-item" key={b.name}>
              <span className="mono small">{b.name}</span>
              <span className="small muted">{fmtBytes(b.size)}</span>
              <span className="spacer" />
              <button className="btn sm" disabled={!!busy} onClick={() => restore(b.name)}>
                {busy === b.name ? t("恢复中…") : t("恢复")}
              </button>
              <button
                className="btn sm danger"
                disabled={!!busy}
                onClick={() => confirm(t("删除云端备份「{name}」？", { name: b.name })) && act("del", () => api.backupDelete(b.name), t("已删除")).then(refreshList)}
              >
                {t("删除")}
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="small muted" style={{ marginTop: 10 }}>
        {t("备份包含所有订阅和设置，保存在 WebDAV 的 prism/ 目录下。不包含 WebDAV 密码和本机的内核路径；恢复时本机的系统代理、TUN 开关也保持不变。密码用 Windows 账户加密保存在本机。")}
      </div>
    </div>
  );
}

/** TUN 服务: lets a normal-user Prism run TUN cores. */
function TunServiceCard() {
  const { snap, run } = useApp();
  const [busy, setBusy] = useState(false);
  if (!snap) return null;
  const st = snap.tun_service;
  const mac = snap.platform === "macos";
  const act = async (fn: () => Promise<unknown>, ok: string) => {
    setBusy(true);
    await run(fn, ok);
    setBusy(false);
  };
  return (
    <div className="card span-2">
      <h3>{t("TUN 服务")}</h3>
      <div className="row wrap">
        <span className={`dot ${st === "running" ? "on" : ""}`} />
        <span>{st === "running" ? t("已安装，运行中") : st === "stopped" ? t("已安装，未运行（请重新安装）") : t("未安装")}</span>
        {snap.elevated && <span className="tag">{t("Prism 当前以管理员身份运行，TUN 不经过服务")}</span>}
        <span className="spacer" />
        <button className="btn" disabled={busy} onClick={() => act(api.installTunService, st === "missing" ? t("TUN 服务已安装") : t("TUN 服务已更新"))}>
          {busy ? t("处理中…") : st === "missing" ? t("安装") : t("重新安装 / 更新内核")}
        </button>
        {st !== "missing" && (
          <button
            className="btn danger"
            disabled={busy}
            onClick={() =>
              confirm(mac ? t("卸载 TUN 服务？之后需要重新安装才能开启 TUN。") : t("卸载 TUN 服务？之后开启 TUN 需要以管理员身份运行 Prism。")) &&
              act(api.uninstallTunService, t("TUN 服务已卸载"))
            }
          >
            {t("卸载")}
          </button>
        )}
      </div>
      <div className="small muted" style={{ marginTop: 10 }}>
        {mac
          ? t("macOS 上 TUN 需要这个服务：开启 TUN 时内核由系统后台服务（LaunchDaemon，以 root 运行）运行，Prism 本身不需要管理员权限。安装、更新和卸载各需要输入一次 Mac 的登录密码。服务只运行安装时复制到受保护目录的内核，只接受当前用户的 Prism 的请求。Prism 或内核更新后会提示更新服务。")
          : t("安装后，开启 TUN 时内核由系统服务（Prism TUN Service）运行，Prism 本身不需要管理员权限，开机自启也能直接使用 TUN。安装、更新和卸载各需要一次管理员授权。服务只运行安装时复制到受保护目录的内核，只接受当前 Windows 用户的 Prism 的请求。Prism 或内核更新后会提示更新服务。")}
      </div>
    </div>
  );
}

/** 开机自启 / 静默启动 and which program opens clash:// links. */
function StartupCard({ form, set }: { form: Settings; set: <K extends keyof Settings>(k: K, v: Settings[K]) => void }) {
  const { snap, run } = useApp();
  if (!snap) return null;
  const handler = snap.clash_handler;
  const mac = snap.platform === "macos";
  return (
    <div className="card span-2">
      <h3>{t("启动与链接")}</h3>
      <div className="field">
        <label>{t("开机自启")}</label>
        <div className="row">
          <button
            className={`switch ${snap.autostart ? "on" : ""}`}
            onClick={() => run(() => api.setAutostart(!snap.autostart), snap.autostart ? t("已关闭开机自启") : t("已开启开机自启"))}
          />
          <span className="small muted">{mac ? t("登录 Mac 时启动 Prism（立即生效）") : t("登录 Windows 时启动 Prism（立即生效）")}</span>
        </div>
      </div>
      <div className="field">
        <label>{t("静默启动")}</label>
        <div className="row">
          <button className={`switch ${form.silent_start ? "on" : ""}`} onClick={() => set("silent_start", !form.silent_start)} />
          <span className="small muted">{t("开机自启时只在托盘运行，不弹出主界面")}</span>
        </div>
      </div>
      {!mac && <div className="field">
        <label>{t("接管 clash:// 链接")}</label>
        <div className="row">
          <button
            className={`switch ${snap.clash_links ? "on" : ""}`}
            onClick={() =>
              run(() => api.setClashLinks(!snap.clash_links), snap.clash_links ? t("clash:// 链接已还给原来的程序") : t("clash:// 链接将由 Prism 打开"))
            }
          />
          <span className="small muted">
            {snap.clash_links
              ? t("机场面板的「一键导入 Clash」会打开 Prism；关闭后还给原来的程序")
              : handler
                ? t("当前由 {app} 打开；开启后由 Prism 打开，关闭时还原", { app: handler })
                : t("开启后机场面板的「一键导入 Clash」会打开 Prism")}
          </span>
        </div>
      </div>}
      <div className="small muted" style={{ marginTop: 8 }}>
        {mac
          ? t("prism:// 和 clash:// 链接由 Prism 打开（装有其他 Clash 客户端时，clash:// 可能由 macOS 选定的程序打开）。通过链接导入订阅前都会先弹窗确认。")
          : t("prism:// 链接始终由 Prism 打开（安装版）。通过链接导入订阅前都会先弹窗确认。")}
      </div>
    </div>
  );
}

export default function SettingsPage() {
  const { snap, run } = useApp();
  const [form, setForm] = useState<Settings | null>(null);
  const [versions, setVersions] = useState<Record<string, string | null>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);

  useEffect(() => {
    if (snap && !form) setForm(snap.settings);
  }, [snap, form]);

  const loadVersions = () => api.versions().then(setVersions).catch(() => {});
  useEffect(() => {
    loadVersions();
  }, []);

  if (!snap || !form) return null;
  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => setForm({ ...form, [k]: v });
  const dirty = JSON.stringify({ ...form, selections: null }) !== JSON.stringify({ ...snap.settings, selections: null });

  const save = async () => {
    setBusy("save");
    if (await run(() => api.saveSettings(form), t("设置已保存"))) setForm(null);
    setBusy(null);
  };

  const download = async (k: CoreKind) => {
    setBusy(k);
    await run(async () => {
      const tag = await api.downloadCore(k);
      await loadVersions();
      return tag;
    }, t("{core} 下载完成", { core: t(CORE_NAMES[k]) }));
    setBusy(null);
  };

  return (
    <>
      <div className="page-head">
        <h1>{t("设置")}</h1>
        {dirty && (
          <button className="btn" onClick={() => setForm(snap.settings)}>
            {t("还原")}
          </button>
        )}
        <button className="btn primary" disabled={!dirty || busy === "save"} onClick={save}>
          {busy === "save" ? t("保存中…") : t("保存")}
        </button>
      </div>

      <div className="grid cols-2">
        <div className="card span-2">
          <h3>{t("内核")}</h3>
          {(["prism", "mihomo", "singbox"] as CoreKind[]).map((k) => (
            <div className="field" key={k}>
              <label>{t(CORE_NAMES[k])}</label>
              <div className="row">
                <span className={`dot ${snap.core_paths[k].exists ? "on" : ""}`} />
                <span className="small mono" style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }} title={snap.core_paths[k].path}>
                  {versions[k] ?? (snap.core_paths[k].exists ? t("已安装") : t("未安装"))}
                </span>
                {k !== "prism" && (
                  <>
                    <input
                      className="input small"
                      style={{ width: 260 }}
                      placeholder={t("自定义路径（默认 {exe}）", {
                        exe: (k === "mihomo" ? "mihomo" : "sing-box") + (snap.platform === "macos" ? "" : ".exe"),
                      })}
                      value={(k === "mihomo" ? form.mihomo_path : form.singbox_path) ?? ""}
                      onChange={(e) => set(k === "mihomo" ? "mihomo_path" : "singbox_path", e.target.value || null)}
                    />
                    <button className="btn sm" disabled={!!busy} onClick={() => download(k)}>
                      {busy === k ? t("下载中…") : snap.core_paths[k].exists ? t("更新") : t("下载")}
                    </button>
                  </>
                )}
              </div>
            </div>
          ))}
          <div className="small muted" style={{ marginTop: 8 }}>
            {t("内核从 GitHub Releases 下载；内核运行中时会通过代理下载。")}
          </div>
        </div>

        <div className="card">
          <h3>{t("端口与网络")}</h3>
          <div className="field">
            <label>{t("混合代理端口")}</label>
            <input className="input" type="number" value={form.mixed_port} onChange={(e) => set("mixed_port", Number(e.target.value))} />
          </div>
          <div className="field">
            <label>{t("API 端口")}</label>
            <input className="input" type="number" value={form.controller_port} onChange={(e) => set("controller_port", Number(e.target.value))} />
          </div>
          <div className="field">
            <label>{t("API 密钥")}</label>
            <input className="input mono" value={form.secret} onChange={(e) => set("secret", e.target.value)} />
          </div>
          <div className="field">
            <label>{t("允许局域网连接")}</label>
            <button className={`switch ${form.allow_lan ? "on" : ""}`} onClick={() => set("allow_lan", !form.allow_lan)} />
          </div>
          <div className="field">
            <label>{t("日志级别")}</label>
            <select className="input" value={form.log_level} onChange={(e) => set("log_level", e.target.value)}>
              {["debug", "info", "warning", "error", "silent"].map((l) => (
                <option key={l}>{l}</option>
              ))}
            </select>
          </div>
        </div>

        <div className="card">
          <h3>{t("系统代理与测速")}</h3>
          <div className="field">
            <label>{t("绕过地址")}</label>
            <textarea className="input" rows={4} value={form.proxy_bypass} onChange={(e) => set("proxy_bypass", e.target.value)} />
          </div>
          <div className="field">
            <label>{t("测速地址")}</label>
            <input className="input" value={form.test_url} onChange={(e) => set("test_url", e.target.value)} />
          </div>
          <div className="field">
            <label>{t("统一延迟")}</label>
            <div className="row">
              <button className={`switch ${form.unified_delay ? "on" : ""}`} onClick={() => set("unified_delay", !form.unified_delay)} />
              <span className="small muted">{t("只计第二次请求，排除握手耗时（Prism / mihomo 内核）")}</span>
            </div>
          </div>
        </div>

        <div className="card span-2">
          <h3>{t("数据")}</h3>
          <div className="row wrap">
            <button
              className="btn"
              disabled={!!busy}
              onClick={async () => {
                setBusy("geo");
                await run(api.updateGeodata, t("GeoSite / GeoIP 数据已更新，重启内核后生效"));
                setBusy(null);
              }}
            >
              {busy === "geo" ? t("更新中…") : t("更新 GeoSite / GeoIP")}
            </button>
            <button className="btn" onClick={() => run(async () => setPreview(await api.previewConfig()))}>
              {t("查看生成的内核配置")}
            </button>
            <button className="btn" onClick={() => run(api.openDataDir)}>
              {t("打开数据目录")}
            </button>
            <span className="small muted mono">{snap.data_dir}</span>
          </div>
        </div>

        <AppearanceCard form={form} set={set} />

        <StartupCard form={form} set={set} />

        <TunServiceCard />

        <BackupCard />

        <UpdateCard form={form} set={set} />
      </div>

      {preview !== null && (
        <div className="modal-bg" onClick={() => setPreview(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>{t("运行时配置（{core}）", { core: t(CORE_NAMES[snap.settings.core]) })}</h2>
            <textarea className="input" rows={26} readOnly value={preview} />
            <div className="row">
              <span className="spacer" />
              <button className="btn" onClick={() => setPreview(null)}>
                {t("关闭")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
