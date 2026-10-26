import { useEffect, useState } from "react";
import { api, errText, type RouteKind, type RouteRule } from "../api";
import { useApp } from "../state";
import { TargetSelect, useOutbounds } from "./Apps";
import { t } from "../i18n";

const KINDS: { kind: RouteKind; label: string; hint: string }[] = [
  { kind: "DOMAIN-SUFFIX", label: "域名及子域名", hint: "google.com（也匹配 www.google.com）" },
  { kind: "DOMAIN", label: "完整域名", hint: "www.google.com" },
  { kind: "DOMAIN-KEYWORD", label: "域名关键字", hint: "youtube" },
  { kind: "IP-CIDR", label: "IP / 网段", hint: "8.8.8.8 或 192.168.1.0/24" },
  { kind: "GEOSITE", label: "域名分类", hint: "netflix、openai、category-ads-all" },
  { kind: "GEOIP", label: "IP 地区", hint: "JP、US、telegram" },
];

const kindLabel = (k: RouteKind) => t(KINDS.find((x) => x.kind === k)?.label ?? k);
const isIp = (k: RouteKind) => k === "IP-CIDR" || k === "GEOIP";

/** Rough check before saving; the backend has the final word. */
function badValue(kind: RouteKind, v: string): boolean {
  if (!v || /[,\s]/.test(v)) return true;
  if (kind === "IP-CIDR") return !/^[0-9a-fA-F:.]+(\/\d{1,3})?$/.test(v);
  if (kind === "GEOSITE" || kind === "GEOIP") return !/^[A-Za-z0-9_\-@!.]+$/.test(v);
  return false;
}

