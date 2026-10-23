import { useRef, useState } from "react";
import { api, fmtBytes, fmtSpeed, type Connection, type ConnectionsResp } from "../api";
import { useApp, usePoll } from "../state";

interface Row extends Connection {
  upSpeed: number;
  downSpeed: number;
}

export default function Connections() {
  const { snap, run } = useApp();
  const [rows, setRows] = useState<Row[]>([]);
  const [filter, setFilter] = useState("");
  const [paused, setPaused] = useState(false);
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
        [c.metadata.host, c.metadata.destinationIP, c.rule, c.rulePayload, ...c.chains].some((x) => x?.toLowerCase().includes(f)),
      )
    : rows;

  return (
    <>
      <div className="page-head">
        <h1>连接</h1>
        <span className="tag">{rows.length} 个</span>
        <input className="input" placeholder="过滤主机 / 规则 / 节点" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="btn" onClick={() => setPaused(!paused)}>
          {paused ? "继续" : "暂停"}
        </button>
        <button className="btn danger" disabled={!running} onClick={() => run(() => api.core("DELETE", "/connections"), "已关闭全部连接")}>
          全部关闭
        </button>
      </div>
      {!running ? (
        <div className="empty">内核未运行</div>
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>主机</th>
                <th>类型</th>
                <th>规则</th>
                <th>链路</th>
                <th className="num">↑ 速度</th>
                <th className="num">↓ 速度</th>
                <th className="num">上传</th>
                <th className="num">下载</th>
                <th>时间</th>
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
                      <button className="btn sm danger" onClick={() => api.core("DELETE", `/connections/${c.id}`)}>
                        关闭
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {shown.length === 0 && <div className="empty">暂无连接</div>}
        </div>
      )}
    </>
  );
}
