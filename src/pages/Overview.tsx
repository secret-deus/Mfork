import { useEffect, useMemo, useState } from "react";
import { useApp } from "../store";
import { apiGet, formatBytes, formatRate } from "../lib/core";
import { useT } from "../i18n";
import Globe, { GlobeNode } from "../components/Globe";
import { locateNode, locateSelf } from "../lib/geo";
import { ProxyGroup, ProxyNode } from "../lib/types";

const GROUP_TYPES = new Set(["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"]);
const BUILTIN = new Set(["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE", "GLOBAL"]);

/** Reachable nodes (last delay test succeeded) clustered by location, plus
 *  the node currently in use, resolved through the selector chain. */
function useGlobeData() {
  const { api, phase, mode } = useApp();
  const [data, setData] = useState<{ nodes: GlobeNode[]; current: GlobeNode | null }>({
    nodes: [],
    current: null,
  });

  useEffect(() => {
    if (phase !== "running" || !api) {
      setData({ nodes: [], current: null });
      return;
    }
    let stop = false;
    const tick = async () => {
      try {
        const p = await apiGet<{ proxies: Record<string, ProxyNode & { all?: string[]; now?: string }> }>(
          api,
          "/proxies",
        );
        const g = await apiGet<{ proxies: ProxyGroup[] }>(api, "/group");
        if (stop) return;
        const all = p.proxies;
        const clusters = new Map<string, GlobeNode>();
        for (const n of Object.values(all)) {
          if (n.all || GROUP_TYPES.has(n.type) || BUILTIN.has(n.name.toUpperCase())) continue;
          const h = n.history;
          if (!h || h.length === 0 || h[h.length - 1].delay <= 0) continue;
          const pos = locateNode(n.name);
          if (!pos) continue;
          const key = pos.join(",");
          const c = clusters.get(key);
          if (c) c.count += 1;
          else clusters.set(key, { name: n.name, pos, count: 1 });
        }
        // current node: GLOBAL's pick in global mode, else the first group's
        let current: GlobeNode | null = null;
        if (mode !== "direct") {
          let name: string | undefined =
            mode === "global"
              ? all["GLOBAL"]?.now
              : g.proxies.find((x) => x.name !== "GLOBAL")?.now;
          const seen = new Set<string>();
          while (name && all[name]?.now && !seen.has(name)) {
            seen.add(name);
            name = all[name].now;
          }
          if (name && !BUILTIN.has(name.toUpperCase())) {
            const pos = locateNode(name);
            if (pos) current = { name, pos, count: 1 };
          }
        }
        setData({ nodes: [...clusters.values()], current });
      } catch {
        /* core restarting: keep the last frame */
      }
    };
    void tick();
    const timer = setInterval(() => void tick(), 8000);
    return () => {
      stop = true;
      clearInterval(timer);
    };
  }, [api, phase, mode]);

  return data;
}

