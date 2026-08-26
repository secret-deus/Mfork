import { useEffect, useState } from "react";
import { useApp } from "./store";
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

function Placeholder({ label }: { label: string }) {
  return (
    <div className="flex h-full flex-col px-14 pt-13">
      <div className="cap">Mfork · {label}</div>
      <div className="flex flex-1 items-center justify-center">
        <span className="cap">按里程碑推进 · 建设中</span>
      </div>
    </div>
  );
}

export default function App() {
  const [page, setPage] = useState<PageKey>("overview");
  const { phase, init } = useApp();

  useEffect(() => {
    void init();
    // init subscribes to core-exited and auto-starts the core; run once
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-full">
      <aside className="z-10 flex w-16 shrink-0 flex-col items-center gap-1 border-r border-[#16181d] py-3.5">
        <div className="mb-4 flex h-[30px] w-[30px] items-center justify-center rounded-[9px] bg-[#f2f3f5] text-[#101114]">
          <ForkLogo />
        </div>
        {NAV.map((item) => (
          <button
            key={item.key}
            title={item.label}
            onClick={() => setPage(item.key)}
            className={`flex h-[38px] w-[38px] items-center justify-center rounded-[10px] transition-colors ${
              page === item.key
                ? "bg-white/6 text-[#f0f1f3]"
                : "text-[#54585f] hover:text-mut"
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
          title={phase === "running" ? "内核运行中" : "内核未运行"}
        />
      </aside>
      <main className="min-w-0 flex-1">
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
        ) : page === "settings" ? (
          <Settings />
        ) : (
          <Placeholder label={NAV.find((n) => n.key === page)?.label ?? ""} />
        )}
      </main>
    </div>
  );
}
