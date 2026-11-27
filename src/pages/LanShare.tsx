import Help from "../Help";
import { useEffect, useState } from "react";
import QRCode from "qrcode";
import { api, type LanAddr } from "../api";
import { useApp } from "../state";
import { t } from "../i18n";

/** 局域网共享: let phones / other computers use this proxy, with a QR code of the address. */
export default function LanShareDialog({ onClose }: { onClose: () => void }) {
  const { snap, run } = useApp();
  const [addrs, setAddrs] = useState<LanAddr[] | null>(null);
  const [pick, setPick] = useState(0);
  const [svg, setSvg] = useState("");
  const on = !!snap?.settings.allow_lan;
  const port = snap?.settings.mixed_port ?? 7890;
  const mac = snap?.platform === "macos";
  const ip = addrs?.[pick]?.ip;
  const text = ip ? `http://${ip}:${port}` : "";

  useEffect(() => {
    api.lanAddresses().then(setAddrs, () => setAddrs([]));
  }, []);
  useEffect(() => {
    if (!text) return setSvg("");
    QRCode.toString(text, { type: "svg", margin: 1, errorCorrectionLevel: "M", color: { dark: "#000000", light: "#ffffff" } }).then(setSvg, () => setSvg(""));
  }, [text]);

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal lan-modal" onClick={(e) => e.stopPropagation()}>
        <h2>{t("局域网共享")}</h2>
        <div className="row">
          <div>
            <div>{t("允许局域网连接")}</div>
            <div className="small muted">{t("同一网络的手机、平板、电脑可以使用本机的代理")}</div>
          </div>
          <span className="spacer" />
          <button className={`switch ${on ? "on" : ""}`} onClick={() => run(() => api.setAllowLan(!on), on ? t("已关闭局域网共享") : t("已开启局域网共享"))} />
        </div>

        {addrs !== null && addrs.length === 0 && <div className="notice">{t("没有找到局域网地址，请确认电脑已连接到 Wi-Fi 或有线网络。")}</div>}
        {addrs && addrs.length > 0 && (
          <div className={`lan-body ${on ? "" : "off"}`}>
            <div className="lan-qr" dangerouslySetInnerHTML={{ __html: svg }} />
            <div className="lan-info">
              {addrs.length > 1 && (
                <select className="input" value={pick} onChange={(e) => setPick(Number(e.target.value))}>
                  {addrs.map((a, i) => (
                    <option key={a.ip} value={i}>
                      {a.ip} · {a.iface}
                    </option>
                  ))}
                </select>
              )}
              <div className="lan-kv">
                <span className="muted">{t("服务器")}</span>
                <b className="mono">{ip}</b>
                <span className="muted">{t("端口")}</span>
                <b className="mono">{port}</b>
                <span className="muted">{t("类型")}</span>
                <span>HTTP / SOCKS5</span>
              </div>
              <button className="btn sm" onClick={() => navigator.clipboard?.writeText(text)}>
                {t("复制 {addr}", { addr: text })}
              </button>
            </div>
          </div>
        )}
        {!on && addrs && addrs.length > 0 && <div className="small muted">{t("打开上方开关后，其他设备才能连接。")}</div>}

        <Help title={t("手机上设置")}>
          <b>{t("手机上设置")}</b>
          <div>
            <b>iPhone / iPad</b>：{t("设置 → 无线局域网 → 当前网络旁的 ⓘ → 配置代理 → 手动，填入上面的服务器和端口。")}
          </div>
          <div>
            <b>Android</b>：{t("设置 → WLAN → 长按当前网络 → 修改网络 → 高级 → 代理：手动，填入服务器和端口。")}
          </div>
          <div className="muted">{t("也可以用相机扫码复制地址。共享没有密码，只在可信的网络中开启。")}</div>
          <div className="muted">
            {mac
              ? t("连不上时：在「系统设置 → 网络 → 防火墙」中允许 Prism 的内核接收连接。")
              : t("连不上时：在 Windows 防火墙中允许 prism-core / mihomo / sing-box 通过「专用网络」。")}
          </div>
        </Help>

        <div className="row">
          <span className="spacer" />
          <button className="btn primary" onClick={onClose}>
            {t("关闭")}
          </button>
        </div>
      </div>
    </div>
  );
}
