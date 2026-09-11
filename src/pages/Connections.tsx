import { Fragment, useMemo, useState } from "react";
import { useApp } from "../store";
import { apiDelete, formatBytes, formatDuration, formatRate } from "../lib/core";
import { ConnItem } from "../lib/types";
import { t, useT } from "../i18n";

type View = "socket" | "app";
type Net = "all" | "tcp" | "udp";
type SortKey = "start" | "host" | "process" | "downSpeed" | "upSpeed" | "download" | "upload";

function procName(c: ConnItem): string {
  if (c.metadata.process) return c.metadata.process;
  const p = c.metadata.processPath;
  if (p) return p.split(/[\\/]/).pop() ?? p;
  return t("未知进程");
}

function hostOf(c: ConnItem): string {
  const host = c.metadata.host || c.metadata.destinationIP;
  return `${host}:${c.metadata.destinationPort}`;
}

function endpoint(ip: string, port: string): string {
  if (!ip) return "—";
  return ip.includes(":") ? `[${ip}]:${port}` : `${ip}:${port}`;
}

function rate(n: number): string {
  if (n <= 0) return "—";
  const r = formatRate(n);
  return `${r.value} ${r.unit}`;
}

const thBase =
  "text-left whitespace-nowrap py-3 pr-4 text-[11px] font-semibold uppercase tracking-[1.5px] select-none";
const tdCls =
  "whitespace-nowrap overflow-hidden text-ellipsis py-3 pr-4 text-[13.5px] border-t border-line-soft";

/** "↓ a / ↑ b" pair for one cell; dims the arrow of an idle direction */
function Pair({ down, up, downOn, upOn }: { down: string; up: string; downOn: boolean; upOn: boolean }) {
  return (
    <span className="font-sg">
      <span className={downOn ? "text-ok" : "text-dim"}>↓ {down}</span>
      <span className="mx-1.5 text-faint">/</span>
      <span className={upOn ? "text-accent" : "text-dim"}>↑ {up}</span>
    </span>
  );
}

function Th({
  label,
  k,
  sort,
  onSort,
  width,
}: {
  label: string;
  k?: SortKey;
  sort: { key: SortKey; desc: boolean } | null;
  onSort: (k: SortKey) => void;
  width: string;
}) {
  const active = k && sort?.key === k;
  return (
    <th
      className={`${thBase} ${k ? "cursor-pointer hover:text-fg" : ""} ${active ? "text-accent" : "text-label"}`}
      style={{ width }}
      onClick={k ? () => onSort(k) : undefined}
    >
      {label}
      {active ? <span className="ml-1 text-[8px]">{sort.desc ? "▼" : "▲"}</span> : null}
    </th>
  );
}

