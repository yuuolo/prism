import { useEffect, useRef, useState } from "react";
import { api, CORE_NAMES, fmtBytes, fmtSpeed, type AppTotal, type ConnectionsResp, type CoreKind, type Mode, type ProxyItem, type TrafficDay } from "../api";
import { useApp, usePoll } from "../state";
import type { PageKey } from "../App";
import { t } from "../i18n";
import LanShareDialog from "./LanShare";

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

const HISTORY_DAYS = 30;
const APP_RANGES: [number, string][] = [
  [1, "今天"],
  [7, "7 天"],
  [30, "30 天"],
];

/** 应用流量: which programs used the most traffic, and through which exits. */
function AppTraffic() {
  const [days, setDays] = useState(1);
  const [list, setList] = useState<AppTotal[]>([]);
  const [all, setAll] = useState(false);
  useEffect(() => {
    const load = () => api.appTraffic(days).then(setList).catch(() => {});
    load();
    const timer = setInterval(load, 15_000);
    return () => clearInterval(timer);
  }, [days]);
  const max = Math.max(1, ...list.map((a) => a.upload + a.download));
  const shown = all ? list : list.slice(0, 10);
  return (
    <>
      <div className="row">
        <h3 style={{ margin: 0 }}>{t("应用流量")}</h3>
        <span className="spacer" />
        <div className="seg">
          {APP_RANGES.map(([d, label]) => (
            <button key={d} className={days === d ? "on" : ""} onClick={() => setDays(d)}>
              {t(label)}
            </button>
          ))}
        </div>
      </div>
      {list.length === 0 && <div className="small muted" style={{ marginTop: 10 }}>{t("暂无数据：内核运行时按连接统计每个程序的流量")}</div>}
      <div className="app-traffic">
        {shown.map((a) => {
          const total = a.upload + a.download;
          return (
            <div className="app-traffic-row" key={a.name || "?"} title={a.path}>
              <span className="app-traffic-name">{a.name || <span className="muted">{t("未知程序")}</span>}</span>
              <span className="app-traffic-bar">
                <i style={{ width: `${(100 * total) / max}%` }}>
                  <b style={{ width: `${total ? (100 * a.upload) / total : 0}%` }} />
                </i>
              </span>
              <span className="small mono">{fmtBytes(total)}</span>
              <span className="small muted mono">
                ↑{fmtBytes(a.upload)} ↓{fmtBytes(a.download)}
              </span>
              <span className="small muted app-traffic-exits">
                {a.exits
                  .slice(0, 3)
                  .map(([e, b]) => `${e} ${Math.round((100 * b) / Math.max(1, total))}%`)
                  .join(" · ")}
              </span>
            </div>
          );
        })}
      </div>
      {list.length > 10 && (
        <button className="btn sm" style={{ marginTop: 8 }} onClick={() => setAll(!all)}>
          {all ? t("收起") : t("显示全部 {n} 个", { n: list.length })}
        </button>
      )}
    </>
  );
}

