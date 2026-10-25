import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type ImportRequest } from "./api";
import { useApp } from "./state";
import type { PageKey } from "./App";
import { t } from "./i18n";

/** Confirms subscriptions arriving through prism:// and clash:// links. */
export default function ImportDialog({ go }: { go: (p: PageKey) => void }) {
  const { snap, run, toast } = useApp();
  const [queue, setQueue] = useState<ImportRequest[]>([]);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    // Links that arrived before the window loaded wait in the backend.
    const take = () =>
      api
        .takeImportRequests()
        .then((links) => {
          for (const l of links) if (l.error) toast(l.error, "err");
          const reqs = links.flatMap((l) => (l.request ? [l.request] : []));
          if (reqs.length) setQueue((q) => [...q, ...reqs]);
        })
        .catch(() => {});
    take();
    const un = listen("import-request", take);
    return () => {
      un.then((f) => f());
    };
  }, [toast]);

  const cur = queue[0];
  useEffect(() => setName(cur?.name ?? ""), [cur]);
  if (!cur || !snap) return null;

  const existing = snap.profiles.find((p) => p.url === cur.url);
  const next = () => setQueue((q) => q.slice(1));
  const confirm = async () => {
    setBusy(true);
    const ok = await run(
      () => (existing ? api.updateProfile(existing.id) : api.importProfile({ url: cur.url, name: name.trim() || undefined })),
      existing ? t("订阅「{name}」已更新", { name: existing.name }) : t("订阅已导入"),
    );
    setBusy(false);
    if (ok) {
      next();
      go("profiles");
    }
  };

  return (
    <div className="modal-bg">
      <div className="modal import-modal">
        <h2>{existing ? t("更新订阅") : t("导入订阅")}</h2>
        <div className="small muted">{t("网页或其他程序请求把下面的订阅添加到 Prism，请确认它来自你信任的服务。")}</div>
        <div>
          {!existing && (
            <div className="field">
              <label>{t("名称")}</label>
              <input className="input" placeholder={t("留空则使用订阅提供的名称")} value={name} onChange={(e) => setName(e.target.value)} />
            </div>
          )}
          <div className="field">
            <label>{t("订阅地址")}</label>
            <div className="small mono import-url">{cur.url}</div>
          </div>
        </div>
        {existing && <div className="notice" style={{ marginBottom: 0 }}>{t("已有相同地址的订阅「{name}」，确认后会重新下载并更新它。", { name: existing.name })}</div>}
        <div className="row">
          {queue.length > 1 && <span className="small muted">{t("还有 {n} 个链接待确认", { n: queue.length - 1 })}</span>}
          <span className="spacer" />
          <button className="btn" disabled={busy} onClick={next}>
            {t("取消")}
          </button>
          <button className="btn primary" disabled={busy} onClick={confirm}>
            {busy ? t("下载中…") : existing ? t("更新") : t("导入")}
          </button>
        </div>
      </div>
    </div>
  );
}
