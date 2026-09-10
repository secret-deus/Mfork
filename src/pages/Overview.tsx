import { useEffect, useMemo, useState } from "react";
import { useApp } from "../store";
import { apiGet, formatBytes, formatRate, serviceStatus, type AppSettings, type ServiceStatus } from "../lib/core";
import { useT } from "../i18n";
import Globe, { GlobeNode, GlobeRoute } from "../components/Globe";
import { locateNode, locateNodeDetail, locateSelfDetail } from "../lib/geo";
import { ProxyGroup, ProxyNode } from "../lib/types";

const GROUP_TYPES = new Set(["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"]);
const BUILTIN = new Set(["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE", "GLOBAL"]);

export interface Hop {
  name: string;
  pos: [number, number];
  region: string;
}

interface GlobeData {
  nodes: GlobeNode[];
  /** hops of the selected route in travel order (chains expanded) */
  currentHops: Hop[];
  /** the selected route, hop names joined for display */
  currentName: string | null;
  /** every real outbound node → its full hop chain (dialer-proxy legs first,
   *  the node itself last); nodes without a location still get an entry */
  chains: Map<string, Hop[]>;
  /** nodes that only serve as someone's dialer-proxy leg */
  innerLegs: Set<string>;
}

const EMPTY: GlobeData = { nodes: [], currentHops: [], currentName: null, chains: new Map(), innerLegs: new Set() };

/** Reachable nodes (last delay test succeeded) clustered by location, plus
 *  the route currently selected, resolved through the selector chain and
 *  expanded into its full hop chain. mihomo chains outbounds with
 *  `dialer-proxy` (a node dials through another node or group); legacy relay
 *  groups are expanded member by member. */
