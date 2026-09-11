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
    <div className="pointer-events-none absolute inset-x-0 bottom-[calc(80px+env(safe-area-inset-bottom))] z-40 flex justify-center px-4 md:bottom-6" role="status" aria-live="polite">
      <div
        onClick={dismiss}
        className={`pointer-events-auto w-full min-w-0 max-w-lg max-h-[35dvh] overflow-y-auto [overflow-wrap:anywhere] cursor-pointer select-text rounded-[14px] border border-line-soft bg-surface px-4 py-3 text-[14px] leading-relaxed shadow-md ${
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

/** Phone bottom bar: four primary destinations, the rest behind "more". */
const PRIMARY: PageKey[] = ["overview", "proxies", "profiles", "settings"];

const IconMore = ({ size = 24 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
    <circle cx="5" cy="12" r="2" />
    <circle cx="12" cy="12" r="2" />
    <circle cx="19" cy="12" r="2" />
  </svg>
);

export default function App() {
  const t = useT();
  const [page, setPage] = useState<PageKey>("overview");
  const [moreOpen, setMoreOpen] = useState(false);
  const { phase, init } = useApp();
  const primary = NAV.filter((n) => PRIMARY.includes(n.key));
  const secondary = NAV.filter((n) => !PRIMARY.includes(n.key));
  const go = (key: PageKey) => {
    setPage(key);
    setMoreOpen(false);
  };

  useEffect(() => {
    void init();
    // init subscribes to core-exited and auto-starts the core; run once
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="app-shell relative flex h-full min-w-0 flex-col-reverse md:flex-row">
      {/* frameless window: the top strip (and the sidebar background) drags the window */}
      {IS_DESKTOP ? <div data-tauri-drag-region className="absolute inset-x-0 top-0 z-20 h-8" /> : null}
      {IS_DESKTOP && !IS_MAC ? (
        <div className="absolute top-0 right-0 z-30">
          <WindowControls />
        </div>
      ) : null}
      {/* desktop: icon sidebar with every page */}
      <aside
        data-tauri-drag-region
        className={`z-10 hidden h-full w-16 shrink-0 flex-col items-center gap-1 border-r border-line-soft pb-3.5 md:flex ${IS_MAC ? "pt-9" : "pt-3.5"}`}
      >
        <div className="mb-4 flex h-[30px] w-[30px] items-center justify-center rounded-[9px] bg-logo-bg text-logo-fg">
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
        <div className="flex-1" />
        <span
          className={`block h-[7px] w-[7px] rounded-full ${
            phase === "running" ? "bg-ok" : phase === "error" ? "bg-err" : "bg-dim"
          }`}
          title={phase === "running" ? t("内核运行中") : t("内核未运行")}
        />
      </aside>

      {/* phone: four primary tabs + "more" */}
      <nav className="z-30 flex w-full shrink-0 items-stretch justify-around border-t border-line-soft bg-bg px-1 pt-1.5 pb-[max(0.5rem,env(safe-area-inset-bottom))] md:hidden">
        {primary.map((item) => {
          const active = page === item.key;
          return (
            <button
              key={item.key}
              onClick={() => go(item.key)}
              className={`flex min-w-[64px] flex-col items-center gap-1 rounded-[12px] px-2 py-1.5 transition-colors ${
                active ? "text-fg" : "text-dim"
              }`}
            >
              <item.icon size={24} />
              <span className={`text-[11px] leading-none ${active ? "text-fg" : "text-dim"}`}>{t(item.label)}</span>
            </button>
          );
        })}
        <button
          onClick={() => setMoreOpen((v) => !v)}
          className={`flex min-w-[64px] flex-col items-center gap-1 rounded-[12px] px-2 py-1.5 transition-colors ${
            moreOpen || secondary.some((n) => n.key === page) ? "text-fg" : "text-dim"
          }`}
        >
          <IconMore />
          <span className="text-[11px] leading-none">{t("更多")}</span>
        </button>
      </nav>
      {moreOpen ? (
        <>
          <div className="absolute inset-0 z-20 md:hidden" onClick={() => setMoreOpen(false)} />
          <div className="absolute inset-x-3 bottom-[calc(72px+env(safe-area-inset-bottom))] z-30 grid grid-cols-4 gap-1 rounded-[16px] border border-line-soft bg-surface p-2 shadow-md md:hidden">
            {secondary.map((item) => {
              const active = page === item.key;
              return (
                <button
                  key={item.key}
                  onClick={() => go(item.key)}
                  className={`flex flex-col items-center gap-1.5 rounded-[12px] px-2 py-3 ${
                    active ? "bg-nav-active text-fg" : "text-dim"
                  }`}
                >
                  <item.icon size={24} />
                  <span className="text-[11px] leading-none">{t(item.label)}</span>
                </button>
              );
            })}
          </div>
        </>
      ) : null}
      <main className="min-h-0 min-w-0 flex-1 overflow-x-hidden">
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