function Uptime({ since }: { since: number }) {
  const t = useT();
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, []);
  const total = Math.max(0, Math.floor((Date.now() - since) / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return (
    <span>
      {h > 0
        ? t("已运行 {h} 小时 {m} 分 {s} 秒", { h, m, s })
        : t("已运行 {m} 分 {s} 秒", { m, s })}
    </span>
  );
}

const MODES = [
  // context-keyed: "规则" alone collides with the Rules nav entry
  { key: "rule", label: "mode.rule" },
  { key: "global", label: "mode.global" },
  { key: "direct", label: "mode.direct" },
] as const;

export default function Overview() {
  const t = useT();
  const {
    phase,
    api,
    error,
    version,
    mode,
    up,
    down,
    memory,
    connections,
    start,
    stop,
    setMode,
    settings,
    setSystemProxy,
  } = useApp();

  const dl = formatRate(down);
  const ul = formatRate(up);
  const globe = useGlobeData();
  const self = useMemo(locateSelf, []);

  return (
    <div className="relative flex h-full flex-col overflow-hidden px-14 pb-10 pt-13">
      {/* globe bleeds off the right edge; content stays on top */}
      <div
        className="pointer-events-none absolute top-1/2 -right-[14vh] h-[96vh] w-[96vh] -translate-y-1/2 select-none"
        aria-hidden
      >
        <Globe
          self={self}
          nodes={globe.nodes}
          current={globe.current}
          active={phase === "running"}
        />
      </div>
      <div className="relative z-10 flex items-center justify-between">
        <div className="cap">{t("Mfork · 概览 Overview")}</div>
        <div className="cap2">
          {phase === "running" && api ? <Uptime since={api.started_at_ms} /> : null}
        </div>
      </div>

      <div className="relative z-10 mt-8 flex items-baseline gap-3.5">
        <span className="font-sg text-[100px] leading-[0.9] font-light tracking-[-2px]">
          {phase === "running" ? dl.value : "0"}
        </span>
        <div className="flex flex-col gap-0.5">
          <span className="text-[13px] text-mut">{dl.unit}</span>
          <span className="cap2 !text-ok tracking-[2px]">{t("↓ 下载")}</span>
        </div>
      </div>
      <div className="relative z-10 mt-4 flex items-baseline gap-3">
        <span className="font-sg text-[38px] leading-none font-light text-fg-3">
          {phase === "running" ? ul.value : "0"}
        </span>
        <span className="text-[11px] text-dim">{ul.unit}</span>
        <span className="cap2 !text-accent tracking-[2px]">{t("↑ 上传")}</span>
      </div>

      <div className="relative z-10 mt-9 h-px w-[470px] bg-line" />
      <div className="relative z-10 flex w-[470px]">
        <div className="flex flex-1 flex-col gap-1.5 py-4">
          <span className="cap">{t("活动连接")}</span>
          <span className="font-sg text-2xl">
            {phase === "running" ? connections : "—"}
          </span>
        </div>
        <div className="w-px bg-line" />
        <div className="flex flex-1 flex-col gap-1.5 py-4 pl-6">
          <span className="cap">{t("内核内存")}</span>
          <span className="font-sg text-2xl">
            {phase === "running" ? formatBytes(memory) : "—"}
          </span>
        </div>
        <div className="w-px bg-line" />
        <div className="flex flex-1 flex-col gap-1.5 py-4 pl-6">
          <span className="cap">{t("内核版本")}</span>
          <span className="font-sg text-2xl">
            {phase === "running" && version ? version.replace(/^v/, "") : "—"}
          </span>
        </div>
      </div>
      <div className="relative z-10 h-px w-[470px] bg-line" />

      <div className="relative z-10 mt-6 flex items-center gap-6">
        {MODES.map((m) => (
          <button
            key={m.key}
            onClick={() => setMode(m.key)}
            disabled={phase !== "running"}
            className={`border-b-2 pb-1.5 text-[13px] transition-colors ${
              mode === m.key
                ? "border-accent text-fg"
                : "border-transparent text-dim hover:text-mut"
            } disabled:opacity-40`}
          >
            {t(m.label)}
          </button>
        ))}
        <div className="h-4 w-px bg-line" />
        <button
          onClick={() => void setSystemProxy(!(settings?.system_proxy ?? false))}
          disabled={!settings}
          className="flex items-center gap-2 pb-1.5 text-[13px] text-dim transition-colors hover:text-mut disabled:opacity-40"
        >
          <span
            className={`block h-1.5 w-1.5 rounded-full ${
              settings?.system_proxy ? "bg-ok" : "bg-faint"
            }`}
          />
          <span className={settings?.system_proxy ? "text-fg" : undefined}>{t("系统代理")}</span>
        </button>
        <div className="h-4 w-px bg-line" />
        {phase === "running" ? (
          <button onClick={stop} className="text-xs text-mut hover:text-fg">
            {t("停止内核")}
          </button>
        ) : (
          <button
            onClick={start}
            disabled={phase === "starting"}
            className="text-xs text-accent hover:text-fg disabled:opacity-50"
          >
            {phase === "starting" ? t("启动中…") : t("启动内核")}
          </button>
        )}
      </div>

      <div className="flex-1" />

      <div className="relative z-10 flex w-[470px] flex-col gap-3">
        {phase === "running" ? (
          <div className="cap2">
            {globe.current
              ? t("当前节点 · {name}", { name: globe.current.name })
              : mode === "direct"
                ? t("直连模式 · 不经过节点")
                : t("当前节点位置未知")}
            {globe.nodes.length > 0
              ? ` · ${t("{n} 处可用节点", { n: globe.nodes.length })}`
              : ""}
          </div>
        ) : null}
        {error ? (
          <div className="text-xs text-err select-text">{error}</div>
        ) : null}
        <div className="flex items-center gap-2.5">
          <span
            className={`block h-2 w-2 rounded-full ${
              phase === "running"
                ? "bg-ok"
                : phase === "starting"
                  ? "bg-warn animate-pulse"
                  : phase === "error"
                    ? "bg-err"
                    : "bg-dim"
            }`}
          />
          <span className="text-[13px]">
            {phase === "running"
              ? t("mihomo {v} · 运行中", { v: version ?? "" })
              : phase === "starting"
                ? t("内核启动中")
                : phase === "error"
                  ? t("内核异常")
                  : t("内核已停止")}
          </span>
          {phase === "running" && api ? (
            <span className="cap2">
              {t("API 127.0.0.1:{port} · 混合端口 {mixed}", {
                port: api.port,
                mixed: api.mixed_port,
              })}
            </span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
