import Help from "../Help";
import { useEffect, useMemo, useState } from "react";
import { api, type NodeStats, type ProxyItem } from "../api";
import { t } from "../i18n";

const RANGES: [number, string][] = [
  [3600, "1 小时"],
  [6 * 3600, "6 小时"],
  [24 * 3600, "24 小时"],
];

/** Latency over time: a line through successful tests, red ticks for failures. */
function Spark({ samples, from, to }: { samples: [number, number][]; from: number; to: number }) {
  const W = 260;
  const H = 40;
  const ok = samples.filter(([, d]) => d > 0).map(([, d]) => d);
  if (samples.length === 0) return <svg className="spark" width={W} height={H} />;
  const sorted = [...ok].sort((a, b) => a - b);
  // Scale to the 95th percentile so one spike doesn't flatten the rest.
  const top = Math.max(100, sorted[Math.floor(sorted.length * 0.95)] ?? 100) * 1.1;
  const x = (ts: number) => ((ts - from) / (to - from)) * W;
  const y = (d: number) => H - 3 - (Math.min(d, top) / top) * (H - 6);
  const pts = samples.filter(([, d]) => d > 0).map(([ts, d]) => `${x(ts).toFixed(1)},${y(d).toFixed(1)}`);
  return (
    <svg className="spark" width={W} height={H} viewBox={`0 0 ${W} ${H}`}>
      <line x1={0} x2={W} y1={H - 0.5} y2={H - 0.5} className="spark-axis" />
      {pts.length > 1 && <polyline points={pts.join(" ")} className="spark-line" />}
      {samples
        .filter(([, d]) => d > 0)
        .map(([ts, d], i) => (
          <circle key={i} cx={x(ts)} cy={y(d)} r={pts.length > 60 ? 1 : 1.8} className="spark-dot" />
        ))}
      {samples
        .filter(([, d]) => d === 0)
        .map(([ts], i) => (
          <line key={`f${i}`} x1={x(ts)} x2={x(ts)} y1={2} y2={H - 2} className="spark-fail" />
        ))}
    </svg>
  );
}

function statsIn(n: NodeStats, from: number) {
  const s = n.samples.filter(([ts]) => ts >= from);
  const ok = s.filter(([, d]) => d > 0).map(([, d]) => d);
  const loss = s.length ? (s.length - ok.length) / s.length : 0;
  const avg = ok.length ? Math.round(ok.reduce((a, b) => a + b, 0) / ok.length) : null;
  let jitter: number | null = null;
  if (ok.length > 1) {
    let sum = 0;
    for (let i = 1; i < ok.length; i++) sum += Math.abs(ok[i] - ok[i - 1]);
    jitter = Math.round(sum / (ok.length - 1));
  }
  const score = avg == null ? null : avg + 2 * (jitter ?? 0) + Math.round(loss * 1000);
  return { samples: s, avg, jitter, loss, score };
}

/** 节点质量: each node's latency over the last hours, ranked by stability. */
export default function QualityDialog({
  proxies,
  groups,
  onSelect,
  onClose,
}: {
  proxies: Record<string, ProxyItem>;
  groups: ProxyItem[];
  onSelect: (group: ProxyItem, node: string) => void;
  onClose: () => void;
}) {
  const [stats, setStats] = useState<NodeStats[] | null>(null);
  const [range, setRange] = useState(6 * 3600);
  const [group, setGroup] = useState(groups.find((g) => g.type === "Selector")?.name ?? groups[0]?.name ?? "");
  useEffect(() => {
    api.nodeQuality().then(setStats, () => setStats([]));
  }, []);

  const g = groups.find((x) => x.name === group);
  const now = Date.now() / 1000;
  const from = now - range;
  const rows = useMemo(() => {
    const byName = new Map((stats ?? []).map((s) => [s.name, s]));
    // Nodes of the group, nested groups flattened.
    const members: string[] = [];
    const walk = (name: string, depth: number) => {
      const p = proxies[name];
      if (p?.all && depth < 4) p.all.forEach((m) => walk(m, depth + 1));
      else if (p && !["Direct", "Reject", "RejectDrop", "Pass", "Compatible"].includes(p.type) && !members.includes(name)) members.push(name);
    };
    if (g) walk(g.name, 0);
    return members
      .map((name) => ({ name, ...statsIn(byName.get(name) ?? { name, samples: [], avg: null, jitter: null, loss: 0, score: null }, from) }))
      .sort((a, b) => (a.score ?? Infinity) - (b.score ?? Infinity) || a.name.localeCompare(b.name));
  }, [stats, g, proxies, from]);
  const best = rows.find((r) => r.score != null && r.samples.length >= 3);
  const selectable = g?.type === "Selector";

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal quality-modal" onClick={(e) => e.stopPropagation()}>
        <h2>{t("节点质量")}</h2>
        <Help title={t("节点质量")}><div>
          {t("记录最近 24 小时的测速结果：自动测速组的定时测试、手动测速，以及每 5 分钟对正在使用的节点的测试。按稳定性排序（延迟 + 2×抖动 + 丢包惩罚）。")}
        </div></Help>
        <div className="row">
          <select className="input" style={{ maxWidth: 260 }} value={group} onChange={(e) => setGroup(e.target.value)}>
            {groups.map((x) => (
              <option key={x.name} value={x.name}>
                {x.name}
              </option>
            ))}
          </select>
          <div className="seg">
            {RANGES.map(([v, label]) => (
              <button key={v} className={range === v ? "on" : ""} onClick={() => setRange(v)}>
                {t(label)}
              </button>
            ))}
          </div>
          <span className="spacer" />
          {selectable && best && (
            <button className="btn primary" disabled={g?.now === best.name} onClick={() => onSelect(g!, best.name)}>
              {t("选用最稳定的：{name}", { name: best.name })}
            </button>
          )}
        </div>
        <div className="quality-list">
          {stats === null && <div className="muted">{t("读取中…")}</div>}
          {stats !== null && rows.length === 0 && <div className="empty">{t("这个组没有节点")}</div>}
          {rows.map((r, i) => (
            <div className={`quality-row ${g?.now === r.name ? "current" : ""}`} key={r.name}>
              <span className="muted small q-rank">{r.score != null ? i + 1 : "–"}</span>
              <span className="q-name" title={r.name}>
                {r.name}
              </span>
              <Spark samples={r.samples} from={from} to={now} />
              <span className="q-num" title={t("平均延迟")}>
                {r.avg != null ? `${r.avg} ms` : "–"}
              </span>
              <span className="q-num muted" title={t("抖动")}>
                {r.jitter != null ? `±${r.jitter}` : "–"}
              </span>
              <span className={`q-num ${r.loss > 0.2 ? "d-bad" : r.loss > 0 ? "d-mid" : "muted"}`} title={t("失败比例")}>
                {r.samples.length ? `${Math.round(r.loss * 100)}%` : t("无数据")}
              </span>
              <span className="q-act">
                {selectable && g?.now !== r.name && (
                  <button className="btn sm" onClick={() => onSelect(g!, r.name)}>
                    {t("使用")}
                  </button>
                )}
                {g?.now === r.name && <span className="tag accent">{t("当前")}</span>}
              </span>
            </div>
          ))}
        </div>
        <div className="row">
          <span className="small muted">{t("平均延迟 · 抖动 · 失败比例")}</span>
          <span className="spacer" />
          <button className="btn primary" onClick={onClose}>
            {t("关闭")}
          </button>
        </div>
      </div>
    </div>
  );
}
