import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errText, type Snapshot } from "./api";

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
}

const AppCtx = createContext<Ctx>(null as unknown as Ctx);
export const useApp = () => useContext(AppCtx);

export function AppProvider({ children }: { children: ReactNode }) {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
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

  const run = useCallback(
    async (fn: () => Promise<unknown>, ok?: string) => {
      try {
        await fn();
        if (ok) toast(ok);
        return true;
      } catch (e) {
        const msg = errText(e);
        if (msg.startsWith("NEED_ADMIN")) {
          if (confirm("TUN 模式需要管理员权限。\n是否以管理员身份重新启动 Prism？")) {
            api.restartAsAdmin().catch((err) => toast(errText(err), "err"));
          }
        } else {
          toast(msg, "err");
        }
        return false;
      } finally {
        refresh();
      }
    },
    [toast, refresh],
  );

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    const un = listen("core-changed", () => refresh());
    return () => {
      clearInterval(t);
      un.then((f) => f());
    };
  }, [refresh]);

  return (
    <AppCtx.Provider value={{ snap, refresh, toast, run }}>
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