/** Add / edit dialog. Adding takes several values, one per line. */
export function RouteRuleDialog({
  initial,
  onClose,
  onSave,
}: {
  initial: Partial<RouteRule>;
  onClose: () => void;
  onSave: (rules: RouteRule[]) => Promise<boolean>;
}) {
  const outs = useOutbounds();
  const editing = !!initial.id;
  const [kind, setKind] = useState<RouteKind>(initial.kind ?? "DOMAIN-SUFFIX");
  const [text, setText] = useState(initial.value ?? "");
  const [target, setTarget] = useState(initial.target ?? "");
  const [resolve, setResolve] = useState(!!initial.resolve);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (!target && outs) setTarget(outs.groups[0] ?? "DIRECT");
  }, [outs, target]);

  const values = text
    .split(/\r?\n/)
    .map((v) => v.trim())
    .filter(Boolean);
  const bad = values.filter((v) => badValue(kind, v));
  const hint = KINDS.find((x) => x.kind === kind)!.hint;

  const save = async () => {
    setSaving(true);
    const ok = await onSave(
      values.map((value, i) => ({
        id: editing && i === 0 ? initial.id! : crypto.randomUUID(),
        enabled: initial.enabled ?? true,
        kind,
        value,
        target: target || "DIRECT",
        resolve: isIp(kind) && resolve,
      })),
    );
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{editing ? t("编辑分流规则") : t("添加分流规则")}</h2>
        <div className="field">
          <label>{t("匹配方式")}</label>
          <select className="input" value={kind} onChange={(e) => setKind(e.target.value as RouteKind)}>
            {KINDS.map((k) => (
              <option key={k.kind} value={k.kind}>
                {t(k.label)} · {k.kind}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <label>{editing ? t("内容") : t("内容（每行一个，可一次添加多条）")}</label>
          {editing ? (
            <input className="input mono" placeholder={t(hint)} value={text} onChange={(e) => setText(e.target.value)} />
          ) : (
            <textarea className="input mono" rows={5} placeholder={t(hint)} value={text} onChange={(e) => setText(e.target.value)} />
          )}
        </div>
        <div className="field">
          <label>{t("出口")}</label>
          <TargetSelect value={target} outs={outs} onChange={setTarget} />
        </div>
        {isIp(kind) && (
          <label className="row small" style={{ gap: 8, cursor: "pointer" }}>
            <input type="checkbox" checked={resolve} onChange={(e) => setResolve(e.target.checked)} />
            <span>{t("也匹配解析到这些 IP 的域名（需要先解析域名，会慢一些；sing-box 内核不支持）")}</span>
          </label>
        )}
        <div className="small muted">
          {kind === "DOMAIN-SUFFIX" && t("可以直接粘贴网址，会自动取出域名。")}
          {(kind === "GEOSITE" || kind === "GEOIP") && t("分类名见 geosite.dat / geoip.dat（MetaCubeX meta-rules-dat）。")}{" "}
          {t("规则模式下生效，排在应用分流之后、订阅规则之前，从上到下第一条匹配的生效。")}
          {bad.length > 0 && <span style={{ color: "var(--bad)" }}> {t("格式不对：{v}", { v: bad.slice(0, 3).join("、") })}</span>}
        </div>
        <div className="row">
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            {t("取消")}
          </button>
          <button className="btn primary" disabled={saving || values.length === 0 || bad.length > 0 || (editing && values.length !== 1)} onClick={save}>
            {saving ? t("保存中…") : editing ? t("保存") : t("添加 {n} 条", { n: values.length })}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Saves the rule list, telling the user whether the core reloaded. */
export function useSaveRouteRules() {
  const { snap, toast, refresh } = useApp();
  return async (next: RouteRule[], ok = t("已保存")) => {
    try {
      await api.setRouteRules(next);
      toast(snap?.running ? t("{msg}，内核已重新加载", { msg: ok }) : ok);
      return true;
    } catch (e) {
      toast(errText(e), "err");
      return false;
    } finally {
      refresh();
    }
  };
}

export default function Routes() {
  const { snap } = useApp();
  const outs = useOutbounds();
  const save = useSaveRouteRules();
  const [dialog, setDialog] = useState<Partial<RouteRule> | null>(null);
  const rules = snap?.settings.route_rules ?? [];

  const upsert = (added: RouteRule[]) => {
    if (dialog?.id) return save(rules.map((x) => (x.id === dialog.id ? added[0] : x)));
    return save([...rules, ...added], t("已添加 {n} 条", { n: added.length }));
  };
  const move = (i: number) => {
    const next = [...rules];
    [next[i - 1], next[i]] = [next[i], next[i - 1]];
    save(next);
  };

  return (
    <>
      <div className="page-head">
        <h1>{t("域名 / IP 分流")}</h1>
        <span className="tag">{t("{n} 条", { n: rules.length })}</span>
        <button className="btn primary" onClick={() => setDialog({})}>
          {t("添加规则")}
        </button>
      </div>

      <div className="notice">
        {t("让指定的网站或 IP 走固定出口：直连、拒绝，或某个代理组 / 节点。规则模式下生效，排在应用分流之后、订阅规则之前；三种内核都支持，换订阅也保留。")}
        {t("IP 规则默认只匹配直接按 IP 访问的连接。")}
      </div>

      {rules.length === 0 && <div className="empty">{t("还没有分流规则。点「添加规则」，或在「连接」页从某条连接直接添加。")}</div>}

      {rules.map((r, i) => (
        <div className="card app-rule" key={r.id}>
          <button className={`switch ${r.enabled ? "on" : ""}`} title={r.enabled ? t("停用") : t("启用")} onClick={() => save(rules.map((x) => (x.id === r.id ? { ...x, enabled: !x.enabled } : x)))} />
          <span className="tag">{kindLabel(r.kind)}</span>
          <div className="app-name" title={r.value}>
            <b className="mono">{r.value}</b>
            {r.resolve && <span className="muted small">{t("也匹配解析后的域名")}</span>}
          </div>
          <span className="muted">→</span>
          <div style={{ width: 200 }}>
            <TargetSelect value={r.target} outs={outs} onChange={(v) => save(rules.map((x) => (x.id === r.id ? { ...x, target: v } : x)))} />
          </div>
          <button className="btn sm" disabled={i === 0} title={t("上移（越靠前越先匹配）")} onClick={() => move(i)}>
            ↑
          </button>
          <button className="btn sm" onClick={() => setDialog(r)}>
            {t("编辑")}
          </button>
          <button className="btn sm danger" onClick={() => save(rules.filter((x) => x.id !== r.id), t("已删除"))}>
            {t("删除")}
          </button>
        </div>
      ))}

      {dialog && <RouteRuleDialog initial={dialog} onClose={() => setDialog(null)} onSave={upsert} />}
    </>
  );
}
