import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";

function cls(line: string): string {
  if (line.startsWith("[prism]")) return "app";
  if (/\b(ERROR|error|FATAL|fatal)\b|level=error/.test(line)) return "err";
  if (/\b(WARN|warn|warning)\b|level=warning/.test(line)) return "warn";
  return "";
}

export default function Logs() {
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
  }, [lines, follow]);

  const f = filter.trim().toLowerCase();
  const shown = f ? lines.filter((l) => l.toLowerCase().includes(f)) : lines;

  return (
    <>
      <div className="page-head">
        <h1>日志</h1>
        <input className="input" placeholder="过滤" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="btn" onClick={() => setFollow(!follow)}>
          {follow ? "停止滚动" : "自动滚动"}
        </button>
        <button
          className="btn"
          onClick={() => {
            api.clearLogs();
            setLines([]);
          }}
        >
          清空
        </button>
      </div>
      <div className="logs" ref={box}>
        {shown.map((l, i) => (
          <div key={i} className={`log-line ${cls(l)}`}>
            {l}
          </div>
        ))}
        {shown.length === 0 && <div className="muted">暂无日志</div>}
      </div>
    </>
  );
}
