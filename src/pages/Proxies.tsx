import { useCallback, useEffect, useState } from "react";
import { api, delayClass, enc, lastDelay, type ProxyItem } from "../api";
import { useApp } from "../state";
import type { PageKey } from "../App";
import { ProxyProviders } from "./Providers";

type ProxyMap = Record<string, ProxyItem>;

function DelayView({ d, testing, onTest }: { d?: number; testing: boolean; onTest: () => void }) {
  return (
    <span
      className={`delay ${delayClass(d)}`}
      title="点击测速"
      onClick={(e) => {
        e.stopPropagation();
        onTest();
      }}
    >
      {testing ? "…" : d === undefined ? "测速" : d === 0 ? "超时" : `${d} ms`}
    </span>
  );
}

export default function Proxies({ go }: { go: (p: PageKey) => void }) {
  const { snap, run, toast } = useApp();
  const [proxies, setProxies] = useState<ProxyMap>({});
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [testing, setTesting] = useState<Record<string, boolean>>({});
  const running = !!snap?.running;
  const testUrl = snap?.settings.test_url ?? "";

  const load = useCallback(async () => {
    try {
      const r = await api.core<{ proxies: ProxyMap }>("GET", "/proxies");
      setProxies(r.proxies);
    } catch {
      setProxies({});
    }
  }, []);

  useEffect(() => {
    if (running) load();
  }, [running, load]);

  if (!running) {
    return (
      <>
        <div className="page-head">
          <h1>代理</h1>
        </div>
        <div className="empty">
          内核未运行
          <br />
          <button className="btn primary" onClick={() => go("home")}>
            去启动
          </button>
        </div>
      </>
    );
  }

  const mode = snap!.settings.mode;
  const order = proxies["GLOBAL"]?.all ?? Object.keys(proxies);
  let groups = order.map((n) => proxies[n]).filter((p): p is ProxyItem => !!p?.all && !p.hidden);
  if (mode === "global" && proxies["GLOBAL"]) groups = [proxies["GLOBAL"], ...groups];
  // Members of one group can't be selected if the group is automatic.
  const selectable = (g: ProxyItem) => g.type === "Selector";

  const testOne = async (name: string) => {
    setTesting((t) => ({ ...t, [name]: true }));
    try {
      await api.core("GET", `/proxies/${enc(name)}/delay?timeout=5000&url=${enc(testUrl)}`);
    } catch {
      /* failure is recorded in history */
    }
    setTesting((t) => ({ ...t, [name]: false }));
    load();
  };

  const testGroup = async (g: ProxyItem) => {
    const names = g.all ?? [];
    setTesting((t) => ({ ...t, ...Object.fromEntries(names.map((n) => [n, true])), [g.name]: true }));
    try {
      await api.core("GET", `/group/${enc(g.name)}/delay?timeout=5000&url=${enc(testUrl)}`);
    } catch {
      // Older cores without /group: test members one by one.
      await Promise.all(names.map((n) => api.core("GET", `/proxies/${enc(n)}/delay?timeout=5000&url=${enc(testUrl)}`).catch(() => {})));
    }
    setTesting((t) => ({ ...t, ...Object.fromEntries(names.map((n) => [n, false])), [g.name]: false }));
    load();
  };

  const select = async (g: ProxyItem, name: string) => {
    if (!selectable(g)) {
      toast(`${g.name} 是自动选择组，不能手动选择`, "err");
      return;
    }
    if (await run(() => api.selectProxy(g.name, name))) load();
  };

  return (
    <>
      <div className="page-head">
        <h1>代理</h1>
        <span className="tag accent">{mode === "rule" ? "规则模式" : mode === "global" ? "全局模式" : "直连模式"}</span>
        <button className="btn" onClick={load}>
          刷新
        </button>
      </div>

      <ProxyProviders onChange={load} />

      {groups.map((g) => {
        const isOpen = open[g.name] ?? groups.length <= 3;
        const delayOfNow = lastDelay(proxies[g.now ?? ""]);
        return (
          <div className="card group" key={g.name}>
            <div className="group-head" onClick={() => setOpen((o) => ({ ...o, [g.name]: !isOpen }))}>
              <span className={`chev ${isOpen ? "open" : ""}`}>▶</span>
              <span className="name">{g.name}</span>
              <span className="tag">{g.type}</span>
              <span className="now">
                {g.now} {delayOfNow ? <span className={delayClass(delayOfNow)}>· {delayOfNow} ms</span> : null}
              </span>
              <span className="spacer" />
              <span className="small muted">{g.all?.length} 个</span>
              <button
                className="btn sm"
                disabled={testing[g.name]}
                onClick={(e) => {
                  e.stopPropagation();
                  testGroup(g);
                }}
              >
                {testing[g.name] ? "测速中…" : "测速"}
              </button>
            </div>
            {isOpen && (
              <div className="proxy-grid">
                {(g.all ?? []).map((n) => {
                  const p = proxies[n];
                  return (
                    <div
                      key={n}
                      className={`proxy ${g.now === n ? "sel" : ""} ${selectable(g) ? "" : "static"}`}
                      onClick={() => select(g, n)}
                      title={n}
                    >
                      <div className="pname">{n}</div>
                      <div className="pmeta">
                        <span>{p?.type ?? ""}</span>
                        <DelayView d={lastDelay(p)} testing={!!testing[n]} onTest={() => testOne(n)} />
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
      {groups.length === 0 && <div className="empty">没有代理组</div>}
    </>
  );
}
