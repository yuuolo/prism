import { useCallback, useEffect, useState } from "react";
import { api, enc, errText, fmtAgo } from "../api";
import { useApp } from "../state";
import { t } from "../i18n";

interface ProxyProvider {
  name: string;
  vehicleType: string;
  proxies: { name: string }[];
  updatedAt?: string | null;
}

interface RuleProvider {
  name: string;
  vehicleType: string;
  behavior: string;
  ruleCount: number;
  updatedAt?: string | null;
}

const ago = (when?: string | null) => (when ? t("{ago}更新", { ago: fmtAgo(Date.parse(when) / 1000) }) : "");

/** Fetches a provider map, skipping mihomo's built-in "default" provider. */
function useProviders<T extends { vehicleType: string }>(path: string, running: boolean) {
  const [list, setList] = useState<T[]>([]);
  const load = useCallback(async () => {
    try {
      const r = await api.core<{ providers: Record<string, T> }>("GET", path);
      setList(Object.values(r.providers ?? {}).filter((p) => p.vehicleType !== "Compatible"));
    } catch {
      setList([]);
    }
  }, [path]);
  useEffect(() => {
    if (running) load();
  }, [running, load]);
  return { list, load };
}

/** 代理集合 (proxy-providers) with update / health-check buttons. */
export function ProxyProviders({ onChange }: { onChange: () => void }) {
  const { snap, toast } = useApp();
  const { list, load } = useProviders<ProxyProvider>("/providers/proxies", !!snap?.running);
  const [busy, setBusy] = useState<string | null>(null);
  if (list.length === 0) return null;

  const act = async (name: string, method: string, path: string, ok: string) => {
    setBusy(name + method);
    try {
      await api.core(method, path);
      toast(ok);
    } catch (e) {
      toast(errText(e), "err");
    }
    setBusy(null);
    await load();
    onChange();
  };

  return (
    <div className="card" style={{ marginBottom: 14 }}>
      <h3>{t("代理集合")}</h3>
      {list.map((p) => (
        <div className="row small" key={p.name} style={{ padding: "6px 0", borderTop: "1px solid var(--line)" }}>
          <b style={{ fontSize: 13 }}>{p.name}</b>
          <span className="tag">{p.vehicleType}</span>
          <span className="muted">{t("{n} 个节点", { n: p.proxies.length })}</span>
          <span className="muted">{ago(p.updatedAt)}</span>
          <span className="spacer" />
          <button className="btn sm" disabled={!!busy} onClick={() => act(p.name, "GET", `/providers/proxies/${enc(p.name)}/healthcheck`, t("{name} 测速完成", { name: p.name }))}>
            {busy === p.name + "GET" ? t("测速中…") : t("测速")}
          </button>
          {p.vehicleType === "HTTP" && (
            <button className="btn sm" disabled={!!busy} onClick={() => act(p.name, "PUT", `/providers/proxies/${enc(p.name)}`, t("{name} 已更新", { name: p.name }))}>
              {busy === p.name + "PUT" ? t("更新中…") : t("更新")}
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

/** 规则集合 (rule-providers) with update buttons. */
export function RuleProviders({ onChange }: { onChange: () => void }) {
  const { snap, toast } = useApp();
  const { list, load } = useProviders<RuleProvider>("/providers/rules", !!snap?.running);
  const [busy, setBusy] = useState<string | null>(null);
  if (list.length === 0) return null;

  const update = async (name: string) => {
    setBusy(name);
    try {
      await api.core("PUT", `/providers/rules/${enc(name)}`);
      toast(t("{name} 已更新", { name }));
    } catch (e) {
      toast(errText(e), "err");
    }
    setBusy(null);
    await load();
    onChange();
  };

  return (
    <div className="card" style={{ marginBottom: 14 }}>
      <h3>{t("规则集合")}</h3>
      {list.map((p) => (
        <div className="row small" key={p.name} style={{ padding: "6px 0", borderTop: "1px solid var(--line)" }}>
          <b style={{ fontSize: 13 }}>{p.name}</b>
          <span className="tag">{p.behavior}</span>
          <span className="tag">{p.vehicleType}</span>
          <span className="muted">{t("{n} 条", { n: p.ruleCount })}</span>
          <span className="muted">{ago(p.updatedAt)}</span>
          <span className="spacer" />
          {p.vehicleType === "HTTP" && (
            <button className="btn sm" disabled={!!busy} onClick={() => update(p.name)}>
              {busy === p.name ? t("更新中…") : t("更新")}
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