/** Daily totals for the last 30 days, as stacked bars. */
function TrafficHistory() {
  const [days, setDays] = useState<TrafficDay[]>([]);
  const [hover, setHover] = useState<number | null>(null);
  useEffect(() => {
    const load = () => api.trafficHistory().then(setDays).catch(() => {});
    load();
    const t = setInterval(load, 60_000);
    return () => clearInterval(t);
  }, []);

  const byDate = new Map(days.map((d) => [d.date, d]));
  const list: TrafficDay[] = [];
  const pad = (n: number) => String(n).padStart(2, "0");
  for (let i = HISTORY_DAYS - 1; i >= 0; i--) {
    const d = new Date();
    d.setDate(d.getDate() - i);
    const key = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
    list.push(byDate.get(key) ?? { date: key, upload: 0, download: 0 });
  }
  const month = list[list.length - 1].date.slice(0, 7);
  const monthTotal = days.filter((d) => d.date.startsWith(month)).reduce((a, d) => a + d.upload + d.download, 0);
  const today = list[list.length - 1];
  const max = Math.max(1, ...list.map((d) => d.upload + d.download));
  const W = 600;
  const H = 90;
  const bw = W / HISTORY_DAYS;
  const shown = hover !== null ? list[hover] : today;
  return (
    <>
      <div className="row small muted" style={{ marginBottom: 8 }}>
        <span>
          {hover !== null ? shown.date : t("今天")}: ↑ {fmtBytes(shown.upload)} ↓ {fmtBytes(shown.download)}
        </span>
        <span className="spacer" />
        <span>{t("本月 {total}", { total: fmtBytes(monthTotal) })}</span>
      </div>
      <svg className="chart" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" onMouseLeave={() => setHover(null)}>
        {list.map((d, i) => {
          const hd = ((d.download / max) * (H - 4)) | 0;
          const hu = ((d.upload / max) * (H - 4)) | 0;
          return (
            <g key={d.date} onMouseEnter={() => setHover(i)} opacity={hover === null || hover === i ? 1 : 0.5}>
              <rect x={i * bw} y={0} width={bw} height={H} fill="transparent" />
              <rect x={i * bw + 2} y={H - hd} width={bw - 4} height={hd} rx={2} fill="var(--accent-2)" />
              <rect x={i * bw + 2} y={H - hd - hu} width={bw - 4} height={hu} rx={2} fill="var(--accent)" />
            </g>
          );
        })}
      </svg>
      <div className="row small muted" style={{ marginTop: 4 }}>
        <span>{list[0].date.slice(5)}</span>
        <span className="spacer" />
        <span>{today.date.slice(5)}</span>
      </div>
    </>
  );
}

