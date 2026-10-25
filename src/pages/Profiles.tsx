import { useEffect, useState } from "react";
import { api, fmtAgo, fmtBytes, fmtInterval, type Patches, type ProfileMeta } from "../api";
import { useApp } from "../state";
import { t } from "../i18n";

const FORMAT_NAMES = { clash: "Clash", singbox: "sing-box", uri: "分享链接" };

function Editor({ profile, onClose }: { profile: ProfileMeta | null; onClose: () => void }) {
  const { run } = useApp();
  const [name, setName] = useState(profile?.name ?? "");
  const [content, setContent] = useState<string | null>(profile ? null : "");
  const [every, setEvery] = useState(String(profile?.interval ?? 1440));
  const [saving, setSaving] = useState(false);
  const minutes = Number(every);
  const badInterval = !/^\d+$/.test(every.trim());

  useEffect(() => {
    if (profile) api.readProfile(profile.id).then(setContent);
  }, [profile]);

  const save = async () => {
    setSaving(true);
    const ok = await run(async () => {
      if (profile) {
        if (name !== profile.name) await api.renameProfile(profile.id, name);
        if (profile.url && minutes !== profile.interval) await api.setProfileInterval(profile.id, minutes);
        await api.saveProfileContent(profile.id, content ?? "");
      } else {
        await api.importProfile({ content: content ?? "", name });
      }
    }, t("已保存"));
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{profile ? t("编辑订阅") : t("新建本地配置")}</h2>
        <input className="input" placeholder={t("名称")} value={name} onChange={(e) => setName(e.target.value)} />
        {profile?.url && (
          <div className="row">
            <span className="small" style={{ whiteSpace: "nowrap" }}>
              {t("自动更新间隔")}
            </span>
            <input className="input" style={{ width: 110 }} value={every} onChange={(e) => setEvery(e.target.value)} />
            <span className="small muted">
              {badInterval ? t("请输入分钟数") : minutes ? t("分钟（每 {every}更新一次）", { every: fmtInterval(minutes) }) : t("分钟（0 = 不自动更新）")}
            </span>
          </div>
        )}
        <textarea
          className="input"
          rows={20}
          spellCheck={false}
          placeholder={t("粘贴 Clash YAML、sing-box JSON，或 anytls:// ss:// trojan:// vless:// 等分享链接（每行一个）")}
          value={content ?? t("加载中…")}
          onChange={(e) => setContent(e.target.value)}
        />
        {profile?.url && <div className="small muted">{t("远程订阅更新时会覆盖这里的修改。")}</div>}
        <div className="row">
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            {t("取消")}
          </button>
          <button className="btn primary" disabled={saving || !content?.trim() || (!!profile?.url && badInterval)} onClick={save}>
            {saving ? t("保存中…") : t("保存")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** 覆写 editor: Merge (YAML) and Script (JS), for every profile or for one
 * (`profile`), applied after the global set. */
function PatchesEditor({ profile, live, onClose }: { profile: ProfileMeta | null; live: boolean; onClose: () => void }) {
  const { run } = useApp();
  const [p, setP] = useState<Patches | null>(null);
  const [tab, setTab] = useState<"merge" | "script">("merge");
  const [saving, setSaving] = useState(false);
  const id = profile?.id ?? null;
  useEffect(() => {
    api.getPatches(id).then(setP);
  }, [id]);
  if (!p) return null;
  const save = async () => {
    setSaving(true);
    const ok = await run(() => api.savePatches(p, id), live ? t("覆写已保存并生效") : t("覆写已保存"));
    setSaving(false);
    if (ok) onClose();
  };
  const enabledKey = tab === "merge" ? "merge_enabled" : "script_enabled";
  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{profile ? t("覆写 · {name}", { name: profile.name }) : t("全局覆写")}</h2>
        <div className="small muted">
          {profile
            ? t("只对这个订阅生效，在全局覆写之后应用（先 Merge，后 Script）。远程订阅更新后也会保留。")
            : t("对所有订阅生效，在生成内核配置前应用（先 Merge，后 Script）。远程订阅更新后也会保留。")}
        </div>
        <div className="row">
          <div className="seg">
            <button className={tab === "merge" ? "on" : ""} onClick={() => setTab("merge")}>
              {t("Merge（YAML）")}{p.merge_enabled ? t(" ·启用") : ""}
            </button>
            <button className={tab === "script" ? "on" : ""} onClick={() => setTab("script")}>
              {t("Script（JavaScript）")}{p.script_enabled ? t(" ·启用") : ""}
            </button>
          </div>
          <span className="spacer" />
          <span className="small muted">{t("启用")}</span>
          <button className={`switch ${p[enabledKey] ? "on" : ""}`} onClick={() => setP({ ...p, [enabledKey]: !p[enabledKey] })} />
        </div>
        <textarea
          className="input mono"
          rows={20}
          spellCheck={false}
          value={tab === "merge" ? p.merge : p.script}
          onChange={(e) => setP({ ...p, [tab]: e.target.value })}
        />
        <div className="small muted">
          {tab === "merge"
            ? t("prepend-rules / append-rules（加在 MATCH 之前）、prepend-proxies / append-proxies、prepend-proxy-groups / append-proxy-groups 增加条目；其他键直接覆盖，字典逐项合并。")
            : t("function main(config, profileName) { … return config; }。config 是订阅转换成的 Clash 配置；不能联网或读写文件。")}
        </div>
        <div className="row">
          <span className="small muted">{profile ? t("保存前会先用这个订阅试运行，出错不会保存。") : t("保存前会先用当前订阅试运行，出错不会保存。")}</span>
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            {t("取消")}
          </button>
          <button className="btn primary" disabled={saving} onClick={save}>
            {saving ? t("保存中…") : t("保存")}
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
  // undefined = closed, null = the global set
  const [patching, setPatching] = useState<ProfileMeta | null | undefined>(undefined);
  const [patched, setPatched] = useState<string[]>([]);
  const refreshPatched = () => api.patchedProfiles().then(setPatched, () => {});
  useEffect(() => {
    refreshPatched();
  }, []);

  if (!snap) return null;
  const active = snap.settings.active_profile;

  const importUrl = async () => {
    setBusy("import");
    if (await run(() => api.importProfile({ url }), t("订阅已导入"))) setUrl("");
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
        <h1>{t("订阅")}</h1>
        <button className="btn" disabled={!!busy} onClick={updateAll}>
          {busy === "all" ? t("更新中…") : t("全部更新")}
        </button>
        <button className="btn" onClick={() => setEditing(null)}>
          {t("新建本地配置")}
        </button>
        <button className="btn" onClick={() => setPatching(null)}>
          {t("全局覆写")}
        </button>
      </div>

      <div className="card" style={{ marginBottom: 16 }}>
        <div className="row">
          <input
            className="input"
            style={{ flex: 1 }}
            placeholder={t("订阅链接（支持 Clash / sing-box / 通用 Base64 订阅）")}
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && url.trim() && importUrl()}
          />
          <button className="btn primary" disabled={!url.trim() || !!busy} onClick={importUrl}>
            {busy === "import" ? t("导入中…") : t("导入")}
          </button>
        </div>
      </div>

      {snap.profiles.length === 0 && <div className="empty">{t("还没有订阅，粘贴订阅链接导入吧")}</div>}

      <div className="grid cols-3">
        {snap.profiles.map((p) => {
          const u = p.usage;
          const used = u ? u.upload + u.download : 0;
          return (
            <div
              key={p.id}
              className={`card profile ${active === p.id ? "active" : ""}`}
              onClick={() => active !== p.id && act(p.id, () => api.activateProfile(p.id), t("已切换到 {name}", { name: p.name }))}
            >
              <div className="row">
                <div className="pname" title={p.name}>
                  {p.name}
                </div>
                <span className="spacer" />
                {patched.includes(p.id) && <span className="tag" title={t("这个订阅有自己的覆写")}>{t("已覆写")}</span>}
                {active === p.id && <span className="tag accent">{t("使用中")}</span>}
              </div>
              <div className="row small muted" style={{ marginTop: 6 }}>
                <span>{t("{n} 个节点", { n: p.nodes })}</span>
                <span>·</span>
                <span>{t(FORMAT_NAMES[p.format])}</span>
                <span>·</span>
                <span>{t("{ago}更新", { ago: fmtAgo(p.updated_at) })}</span>
                {p.url && (
                  <>
                    <span>·</span>
                    <span title={t("可在「编辑」中修改")}>{p.interval ? t("每 {every}", { every: fmtInterval(p.interval) }) : t("不自动更新")}</span>
                  </>
                )}
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
                    {u.expire > 0 && <span>{t("到期 {date}", { date: new Date(u.expire * 1000).toLocaleDateString() })}</span>}
                  </div>
                </>
              )}
              <div className="row" style={{ marginTop: 12 }} onClick={(e) => e.stopPropagation()}>
                {p.url && (
                  <button className="btn sm" disabled={!!busy} onClick={() => act(p.id, () => api.updateProfile(p.id), t("订阅已更新"))}>
                    {busy === p.id ? t("更新中…") : t("更新")}
                  </button>
                )}
                <button className="btn sm" onClick={() => setEditing(p)}>
                  {t("编辑")}
                </button>
                <button className="btn sm" onClick={() => setPatching(p)}>
                  {t("覆写")}
                </button>
                <span className="spacer" />
                <button
                  className="btn sm danger"
                  disabled={!!busy}
                  onClick={() => confirm(t("删除订阅「{name}」？", { name: p.name })) && act(p.id, () => api.deleteProfile(p.id), t("已删除"))}
                >
                  {t("删除")}
                </button>
              </div>
            </div>
          );
        })}
      </div>

      {editing !== undefined && <Editor profile={editing} onClose={() => setEditing(undefined)} />}
      {patching !== undefined && (
        <PatchesEditor
          profile={patching}
          live={!patching || patching.id === active}
          onClose={() => {
            setPatching(undefined);
            refreshPatched();
          }}
        />
      )}
    </>
  );
}
