import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useApp } from "./store";
import { useT } from "./i18n";
import { IS_DESKTOP, IS_MAC } from "./lib/platform";
import Overview from "./pages/Overview";
import Proxies from "./pages/Proxies";
import Profiles from "./pages/Profiles";
import Connections from "./pages/Connections";
import Overrides from "./pages/Overrides";
import Rules from "./pages/Rules";
import Logs from "./pages/Logs";
import Settings from "./pages/Settings";
import {
  ForkLogo,
  IconConnections,
  IconLogs,
  IconOverrides,
  IconOverview,
  IconProfiles,
  IconProxies,
  IconRules,
  IconSettings,
} from "./components/icons";

/** Global toast: operation failures from any page land here (store.notify). */
function Toast() {
  const notice = useApp((s) => s.notice);
  const dismiss = useApp((s) => s.dismissNotice);
  if (!notice) return null;
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-6 z-40 flex justify-center">
      <div
        onClick={dismiss}
        className={`pointer-events-auto max-w-[60%] cursor-pointer select-text rounded-[10px] border border-line-soft bg-surface px-4 py-2.5 text-[13px] leading-snug shadow-md ${
          notice.err ? "text-err" : "text-fg-2"
        }`}
      >
        {notice.text}
      </div>
    </div>
  );
}

/** Windows / Linux: the window is frameless, so draw our own controls. */
function WindowControls() {
  const win = getCurrentWindow();
  const btn = "flex h-7 w-9 items-center justify-center text-mut transition-colors hover:text-fg";
  return (
    <div className="flex">
      <button className={btn} onClick={() => void win.minimize()} title="—">
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2"><path d="M1 5h8" /></svg>
      </button>
      <button className={btn} onClick={() => void win.toggleMaximize()}>
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2"><rect x="1.5" y="1.5" width="7" height="7" rx="1" /></svg>
      </button>
      <button className={`${btn} hover:text-err`} onClick={() => void win.close()}>
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" /></svg>
      </button>
    </div>
  );
}

const NAV = [
  { key: "overview", label: "概览", icon: IconOverview },
  { key: "proxies", label: "代理", icon: IconProxies },
  { key: "profiles", label: "订阅", icon: IconProfiles },
  { key: "overrides", label: "覆写", icon: IconOverrides },
  { key: "connections", label: "连接", icon: IconConnections },
  { key: "rules", label: "规则", icon: IconRules },
  { key: "logs", label: "日志", icon: IconLogs },
  { key: "settings", label: "设置", icon: IconSettings },
] as const;

type PageKey = (typeof NAV)[number]["key"];

export default function App() {
  const t = useT();
  const [page, setPage] = useState<PageKey>("overview");
  const { phase, init } = useApp();

  useEffect(() => {
    void init();
    // init subscribes to core-exited and auto-starts the core; run once
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="relative flex h-full flex-col-reverse md:flex-row">
      {/* frameless window: the top strip (and the sidebar background) drags the window */}
      {IS_DESKTOP ? <div data-tauri-drag-region className="absolute inset-x-0 top-0 z-20 h-8" /> : null}
      {IS_DESKTOP && !IS_MAC ? (
        <div className="absolute top-0 right-0 z-30">
          <WindowControls />
        </div>
      ) : null}
      {/* sidebar on desktop, bottom bar on phones */}
      <aside
        data-tauri-drag-region
        className={`z-10 flex w-full shrink-0 flex-row items-center justify-around border-t border-line-soft py-1.5 pb-[max(0.375rem,env(safe-area-inset-bottom))] md:h-full md:w-16 md:flex-col md:justify-start md:gap-1 md:border-t-0 md:border-r md:pb-3.5 ${IS_MAC ? "md:pt-9" : "md:pt-3.5"}`}
      >
        <div className="mb-4 hidden h-[30px] w-[30px] items-center justify-center rounded-[9px] bg-logo-bg text-logo-fg md:flex">
          <ForkLogo />
        </div>
        {NAV.map((item) => (
          <button
            key={item.key}
            title={t(item.label)}
            onClick={() => setPage(item.key)}
            className={`flex h-[38px] w-[38px] items-center justify-center rounded-[10px] transition-colors ${
              page === item.key
                ? "bg-nav-active text-fg"
                : "text-dim hover:text-mut"
            }`}
          >
            <item.icon />
          </button>
        ))}
        <div className="hidden flex-1 md:block" />
        <span
          className={`hidden h-[7px] w-[7px] rounded-full md:block ${
            phase === "running" ? "bg-ok" : phase === "error" ? "bg-err" : "bg-dim"
          }`}
          title={phase === "running" ? t("内核运行中") : t("内核未运行")}
        />
      </aside>
      <main className="min-h-0 min-w-0 flex-1">
        {page === "overview" ? (
          <Overview />
        ) : page === "proxies" ? (
          <Proxies />
        ) : page === "profiles" ? (
          <Profiles />
        ) : page === "overrides" ? (
          <Overrides />
        ) : page === "connections" ? (
          <Connections />
        ) : page === "rules" ? (
          <Rules />
        ) : page === "logs" ? (
          <Logs />
        ) : (
          <Settings />
        )}
      </main>
      <Toast />
    </div>
  );
}
