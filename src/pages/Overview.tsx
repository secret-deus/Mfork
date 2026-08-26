import { useEffect, useState } from "react";
import { useApp } from "../store";
import { formatBytes, formatRate } from "../lib/core";

function Uptime({ since }: { since: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, []);
  const s = Math.max(0, Math.floor((Date.now() - since) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return (
    <span>
      已运行 {h > 0 ? `${h} 小时 ` : ""}
      {m} 分 {s % 60} 秒
    </span>
  );
}

const MODES = [
  { key: "rule", label: "规则" },
  { key: "global", label: "全局" },
  { key: "direct", label: "直连" },
] as const;

export default function Overview() {
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
  } = useApp();

  const dl = formatRate(down);
  const ul = formatRate(up);

  return (
    <div className="flex h-full flex-col px-14 pb-10 pt-13">
      <div className="flex items-center justify-between">
        <div className="cap">Mfork · 概览 Overview</div>
        <div className="cap2">
          {phase === "running" && api ? <Uptime since={api.started_at_ms} /> : null}
        </div>
      </div>

      <div className="mt-8 flex items-baseline gap-3.5">
        <span className="font-sg text-[100px] leading-[0.9] font-light tracking-[-2px]">
          {phase === "running" ? dl.value : "0"}
        </span>
        <div className="flex flex-col gap-0.5">
          <span className="text-[13px] text-mut">{dl.unit}</span>
          <span className="cap2 !text-ok tracking-[2px]">↓ 下载</span>
        </div>
      </div>
      <div className="mt-4 flex items-baseline gap-3">
        <span className="font-sg text-[38px] leading-none font-light text-[#9aa0a8]">
          {phase === "running" ? ul.value : "0"}
        </span>
        <span className="text-[11px] text-dim">{ul.unit}</span>
        <span className="cap2 !text-accent tracking-[2px]">↑ 上传</span>
      </div>

      <div className="mt-9 h-px w-[470px] bg-line" />
      <div className="flex w-[470px]">
        <div className="flex flex-1 flex-col gap-1.5 py-4">
          <span className="cap">活动连接</span>
          <span className="font-sg text-2xl">
            {phase === "running" ? connections : "—"}
          </span>
        </div>
        <div className="w-px bg-line" />
        <div className="flex flex-1 flex-col gap-1.5 py-4 pl-6">
          <span className="cap">内核内存</span>
          <span className="font-sg text-2xl">
            {phase === "running" ? formatBytes(memory) : "—"}
          </span>
        </div>
        <div className="w-px bg-line" />
        <div className="flex flex-1 flex-col gap-1.5 py-4 pl-6">
          <span className="cap">内核版本</span>
          <span className="font-sg text-2xl">
            {phase === "running" && version ? version.replace(/^v/, "") : "—"}
          </span>
        </div>
      </div>
      <div className="h-px w-[470px] bg-line" />

      <div className="mt-6 flex items-center gap-6">
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
            {m.label}
          </button>
        ))}
        <div className="h-4 w-px bg-line" />
        {phase === "running" ? (
          <button onClick={stop} className="text-xs text-mut hover:text-fg">
            停止内核
          </button>
        ) : (
          <button
            onClick={start}
            disabled={phase === "starting"}
            className="text-xs text-accent hover:text-fg disabled:opacity-50"
          >
            {phase === "starting" ? "启动中…" : "启动内核"}
          </button>
        )}
      </div>

      <div className="flex-1" />

      <div className="flex w-[470px] flex-col gap-3">
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
              ? `mihomo ${version ?? ""} · 运行中`
              : phase === "starting"
                ? "内核启动中"
                : phase === "error"
                  ? "内核异常"
                  : "内核已停止"}
          </span>
          {phase === "running" && api ? (
            <span className="cap2">
              API 127.0.0.1:{api.port} · 混合端口 {api.mixed_port}
            </span>
          ) : null}
        </div>
        <div className="cap2">节点地图与订阅信息将在 M2/M4 接入</div>
      </div>
    </div>
  );
}
