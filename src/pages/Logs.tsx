import { useEffect, useRef, useState } from "react";
import { useApp } from "../store";
import { useT } from "../i18n";

const LEVELS = ["all", "info", "warning", "error", "debug"] as const;
const COLOR: Record<string, string> = {
  info: "text-mut",
  warning: "text-warn",
  error: "text-err",
  debug: "text-dim",
};

export default function Logs() {
  const t = useT();
  const { phase, logs, logsPaused, setLogsPaused, clearLogs } = useApp();
  const [level, setLevel] = useState<(typeof LEVELS)[number]>("all");
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);

  const shown = level === "all" ? logs : logs.filter((l) => l.type === level);

  // follow new output only while the user is at (or near) the bottom
  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, [shown.length]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  };

  return (
    <div className="flex h-full min-w-0 flex-col px-5 md:px-14 pb-6 pt-6 md:pt-13">
      <div className="flex items-center justify-between">
        <div className="cap">{t("Mfork · 日志 Logs")}</div>
        <div className="flex items-center gap-5">
          {LEVELS.map((lv) => (
            <button
              key={lv}
              onClick={() => setLevel(lv)}
              className={`border-b-2 pb-1 text-[13px] uppercase tracking-[1px] ${
                level === lv ? "border-accent text-fg" : "border-transparent text-dim hover:text-mut"
              }`}
            >
              {lv}
            </button>
          ))}
          <div className="h-4 w-px bg-line" />
          <button
            onClick={() => setLogsPaused(!logsPaused)}
            className={`text-[13px] ${logsPaused ? "text-warn" : "text-mut hover:text-fg"}`}
          >
            {logsPaused ? t("继续") : t("暂停")}
          </button>
          <button onClick={clearLogs} className="text-[13px] text-mut hover:text-fg">
            {t("清空")}
          </button>
        </div>
      </div>
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="mt-5 min-h-0 flex-1 overflow-y-auto border-t border-line-strong pt-3 select-text"
      >
        {phase !== "running" && shown.length === 0 ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">{t("内核未运行")}</span>
          </div>
        ) : shown.length === 0 ? (
          <div className="flex h-32 items-center justify-center">
            <span className="cap">{t("暂无日志")}</span>
          </div>
        ) : (
          shown.map((l) => (
            <div key={l.seq} className="flex gap-3 py-0.5 font-sg text-[12.5px] leading-relaxed">
              <span className="shrink-0 text-dim">
                {new Date(l.at).toLocaleTimeString("en-GB")}
              </span>
              <span className={`w-14 shrink-0 uppercase tracking-[1px] text-[11px] pt-0.5 ${COLOR[l.type] ?? "text-mut"}`}>
                {l.type}
              </span>
              <span className="break-all text-fg-3">{l.payload}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