export default function Home({ go }: { go: (p: PageKey) => void }) {
  const { snap, run } = useApp();
  const [busy, setBusy] = useState(false);
  const [lan, setLan] = useState(false);
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
    await run(running ? api.stop : api.start, running ? t("内核已停止") : t("内核已启动"));
    last.current = null;
    setBusy(false);
  };

  const switchCore = async (core: CoreKind) => {
    if (core === s.core) return;
    setBusy(true);
    await run(() => api.saveSettings({ ...s, core }), t("已切换到 {name}", { name: t(CORE_NAMES[core]) }));
    setBusy(false);
  };

  const cur = { up: up[POINTS - 1], down: down[POINTS - 1] };

  return (
    <>
      <div className="page-head">
        <h1>{t("概览")}</h1>
        <div className="core-power">
          <div className="core-power-text">
            <b>{busy ? t("处理中…") : running ? t("停止内核") : t("启动内核")}</b>
            <span className="small muted">
              {running && <i className="core-toggle-dot" />}
              {snap.running ? t("{core} 运行中", { core: t(CORE_NAMES[snap.running]) }) : t(CORE_NAMES[s.core])}
              {running && <span> · {t("持续运行")} {Math.floor((snap.uptime_secs ?? 0) / 3600)}h {Math.floor(((snap.uptime_secs ?? 0) % 3600) / 60)}m {(snap.uptime_secs ?? 0) % 60}s</span>}
            </span>
          </div>
          <button
            className={`core-toggle ${running ? "on" : ""} ${busy ? "busy" : ""}`}
            disabled={busy}
            aria-busy={busy}
            aria-label={running ? t("停止内核") : t("启动内核")}
            title={running ? t("停止内核") : t("启动内核")}
            onClick={toggleCore}
          >
            {!busy && (
              <svg viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" aria-hidden="true">
                <path d="M12 3v8" />
                <path d="M6.6 6.8a8 8 0 1 0 10.8 0" />
              </svg>
            )}
          </button>
        </div>
      </div>

      {!active && (
        <div className="notice">
          {t("还没有启用的订阅。")}<a href="#" onClick={(e) => (e.preventDefault(), go("profiles"))}>{t("去导入订阅 →")}</a>
        </div>
      )}

      <div className="grid cols-3">
        <div className="card span-2">
          <h3>{t("实时速度")}</h3>
          <div className="speed">
            <div>
              <div className="lbl">{t("↑ 上传")}</div>
              <div className="big up">{fmtSpeed(cur.up)}</div>
            </div>
            <div>
              <div className="lbl">{t("↓ 下载")}</div>
              <div className="big down">{fmtSpeed(cur.down)}</div>
            </div>
          </div>
          <Chart up={up} down={down} />
        </div>

        <div className="card">
          <h3>{t("流量统计")}</h3>
          <div className="grid" style={{ gap: 10 }}>
            <div className="row">
              <span className="muted">{t("上传总量")}</span>
              <span className="spacer" />
              <b>{fmtBytes(totals.up)}</b>
            </div>
            <div className="row">
              <span className="muted">{t("下载总量")}</span>
              <span className="spacer" />
              <b>{fmtBytes(totals.down)}</b>
            </div>
            <div className="row">
              <span className="muted">{t("活动连接")}</span>
              <span className="spacer" />
              <b>{totals.conns}</b>
            </div>
            <div className="row">
              <span className="muted">{t("代理端口")}</span>
              <span className="spacer" />
              <b className="mono">127.0.0.1:{s.mixed_port}</b>
            </div>
          </div>
        </div>

        <div className="card span-2">
          <h3>{t("内核")}</h3>
          <div className="core-pick">
            {(["prism", "mihomo", "singbox"] as CoreKind[]).map((k) => (
              <button key={k} className={`core-opt ${s.core === k ? "on" : ""}`} disabled={busy} onClick={() => switchCore(k)}>
                <b>{t(CORE_NAMES[k])}</b>
                <span className="small muted">
                  {k === "prism" ? t("自研 Rust 内核") : snap.core_paths[k].exists ? t("已安装") : t("未安装 · 去设置下载")}
                </span>
              </button>
            ))}
          </div>
        </div>

        <div className="card">
          <h3>{t("流量接管")}</h3>
          <div className="row">
            <div>
              <div>{t("系统代理")}</div>
              <div className="small muted">{t("浏览器等应用走 127.0.0.1:{port}", { port: s.mixed_port })}</div>
            </div>
            <span className="spacer" />
            <button className={`switch ${snap.system_proxy ? "on" : ""}`} onClick={() => run(() => api.setSystemProxy(!snap.system_proxy))} />
          </div>
          <div className="row" style={{ marginTop: 14 }}>
            <div>
              <div>
                {t("TUN 模式")} {!snap.elevated && <span className="tag">{t("需管理员")}</span>}
              </div>
              <div className="small muted">{t("虚拟网卡接管所有应用（含游戏、命令行）")}</div>
            </div>
            <span className="spacer" />
            <button
              className={`switch ${s.tun ? "on" : ""}`}
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                await run(() => api.setTun(!s.tun), s.tun ? t("TUN 模式已关闭") : t("TUN 模式已开启"));
                setBusy(false);
              }}
            />
          </div>
          <div className="row" style={{ marginTop: 14 }}>
            <div>
              <div>
                {t("局域网共享")} {s.allow_lan && <span className="tag accent">{t("已开启")}</span>}
              </div>
              <div className="small muted">{t("让手机等设备使用本机代理")}</div>
            </div>
            <span className="spacer" />
            <button className="btn sm" onClick={() => setLan(true)}>
              {t("二维码")}
            </button>
          </div>
        </div>

        <div className="card span-2">
          <h3>{t("代理模式")}</h3>
          <div className="seg">
            {(
              [
                ["rule", t("规则")],
                ["global", t("全局")],
                ["direct", t("直连")],
              ] as [Mode, string][]
            ).map(([m, l]) => (
              <button key={m} className={s.mode === m ? "on" : ""} onClick={() => run(() => api.setMode(m))}>
                {l}
              </button>
            ))}
          </div>
          <div className="small muted" style={{ marginTop: 10 }}>
            {s.mode === "rule" && t("按订阅规则分流：国内直连，其余走代理")}
            {s.mode === "global" && t("所有流量走 GLOBAL 组选中的节点")}
            {s.mode === "direct" && t("所有流量直连，不经过代理")}
          </div>
        </div>

        <div className="card">
          <h3>{t("当前订阅")}</h3>
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
              {!running && <div className="small muted">{t("{n} 个节点", { n: active.nodes })}</div>}
            </>
          ) : (
            <div className="muted">{t("无")}</div>
          )}
        </div>

        <div className="card span-3">
          <h3>{t("最近 30 天流量")}</h3>
          <TrafficHistory />
        </div>

        <div className="card span-3">
          <AppTraffic />
        </div>
      </div>
      {lan && <LanShareDialog onClose={() => setLan(false)} />}
    </>
  );
}
