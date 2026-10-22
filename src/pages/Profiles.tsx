import { useEffect, useState } from "react";
import { api, fmtAgo, fmtBytes, type ProfileMeta } from "../api";
import { useApp } from "../state";

const FORMAT_NAMES = { clash: "Clash", singbox: "sing-box", uri: "分享链接" };

function Editor({ profile, onClose }: { profile: ProfileMeta | null; onClose: () => void }) {
  const { run } = useApp();
  const [name, setName] = useState(profile?.name ?? "");
  const [content, setContent] = useState<string | null>(profile ? null : "");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (profile) api.readProfile(profile.id).then(setContent);
  }, [profile]);

  const save = async () => {
    setSaving(true);
    const ok = await run(async () => {
      if (profile) {
        if (name !== profile.name) await api.renameProfile(profile.id, name);
        await api.saveProfileContent(profile.id, content ?? "");
      } else {
        await api.importProfile({ content: content ?? "", name });
      }
    }, "已保存");
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{profile ? "编辑订阅" : "新建本地配置"}</h2>
        <input className="input" placeholder="名称" value={name} onChange={(e) => setName(e.target.value)} />
        <textarea
          className="input"
          rows={20}
          spellCheck={false}
          placeholder="粘贴 Clash YAML、sing-box JSON，或 anytls:// ss:// trojan:// vless:// 等分享链接（每行一个）"
          value={content ?? "加载中…"}
          onChange={(e) => setContent(e.target.value)}
        />
        {profile?.url && <div className="small muted">远程订阅更新时会覆盖这里的修改。</div>}
        <div className="row">
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            取消
          </button>
          <button className="btn primary" disabled={saving || !content?.trim()} onClick={save}>
            {saving ? "保存中…" : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default function Profiles() {
  const { snap, run } = useApp();
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [editing, setEditing] = useState<ProfileMeta | null | undefined>(undefined);

  if (!snap) return null;
  const active = snap.settings.active_profile;

  const importUrl = async () => {
    setBusy("import");
    if (await run(() => api.importProfile({ url }), "订阅已导入")) setUrl("");
    setBusy(null);
  };

  const act = async (id: string, fn: () => Promise<unknown>, ok: string) => {
    setBusy(id);
    await run(fn, ok);
    setBusy(null);
  };

  const updateAll = async () => {
    setBusy("all");
    for (const p of snap.profiles.filter((p) => p.url)) {
      await run(() => api.updateProfile(p.id));
    }
    setBusy(null);
  };

  return (
    <>
      <div className="page-head">
        <h1>订阅</h1>
        <button className="btn" disabled={!!busy} onClick={updateAll}>
          {busy === "all" ? "更新中…" : "全部更新"}
        </button>
        <button className="btn" onClick={() => setEditing(null)}>
          新建本地配置
        </button>
      </div>

      <div className="card" style={{ marginBottom: 16 }}>
        <div className="row">
          <input
            className="input"
            style={{ flex: 1 }}
            placeholder="订阅链接（支持 Clash / sing-box / 通用 Base64 订阅）"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && url.trim() && importUrl()}
          />
          <button className="btn primary" disabled={!url.trim() || !!busy} onClick={importUrl}>
            {busy === "import" ? "导入中…" : "导入"}
          </button>
        </div>
      </div>

      {snap.profiles.length === 0 && <div className="empty">还没有订阅，粘贴订阅链接导入吧</div>}

      <div className="grid cols-3">
        {snap.profiles.map((p) => {
          const u = p.usage;
          const used = u ? u.upload + u.download : 0;
          return (
            <div
              key={p.id}
              className={`card profile ${active === p.id ? "active" : ""}`}
              onClick={() => active !== p.id && act(p.id, () => api.activateProfile(p.id), `已切换到 ${p.name}`)}
            >
              <div className="row">
                <div className="pname" title={p.name}>
                  {p.name}
                </div>
                <span className="spacer" />
                {active === p.id && <span className="tag accent">使用中</span>}
              </div>
              <div className="row small muted" style={{ marginTop: 6 }}>
                <span>{p.nodes} 个节点</span>
                <span>·</span>
                <span>{FORMAT_NAMES[p.format]}</span>
                <span>·</span>
                <span>{fmtAgo(p.updated_at)}更新</span>
              </div>
              {u && u.total > 0 && (
                <>
                  <div className="bar">
                    <div style={{ width: `${Math.min(100, (used / u.total) * 100)}%` }} />
                  </div>
                  <div className="row small muted">
                    <span>
                      {fmtBytes(used)} / {fmtBytes(u.total)}
                    </span>
                    <span className="spacer" />
                    {u.expire > 0 && <span>到期 {new Date(u.expire * 1000).toLocaleDateString()}</span>}
                  </div>
                </>
              )}
              <div className="row" style={{ marginTop: 12 }} onClick={(e) => e.stopPropagation()}>
                {p.url && (
                  <button className="btn sm" disabled={!!busy} onClick={() => act(p.id, () => api.updateProfile(p.id), "订阅已更新")}>
                    {busy === p.id ? "更新中…" : "更新"}
                  </button>
                )}
                <button className="btn sm" onClick={() => setEditing(p)}>
                  编辑
                </button>
                <span className="spacer" />
                <button
                  className="btn sm danger"
                  disabled={!!busy}
                  onClick={() => confirm(`删除订阅「${p.name}」？`) && act(p.id, () => api.deleteProfile(p.id), "已删除")}
                >
                  删除
                </button>
              </div>
            </div>
          );
        })}
      </div>

      {editing !== undefined && <Editor profile={editing} onClose={() => setEditing(undefined)} />}
    </>
  );
}
