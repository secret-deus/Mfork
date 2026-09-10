import { useEffect, useState } from "react";
import { useApp } from "../store";
import { apiGet, apiPut, relativeTime } from "../lib/core";
import { RuleItem, RuleProvider } from "../lib/types";
import { useT } from "../i18n";

export default function Rules() {
  const t = useT();
  const { api, phase } = useApp();
  const [rules, setRules] = useState<RuleItem[]>([]);
  const [providers, setProviders] = useState<RuleProvider[]>([]);
  const [filter, setFilter] = useState("");
  const [updating, setUpdating] = useState<string | null>(null);

  const refresh = async () => {
    if (!api) return;
    const r = await apiGet<{ rules: RuleItem[] }>(api, "/rules");
    const p = await apiGet<{ providers: Record<string, RuleProvider> }>(api, "/providers/rules");
    setRules(r.rules);
    setProviders(Object.values(p.providers ?? {}));
  };

  useEffect(() => {
    if (phase === "running") void refresh();
    else {
      setRules([]);
      setProviders([]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, api]);

  const updateProvider = async (name: string) => {
    if (!api) return;
    setUpdating(name);
    try {
      await apiPut(api, `/providers/rules/${encodeURIComponent(name)}`);
      await refresh();
    } finally {
      setUpdating(null);
    }
  };

  const q = filter.toLowerCase();
  const shown = q
    ? rules.filter(
        (r) =>
          r.payload.toLowerCase().includes(q) ||
          r.type.toLowerCase().includes(q) ||
          r.proxy.toLowerCase().includes(q),
      )
    : rules;

  return (
    <div className="flex h-full min-w-0 flex-col px-5 md:px-14 pb-6 pt-6 md:pt-13">
      <div className="flex items-center justify-between">
        <div className="flex items-baseline gap-5">
          <div className="cap">{t("Mfork · 规则 Rules")}</div>
          <span className="cap2">
            {t("共")} <span className="font-sg text-[12px] text-fg-2">{rules.length}</span> {t("条")}
          </span>
        </div>
      </div>

      {providers.length > 0 ? (
        <div className="mt-5 flex flex-wrap gap-x-8 gap-y-2 border-b border-line pb-4">
          {providers.map((p) => (
            <div key={p.name} className="flex items-center gap-3">
              <span className="text-[13px] text-fg-3">{p.name}</span>
              <span className="cap2">{p.behavior}</span>
              <span className="cap2">{t("{n} rules", { n: p.ruleCount })}</span>
              {p.updatedAt ? (
                <span className="cap2">
                  {t("{time}更新", { time: relativeTime(Date.parse(p.updatedAt) / 1000) })}
                </span>
              ) : null}
              <button
                onClick={() => void updateProvider(p.name)}
                disabled={updating === p.name}
                className="text-[12px] text-accent hover:text-fg disabled:opacity-50"
              >
                {updating === p.name ? t("更新中…") : t("更新")}
              </button>
            </div>
          ))}
        </div>
      ) : null}

      <div className="mt-5 flex items-center gap-3 border-b border-line-strong pb-2.5">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" className="text-dim">
          <circle cx="11" cy="11" r="7" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("搜索规则 / 类型 / 目标")}
          className="flex-1 bg-transparent text-[14px] outline-none placeholder:text-dim"
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {phase !== "running" ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">{t("内核未运行")}</span>
          </div>
        ) : (
          shown.map((r, i) => (
            <div key={i} className="flex items-center gap-4 border-b border-line-faint py-2.5">
              <span className="cap2 w-36 shrink-0">{r.type}</span>
              <span className="min-w-0 flex-1 truncate text-[13px] text-fg-3">{r.payload || "—"}</span>
              <span className={`shrink-0 text-[13px] ${r.proxy === "DIRECT" ? "text-ok" : r.proxy === "REJECT" ? "text-err" : "text-mut"}`}>
                {r.proxy}
              </span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