export default function Connections() {
  const t = useT();
  const { api, phase, snapshot, notify } = useApp();
  const [view, setView] = useState<View>("socket");
  const [filter, setFilter] = useState("");
  const [net, setNet] = useState<Net>("all");
  const [activeOnly, setActiveOnly] = useState(false);
  const [chain, setChain] = useState<string>("all");
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean } | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [paused, setPaused] = useState(false);
  const [frozen, setFrozen] = useState<typeof snapshot>(null);

  const snap = paused ? frozen : snapshot;
  const all = snap?.connections ?? [];

  // outbound (last hop) names for the chain filter
  const chains = useMemo(() => {
    const s = new Set<string>();
    for (const c of all) if (c.chains.length) s.add(c.chains[0]);
    return [...s].sort();
  }, [all]);

  const items = useMemo(() => {
    let list = all;
    if (net !== "all") list = list.filter((c) => c.metadata.network.toLowerCase() === net);
    if (activeOnly) list = list.filter((c) => c.downSpeed > 0 || c.upSpeed > 0);
    if (chain !== "all") list = list.filter((c) => c.chains[0] === chain);
    if (filter) {
      const q = filter.toLowerCase();
      list = list.filter(
        (c) =>
          hostOf(c).toLowerCase().includes(q) ||
          c.metadata.sourceIP.includes(q) ||
          c.metadata.destinationIP.includes(q) ||
          c.metadata.sourcePort === q ||
          c.metadata.destinationPort === q ||
          c.rule.toLowerCase().includes(q) ||
          (c.rulePayload ?? "").toLowerCase().includes(q) ||
          procName(c).toLowerCase().includes(q) ||
          c.chains.join(" ").toLowerCase().includes(q),
      );
    }
    if (sort) {
      const dir = sort.desc ? -1 : 1;
      const key = sort.key;
      list = [...list].sort((a, b) => {
        let r = 0;
        if (key === "host") r = hostOf(a).localeCompare(hostOf(b));
        else if (key === "process") r = procName(a).localeCompare(procName(b));
        else if (key === "start") r = Date.parse(a.start) - Date.parse(b.start);
        else r = a[key] - b[key];
        return r * dir;
      });
    }
    return list;
  }, [all, net, activeOnly, chain, filter, sort]);

  const onSort = (k: SortKey) =>
    setSort((s) => (s?.key === k ? (s.desc ? { key: k, desc: false } : null) : { key: k, desc: true }));

  const apps = useMemo(() => {
    const m = new Map<
      string,
      { name: string; path: string; count: number; up: number; down: number; upSpeed: number; downSpeed: number }
    >();
    for (const c of items) {
      const name = procName(c);
      const e = m.get(name) ?? {
        name,
        path: c.metadata.processPath ?? "",
        count: 0,
        up: 0,
        down: 0,
        upSpeed: 0,
        downSpeed: 0,
      };
      e.count += 1;
      e.up += c.upload;
      e.down += c.download;
      e.upSpeed += c.upSpeed;
      e.downSpeed += c.downSpeed;
      m.set(name, e);
    }
    return [...m.values()].sort((a, b) => b.downSpeed - a.downSpeed || b.down - a.down);
  }, [items]);

  const fail = (e: unknown) => notify(t("关闭连接失败: {msg}", { msg: String(e) }));
  const closeOne = (id: string) => {
    if (api) void apiDelete(api, `/connections/${id}`).catch(fail);
  };
  const closeAll = () => {
    if (api) void apiDelete(api, "/connections").catch(fail);
  };

  const chip = (on: boolean) =>
    `text-[13px] transition-colors ${on ? "text-fg font-semibold" : "text-mut hover:text-fg"}`;

  return (
    <div className="flex h-full min-w-0 flex-col px-5 md:px-14 pb-6 pt-6 md:pt-13">
      <div className="flex flex-col items-start gap-3 md:flex-row md:items-center md:justify-between">
        <div className="flex items-baseline gap-5 whitespace-nowrap">
          <div className="cap">{t("Mfork · 连接 Connections")}</div>
          <span className="cap2">
            {t("活动")} <span className="font-sg text-[12px] text-fg">{all.length}</span>
          </span>
          <span className="cap2">
            {t("下载")} <span className="font-sg text-[12px] text-fg">{formatBytes(snap?.downloadTotal ?? 0)}</span>
          </span>
          <span className="cap2">
            {t("上传")} <span className="font-sg text-[12px] text-fg">{formatBytes(snap?.uploadTotal ?? 0)}</span>
          </span>
        </div>
        <div className="flex w-full items-center gap-6 overflow-x-auto whitespace-nowrap pb-1 [scrollbar-width:none] md:w-auto md:overflow-visible md:pb-0">
          <div className="flex shrink-0 gap-4">
            <button
              onClick={() => setView("socket")}
              className={`border-b-2 pb-1 text-[13px] ${view === "socket" ? "border-accent text-fg" : "border-transparent text-mut hover:text-fg"}`}
            >
              {t("连接视图")}
            </button>
            <button
              onClick={() => setView("app")}
              className={`border-b-2 pb-1 text-[13px] ${view === "app" ? "border-accent text-fg" : "border-transparent text-mut hover:text-fg"}`}
            >
              {t("应用视图")}
            </button>
          </div>
          <div className="h-4 w-px bg-line" />
          <button
            onClick={() => {
              if (!paused) setFrozen(snapshot);
              setPaused(!paused);
            }}
            className={`text-[13px] ${paused ? "text-warn" : "text-mut hover:text-fg"}`}
          >
            {paused ? t("继续") : t("暂停")}
          </button>
          <button onClick={closeAll} className="text-[13px] text-err/80 hover:text-err">
            {t("关闭全部")}
          </button>
        </div>
      </div>

      <div className="mt-7 flex items-center gap-4 border-b border-line-strong pb-3">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" className="text-mut">
          <circle cx="11" cy="11" r="7" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("搜索主机 / IP / 端口 / 进程 / 规则 / 节点")}
          className="flex-1 bg-transparent text-[14px] outline-none placeholder:text-dim"
        />
        <div className="flex items-center gap-3">
          {(["all", "tcp", "udp"] as Net[]).map((n) => (
            <button key={n} onClick={() => setNet(n)} className={chip(net === n)}>
              {n === "all" ? t("全部") : n.toUpperCase()}
            </button>
          ))}
        </div>
        <div className="h-3.5 w-px bg-line" />
        <button onClick={() => setActiveOnly((v) => !v)} className={chip(activeOnly)}>
          {t("仅活跃")}
        </button>
        <div className="h-3.5 w-px bg-line" />
        <select
          value={chain}
          onChange={(e) => setChain(e.target.value)}
          className="bg-transparent text-[13px] text-mut outline-none hover:text-fg"
        >
          <option value="all">{t("全部出站")}</option>
          {chains.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
      </div>

      <div className="min-h-0 flex-1 overflow-x-auto overflow-y-auto">
        {phase !== "running" ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">{t("内核未运行")}</span>
          </div>
        ) : view === "socket" ? (
          <table className="w-full min-w-[960px] table-fixed border-collapse md:min-w-0">
            <thead>
              <tr>
                <Th label={t("主机")} k="host" sort={sort} onSort={onSort} width="19%" />
                <Th label={t("来源")} sort={sort} onSort={onSort} width="11%" />
                <Th label={t("目标")} sort={sort} onSort={onSort} width="12%" />
                <Th label={t("进程")} k="process" sort={sort} onSort={onSort} width="8%" />
                <Th label={t("规则")} sort={sort} onSort={onSort} width="11%" />
                <Th label={t("链路")} sort={sort} onSort={onSort} width="12%" />
                <Th label={t("速率")} k="downSpeed" sort={sort} onSort={onSort} width="10%" />
                <Th label={t("流量")} k="download" sort={sort} onSort={onSort} width="8%" />
                <Th label={t("时长")} k="start" sort={sort} onSort={onSort} width="6%" />
                <Th label="" sort={sort} onSort={onSort} width="3%" />
              </tr>
            </thead>
            <tbody>
              {items.map((c) => {
                const isOpen = open === c.id;
                return (
                  <Fragment key={c.id}>
                    <tr
                      className={`group cursor-pointer ${isOpen ? "bg-surface" : "hover:bg-surface/60"}`}
                      onClick={() => setOpen(isOpen ? null : c.id)}
                    >
                      <td className={`${tdCls} text-fg`}>
                        {hostOf(c)}
                        <span className="ml-2 text-[10.5px] font-semibold uppercase tracking-[1px] text-label">
                          {c.metadata.network}
                        </span>
                      </td>
                      <td className={`${tdCls} font-sg text-[12px] text-fg-3`}>{endpoint(c.metadata.sourceIP, c.metadata.sourcePort)}</td>
                      <td className={`${tdCls} font-sg text-[12px] ${c.metadata.destinationIP ? "text-fg-3" : "text-dim"}`}>
                        {c.metadata.destinationIP
                          ? endpoint(c.metadata.destinationIP, c.metadata.destinationPort)
                          : t("远端解析")}
                      </td>
                      <td className={`${tdCls} text-fg-3`}>{procName(c)}</td>
                      <td className={`${tdCls} text-[13px] text-fg-3`}>{c.rule}{c.rulePayload ? ` · ${c.rulePayload}` : ""}</td>
                      <td className={`${tdCls} text-[13px] text-fg-3`}>{[...c.chains].reverse().join(" » ")}</td>
                      <td className={tdCls}>
                        <Pair down={rate(c.downSpeed)} up={rate(c.upSpeed)} downOn={c.downSpeed > 0} upOn={c.upSpeed > 0} />
                      </td>
                      <td className={`${tdCls} font-sg text-fg-3`}>
                        {formatBytes(c.download)}
                        <span className="mx-1.5 text-faint">/</span>
                        {formatBytes(c.upload)}
                      </td>
                      <td className={`${tdCls} font-sg text-fg-3`}>{formatDuration(c.start)}</td>
                      <td className={tdCls}>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            closeOne(c.id);
                          }}
                          className="text-mut opacity-0 transition-opacity group-hover:opacity-100 hover:text-err"
                          title={t("关闭连接")}
                        >
                          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><path d="m6 6 12 12M18 6 6 18" /></svg>
                        </button>
                      </td>
                    </tr>
                    {isOpen ? (
                      <tr className="bg-surface">
                        <td colSpan={10} className="border-t border-line-soft px-3 pb-5 pt-3">
                          <div className="grid grid-cols-4 gap-x-10 gap-y-4 select-text">
                            <Detail label={t("五元组")} value={`${c.metadata.network.toUpperCase()}  ${endpoint(c.metadata.sourceIP, c.metadata.sourcePort)}  →  ${c.metadata.destinationIP ? endpoint(c.metadata.destinationIP, c.metadata.destinationPort) : `${c.metadata.host}:${c.metadata.destinationPort}（${t("远端解析")}）`}`} wide />
                            <Detail label={t("入站")} value={c.metadata.type || "—"} />
                            <Detail label={t("主机")} value={c.metadata.host || "—"} />
                            <Detail label={t("规则")} value={`${c.rule}${c.rulePayload ? ` · ${c.rulePayload}` : ""}`} />
                            <Detail label={t("链路")} value={[...c.chains].reverse().join(" » ") || "—"} />
                            <Detail label={t("进程")} value={c.metadata.processPath || procName(c)} wide />
                            <Detail label={t("开始于")} value={new Date(c.start).toLocaleString()} />
                            <Detail label={t("连接 ID")} value={c.id} />
                          </div>
                        </td>
                      </tr>
                    ) : null}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        ) : (
          <table className="w-full min-w-[960px] table-fixed border-collapse md:min-w-0">
            <thead>
              <tr>
                <th className={`${thBase} text-label`} style={{ width: "40%" }}>{t("应用")}</th>
                <th className={`${thBase} text-label`} style={{ width: "12%" }}>{t("连接数")}</th>
                <th className={`${thBase} text-label`} style={{ width: "24%" }}>{t("速率")}</th>
                <th className={`${thBase} text-label`} style={{ width: "24%" }}>{t("流量")}</th>
              </tr>
            </thead>
            <tbody>
              {apps.map((a) => (
                <tr key={a.name} title={a.path}>
                  <td className={`${tdCls} text-fg`}>{a.name}</td>
                  <td className={`${tdCls} font-sg text-fg-3`}>{a.count}</td>
                  <td className={tdCls}>
                    <Pair down={rate(a.downSpeed)} up={rate(a.upSpeed)} downOn={a.downSpeed > 0} upOn={a.upSpeed > 0} />
                  </td>
                  <td className={`${tdCls} font-sg text-fg-3`}>
                    {formatBytes(a.down)}
                    <span className="mx-1.5 text-faint">/</span>
                    {formatBytes(a.up)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {phase === "running" && items.length === 0 ? (
          <div className="flex h-32 items-center justify-center">
            <span className="cap">{all.length === 0 ? t("暂无经过内核的连接") : t("没有符合筛选条件的连接")}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function Detail({ label, value, wide }: { label: string; value: string; wide?: boolean }) {
  return (
    <div className={wide ? "col-span-2" : ""}>
      <div className="cap2">{label}</div>
      <div className="mt-1 break-all font-sg text-[13px] text-fg-2">{value}</div>
    </div>
  );
}
