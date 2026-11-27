import Help from "../Help";
import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errText, type ProxyItem, type SpeedProgress } from "../api";
import { t } from "../i18n";

const URLS: [string, string][] = [
  ["https://speed.cloudflare.com/__down?bytes=50000000", "Cloudflare"],
  ["http://cachefly.cachefly.net/100mb.test", "CacheFly"],
];
const BUILTIN = ["Direct", "Reject", "RejectDrop", "Pass", "Compatible"];

function fmtMbps(m?: number) {
  if (m === undefined) return "";
  return m >= 100 ? `${m.toFixed(0)} Mbps` : `${m.toFixed(1)} Mbps`;
}

/** 带宽测速: download speed through each node of a group, one at a time. */
export default function SpeedDialog({ proxies, groups, onClose }: { proxies: Record<string, ProxyItem>; groups: ProxyItem[]; onClose: () => void }) {
  const [group, setGroup] = useState(groups.find((g) => g.type === "Selector")?.name ?? groups[0]?.name ?? "");
  const [url, setUrl] = useState(URLS[0][0]);
  const [secs, setSecs] = useState(8);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [results, setResults] = useState<Record<string, SpeedProgress>>({});
  const [running, setRunning] = useState<string[] | null>(null);
  const [error, setError] = useState("");

  const nodes = useMemo(() => {
    const out: string[] = [];
    const walk = (name: string, depth: number) => {
      const p = proxies[name];
      if (p?.all && depth < 4) p.all.forEach((m) => walk(m, depth + 1));
      else if (p && !BUILTIN.includes(p.type) && !out.includes(name)) out.push(name);
    };
    walk(group, 0);
    return out;
  }, [group, proxies]);
  useEffect(() => setPicked(new Set(nodes)), [nodes]);

  useEffect(() => {
    const un = listen<SpeedProgress>("speed-progress", (e) => setResults((r) => ({ ...r, [e.payload.name]: e.payload })));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const start = async () => {
    const list = nodes.filter((n) => picked.has(n));
    setError("");
    setRunning(list);
    setResults((r) => {
      const next = { ...r };
      list.forEach((n) => delete next[n]);
      return next;
    });
    try {
      const final = await api.speedTest(list, url, secs);
      setResults((r) => ({ ...r, ...Object.fromEntries(final.map((x) => [x.name, x])) }));
    } catch (e) {
      setError(errText(e));
    } finally {
      setRunning(null);
    }
  };

  const max = Math.max(1, ...Object.values(results).map((r) => r.speed?.mbps ?? 0));
  const done = running ? running.filter((n) => results[n]?.done).length : 0;
  const current = running?.find((n) => !results[n]?.done);
  const order = running
    ? nodes
    : [...nodes].sort((a, b) => (results[b]?.speed?.mbps ?? -1) - (results[a]?.speed?.mbps ?? -1) || a.localeCompare(b));

  return (
    <div className="modal-bg" onClick={running ? undefined : onClose}>
      <div className="modal quality-modal" onClick={(e) => e.stopPropagation()}>
        <h2>{t("带宽测速")}</h2>
        <Help title={t("带宽测速")}><div>
          {t("逐个经节点下载测试文件，测量实际下载速度（会消耗流量：每个节点最多约 {s} 秒的下载量）。", { s: secs })}
        </div></Help>
        <div className="row">
          <select className="input" style={{ maxWidth: 220 }} value={group} disabled={!!running} onChange={(e) => setGroup(e.target.value)}>
            {groups.map((x) => (
              <option key={x.name} value={x.name}>
                {x.name}
              </option>
            ))}
          </select>
          <select className="input" style={{ maxWidth: 160 }} value={url} disabled={!!running} onChange={(e) => setUrl(e.target.value)}>
            {URLS.map(([u, label]) => (
              <option key={u} value={u}>
                {label}
              </option>
            ))}
          </select>
          <select className="input" style={{ maxWidth: 110 }} value={secs} disabled={!!running} onChange={(e) => setSecs(Number(e.target.value))}>
            {[5, 8, 15].map((s) => (
              <option key={s} value={s}>
                {t("每个 {s} 秒", { s })}
              </option>
            ))}
          </select>
          <span className="spacer" />
          <button className="btn primary" disabled={!!running || picked.size === 0} onClick={start}>
            {running ? t("测速中 {done}/{total}", { done, total: running.length }) : t("开始测速（{n} 个）", { n: picked.size })}
          </button>
        </div>
        {error && <div className="notice small">{error}</div>}
        <div className="quality-list">
          {order.map((n) => {
            const r = results[n];
            const mbps = r?.speed?.mbps;
            return (
              <div className={`speed-row ${current === n ? "current" : ""}`} key={n}>
                <input
                  type="checkbox"
                  checked={picked.has(n)}
                  disabled={!!running}
                  onChange={(e) => {
                    const next = new Set(picked);
                    if (e.target.checked) next.add(n);
                    else next.delete(n);
                    setPicked(next);
                  }}
                />
                <span className="q-name" title={n}>
                  {n}
                </span>
                <span className="app-traffic-bar">
                  <i className={r && !r.done ? "live" : ""} style={{ width: `${mbps ? (100 * mbps) / max : 0}%` }} />
                </span>
                <span className="q-num">{r?.error ? <span className="d-bad" title={r.error}>{t("失败")}</span> : fmtMbps(mbps)}</span>
                <span className="q-num muted" title={t("连接并收到响应所用的时间")}>
                  {r?.speed ? `${r.speed.ttfb} ms` : ""}
                </span>
              </div>
            );
          })}
        </div>
        <div className="row">
          <span className="small muted">{t("速度 · 首包时间")}</span>
          <span className="spacer" />
          <button className="btn" disabled={!!running} onClick={() => setPicked(picked.size === nodes.length ? new Set() : new Set(nodes))}>
            {picked.size === nodes.length ? t("全不选") : t("全选")}
          </button>
          <button className="btn primary" disabled={!!running} onClick={onClose}>
            {t("关闭")}
          </button>
        </div>
      </div>
    </div>
  );
}
