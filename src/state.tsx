import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errText, type Snapshot, type UpdateInfo } from "./api";
import { t } from "./i18n";

interface Toast {
  id: number;
  text: string;
  kind: "ok" | "err";
}

interface Ctx {
  snap: Snapshot | null;
  refresh: () => Promise<void>;
  toast: (text: string, kind?: "ok" | "err") => void;
  /** Runs an action, toasting its error; returns false on failure. */
  run: (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;
  /** A newer Prism release, once a check found one. */
  update: UpdateInfo | null;
  setUpdate: (u: UpdateInfo | null) => void;
}

const AppCtx = createContext<Ctx>(null as unknown as Ctx);
export const useApp = () => useContext(AppCtx);

export function AppProvider({ children }: { children: ReactNode }) {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    try {
      setSnap(await api.state());
    } catch (e) {
      console.error(e);
    }
  }, []);

  const toast = useCallback((text: string, kind: "ok" | "err" = "ok") => {
    const id = ++seq.current;
    setToasts((t) => [...t, { id, text, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "err" ? 6000 : 2500);
  }, []);

  const mac = snap?.platform === "macos";

  /** Installs / refreshes the TUN service (UAC / administrator password prompt). */
  const installTunService = useCallback(async () => {
    toast(mac ? t("请在弹出的窗口中输入 Mac 的登录密码…") : t("请在弹出的 Windows 窗口中允许管理员授权…"));
    try {
      await api.installTunService();
      toast("TUN 服务已就绪");
      return true;
    } catch (e) {
      toast(errText(e), "err");
      return false;
    }
  }, [toast, mac]);

  const run = useCallback(
    async (fn: () => Promise<unknown>, ok?: string) => {
      // TUN errors can be fixed by (re)installing the service; then retry once.
      const attempt = async (retried: boolean): Promise<boolean> => {
        try {
          await fn();
          if (ok) toast(ok);
          return true;
        } catch (e) {
          const msg = errText(e);
          if (msg.startsWith("NEED_ADMIN") && !retried && mac) {
            // macOS: TUN only runs through the service.
            if (confirm(t("TUN 模式需要先安装「TUN 服务」（只需输入一次 Mac 的登录密码，以后开启 TUN 不再需要）。现在安装？"))) {
              if (await installTunService()) return attempt(true);
            }
          } else if (msg.startsWith("NEED_ADMIN") && !retried) {
            if (
              confirm(
                t("TUN 模式需要管理员权限。\n\n推荐安装「TUN 服务」：只需授权一次，以后开启 TUN 无需以管理员身份运行 Prism。\n\n确定：安装 TUN 服务\n取消：查看其他选择"),
              )
            ) {
              if (await installTunService()) return attempt(true);
            } else if (confirm(t("改为以管理员身份重新启动 Prism？"))) {
              api.restartAsAdmin().catch((err) => toast(errText(err), "err"));
            }
          } else if (msg.startsWith("NEED_SERVICE_UPDATE") && !retried) {
            if (confirm(t("TUN 服务中的内核与当前版本不一致（Prism 或内核更新过）。\n需要管理员授权一次来更新 TUN 服务，现在更新？"))) {
              if (await installTunService()) return attempt(true);
            }
          } else {
            toast(msg.replace(/^NEED_\w+: /, ""), "err");
          }
          return false;
        }
      };
      try {
        return await attempt(false);
      } finally {
        refresh();
      }
    },
    [toast, refresh, installTunService, mac],
  );

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    const un = listen("core-changed", () => refresh());
    const unUpdate = listen<UpdateInfo>("update-available", (e) => setUpdate(e.payload));
    return () => {
      clearInterval(t);
      un.then((f) => f());
      unUpdate.then((f) => f());
    };
  }, [refresh]);

  return (
    <AppCtx.Provider value={{ snap, refresh, toast, run, update, setUpdate }}>
      {children}
      <div className="toasts">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </AppCtx.Provider>
  );
}

/** Polls `fn` every `ms` while `enabled`. */
export function usePoll(fn: () => void | Promise<void>, ms: number, enabled = true) {
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const tick = async () => {
      if (!alive) return;
      try {
        await ref.current();
      } catch {
        /* core may be restarting */
      }
    };
    tick();
    const t = setInterval(tick, ms);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [ms, enabled]);
}
