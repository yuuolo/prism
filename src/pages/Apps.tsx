import { useEffect, useMemo, useState } from "react";
import { api, errText, type AppRule, type Outbounds, type ProcessInfo } from "../api";
import { useApp } from "../state";

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** Exit picker: DIRECT / REJECT, then the profile's groups and nodes. */
function TargetSelect({ value, outs, onChange }: { value: string; outs: Outbounds | null; onChange: (v: string) => void }) {
  const known = !outs || value === "DIRECT" || value === "REJECT" || outs.groups.includes(value) || outs.proxies.includes(value);
  return (
    <select className="input" value={value} onChange={(e) => onChange(e.target.value)}>
      {!known && <option value={value}>{value}（当前订阅中不存在）</option>}
      <option value="DIRECT">DIRECT · 直连</option>
      <option value="REJECT">REJECT · 拒绝</option>
      {outs && outs.groups.length > 0 && (
        <optgroup label="代理组">
          {outs.groups.map((g) => (
            <option key={g} value={g}>
              {g}
            </option>
          ))}
        </optgroup>
      )}
      {outs && outs.proxies.length > 0 && (
        <optgroup label="节点">
          {outs.proxies.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </optgroup>
      )}
    </select>
  );
}

function useOutbounds() {
  const { snap } = useApp();
  const [outs, setOuts] = useState<Outbounds | null>(null);
  useEffect(() => {
    api.profileOutbounds().then(setOuts).catch(() => setOuts(null));
  }, [snap?.settings.active_profile]);
  return outs;
}

/** Add / edit dialog: pick a running program (or type one) and an exit. */
export function AppRuleDialog({
  initial,
  onClose,
  onSave,
}: {
  initial: Partial<AppRule> & { path?: string };
  onClose: () => void;
  onSave: (rule: AppRule) => Promise<boolean>;
}) {
  const outs = useOutbounds();
  const [procs, setProcs] = useState<ProcessInfo[] | null>(null);
  const [search, setSearch] = useState("");
  const [picked, setPicked] = useState<{ name: string; path: string } | null>(
    initial.path ? { name: fileName(initial.path), path: initial.path } : null,
  );
  const [byPath, setByPath] = useState(!!initial.by_path);
  const [value, setValue] = useState(initial.value ?? "");
  const [target, setTarget] = useState(initial.target ?? "");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.listProcesses().then(setProcs).catch(() => setProcs([]));
  }, []);
  useEffect(() => {
    if (!target && outs) setTarget(outs.groups[0] ?? "DIRECT");
  }, [outs, target]);

  const pick = (p: { name: string; path: string }, path = byPath) => {
    setPicked(p);
    setValue(path && p.path ? p.path : p.name);
  };
  const setMode = (path: boolean) => {
    setByPath(path);
    if (picked) pick(picked, path);
  };

  const f = search.trim().toLowerCase();
  const shown = useMemo(
    () => (procs ?? []).filter((p) => !f || p.name.toLowerCase().includes(f) || p.path.toLowerCase().includes(f)),
    [procs, f],
  );
  const invalid = !value.trim() || value.includes(",");

  const save = async () => {
    setSaving(true);
    const ok = await onSave({
      id: initial.id ?? crypto.randomUUID(),
      enabled: initial.enabled ?? true,
      by_path: byPath,
      value: value.trim(),
      target: target || "DIRECT",
    });
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{initial.id ? "编辑应用规则" : "添加应用规则"}</h2>

        <div className="row">
          <input className="input" style={{ flex: 1 }} placeholder="搜索正在运行的程序" value={search} onChange={(e) => setSearch(e.target.value)} />
          <button className="btn" onClick={() => api.listProcesses().then(setProcs)}>
            刷新
          </button>
        </div>
        <div className="proc-list">
          {procs === null && <div className="muted small" style={{ padding: 10 }}>读取中…</div>}
          {shown.map((p) => (
            <div key={p.path} className={`proc-item ${picked?.path === p.path ? "sel" : ""}`} onClick={() => pick(p)} title={p.path}>
              <b>{p.name}</b>
              <span className="muted small">{p.path}</span>
            </div>
          ))}
          {procs !== null && shown.length === 0 && <div className="muted small" style={{ padding: 10 }}>没有匹配的程序，可在下方手动输入</div>}
        </div>

        <div className="field">
          <label>匹配方式</label>
          <div className="seg">
            <button className={!byPath ? "on" : ""} onClick={() => setMode(false)}>
              按程序名
            </button>
            <button className={byPath ? "on" : ""} onClick={() => setMode(true)}>
              按完整路径
            </button>
          </div>
        </div>
        <div className="field">
          <label>{byPath ? "程序路径" : "程序名"}</label>
          <input
            className="input mono"
            placeholder={byPath ? "C:\\Program Files\\App\\app.exe" : "app.exe"}
            value={value}
            onChange={(e) => {
              setValue(e.target.value);
              setPicked(null);
            }}
          />
        </div>
        <div className="field">
          <label>出口</label>
          <TargetSelect value={target} outs={outs} onChange={setTarget} />
        </div>
        <div className="small muted">
          {byPath ? "只匹配这个路径的程序。" : "所有同名程序都会匹配（不区分大小写）。"} 规则模式下生效，优先于订阅中的规则。
          {value.includes(",") && <span style={{ color: "var(--bad)" }}> 不能包含逗号。</span>}
        </div>

        <div className="row">
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            取消
          </button>
          <button className="btn primary" disabled={saving || invalid} onClick={save}>
            {saving ? "保存中…" : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default function Apps() {
  const { snap, toast, refresh } = useApp();
  const outs = useOutbounds();
  const [dialog, setDialog] = useState<(Partial<AppRule> & { path?: string }) | null>(null);
  const rules = snap?.settings.app_rules ?? [];

  const save = async (next: AppRule[], ok = "已保存") => {
    try {
      await api.setAppRules(next);
      toast(snap?.running ? `${ok}，内核已重新加载` : ok);
      return true;
    } catch (e) {
      toast(errText(e), "err");
      return false;
    } finally {
      refresh();
    }
  };
  const upsert = (r: AppRule) => save(rules.some((x) => x.id === r.id) ? rules.map((x) => (x.id === r.id ? r : x)) : [...rules, r]);

  return (
    <>
      <div className="page-head">
        <h1>应用分流</h1>
        <span className="tag">{rules.length} 条</span>
        <button className="btn primary" onClick={() => setDialog({})}>
          添加应用
        </button>
      </div>

      <div className="notice">
        指定程序的连接走固定出口：直连、拒绝，或某个代理组 / 节点。规则模式下生效，排在订阅规则之前；三种内核都支持。
        系统代理只对走系统代理的程序有效，其他程序（游戏、命令行等）请开启 TUN 模式。
      </div>

      {rules.length === 0 && <div className="empty">还没有应用规则。点「添加应用」从正在运行的程序中选择，或在「连接」页从某条连接直接添加。</div>}

      {rules.map((r) => (
        <div className="card app-rule" key={r.id}>
          <button className={`switch ${r.enabled ? "on" : ""}`} title={r.enabled ? "停用" : "启用"} onClick={() => save(rules.map((x) => (x.id === r.id ? { ...x, enabled: !x.enabled } : x)))} />
          <div className="app-name" title={r.value}>
            <b>{r.by_path ? fileName(r.value) : r.value}</b>
            {r.by_path && <span className="muted small mono">{r.value}</span>}
          </div>
          <span className="tag">{r.by_path ? "按路径" : "按程序名"}</span>
          <span className="muted">→</span>
          <div style={{ width: 200 }}>
            <TargetSelect value={r.target} outs={outs} onChange={(t) => save(rules.map((x) => (x.id === r.id ? { ...x, target: t } : x)))} />
          </div>
          <button className="btn sm" onClick={() => setDialog(r)}>
            编辑
          </button>
          <button className="btn sm danger" onClick={() => save(rules.filter((x) => x.id !== r.id), "已删除")}>
            删除
          </button>
        </div>
      ))}

      {dialog && <AppRuleDialog initial={dialog} onClose={() => setDialog(null)} onSave={upsert} />}
    </>
  );
}
