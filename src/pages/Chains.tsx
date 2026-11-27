import Help from "../Help";
import { useEffect, useState } from "react";
import { api, errText, type ChainRule, type Outbounds } from "../api";
import { useApp } from "../state";
import { useOutbounds } from "./Apps";
import { t } from "../i18n";

const enc = encodeURIComponent;

/** Add / edit dialog: a name and the hops, nodes or groups, in order. */
function ChainDialog({
  initial,
  outs,
  taken,
  onClose,
  onSave,
}: {
  initial: Partial<ChainRule>;
  outs: Outbounds | null;
  taken: string[];
  onClose: () => void;
  onSave: (c: ChainRule) => Promise<boolean>;
}) {
  const [name, setName] = useState(initial.name ?? "");
  const [hops, setHops] = useState<string[]>(initial.hops ?? []);
  const [saving, setSaving] = useState(false);
  const groups = outs?.groups ?? [];
  const nodes = outs?.proxies ?? [];
  const all = [...nodes, ...groups];
  useEffect(() => {
    if (hops.length === 0 && nodes.length > 0) setHops([nodes[0], nodes[1] ?? nodes[0]]);
  }, [hops.length, nodes]);

  const clean = name.trim();
  const nameBad = clean.includes(",") || (clean !== initial.name && taken.includes(clean));
  const hopsBad = hops.length < 2 || new Set(hops).size !== hops.length;

  const save = async () => {
    setSaving(true);
    const ok = await onSave({ id: initial.id ?? crypto.randomUUID(), enabled: initial.enabled ?? true, name: clean, hops });
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{initial.id ? t("编辑链式代理") : t("添加链式代理")}</h2>
        <div className="field">
          <label>{t("链路名称")}</label>
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        {hops.map((h, i) => (
          <div className="field" key={i}>
            <label>{i === 0 ? t("第一跳") : i === hops.length - 1 ? t("落地") : t("第 {n} 跳", { n: i + 1 })}</label>
            <div className="row">
              <select className="input" style={{ flex: 1 }} value={h} onChange={(e) => setHops(hops.map((x, j) => (j === i ? e.target.value : x)))}>
                {!all.includes(h) && <option value={h}>{t("{v}（当前订阅中不存在）", { v: h })}</option>}
                {groups.length > 0 && (
                  <optgroup label={t("代理组")}>
                    {groups.map((g) => (
                      <option key={g} value={g}>
                        {g}
                      </option>
                    ))}
                  </optgroup>
                )}
                {nodes.length > 0 && (
                  <optgroup label={t("节点")}>
                    {nodes.map((n) => (
                      <option key={n} value={n}>
                        {n}
                      </option>
                    ))}
                  </optgroup>
                )}
              </select>
              <button className="btn sm danger" disabled={hops.length <= 2} onClick={() => setHops(hops.filter((_, j) => j !== i))}>
                {t("删除")}
              </button>
            </div>
          </div>
        ))}
        <div className="row">
          <button className="btn sm" disabled={nodes.length === 0} onClick={() => setHops([...hops, all.find((n) => !hops.includes(n)) ?? nodes[0]])}>
            {t("添加一跳")}
          </button>
        </div>
        <div className="small" style={{ color: "var(--bad)" }}>
          {nameBad && t("名字已被订阅中的节点或代理组使用")}
          {hopsBad && nodes.length > 0 && t("至少选择两个不同的节点或代理组")}
        </div>
        <div className="row">
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            {t("取消")}
          </button>
          <button className="btn primary" disabled={saving || !clean || nameBad || hopsBad} onClick={save}>
            {saving ? t("保存中…") : t("保存")}
          </button>
        </div>
      </div>
    </div>
  );
}

