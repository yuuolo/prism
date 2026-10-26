import { useRef, useState } from "react";
import { api, errText, fmtBytes, fmtSpeed, type AppRule, type Connection, type ConnectionsResp, type ExportFormat, type ExportTable, type RouteRule } from "../api";
import { useApp, usePoll } from "../state";
import { AppRuleDialog } from "./Apps";
import { RouteRuleDialog, useSaveRouteRules } from "./Routes";
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
  const [paused, setPaused] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [routeFor, setRouteFor] = useState<Partial<RouteRule> | null>(null);
  const saveRoutes = useSaveRouteRules();
  const prev = useRef<Map<string, Connection>>(new Map());
  const running = !!snap?.running;

  usePoll(
    async () => {
      const r = await api.core<ConnectionsResp>("GET", "/connections");
      const list = r.connections ?? [];
      const next = new Map<string, Connection>();
      const out = list.map((c) => {
        next.set(c.id, c);
        const p = prev.current.get(c.id);
        return { ...c, upSpeed: p ? c.upload - p.upload : 0, downSpeed: p ? c.download - p.download : 0 };
      });
      prev.current = next;
      out.sort((a, b) => b.start.localeCompare(a.start));
      setRows(out);
    },
    1000,
    running && !paused,
  );

  const f = filter.trim().toLowerCase();
  const shown = f
    ? rows.filter((c) =>
        [c.metadata.host, c.metadata.destinationIP, c.metadata.process, c.rule, c.rulePayload, ...c.chains].some((x) =>
          x?.toLowerCase().includes(f),
        ),
      )
    : rows;

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
      <div className="page-head">
        <h1>{t("连接")}</h1>
        <span className="tag">{t("{n} 个", { n: rows.length })}</span>
        <input className="input" placeholder={t("过滤主机 / 程序 / 规则 / 节点")} value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="btn" onClick={() => setPaused(!paused)}>
          {paused ? t("继续") : t("暂停")}
        </button>
        <div className="menu-wrap">
          <button
            className="btn"
            disabled={!running || shown.length === 0}
            title={f ? t("导出过滤后的 {n} 条连接", { n: shown.length }) : t("导出全部 {n} 条连接", { n: shown.length })}
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
      ) : (
        <div className="table-wrap">
          <table>
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
            <tbody>
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
                            {t("分流")}
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
                        <button className="btn sm danger" onClick={() => api.core("DELETE", `/connections/${c.id}`)}>
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
