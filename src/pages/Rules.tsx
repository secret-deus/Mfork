import { useEffect, useState } from "react";
import { useApp } from "../store";
import { apiGet, apiPut, relativeTime } from "../lib/core";
import { RuleItem, RuleProvider } from "../lib/types";

export default function Rules() {
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
    <div className="flex h-full min-w-0 flex-col px-14 pb-6 pt-13">
      <div className="flex items-center justify-between">
        <div className="flex items-baseline gap-5">
          <div className="cap">Mfork · 规则 Rules</div>
          <span className="cap2">
            共 <span className="font-sg text-[11px] text-[#c3c8d0]">{rules.length}</span> 条
          </span>
        </div>
      </div>

      {providers.length > 0 ? (
        <div className="mt-5 flex flex-wrap gap-x-8 gap-y-2 border-b border-line pb-4">
          {providers.map((p) => (
            <div key={p.name} className="flex items-center gap-3">
              <span className="text-xs text-[#b6bac2]">{p.name}</span>
              <span className="cap2">{p.behavior}</span>
              <span className="cap2">{p.ruleCount} rules</span>
              {p.updatedAt ? (
                <span className="cap2">{relativeTime(Date.parse(p.updatedAt) / 1000)}更新</span>
              ) : null}
              <button
                onClick={() => void updateProvider(p.name)}
                disabled={updating === p.name}
                className="text-[11px] text-accent hover:text-fg disabled:opacity-50"
              >
                {updating === p.name ? "更新中…" : "更新"}
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
          placeholder="搜索规则 / 类型 / 目标"
          className="flex-1 bg-transparent text-[13px] outline-none placeholder:text-dim"
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {phase !== "running" ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">内核未运行</span>
          </div>
        ) : (
          shown.map((r, i) => (
            <div key={i} className="flex items-center gap-4 border-b border-[#131519] py-2.5">
              <span className="cap2 w-36 shrink-0">{r.type}</span>
              <span className="min-w-0 flex-1 truncate text-xs text-[#b6bac2]">{r.payload || "—"}</span>
              <span className={`shrink-0 text-xs ${r.proxy === "DIRECT" ? "text-ok" : r.proxy === "REJECT" ? "text-err" : "text-mut"}`}>
                {r.proxy}
              </span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