export default function Chains() {
  const { snap, toast, refresh } = useApp();
  const outs = useOutbounds();
  const [dialog, setDialog] = useState<Partial<ChainRule> | null>(null);
  const [delays, setDelays] = useState<Record<string, string>>({});
  const chains = snap?.settings.chains ?? [];
  const testUrl = snap?.settings.test_url ?? "";
  const mine = chains.map((c) => c.name);
  const taken = [...(outs?.proxies ?? []), ...(outs?.groups ?? []), "DIRECT", "REJECT", "GLOBAL", ...mine];

  const save = async (next: ChainRule[], ok = t("已保存")) => {
    try {
      await api.setChains(next);
      toast(snap?.running ? t("{msg}，内核已重新加载", { msg: ok }) : ok);
      return true;
    } catch (e) {
      toast(errText(e), "err");
      return false;
    } finally {
      refresh();
    }
  };
  const upsert = (c: ChainRule) => save(chains.some((x) => x.id === c.id) ? chains.map((x) => (x.id === c.id ? c : x)) : [...chains, c]);

  const test = async (c: ChainRule) => {
    setDelays((d) => ({ ...d, [c.id]: "…" }));
    try {
      const r = await api.core<{ delay: number }>("GET", `/proxies/${enc(c.name)}/delay?timeout=8000&url=${enc(testUrl)}`);
      setDelays((d) => ({ ...d, [c.id]: `${r.delay} ms` }));
    } catch {
      setDelays((d) => ({ ...d, [c.id]: t("连接失败") }));
    }
  };

  return (
    <>
      <div className="page-head">
        <h1>{t("链式代理")}</h1>
        <span className="tag">{t("{n} 条", { n: chains.length })}</span>
        <button className="btn primary" onClick={() => setDialog({})}>
          {t("添加链路")}
        </button>
      </div>

      <Help title={t("链式代理")}><div>
        {t("让流量依次经过多个节点或代理组，网站看到的是最后一跳的 IP。保存后，可以作为应用分流、域名分流的出口，也可以在全局模式下选择。三种内核都支持。")}
        {t("每多一跳，延迟通常更高，任何一个节点出问题都会让整条链路中断。走 UDP 的节点（TUIC、Hysteria2、WireGuard）要求前一跳支持 UDP。")}
      </div></Help>

      {chains.length === 0 && <div className="empty">{t("还没有链路。点「添加链路」，选择流量依次经过的节点或代理组。")}</div>}

      {chains.map((c) => {
        const missing = outs ? c.hops.find((h) => !outs.proxies.includes(h) && !outs.groups.includes(h)) : undefined;
        const unusable = !!outs && c.enabled && !missing && !outs.chains.includes(c.name);
        return (
          <div className="card app-rule" key={c.id}>
            <button className={`switch ${c.enabled ? "on" : ""}`} title={c.enabled ? t("停用") : t("启用")} onClick={() => save(chains.map((x) => (x.id === c.id ? { ...x, enabled: !x.enabled } : x)))} />
            <div className="app-name" title={c.hops.join(" → ")}>
              <b>{c.name}</b>
              <span className="muted small">{c.hops.join(" → ")}</span>
              {missing && <span className="small" style={{ color: "var(--bad)" }}>{t("当前订阅缺少 {v}，这条链路不会生效", { v: missing })}</span>}
              {unusable && <span className="small" style={{ color: "var(--bad)" }}>{t("这条链路不会生效，原因见日志")}</span>}
            </div>
            <span className="tag">{t("{n} 跳", { n: c.hops.length })}</span>
            {delays[c.id] && <span className="small muted">{delays[c.id]}</span>}
            <button className="btn sm" disabled={!snap?.running || !c.enabled || !!missing || unusable} onClick={() => test(c)}>
              {t("测试")}
            </button>
            <button className="btn sm" onClick={() => setDialog(c)}>
              {t("编辑")}
            </button>
            <button className="btn sm danger" onClick={() => save(chains.filter((x) => x.id !== c.id), t("已删除"))}>
              {t("删除")}
            </button>
          </div>
        );
      })}

      {dialog && <ChainDialog initial={dialog} outs={outs} taken={taken} onClose={() => setDialog(null)} onSave={upsert} />}
    </>
  );
}