function useGlobeData(): GlobeData {
  const { api, phase, mode } = useApp();
  const [data, setData] = useState<GlobeData>(EMPTY);

  useEffect(() => {
    if (phase !== "running" || !api) {
      setData(EMPTY);
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
        const isNode = (n: string) =>
          !!all[n] && !all[n].all && !GROUP_TYPES.has(all[n].type) && !BUILTIN.has(n.toUpperCase());
        // group → the node it currently resolves to
        const resolve = (start: string | undefined): string | undefined => {
          const seen = new Set<string>();
          let x = start;
          while (x && all[x]?.now && !seen.has(x)) {
            seen.add(x);
            x = all[x].now;
          }
          return x;
        };
        // full chain in travel order: dialer-proxy legs first, exit last
        const chainNames = (start: string): string[] => {
          const out: string[] = [];
          const seen = new Set<string>();
          let x: string | undefined = start;
          while (x && !seen.has(x)) {
            seen.add(x);
            if (all[x]?.type === "Relay") {
              out.unshift(...(all[x].all ?? []));
              break;
            }
            out.unshift(x);
            x = resolve(all[x]?.["dialer-proxy"]);
          }
          return out.filter((n) => !BUILTIN.has(n.toUpperCase()));
        };
        const toHops = (names: string[]): Hop[] =>
          names
            .map((n) => {
              const d = locateNodeDetail(n);
              return d ? { name: n, pos: d.pos, region: d.region } : null;
            })
            .filter((x): x is Hop => x !== null);
        const clusters = new Map<string, GlobeNode>();
        const chains = new Map<string, Hop[]>();
        const innerLegs = new Set<string>();
        for (const n of Object.values(all)) {
          if (!isNode(n.name)) continue;
          const names = chainNames(n.name);
          chains.set(n.name, toHops(names));
          for (const leg of names.slice(0, -1)) innerLegs.add(leg);
          const h = n.history;
          if (!h || h.length === 0 || h[h.length - 1].delay <= 0) continue;
          const pos = locateNode(n.name);
          if (!pos) continue;
          const key = pos.join(",");
          const c = clusters.get(key);
          if (c) c.count += 1;
          else clusters.set(key, { name: n.name, pos, count: 1 });
        }
        // selected route: GLOBAL's pick in global mode, else the group's
        let currentHops: Hop[] = [];
        let currentName: string | null = null;
        if (mode !== "direct") {
          // rule mode: the group most live connections pass through, else
          // the first Selector, else the first group
          const groups = g.proxies.filter((x) => x.name !== "GLOBAL");
          const votes = new Map<string, number>();
          for (const c of useApp.getState().snapshot?.connections ?? []) {
            const grp = c.chains[1];
            if (grp && groups.some((x) => x.name === grp)) votes.set(grp, (votes.get(grp) ?? 0) + 1);
          }
          const voted = [...votes.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
          const pick =
            groups.find((x) => x.name === voted) ??
            groups.find((x) => x.type === "Selector" && !BUILTIN.has((x.now ?? "").toUpperCase())) ??
            groups.find((x) => x.type === "Selector") ??
            groups[0];
          let name: string | undefined = mode === "global" ? all["GLOBAL"]?.now : pick?.now;
          const seen = new Set<string>();
          while (name && all[name]?.now && !seen.has(name)) {
            seen.add(name);
            name = all[name].now;
          }
          if (name && !BUILTIN.has(name.toUpperCase())) {
            const names = chainNames(name);
            currentName = names.length ? names.join(" → ") : name;
            currentHops = toHops(names);
          }
        }
        setData({ nodes: [...clusters.values()], currentHops, currentName, chains, innerLegs });
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
    updateSettings,
    restart,
  } = useApp();

  const dl = formatRate(down);
  const ul = formatRate(up);
  const globe = useGlobeData();
  const self = useMemo(locateSelfDetail, []);
  const snapshot = useApp((s) => s.snapshot);

  // live routes: connections grouped by their exit node, expanded into the
  // full hop chain. mihomo lists a connection's chain exit-first, and the
  // inner leg of a dialer-proxy chain shows up as its own rule-less
  // connection on the dialer node, so those are skipped rather than counted
  // as a second route.
  const live = useMemo(() => {
    const m = new Map<string, { hops: Hop[]; conns: number }>();
    for (const c of snapshot?.connections ?? []) {
      const exit = c.chains[0];
      if (!exit) continue;
      if (!c.rule && globe.innerLegs.has(exit)) continue;
      if (BUILTIN.has(exit.toUpperCase())) continue;
      let hops = globe.chains.get(exit);
      if (!hops) {
        const d = locateNodeDetail(exit);
        hops = d ? [{ name: exit, pos: d.pos, region: d.region }] : [];
      }
      if (hops.length === 0) continue;
      const key = hops.map((h) => h.name).join(" » ");
      const e = m.get(key);
      if (e) e.conns += 1;
      else m.set(key, { hops, conns: 1 });
    }
    const routes = [...m.entries()]
      .map(([key, v]) => ({ key, ...v }))
      .sort((a, b) => b.conns - a.conns);
    return { routes };
  }, [snapshot, globe.chains, globe.innerLegs]);

  const currentKey = globe.currentHops.map((h) => h.name).join(" » ");
  const currentLiveRoute = live.routes.find((r) => r.key === currentKey);
  const routes: GlobeRoute[] = useMemo(() => {
    const out: GlobeRoute[] = [];
    if (globe.currentHops.length)
      out.push({ key: currentKey, hops: globe.currentHops, conns: currentLiveRoute?.conns ?? 0, tone: "accent" });
    for (const r of live.routes) if (r.key !== currentKey) out.push({ key: r.key, hops: r.hops, conns: r.conns, tone: "ok" });
    return out;
  }, [globe.currentHops, currentKey, currentLiveRoute, live.routes]);

  // TUN needs the elevated service; the switch mirrors the one in Settings
  const [svc, setSvc] = useState<ServiceStatus | null>(null);
  useEffect(() => {
    void serviceStatus().then(setSvc, () => setSvc(null));
  }, [phase]);
  const tun = settings?.tun;
  const tunOn = tun?.enable ?? false;
  const tunAvailable = !!tun && (svc?.reachable || tunOn);
  const toggleTun = async () => {
    if (!tun) return;
    await updateSettings({ tun: { ...tun, enable: !tunOn } } as Partial<AppSettings>);
    if (phase === "running") await restart();
  };

  return (
    <div className="relative flex h-full flex-col overflow-hidden px-5 md:px-14 pb-10 pt-6 md:pt-13">
      {/* globe on the right; drag to rotate, it drifts back home when released */}
      <div className="absolute top-1/2 right-[6vh] h-[72vh] w-[72vh] -translate-y-1/2 select-none">
        <Globe self={self.pos} nodes={globe.nodes} routes={routes} active={phase === "running"} />
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
          <span className="text-[14px] font-semibold text-accent">{dl.unit}</span>
          <span className="cap2 !text-ok tracking-[2px]">{t("↓ 下载")}</span>
        </div>
      </div>
      <div className="relative z-10 mt-4 flex items-baseline gap-3">
        <span className="font-sg text-[38px] leading-none font-light text-fg-3">
          {phase === "running" ? ul.value : "0"}
        </span>
        <span className="text-[12px] font-semibold text-accent">{ul.unit}</span>
        <span className="cap2 !text-accent tracking-[2px]">{t("↑ 上传")}</span>
      </div>

      <div className="relative z-10 mt-9 h-px w-[470px] bg-line" />
      <div className="relative z-10 flex w-[470px]">
        <div className="flex flex-1 flex-col gap-1.5 py-4">
          <span className="cap">{t("活动连接")}</span>
          <span className="font-sg text-[26px]">
            {phase === "running" ? connections : "—"}
          </span>
        </div>
        <div className="w-px bg-line" />
        <div className="flex flex-1 flex-col gap-1.5 py-4 pl-6">
          <span className="cap">{t("内核内存")}</span>
          <span className="font-sg text-[26px]">
            {phase === "running" ? formatBytes(memory) : "—"}
          </span>
        </div>
        <div className="w-px bg-line" />
        <div className="flex flex-1 flex-col gap-1.5 py-4 pl-6">
          <span className="cap">{t("内核版本")}</span>
          <span className="font-sg text-[26px]">
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
            className={`border-b-2 pb-1.5 text-[14px] transition-colors ${
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
          className="flex items-center gap-2 pb-1.5 text-[14px] text-dim transition-colors hover:text-mut disabled:opacity-40"
        >
          <span
            className={`block h-1.5 w-1.5 rounded-full ${
              settings?.system_proxy ? "bg-ok" : "bg-faint"
            }`}
          />
          <span className={settings?.system_proxy ? "text-fg" : undefined}>{t("系统代理")}</span>
        </button>
        <button
          onClick={() => void toggleTun()}
          disabled={!tunAvailable}
          title={tunAvailable ? (tunOn ? t("关闭 TUN") : t("启用 TUN")) : t("需先安装系统服务")}
          className="flex items-center gap-2 pb-1.5 text-[14px] text-dim transition-colors hover:text-mut disabled:opacity-40"
        >
          <span className={`block h-1.5 w-1.5 rounded-full ${tunOn ? "bg-ok" : "bg-faint"}`} />
          <span className={tunOn ? "text-fg" : undefined}>TUN</span>
        </button>
        <div className="h-4 w-px bg-line" />
        {phase === "running" ? (
          <button onClick={stop} className="text-[13px] text-mut hover:text-fg">
            {t("停止内核")}
          </button>
        ) : (
          <button
            onClick={start}
            disabled={phase === "starting"}
            className="text-[13px] text-accent hover:text-fg disabled:opacity-50"
          >
            {phase === "starting" ? t("启动中…") : t("启动内核")}
          </button>
        )}
      </div>

      <div className="flex-1" />

      <div className="relative z-10 flex w-[470px] flex-col gap-3">
        {phase === "running" ? (
          <div className="flex flex-col gap-1.5">
            <div className="cap2">
              {globe.currentName
                ? t("当前节点 · {name}", { name: globe.currentName })
                : mode === "direct"
                  ? t("直连模式 · 不经过节点")
                  : t("当前节点位置未知")}
              {currentLiveRoute ? ` · ${t("{n} 条连接", { n: currentLiveRoute.conns })}` : ""}
              {globe.nodes.length > 0
                ? ` · ${t("{n} 处可用节点", { n: globe.nodes.length })}`
                : ""}
            </div>
          </div>
        ) : null}
        {error ? (
          <div className="text-[13px] text-err select-text">{error}</div>
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
          <span className="text-[14px]">
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
