import { useCallback, useEffect, useState } from "react";
import { api, type RuleItem } from "../api";
import { useApp } from "../state";
import { RuleProviders } from "./Providers";

export default function Rules() {
  const { snap } = useApp();
  const [rules, setRules] = useState<RuleItem[]>([]);
  const [filter, setFilter] = useState("");
  const running = !!snap?.running;

  const load = useCallback(() => {
    api
      .core<{ rules: RuleItem[] }>("GET", "/rules")
      .then((r) => setRules(r.rules ?? []))
      .catch(() => setRules([]));
  }, []);

  useEffect(() => {
    if (running) load();
  }, [running, load, snap?.settings.active_profile, snap?.settings.core]);

  const f = filter.trim().toLowerCase();
  const shown = f ? rules.filter((r) => `${r.type} ${r.payload} ${r.proxy}`.toLowerCase().includes(f)) : rules;

  return (
    <>
      <div className="page-head">
        <h1>规则</h1>
        <span className="tag">{rules.length} 条</span>
        <input className="input" placeholder="过滤" value={filter} onChange={(e) => setFilter(e.target.value)} />
      </div>
      {running && <RuleProviders onChange={load} />}
      {!running ? (
        <div className="empty">内核未运行</div>
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th style={{ width: 60 }}>#</th>
                <th>类型</th>
                <th>内容</th>
                <th>目标</th>
              </tr>
            </thead>
            <tbody>
              {shown.slice(0, 2000).map((r, i) => (
                <tr key={i}>
                  <td className="muted">{rules.indexOf(r) + 1}</td>
                  <td>{r.type}</td>
                  <td className="ellipsis mono">{r.payload}</td>
                  <td>{r.proxy}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {shown.length === 0 && <div className="empty">没有规则</div>}
        </div>
      )}
    </>
  );
}
