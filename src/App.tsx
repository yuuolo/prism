import { useState } from "react";
import { AppProvider, useApp } from "./state";
import { CORE_NAMES } from "./api";
import Home from "./pages/Home";
import Proxies from "./pages/Proxies";
import Profiles from "./pages/Profiles";
import Connections from "./pages/Connections";
import Rules from "./pages/Rules";
import Logs from "./pages/Logs";
import SettingsPage from "./pages/Settings";
import Apps from "./pages/Apps";

const PAGES = [
  { key: "home", label: "概览", icon: "◎", el: Home },
  { key: "proxies", label: "代理", icon: "⇄", el: Proxies },
  { key: "profiles", label: "订阅", icon: "☰", el: Profiles },
  { key: "connections", label: "连接", icon: "⌁", el: Connections },
  { key: "rules", label: "规则", icon: "≡", el: Rules },
  { key: "apps", label: "应用分流", icon: "⊞", el: Apps },
  { key: "logs", label: "日志", icon: "▤", el: Logs },
  { key: "settings", label: "设置", icon: "⚙", el: SettingsPage },
] as const;

export type PageKey = (typeof PAGES)[number]["key"];

function Shell() {
  // `#apps` etc. opens a page directly.
  const [page, setPage] = useState<PageKey>(() => {
    const h = window.location.hash.slice(1);
    return (PAGES.find((p) => p.key === h)?.key ?? "home") as PageKey;
  });
  const { snap, update } = useApp();
  const Page = PAGES.find((p) => p.key === page)!.el;
  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="logo" />
          <span>Prism</span>
        </div>
        <nav>
          {PAGES.map((p) => (
            <button key={p.key} className={`nav ${page === p.key ? "active" : ""}`} onClick={() => setPage(p.key)}>
              <span className="nav-icon">{p.icon}</span>
              {p.label}
            </button>
          ))}
        </nav>
        {update && (
          <button className="update-badge" onClick={() => setPage("settings")} title="去设置页安装">
            发现新版本 v{update.version}
          </button>
        )}
        <div className="side-status">
          <span className={`dot ${snap?.running ? "on" : ""}`} />
          {snap?.running ? `${CORE_NAMES[snap.running]} 运行中` : "内核未运行"}
        </div>
      </aside>
      <main className="main">
        <Page go={setPage} />
      </main>
    </div>
  );
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
