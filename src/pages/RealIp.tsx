import Help from "../Help";
import { useState } from "react";
import { api, errText, type RealIpResult, type RealIpRule } from "../api";
import { useApp } from "../state";
import { t } from "../i18n";

/** Rough check before saving; the backend has the final word. */
function badValue(kind: RealIpRule["kind"], v: string): boolean {
  if (!v || /[,\s]/.test(v)) return true;
  return kind === "network" && !/^[0-9a-fA-F:.]+(\/\d{1,3})?$/.test(v);
}

function badDns(v: string): boolean {
  return !!v && (/[\s#]/.test(v) || !/^((udp|tcp|tls|https|quic|h3):\/\/.+|\[?[0-9a-fA-F:.]+\]?(:\d+)?)$/.test(v));
}

/** Add / edit dialog. */
function RealIpDialog({ initial, onClose, onSave }: { initial: Partial<RealIpRule>; onClose: () => void; onSave: (r: RealIpRule) => Promise<boolean> }) {
  const [kind, setKind] = useState<RealIpRule["kind"]>(initial.kind ?? "domain");
  const [value, setValue] = useState(initial.value ?? "");
  const [subdomains, setSubdomains] = useState(initial.subdomains ?? true);
  const [dns, setDns] = useState(initial.dns ?? "");
  const [saving, setSaving] = useState(false);
  const clean = value.trim();
  const server = dns.trim();
  const bad = !!clean && badValue(kind, clean);
  const dnsBad = kind === "domain" && badDns(server);

  const save = async () => {
    setSaving(true);
    const ok = await onSave({
      id: initial.id ?? crypto.randomUUID(),
      enabled: initial.enabled ?? true,
      kind,
      value: clean,
      subdomains: kind === "domain" && subdomains,
      dns: kind === "domain" ? server : "",
    });
    setSaving(false);
    if (ok) onClose();
  };

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{initial.id ? t("编辑真实 IP 规则") : t("添加真实 IP 规则")}</h2>
        <div className="field">
          <label>{t("类型")}</label>
          <div className="seg">
            <button className={kind === "domain" ? "on" : ""} onClick={() => setKind("domain")}>
              {t("域名")}
            </button>
            <button className={kind === "network" ? "on" : ""} onClick={() => setKind("network")}>
              {t("IP / 网段")}
            </button>
          </div>
        </div>
        <div className="field">
          <label>{kind === "domain" ? t("域名") : t("IP / 网段")}</label>
          <input className="input mono" placeholder={kind === "domain" ? "corp.example" : "10.0.0.0/8"} value={value} onChange={(e) => setValue(e.target.value)} />
        </div>
        {kind === "domain" && (
          <>
            <label className="row small" style={{ gap: 8, cursor: "pointer" }}>
              <input type="checkbox" checked={subdomains} onChange={(e) => setSubdomains(e.target.checked)} />
              <span>{t("包含所有子域名")}</span>
            </label>
            <div className="field">
              <label>{t("解析用的 DNS（可选）")}</label>
              <input className="input mono" placeholder="10.1.1.1" value={dns} onChange={(e) => setDns(e.target.value)} />
            </div>
          </>
        )}
        <Help title={t("真实 IP")}>
          {kind === "domain"
            ? t("只有公司 DNS 才能解析的内网域名，请填写公司 DNS。留空时用订阅的 DNS 解析。这个域名的连接会直连。")
            : t("这个网段的连接会直连，不走代理。按 IP 访问时不经过 DNS，不需要其他设置。")}
        </Help>
        {bad && <div className="small" style={{ color: "var(--bad)" }}>{t("格式不对：{v}", { v: clean })}</div>}
        {dnsBad && <div className="small" style={{ color: "var(--bad)" }}>{t("DNS 地址格式不对，可以写 10.1.1.1、tcp://10.1.1.1 或 https://dns.example/dns-query。")}</div>}
        <div className="row">
          <span className="spacer" />
          <button className="btn" onClick={onClose}>
            {t("取消")}
          </button>
          <button className="btn primary" disabled={saving || !clean || bad || dnsBad} onClick={save}>
            {saving ? t("保存中…") : t("保存")}
          </button>
        </div>
      </div>
    </div>
  );
}

export default function RealIp() {
  const { snap, toast, refresh } = useApp();
  const [dialog, setDialog] = useState<Partial<RealIpRule> | null>(null);
  const [host, setHost] = useState("");
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<RealIpResult | { error: string } | null>(null);
  const rules = snap?.settings.real_ip_rules ?? [];
  const running = !!snap?.running;
  const mode = snap?.settings.mode;

  const save = async (next: RealIpRule[], ok = t("已保存")) => {
    try {
      await api.setRealIpRules(next);
      toast(running ? t("{msg}，内核已重新加载", { msg: ok }) : ok);
      return true;
    } catch (e) {
      toast(errText(e), "err");
      return false;
    } finally {
      refresh();
    }
  };
  const upsert = (r: RealIpRule) => save(rules.some((x) => x.id === r.id) ? rules.map((x) => (x.id === r.id ? r : x)) : [...rules, r]);

  const test = async () => {
    setTesting(true);
    setResult(null);
    try {
      setResult(await api.testRealIp(host.trim()));
    } catch (e) {
      setResult({ error: errText(e) });
    }
    setTesting(false);
  };

  return (
    <>
      <div className="page-head">
        <h1>{t("真实 IP")}</h1>
        <span className="tag">{t("{n} 条", { n: rules.length })}</span>
        <button className="btn primary" onClick={() => setDialog({})}>
          {t("添加规则")}
        </button>
      </div>

      <Help title={t("真实 IP")}><div>
        {t("让公司内网等地址拿到真实 IP 并直连：域名不再返回 198.18.x.x 这样的假 IP，还可以指定用公司 DNS 解析。优先于应用分流和域名分流；三种内核都支持，换订阅也保留。")}
      </div></Help>
      {mode !== "rule" && <div className="notice">{t("当前不是规则模式：真实 IP 的解析设置仍然生效，但直连规则要在规则模式下才起作用。")}</div>}

      <div className="card" style={{ marginBottom: 14 }}>
        <div className="row wrap">
          <input
            className="input mono"
            style={{ flex: 1, minWidth: 200 }}
            placeholder={t("输入域名，测试这台电脑现在解析到什么")}
            value={host}
            onChange={(e) => setHost(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && host.trim() && test()}
          />
          <button className="btn" disabled={testing || !host.trim()} onClick={test}>
            {testing ? t("测试中…") : t("测试")}
          </button>
        </div>
        {result && (
          <div className="small" style={{ marginTop: 10 }}>
            {"error" in result ? (
              <span style={{ color: "var(--bad)" }}>{t("解析失败：{e}", { e: result.error })}</span>
            ) : result.fake ? (
              <span style={{ color: "var(--bad)" }}>{t("拿到的是假 IP：{ips}。这个域名还不在下面的规则里，或者规则还没生效。", { ips: result.ips.join("、") })}</span>
            ) : (
              <span style={{ color: "var(--good)" }}>{t("拿到的是真实 IP：{ips}", { ips: result.ips.join("、") })}</span>
            )}
            {!snap?.settings.tun && <div className="muted">{t("TUN 模式未开启，这里看到的是系统 DNS 的结果，不经过内核。")}</div>}
          </div>
        )}
      </div>

      {rules.length === 0 && <div className="empty">{t("还没有规则。点「添加规则」，填写公司内网的域名或网段。")}</div>}

      {rules.map((r) => (
        <div className="card app-rule" key={r.id}>
          <button className={`switch ${r.enabled ? "on" : ""}`} title={r.enabled ? t("停用") : t("启用")} onClick={() => save(rules.map((x) => (x.id === r.id ? { ...x, enabled: !x.enabled } : x)))} />
          <span className="tag">{r.kind === "domain" ? t("域名") : t("IP / 网段")}</span>
          <div className="app-name" title={r.value}>
            <b className="mono">{r.value}</b>
            <span className="muted small">
              {r.kind === "domain" ? (r.subdomains ? t("包含所有子域名") : t("仅这个域名")) : t("直连")}
              {r.dns && ` · ${t("DNS：{v}", { v: r.dns })}`}
            </span>
          </div>
          <button className="btn sm" onClick={() => setDialog(r)}>
            {t("编辑")}
          </button>
          <button className="btn sm danger" onClick={() => save(rules.filter((x) => x.id !== r.id), t("已删除"))}>
            {t("删除")}
          </button>
        </div>
      ))}

      {dialog && <RealIpDialog initial={dialog} onClose={() => setDialog(null)} onSave={upsert} />}
    </>
  );
}
