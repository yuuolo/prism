import { useRef, useState } from "react";
import { api, CORE_NAMES, fmtBytes, fmtSpeed, type ConnectionsResp, type CoreKind, type Mode, type ProxyItem } from "../api";
import { useApp, usePoll } from "../state";
import type { PageKey } from "../App";

const POINTS = 60;

function Chart({ up, down }: { up: number[]; down: number[] }) {
  const max = Math.max(1024, ...up, ...down);
  const W = 600;
  const H = 110;
  const path = (d: number[]) =>
    d.map((v, i) => `${i === 0 ? "M" : "L"}${((i / (POINTS - 1)) * W).toFixed(1)},${(H - 4 - (v / max) * (H - 10)).toFixed(1)}`).join(" ");
  const area = (d: number[]) => `${path(d)} L${W},${H} L0,${H} Z`;
  return (
    <svg className="chart" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
      <defs>
        <linearGradient id="gd" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="var(--accent-2)" stopOpacity="0.3" />
          <stop offset="1" stopColor="var(--accent-2)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={area(down)} fill="url(#gd)" />
      <path d={path(down)} fill="none" stroke="var(--accent-2)" strokeWidth="2" vectorEffect="non-scaling-stroke" />
      <path d={path(up)} fill="none" stroke="var(--accent)" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

export default function Home({ go }: { go: (p: PageKey) => void }) {
  const { snap, run } = useApp();
  const [busy, setBusy] = useState(false);
  const [up, setUp] = useState<number[]>(Array(POINTS).fill(0));
  const [down, setDown] = useState<number[]>(Array(POINTS).fill(0));
  const [totals, setTotals] = useState({ up: 0, down: 0, conns: 0 });
  const [nodes, setNodes] = useState<{ group: string; now: string }[]>([]);
  const last = useRef<{ up: number; down: number; t: number } | null>(null);

  const running = !!snap?.running;

  usePoll(
    async () => {
      const c = await api.core<ConnectionsResp>("GET", "/connections");
      const now = Date.now();
      const prev = last.current;
      if (prev) {
        const dt = Math.max(0.5, (now - prev.t) / 1000);
        const u = Math.max(0, (c.uploadTotal - prev.up) / dt);
        const d = Math.max(0, (c.downloadTotal - prev.down) / dt);
        setUp((a) => [...a.slice(1), u]);
        setDown((a) => [...a.slice(1), d]);
      }
      last.current = { up: c.uploadTotal, down: c.downloadTotal, t: now };
      setTotals({ up: c.uploadTotal, down: c.downloadTotal, conns: c.connections?.length ?? 0 });
    },
    1000,
    running,
  );

  usePoll(
    async () => {
      const r = await api.core<{ proxies: Record<string, ProxyItem> }>("GET", "/proxies");
      const order = r.proxies["GLOBAL"]?.all ?? Object.keys(r.proxies);
      const groups = order
        .map((n) => r.proxies[n])
        .filter((p) => p?.all && p.name !== "GLOBAL")
        .slice(0, 3)
        .map((g) => ({ group: g.name, now: g.now ?? "" }));
      setNodes(groups);
    },
    3000,
    running,
  );

  if (!snap) return null;
  const s = snap.settings;
  const active = snap.profiles.find((p) => p.id === s.active_profile);

  const toggleCore = async () => {
    setBusy(true);
    await run(running ? api.stop : api.start, running ? "内核已停止" : "内核已启动");
    last.current = null;
    setBusy(false);
  };

  const switchCore = async (core: CoreKind) => {
    if (core === s.core) return;
    setBusy(true);
    await run(() => api.saveSettings({ ...s, core }), `已切换到 ${CORE_NAMES[core]}`);
    setBusy(false);
  };

  const cur = { up: up[POINTS - 1], down: down[POINTS - 1] };

  return (
    <>
      <div className="page-head">
        <h1>概览</h1>
        <button className={`btn ${running ? "" : "primary"}`} disabled={busy} onClick={toggleCore}>
          {busy ? "处理中…" : running ? "停止内核" : "启动内核"}
        </button>
      </div>

      {!active && (
        <div className="notice">
          还没有启用的订阅。<a href="#" onClick={(e) => (e.preventDefault(), go("profiles"))}>去导入订阅 →</a>
        </div>
      )}

      <div className="grid cols-3">
        <div className="card span-2">
          <h3>实时速度</h3>
          <div className="speed">
            <div>
              <div className="lbl">↑ 上传</div>
              <div className="big up">{fmtSpeed(cur.up)}</div>
            </div>
            <div>
              <div className="lbl">↓ 下载</div>
              <div className="big down">{fmtSpeed(cur.down)}</div>
            </div>
          </div>
          <Chart up={up} down={down} />
        </div>

        <div className="card">
          <h3>流量统计</h3>
          <div className="grid" style={{ gap: 10 }}>
            <div className="row">
              <span className="muted">上传总量</span>
              <span className="spacer" />
              <b>{fmtBytes(totals.up)}</b>
            </div>
            <div className="row">
              <span className="muted">下载总量</span>
              <span className="spacer" />
              <b>{fmtBytes(totals.down)}</b>
            </div>
            <div className="row">
              <span className="muted">活动连接</span>
              <span className="spacer" />
              <b>{totals.conns}</b>
            </div>
            <div className="row">
              <span className="muted">代理端口</span>
              <span className="spacer" />
              <b className="mono">127.0.0.1:{s.mixed_port}</b>
            </div>
          </div>
        </div>

        <div className="card span-2">
          <h3>内核</h3>
          <div className="core-pick">
            {(["prism", "mihomo", "singbox"] as CoreKind[]).map((k) => (
              <button key={k} className={`core-opt ${s.core === k ? "on" : ""}`} disabled={busy} onClick={() => switchCore(k)}>
                <b>{CORE_NAMES[k]}</b>
                <span className="small muted">
                  {k === "prism" ? "自研 Rust 内核" : snap.core_paths[k].exists ? "已安装" : "未安装 · 去设置下载"}
                </span>
              </button>
            ))}
          </div>
        </div>

        <div className="card">
          <h3>系统代理</h3>
          <div className="row">
            <span>{snap.system_proxy ? "已开启" : "已关闭"}</span>
            <span className="spacer" />
            <button className={`switch ${snap.system_proxy ? "on" : ""}`} onClick={() => run(() => api.setSystemProxy(!snap.system_proxy))} />
          </div>
          <div className="small muted" style={{ marginTop: 10 }}>
            开启后浏览器等应用会通过 127.0.0.1:{s.mixed_port} 上网
          </div>
        </div>

        <div className="card span-2">
          <h3>代理模式</h3>
          <div className="seg">
            {(
              [
                ["rule", "规则"],
                ["global", "全局"],
                ["direct", "直连"],
              ] as [Mode, string][]
            ).map(([m, l]) => (
              <button key={m} className={s.mode === m ? "on" : ""} onClick={() => run(() => api.setMode(m))}>
                {l}
              </button>
            ))}
          </div>
          <div className="small muted" style={{ marginTop: 10 }}>
            {s.mode === "rule" && "按订阅规则分流：国内直连，其余走代理"}
            {s.mode === "global" && "所有流量走 GLOBAL 组选中的节点"}
            {s.mode === "direct" && "所有流量直连，不经过代理"}
          </div>
        </div>

        <div className="card">
          <h3>当前订阅</h3>
          {active ? (
            <>
              <div style={{ fontWeight: 600, marginBottom: 8 }}>{active.name}</div>
              {nodes.map((n) => (
                <div key={n.group} className="row small" style={{ marginBottom: 4 }}>
                  <span className="muted">{n.group}</span>
                  <span className="spacer" />
                  <span>{n.now}</span>
                </div>
              ))}
              {!running && <div className="small muted">{active.nodes} 个节点</div>}
            </>
          ) : (
            <div className="muted">无</div>
          )}
        </div>
      </div>
    </>
  );
}
