import { useEffect, useRef, useState } from "react";
import { useApp } from "../store";
import { wsUrl } from "../lib/core";
import { LogLine } from "../lib/types";

const LEVELS = ["all", "info", "warning", "error", "debug"] as const;
const COLOR: Record<string, string> = {
  info: "text-mut",
  warning: "text-warn",
  error: "text-err",
  debug: "text-dim",
};

let seq = 0;

export default function Logs() {
  const { api, phase } = useApp();
  const [lines, setLines] = useState<LogLine[]>([]);
  const [level, setLevel] = useState<(typeof LEVELS)[number]>("all");
  const [paused, setPaused] = useState(false);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!api || phase !== "running") return;
    const ws = new WebSocket(wsUrl(api, "/logs") + "&level=debug");
    ws.onmessage = (e) => {
      if (pausedRef.current) return;
      const d = JSON.parse(e.data);
      setLines((prev) => {
        const next = [...prev, { type: d.type, payload: d.payload, seq: seq++, at: Date.now() }];
        return next.length > 500 ? next.slice(next.length - 500) : next;
      });
    };
    return () => ws.close();
  }, [api, phase]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" as ScrollBehavior });
  }, [lines]);

  const shown = level === "all" ? lines : lines.filter((l) => l.type === level);

  return (
    <div className="flex h-full min-w-0 flex-col px-14 pb-6 pt-13">
      <div className="flex items-center justify-between">
        <div className="cap">Mfork · 日志 Logs</div>
        <div className="flex items-center gap-5">
          {LEVELS.map((lv) => (
            <button
              key={lv}
              onClick={() => setLevel(lv)}
              className={`border-b-2 pb-1 text-xs uppercase tracking-[1px] ${
                level === lv ? "border-accent text-fg" : "border-transparent text-dim hover:text-mut"
              }`}
            >
              {lv}
            </button>
          ))}
          <div className="h-4 w-px bg-line" />
          <button
            onClick={() => setPaused(!paused)}
            className={`text-xs ${paused ? "text-warn" : "text-mut hover:text-fg"}`}
          >
            {paused ? "继续" : "暂停"}
          </button>
          <button onClick={() => setLines([])} className="text-xs text-mut hover:text-fg">
            清空
          </button>
        </div>
      </div>
      <div className="mt-5 min-h-0 flex-1 overflow-y-auto border-t border-line-strong pt-3 select-text">
        {phase !== "running" ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">内核未运行</span>
          </div>
        ) : shown.length === 0 ? (
          <div className="flex h-32 items-center justify-center">
            <span className="cap">暂无日志</span>
          </div>
        ) : (
          shown.map((l) => (
            <div key={l.seq} className="flex gap-3 py-0.5 font-sg text-[11.5px] leading-relaxed">
              <span className="shrink-0 text-dim">
                {new Date(l.at).toLocaleTimeString("en-GB")}
              </span>
              <span className={`w-14 shrink-0 uppercase tracking-[1px] text-[10px] pt-0.5 ${COLOR[l.type] ?? "text-mut"}`}>
                {l.type}
              </span>
              <span className="break-all text-[#b9bec7]">{l.payload}</span>
            </div>
          ))
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
