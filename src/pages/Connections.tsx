import { useEffect, useRef, useState } from "react";
import { api, errText, fmtBytes, fmtSpeed, type AppRule, type Connection, type ConnectionsResp, type ExportFormat, type ExportTable, type RouteRule } from "../api";
import { useApp, usePoll } from "../state";
import { AppRuleDialog } from "./Apps";
import { RouteRuleDialog, useSaveRouteRules } from "./Routes";
import ConnMap from "./ConnMap";
import { t } from "../i18n";

interface Row extends Connection {
  upSpeed: number;
  downSpeed: number;
}

const EXPORT_FORMATS: [ExportFormat, string][] = [
  ["xlsx", "Excel (.xlsx)"],
  ["csv", "CSV (.csv)"],
  ["json", "JSON (.json)"],
  ["html", "HTML (.html)"],
  ["md", "Markdown (.md)"],
];

const pad = (n: number) => String(n).padStart(2, "0");

/** Local time as `2026-10-25 14:03:09`. */
function stamp(iso: string) {
  const d = new Date(iso);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** The rows as shown (filtered), with raw byte counts for spreadsheets. */
function exportTable(rows: Row[]): ExportTable {
  return {
    headers: [
      t("开始时间"),
      t("主机"),
      t("目标 IP"),
      t("端口"),
      t("来源"),
      t("程序"),
      t("程序路径"),
      t("入站"),
      t("网络"),
      t("规则"),
      t("规则内容"),
      t("链路"),
      t("上传（字节）"),
      t("下载（字节）"),
      t("上传速度（字节/秒）"),
      t("下载速度（字节/秒）"),
    ],
    rows: rows.map((c) => {
      const m = c.metadata;
      return [
        stamp(c.start),
        m.host || "",
        m.destinationIP || "",
        Number(m.destinationPort) || m.destinationPort || "",
        m.sourceIP ? `${m.sourceIP}:${m.sourcePort}` : "",
        m.process || "",
        m.processPath || "",
        m.type || "",
        (m.network || "").toUpperCase(),
        c.rule,
        c.rulePayload,
        [...c.chains].reverse().join(" → "),
        c.upload,
        c.download,
        c.upSpeed,
        c.downSpeed,
      ];
    }),
  };
}

export default function Connections() {
  const { snap, run, toast, refresh } = useApp();
  const [ruleFor, setRuleFor] = useState<{ value: string; path: string } | null>(null);
  const [rows, setRows] = useState<Row[]>([]);
  const [filter, setFilter] = useState("");
  const [network, setNetwork] = useState("");
  const [exit, setExit] = useState("");
  const [sort, setSort] = useState("start");
  const [ascending, setAscending] = useState(false);
  const [paused, setPaused] = useState(false);
  const [view, setView] = useState<"table" | "map">("table");
  const [exportOpen, setExportOpen] = useState(false);
  const [routeFor, setRouteFor] = useState<Partial<RouteRule> | null>(null);
  const saveRoutes = useSaveRouteRules();
  const prev = useRef<Map<string, Connection>>(new Map());
  // While the mouse is on a row the list holds still, so its buttons can be hit.
  const hovering = useRef(false);
  const displayedOrder = useRef<string[]>([]);
  const listed = useRef<Set<string>>(new Set());
  const [waiting, setWaiting] = useState(0);
  const running = !!snap?.running;

  // Leaving the table by unmounting it fires no mouseleave.
  useEffect(() => {
    if (!running || view !== "table") {
      hovering.current = false;
      setWaiting(0);
    }
  }, [running, view]);

  usePoll(
    async () => {
      const r = await api.core<ConnectionsResp>("GET", "/connections");
      const list = r.connections ?? [];
      const next = new Map<string, Connection>();
      const out = list.map((c) => {
        next.set(c.id, c);
        const p = prev.current.get(c.id);
        // sing-box reports only the path.
        const process = c.metadata.process || c.metadata.processPath?.split(/[\\/]/).pop();
        return { ...c, metadata: { ...c.metadata, process }, upSpeed: p ? c.upload - p.upload : 0, downSpeed: p ? c.download - p.download : 0 };
      });
      prev.current = next;
      if (hovering.current) {
        // Same rows in the same places: only the numbers update.
        const fresh = new Map(out.map((c) => [c.id, c]));
        setRows((old) => old.map((c) => fresh.get(c.id) ?? { ...c, upSpeed: 0, downSpeed: 0 }));
        setWaiting(out.filter((c) => !listed.current.has(c.id)).length);
        return;
      }
      out.sort((a, b) => b.start.localeCompare(a.start));
      listed.current = new Set(out.map((c) => c.id));
      setWaiting(0);
      setRows(out);
    },
    1000,
    running && !paused,
  );

  const f = filter.trim().toLowerCase();
  const searched = f
    ? rows.filter((c) =>
        [c.metadata.host, c.metadata.destinationIP, c.metadata.process, c.metadata.processPath, c.rule, c.rulePayload, ...c.chains].some((x) =>
          x?.toLowerCase().includes(f),
        ),
      )
    : rows;

  const shown = searched.filter(c => (!network || c.metadata.network?.toLowerCase() === network)
    && (!exit || c.chains.includes(exit))).slice().sort((a, b) => {
      const value = (c: Row): string | number => sort === "host" ? c.metadata.host || c.metadata.destinationIP || ""
        : sort === "process" ? c.metadata.process || "" : sort === "start" ? c.start
        : c[sort as "upload" | "download" | "upSpeed" | "downSpeed"];
      const x = value(a), y = value(b);
      const order = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
      return (ascending ? order : -order) || a.id.localeCompare(b.id);
    });

  if (hovering.current) {
    const positions = new Map(displayedOrder.current.map((id, i) => [id, i]));
    shown.sort((a, b) => (positions.get(a.id) ?? Infinity) - (positions.get(b.id) ?? Infinity));
  } else {
    displayedOrder.current = shown.map(c => c.id);
  }

  const doExport = async (format: ExportFormat) => {
    setExportOpen(false);
    try {
      const path = await api.exportTable(exportTable(shown), format, "connections");
      toast(t("已导出 {n} 条连接到 {path}", { n: shown.length, path }));
    } catch (e) {
      toast(errText(e), "err");
    }
  };

  return (
    <>
      <div className="page-head" style={{ flexWrap: "wrap" }}>
        <h1>{t("连接")}</h1>
        <span className="tag">{shown.length} / {t("{n} 个", { n: rows.length })}</span>
        {waiting > 0 && !paused && (
          <span className="tag accent" title={t("{n} 个新连接，鼠标移开后显示", { n: waiting })}>
            {t("+{n} 新连接", { n: waiting })}
          </span>
        )}
        <input className="input" placeholder={t("过滤主机 / 程序 / 规则 / 节点")} value={filter} onChange={(e) => setFilter(e.target.value)} />
        <select className="input" aria-label={t("网络筛选")} value={network} onChange={e => setNetwork(e.target.value)}>
          <option value="">{t("全部网络")}</option><option value="tcp">TCP</option><option value="udp">UDP</option>
        </select>
        <select className="input" aria-label={t("出口筛选")} value={exit} onChange={e => setExit(e.target.value)}>
          <option value="">{t("全部出口")}</option>
          {[...new Set(rows.flatMap(c => c.chains))].sort().map(x => <option key={x}>{x}</option>)}
        </select>
        <select className="input" aria-label={t("排序")} value={sort} onChange={e => setSort(e.target.value)}>
          {[["start", "开始时间"], ["host", "主机"], ["process", "程序"], ["upload", "上传"], ["download", "下载"], ["upSpeed", "上传速度"], ["downSpeed", "下载速度"]].map(([k, v]) => <option key={k} value={k}>{t(v)}</option>)}
        </select>
        <button className="btn" onClick={() => setAscending(!ascending)}>{ascending ? t("升序") : t("降序")}</button>
        <div className="seg">
          <button className={view === "table" ? "on" : ""} onClick={() => setView("table")}>
            {t("列表")}
          </button>
          <button className={view === "map" ? "on" : ""} onClick={() => setView("map")}>
            {t("地图")}
          </button>
        </div>
        <button className="btn" onClick={() => setPaused(!paused)}>
          {paused ? t("继续") : t("暂停")}
        </button>
        <div className="menu-wrap">
          <button
            className="btn"
            disabled={!running || shown.length === 0}
            title={f || network || exit ? t("导出过滤后的 {n} 条连接", { n: shown.length }) : t("导出全部 {n} 条连接", { n: shown.length })}
            onClick={() => setExportOpen(!exportOpen)}
          >
            {t("导出")} ▾
          </button>
          {exportOpen && (
            <>
              <div className="menu-backdrop" onClick={() => setExportOpen(false)} />
              <div className="menu">
                {EXPORT_FORMATS.map(([fmt, label]) => (
                  <button key={fmt} onClick={() => doExport(fmt)}>
                    {label}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
        <button className="btn danger" disabled={!running} onClick={() => run(() => api.core("DELETE", "/connections"), t("已关闭全部连接"))}>
          {t("全部关闭")}
        </button>
      </div>
      {!running ? (
        <div className="empty">{t("内核未运行")}</div>
      ) : view === "map" ? (
        <ConnMap rows={shown} />
      ) : (
        <div className="table-wrap">
          <table className="conn-table">
            <colgroup>
              <col />
              <col style={{ width: 110 }} />
              <col style={{ width: 100 }} />
              <col style={{ width: 120 }} />
              <col style={{ width: 130 }} />
              <col style={{ width: 92 }} />
              <col style={{ width: 92 }} />
              <col style={{ width: 80 }} />
              <col style={{ width: 80 }} />
              <col style={{ width: 90 }} />
              <col style={{ width: 170 }} />
            </colgroup>
            <thead>
              <tr>
                <th>{t("主机")}</th>
                <th>{t("程序")}</th>
                <th>{t("类型")}</th>
                <th>{t("规则")}</th>
                <th>{t("链路")}</th>
                <th className="num">{t("↑ 速度")}</th>
                <th className="num">{t("↓ 速度")}</th>
                <th className="num">{t("上传")}</th>
                <th className="num">{t("下载")}</th>
                <th>{t("时间")}</th>
                <th />
              </tr>
            </thead>
            <tbody onMouseEnter={() => (hovering.current = true)} onMouseLeave={() => (hovering.current = false)}>
              {shown.map((c) => {
                const m = c.metadata;
                const host = `${m.host || m.destinationIP}:${m.destinationPort}`;
                return (
                  <tr key={c.id}>
                    <td className="ellipsis" title={host}>
                      {host}
                    </td>
                    <td className="ellipsis" title={m.processPath || ""}>
                      {m.process || <span className="muted">-</span>}
                    </td>
                    <td>
                      {m.type} · {m.network?.toUpperCase()}
                    </td>
                    <td className="ellipsis" title={`${c.rule} ${c.rulePayload}`}>
                      {c.rule}
                      {c.rulePayload && !c.rule.includes("(") ? ` (${c.rulePayload})` : ""}
                    </td>
                    <td className="ellipsis" title={[...c.chains].reverse().join(" → ")}>
                      {[...c.chains].reverse().join(" → ")}
                    </td>
                    <td className="num">{fmtSpeed(c.upSpeed)}</td>
                    <td className="num">{fmtSpeed(c.downSpeed)}</td>
                    <td className="num">{fmtBytes(c.upload)}</td>
                    <td className="num">{fmtBytes(c.download)}</td>
                    <td>{new Date(c.start).toLocaleTimeString()}</td>
                    <td>
                      <div className="row" style={{ gap: 6 }}>
                        {m.process && (
                          <button className="btn sm" title={t("为这个程序设置出口")} onClick={() => setRuleFor({ value: m.process!, path: m.processPath || "" })}>
                            {t("应用")}
                          </button>
                        )}
                        {(m.host || m.destinationIP) && (
                          <button
                            className="btn sm"
                            title={m.host ? t("为这个域名设置出口") : t("为这个 IP 设置出口")}
                            onClick={() => setRouteFor(m.host ? { kind: "DOMAIN-SUFFIX", value: m.host } : { kind: "IP-CIDR", value: m.destinationIP })}
                          >
                            {m.host ? t("域名") : "IP"}
                          </button>
                        )}
                        <button
                          className="btn sm danger"
                          onClick={() => {
                            api.core("DELETE", `/connections/${c.id}`);
                            setRows((old) => old.filter((x) => x.id !== c.id));
                          }}
                        >
                          {t("关闭")}
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {shown.length === 0 && <div className="empty">{t("暂无连接")}</div>}
        </div>
      )}
      {routeFor && (
        <RouteRuleDialog
          initial={routeFor}
          onClose={() => setRouteFor(null)}
          onSave={(added) => saveRoutes([...(snap?.settings.route_rules ?? []), ...added], t("已添加 {n} 条", { n: added.length }))}
        />
      )}
      {ruleFor && (
        <AppRuleDialog
          initial={{ value: ruleFor.value, path: ruleFor.path }}
          onClose={() => setRuleFor(null)}
          onSave={async (r: AppRule) => {
            try {
              await api.setAppRules([...(snap?.settings.app_rules ?? []), r]);
              toast(t("已为 {app} 添加应用规则，内核已重新加载", { app: r.value }));
              return true;
            } catch (e) {
              toast(errText(e), "err");
              return false;
            } finally {
              refresh();
            }
          }}
        />
      )}
    </>
  );
}
