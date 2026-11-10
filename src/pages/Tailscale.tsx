import { useState } from "react";
import { api, type TailStatus } from "../api";
import { useApp, usePoll } from "../state";
import { t } from "../i18n";

export default function Tailscale() {
  const { snap, run } = useApp();
  const [status, setStatus] = useState<TailStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [ping, setPing] = useState("");
  usePoll(async () => { if (!busy) setStatus(await api.tailscaleStatus()); }, 10000);
  const action = async (fn: () => Promise<unknown>) => { setBusy(true); await run(fn); setBusy(false); };
  const bypass = !!snap?.settings.tailscale_bypass;
  const peers = status?.peers ?? [];
  return <>
    <div className="page-head"><h1>Tailscale {t("组网")}</h1>
      <button className="btn" disabled={busy} onClick={() => action(async () => setStatus(await api.tailscaleStatus()))}>{t("刷新")}</button>
      <button className="btn" disabled={busy} onClick={() => action(() => api.tailscaleOpen("download"))}>{t("官方安装 / 注册")}</button>
      <button className="btn" disabled={busy} onClick={() => action(() => api.tailscaleOpen("admin"))}>{t("组网管理后台")}</button>
    </div>
    <div className="card" style={{ marginBottom: 16 }}>
      <h3>{t("连接你的设备")}</h3>
      <p className="small muted">{t("先安装官方 Tailscale 客户端并创建账号，再点击登录连接，在浏览器中完成设备授权。其他设备登录同一组网后会出现在下方。")}</p>
      <div className="row wrap"><span className={`dot ${status?.state === "Running" ? "on" : ""}`} />
        <b>{status?.state ?? t("检查中…")}</b><span className="small muted">{status?.version}</span><span>{status?.tailnet}</span>
        <span className="spacer" /><button className="btn primary" disabled={busy || !status?.installed} onClick={() => action(async () => setStatus(await api.tailscaleAction("connect")))}>{busy ? t("处理中…") : t("登录 / 连接")}</button>
        {status?.auth_url && <button className="btn" disabled={busy} onClick={() => action(() => api.tailscaleOpen("login"))}>{t("打开登录授权页")}</button>}
        <button className="btn" disabled={busy || (status?.state !== "Running" && status?.protection?.phase !== "connecting")} onClick={() => action(async () => setStatus(await api.tailscaleAction("disconnect")))}>{t("断开组网")}</button>
      </div>
      {status?.error && <p className="small" style={{ color: "var(--bad)" }}>{status.error}</p>}
      {status && !status.installed && <p>{t("尚未检测到 Tailscale。安装完成并启动官方客户端后点击刷新。")}</p>}
      {status?.self_node && <p className="small mono">{t("本机")}: {status.self_node.name} · {status.self_node.ips.join(" · ")}</p>}
    </div>
    <div className="card" style={{ marginBottom: 16 }}>
      <div className="row wrap"><b>{t("Prism TUN 组网直连")}</b><span className="spacer" />
        <button className={`switch ${bypass ? "on" : ""}`} aria-label={t("Prism TUN 组网直连")} disabled={busy || !snap || (snap.settings.core !== "prism" && !bypass)} onClick={() => action(() => api.saveSettings({ ...snap!.settings, tailscale_bypass: !bypass }))} />
      </div>
      <p className="small muted">{t("开启后，Tailscale 地址和 ts.net 域名使用组网路由，MagicDNS 交给 Tailscale；Prism 不再把这些连接固定到物理网卡。保存会重启运行中的内核。")}</p>
      <p className="small muted">{t("需要官方客户端已连接。此选项仅适用于 Prism，并明确豁免组网 IPv6 目标。Windows 配合新版 TUN 服务可与断网保护同时启用，接口或身份未验证时不放行组网。")}</p>
    </div>
    {status?.protection && <div className="card" style={{ marginBottom: 16 }}>
      <h3>{t("断网保护联动")}</h3>
      <p><b>{status.protection.phase || "—"}</b> · {status.protection.detail}</p>
      <p className="small muted">{t("已验证接口")}: {status.protection.interface_index ?? "—"} · {t("允许的组网地址数")}: {status.protection.peer_addresses}</p>
      {status.protection.lease_seconds > 0 && <p>{t("临时连接授权剩余秒数")}: {status.protection.lease_seconds}</p>}
      <p className="small muted">{t("连接按钮可给通过验证的 Tailscale 守护进程最多 60 秒临时授权；未完成连接会自动撤销，可点击断开立即取消。正常连接只放行已验证接口上的组网地址，互联网阻断保持启用。")}</p>
    </div>}
    <div className="table-wrap"><table><thead><tr><th>{t("设备")}</th><th>IP / MagicDNS</th><th>{t("状态")}</th><th>{t("链路")}</th><th>{t("操作")}</th></tr></thead>
      <tbody>{peers.map(p => <tr key={p.dns_name || p.ips.join(",")}><td>{p.name}</td><td className="mono small">{p.ips.join(" · ")}<div>{p.dns_name}</div></td><td>{p.online ? t("在线") : t("离线")}</td><td>{p.direct ? t("直连") : p.relay ? `DERP ${p.relay}` : "—"}</td><td>
        <button className="btn sm" disabled={busy || !p.ips.length || !p.online} onClick={() => action(async () => setPing(await api.tailscalePing(p.ips[0])))}>{t("连通测试")}</button>
      </td></tr>)}</tbody></table>
      {!peers.length && <div className="empty">{t("暂无组网设备，请在其他设备安装 Tailscale 并登录同一组网。")}</div>}
    </div>
    {ping && <pre className="card small" style={{ whiteSpace: "pre-wrap" }}>{ping}</pre>}
  </>;
}
