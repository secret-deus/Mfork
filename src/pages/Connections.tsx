import { useMemo, useState } from "react";
import { useApp } from "../store";
import { apiDelete, formatBytes, formatDuration, formatRate } from "../lib/core";
import { ConnItem } from "../lib/types";
import { t, useT } from "../i18n";

type View = "socket" | "app";

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

function rate(n: number): string {
  if (n <= 0) return "—";
  const r = formatRate(n);
  return `${r.value} ${r.unit}`;
}

const thCls =
  "text-left whitespace-nowrap py-2.5 pr-3 text-[9px] font-semibold uppercase tracking-[2px] text-dim";
const tdCls = "whitespace-nowrap overflow-hidden text-ellipsis py-2.5 pr-3 text-xs border-t border-line-soft";

export default function Connections() {
  const t = useT();
  const { api, phase, snapshot } = useApp();
  const [view, setView] = useState<View>("socket");
  const [filter, setFilter] = useState("");
  const [paused, setPaused] = useState(false);
  const [frozen, setFrozen] = useState<typeof snapshot>(null);

  const snap = paused ? frozen : snapshot;

  const items = useMemo(() => {
    const list = snap?.connections ?? [];
    if (!filter) return list;
    const q = filter.toLowerCase();
    return list.filter(
      (c) =>
        hostOf(c).toLowerCase().includes(q) ||
        c.rule.toLowerCase().includes(q) ||
        procName(c).toLowerCase().includes(q) ||
        c.chains.join(" ").toLowerCase().includes(q),
    );
  }, [snap, filter]);

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

  const closeOne = (id: string) => {
    if (api) void apiDelete(api, `/connections/${id}`);
  };
  const closeAll = () => {
    if (api) void apiDelete(api, "/connections");
  };

  return (
    <div className="flex h-full min-w-0 flex-col px-14 pb-6 pt-13">
      <div className="flex items-center justify-between">
        <div className="flex items-baseline gap-5">
          <div className="cap">{t("Mfork · 连接 Connections")}</div>
          <span className="cap2">
            {t("活动")} <span className="font-sg text-[11px] text-fg-2">{snap?.connections.length ?? 0}</span>
          </span>
          <span className="cap2">
            {t("下载")} <span className="font-sg text-[11px] text-fg-2">{formatBytes(snap?.downloadTotal ?? 0)}</span>
          </span>
          <span className="cap2">
            {t("上传")} <span className="font-sg text-[11px] text-fg-2">{formatBytes(snap?.uploadTotal ?? 0)}</span>
          </span>
        </div>
        <div className="flex items-center gap-6">
          <div className="flex gap-4">
            <button
              onClick={() => setView("socket")}
              className={`border-b-2 pb-1 text-xs ${view === "socket" ? "border-accent text-fg" : "border-transparent text-dim hover:text-mut"}`}
            >
              {t("连接视图")}
            </button>
            <button
              onClick={() => setView("app")}
              className={`border-b-2 pb-1 text-xs ${view === "app" ? "border-accent text-fg" : "border-transparent text-dim hover:text-mut"}`}
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
            className={`text-xs ${paused ? "text-warn" : "text-mut hover:text-fg"}`}
          >
            {paused ? t("继续") : t("暂停")}
          </button>
          <button onClick={closeAll} className="text-xs text-err/80 hover:text-err">
            {t("关闭全部")}
          </button>
        </div>
      </div>

      <div className="mt-6 flex items-center gap-3 border-b border-line-strong pb-2.5">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" className="text-dim">
          <circle cx="11" cy="11" r="7" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("搜索主机 / 进程 / 规则 / 节点")}
          className="flex-1 bg-transparent text-[13px] outline-none placeholder:text-dim"
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {phase !== "running" ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">{t("内核未运行")}</span>
          </div>
        ) : view === "socket" ? (
          <table className="w-full table-fixed border-collapse">
            <thead>
              <tr>
                <th className={thCls} style={{ width: "24%" }}>{t("主机")}</th>
                <th className={thCls} style={{ width: "13%" }}>{t("进程")}</th>
                <th className={thCls} style={{ width: "5%" }}>{t("网络")}</th>
                <th className={thCls} style={{ width: "13%" }}>{t("规则")}</th>
                <th className={thCls} style={{ width: "16%" }}>{t("链路")}</th>
                <th className={thCls} style={{ width: "9%" }}>{t("速率")}</th>
                <th className={thCls} style={{ width: "7%" }}>{t("下载")}</th>
                <th className={thCls} style={{ width: "6%" }}>{t("上传")}</th>
                <th className={thCls} style={{ width: "5%" }}>{t("时长")}</th>
                <th className={thCls} style={{ width: "3%" }}></th>
              </tr>
            </thead>
            <tbody>
              {items.map((c) => (
                <tr key={c.id} className="group">
                  <td className={`${tdCls} text-fg-2`}>{hostOf(c)}</td>
                  <td className={`${tdCls} text-mut`}>{procName(c)}</td>
                  <td className={`${tdCls} uppercase text-[10px] tracking-[1px] text-dim`}>{c.metadata.network}</td>
                  <td className={`${tdCls} text-[11px] text-mut`}>{c.rule}{c.rulePayload ? ` · ${c.rulePayload}` : ""}</td>
                  <td className={`${tdCls} text-[11px] text-mut`}>{[...c.chains].reverse().join(" » ")}</td>
                  <td className={`${tdCls} font-sg ${c.downSpeed > 0 ? "text-ok" : "text-dim"}`}>{rate(c.downSpeed)}</td>
                  <td className={`${tdCls} font-sg text-mut`}>{formatBytes(c.download)}</td>
                  <td className={`${tdCls} font-sg text-mut`}>{formatBytes(c.upload)}</td>
                  <td className={`${tdCls} font-sg text-dim`}>{formatDuration(c.start)}</td>
                  <td className={tdCls}>
                    <button onClick={() => closeOne(c.id)} className="text-dim opacity-0 transition-opacity group-hover:opacity-100 hover:text-err" title={t("关闭连接")}>
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round"><path d="m6 6 12 12M18 6 6 18" /></svg>
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <table className="w-full table-fixed border-collapse">
            <thead>
              <tr>
                <th className={thCls} style={{ width: "30%" }}>{t("应用")}</th>
                <th className={thCls} style={{ width: "10%" }}>{t("连接数")}</th>
                <th className={thCls} style={{ width: "12%" }}>{t("下载速率")}</th>
                <th className={thCls} style={{ width: "12%" }}>{t("上传速率")}</th>
                <th className={thCls} style={{ width: "12%" }}>{t("下载总量")}</th>
                <th className={thCls} style={{ width: "12%" }}>{t("上传总量")}</th>
              </tr>
            </thead>
            <tbody>
              {apps.map((a) => (
                <tr key={a.name} title={a.path}>
                  <td className={`${tdCls} text-fg-2`}>{a.name}</td>
                  <td className={`${tdCls} font-sg text-mut`}>{a.count}</td>
                  <td className={`${tdCls} font-sg ${a.downSpeed > 0 ? "text-ok" : "text-dim"}`}>{rate(a.downSpeed)}</td>
                  <td className={`${tdCls} font-sg ${a.upSpeed > 0 ? "text-accent" : "text-dim"}`}>{rate(a.upSpeed)}</td>
                  <td className={`${tdCls} font-sg text-mut`}>{formatBytes(a.down)}</td>
                  <td className={`${tdCls} font-sg text-mut`}>{formatBytes(a.up)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {phase === "running" && items.length === 0 ? (
          <div className="flex h-32 items-center justify-center">
            <span className="cap">{t("暂无经过内核的连接")}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
