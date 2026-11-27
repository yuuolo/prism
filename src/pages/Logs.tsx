import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { t } from "../i18n";

function cls(line: string): string {
  if (line.startsWith("[prism]")) return "app";
  if (/\b(ERROR|error|FATAL|fatal)\b|level=error/.test(line)) return "err";
  if (/\b(WARN|warn|warning)\b|level=warning/.test(line)) return "warn";
  return "";
}

export default function Logs() {
  const [security, setSecurity] = useState(false);
  const [eventKind, setEventKind] = useState("");
  const [events, setEvents] = useState<string[]>([]);
  const [lines, setLines] = useState<string[]>([]);
  const [filter, setFilter] = useState("");
  const [follow, setFollow] = useState(true);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.logs().then(setLines);
    const un = listen<string>("core-log", (e) => setLines((l) => [...l.slice(-1999), e.payload]));
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (follow && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines, events, security, follow]);

  useEffect(() => {
    if (!security) return;
    const load = () => api.securityEvents().then(setEvents).catch(console.error);
    load();
    const timer = setInterval(load, 2000);
    return () => clearInterval(timer);
  }, [security]);
  const source = security ? events.map(line => line.replace(/^([0-9]+) /, (_, ts) => new Date(Number(ts) * 1000).toLocaleString() + " ")) : lines;
  const f = filter.trim().toLowerCase();
  const shown = source.filter(l => (!f || l.toLowerCase().includes(f)) && (!security || !eventKind || (
    eventKind === "reject" ? /reject|拒绝/i.test(l) : eventKind === "kill" ? l.includes("断网保护") : /启动|持续运行|异常退出/.test(l)
  )));

  return (
    <>
      <div className="page-head">
        <h1>{t("日志")}</h1>
        <button className="btn" onClick={() => setSecurity(!security)}>{security ? t("内核日志") : t("安全事件")}</button>
        {security && <select className="input" aria-label={t("事件类型")} value={eventKind} onChange={e => setEventKind(e.target.value)}>
          <option value="">{t("全部事件")}</option><option value="kill">{t("断网保护")}</option><option value="reject">{t("异常拒绝")}</option><option value="runtime">{t("运行记录")}</option>
        </select>}
        <input className="input" placeholder={t("过滤")} value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="btn" onClick={() => setFollow(!follow)}>
          {follow ? t("停止滚动") : t("自动滚动")}
        </button>
        <button
          className="btn"
          disabled={security}
          onClick={() => {
            api.clearLogs();
            setLines([]);
          }}
        >
          {t("清空")}
        </button>
      </div>
      <div className="logs" ref={box}>
        {shown.map((l, i) => (
          <div key={i} className={`log-line ${cls(l)}`}>
            {l}
          </div>
        ))}
        {shown.length === 0 && <div className="muted">{t("暂无日志")}</div>}
      </div>
    </>
  );
}
