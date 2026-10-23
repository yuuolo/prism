import { useEffect, useState } from "react";
import { api, CORE_NAMES, type CoreKind, type Settings } from "../api";
import { useApp } from "../state";

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
    if (await run(() => api.saveSettings(form), "设置已保存")) setForm(null);
    setBusy(null);
  };

  const download = async (k: CoreKind) => {
    setBusy(k);
    await run(async () => {
      const tag = await api.downloadCore(k);
      await loadVersions();
      return tag;
    }, `${CORE_NAMES[k]} 下载完成`);
    setBusy(null);
  };

  return (
    <>
      <div className="page-head">
        <h1>设置</h1>
        {dirty && (
          <button className="btn" onClick={() => setForm(snap.settings)}>
            还原
          </button>
        )}
        <button className="btn primary" disabled={!dirty || busy === "save"} onClick={save}>
          {busy === "save" ? "保存中…" : "保存"}
        </button>
      </div>

      <div className="grid cols-2">
        <div className="card span-2">
          <h3>内核</h3>
          {(["prism", "mihomo", "singbox"] as CoreKind[]).map((k) => (
            <div className="field" key={k}>
              <label>{CORE_NAMES[k]}</label>
              <div className="row">
                <span className={`dot ${snap.core_paths[k].exists ? "on" : ""}`} />
                <span className="small mono" style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }} title={snap.core_paths[k].path}>
                  {versions[k] ?? (snap.core_paths[k].exists ? "已安装" : "未安装")}
                </span>
                {k !== "prism" && (
                  <>
                    <input
                      className="input small"
                      style={{ width: 260 }}
                      placeholder={`自定义路径（默认 ${k === "mihomo" ? "mihomo" : "sing-box"}.exe）`}
                      value={(k === "mihomo" ? form.mihomo_path : form.singbox_path) ?? ""}
                      onChange={(e) => set(k === "mihomo" ? "mihomo_path" : "singbox_path", e.target.value || null)}
                    />
                    <button className="btn sm" disabled={!!busy} onClick={() => download(k)}>
                      {busy === k ? "下载中…" : snap.core_paths[k].exists ? "更新" : "下载"}
                    </button>
                  </>
                )}
              </div>
            </div>
          ))}
          <div className="small muted" style={{ marginTop: 8 }}>
            内核从 GitHub Releases 下载；内核运行中时会通过代理下载。
          </div>
        </div>

        <div className="card">
          <h3>端口与网络</h3>
          <div className="field">
            <label>混合代理端口</label>
            <input className="input" type="number" value={form.mixed_port} onChange={(e) => set("mixed_port", Number(e.target.value))} />
          </div>
          <div className="field">
            <label>API 端口</label>
            <input className="input" type="number" value={form.controller_port} onChange={(e) => set("controller_port", Number(e.target.value))} />
          </div>
          <div className="field">
            <label>API 密钥</label>
            <input className="input mono" value={form.secret} onChange={(e) => set("secret", e.target.value)} />
          </div>
          <div className="field">
            <label>允许局域网连接</label>
            <button className={`switch ${form.allow_lan ? "on" : ""}`} onClick={() => set("allow_lan", !form.allow_lan)} />
          </div>
          <div className="field">
            <label>日志级别</label>
            <select className="input" value={form.log_level} onChange={(e) => set("log_level", e.target.value)}>
              {["debug", "info", "warning", "error", "silent"].map((l) => (
                <option key={l}>{l}</option>
              ))}
            </select>
          </div>
        </div>

        <div className="card">
          <h3>系统代理与测速</h3>
          <div className="field">
            <label>绕过地址</label>
            <textarea className="input" rows={4} value={form.proxy_bypass} onChange={(e) => set("proxy_bypass", e.target.value)} />
          </div>
          <div className="field">
            <label>测速地址</label>
            <input className="input" value={form.test_url} onChange={(e) => set("test_url", e.target.value)} />
          </div>
          <div className="field">
            <label>统一延迟</label>
            <div className="row">
              <button className={`switch ${form.unified_delay ? "on" : ""}`} onClick={() => set("unified_delay", !form.unified_delay)} />
              <span className="small muted">只计第二次请求，排除握手耗时（Prism / mihomo 内核）</span>
            </div>
          </div>
        </div>

        <div className="card span-2">
          <h3>数据</h3>
          <div className="row wrap">
            <button
              className="btn"
              disabled={!!busy}
              onClick={async () => {
                setBusy("geo");
                await run(api.updateGeodata, "GeoSite / GeoIP 数据已更新，重启内核后生效");
                setBusy(null);
              }}
            >
              {busy === "geo" ? "更新中…" : "更新 GeoSite / GeoIP"}
            </button>
            <button className="btn" onClick={() => run(async () => setPreview(await api.previewConfig()))}>
              查看生成的内核配置
            </button>
            <button className="btn" onClick={() => run(api.openDataDir)}>
              打开数据目录
            </button>
            <span className="small muted mono">{snap.data_dir}</span>
          </div>
        </div>
      </div>

      {preview !== null && (
        <div className="modal-bg" onClick={() => setPreview(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>运行时配置（{CORE_NAMES[snap.settings.core]}）</h2>
            <textarea className="input" rows={26} readOnly value={preview} />
            <div className="row">
              <span className="spacer" />
              <button className="btn" onClick={() => setPreview(null)}>
                关闭
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
