import { useCallback, useEffect, useState } from "react";
import { api, delayClass, enc, lastDelay, type ProxyItem } from "../api";
import { useApp } from "../state";
import type { PageKey } from "../App";
import { ProxyProviders } from "./Providers";

type ProxyMap = Record<string, ProxyItem>;
type SortKey = "default" | "delay" | "name";

const SORTS: [SortKey, string][] = [
  ["default", "默认"],
  ["delay", "延迟"],
  ["name", "名称"],
];
/** DIRECT, REJECT and friends: nothing to measure. */
const BUILTIN_TYPES = ["Direct", "Reject", "RejectDrop", "Pass", "Compatible"];
/** Parallel delay tests in 全部测速. */
const TEST_CONCURRENCY = 8;

function loadSort(): SortKey {
  try {
    const v = localStorage.getItem("prism.proxySort");
    if (v === "delay" || v === "name") return v;
  } catch {
    /* storage unavailable */
  }
  return "default";
}

/** Sort key for 延迟: tested first (fastest first), then timeouts, then untested. */
function delayRank(d?: number): number {
  if (d === undefined) return Number.MAX_SAFE_INTEGER;
  return d === 0 ? Number.MAX_SAFE_INTEGER - 1 : d;
}

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
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortKey>(loadSort);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
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
    api.updateTray().catch(() => {});
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
    api.updateTray().catch(() => {});
  };

  // Every node (not group) in the groups shown, each tested once.
  const testAll = async () => {
    const names = [...new Set(groups.flatMap((g) => g.all ?? []))].filter(
      (n) => proxies[n] && !proxies[n].all && !BUILTIN_TYPES.includes(proxies[n].type),
    );
    if (!names.length) return;
    setTesting((t) => ({ ...t, ...Object.fromEntries(names.map((n) => [n, true])) }));
    setProgress({ done: 0, total: names.length });
    let next = 0;
    let done = 0;
    const worker = async () => {
      while (next < names.length) {
        const n = names[next++];
        await api.core("GET", `/proxies/${enc(n)}/delay?timeout=5000&url=${enc(testUrl)}`).catch(() => {});
        done++;
        setTesting((t) => ({ ...t, [n]: false }));
        setProgress({ done, total: names.length });
        // Show results as they come in, without reloading on every node.
        if (done % TEST_CONCURRENCY === 0) load();
      }
    };
    await Promise.all(Array.from({ length: Math.min(TEST_CONCURRENCY, names.length) }, worker));
    setProgress(null);
    load();
    api.updateTray().catch(() => {});
  };

  const changeSort = (k: SortKey) => {
    setSort(k);
    try {
      localStorage.setItem("prism.proxySort", k);
    } catch {
      /* storage unavailable */
    }
  };

  const q = query.trim().toLowerCase();
  const membersOf = (g: ProxyItem) => {
    let list = g.all ?? [];
    // A group whose own name matches shows all its members.
    if (q && !g.name.toLowerCase().includes(q)) list = list.filter((n) => n.toLowerCase().includes(q));
    if (sort === "delay") list = [...list].sort((a, b) => delayRank(lastDelay(proxies[a])) - delayRank(lastDelay(proxies[b])));
    if (sort === "name") list = [...list].sort((a, b) => a.localeCompare(b, "zh-CN", { numeric: true }));
    return list;
  };
  const shown = groups.map((g) => ({ g, members: membersOf(g) })).filter((x) => !q || x.members.length > 0);

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
        <button className="btn primary" disabled={!!progress} onClick={testAll}>
          {progress ? `测速中 ${progress.done}/${progress.total}` : "全部测速"}
        </button>
      </div>

      <div className="row proxy-tools">
        <input
          className="input"
          style={{ flex: 1 }}
          placeholder="搜索节点或代理组"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setQuery("")}
        />
        <span className="small muted">排序</span>
        <div className="seg">
          {SORTS.map(([k, label]) => (
            <button key={k} className={sort === k ? "on" : ""} onClick={() => changeSort(k)}>
              {label}
            </button>
          ))}
        </div>
      </div>

      <ProxyProviders onChange={load} />

      {shown.map(({ g, members }) => {
        // Searching opens every group that has a match.
        const isOpen = q ? true : (open[g.name] ?? groups.length <= 3);
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
              <span className="small muted">{q ? `${members.length} / ${g.all?.length}` : g.all?.length} 个</span>
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
                {members.map((n) => {
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
      {groups.length > 0 && shown.length === 0 && <div className="empty">没有匹配「{query.trim()}」的节点</div>}
    </>
  );
}
