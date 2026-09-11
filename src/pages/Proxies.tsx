import { useCallback, useEffect, useState } from "react";
import { useApp } from "../store";
import { useT } from "../i18n";
import { apiGet, apiPut } from "../lib/core";
import { ProxyGroup, ProxyNode } from "../lib/types";

const TEST_URL = "https://www.gstatic.com/generate_204";

function delayColor(d: number | undefined): string {
  if (!d || d <= 0) return "text-dim";
  if (d < 100) return "text-ok";
  if (d < 300) return "text-warn";
  return "text-err";
}

function lastDelay(n: ProxyNode | undefined): number | undefined {
  const h = n?.history;
  return h && h.length > 0 ? h[h.length - 1].delay : undefined;
}

type SortBy = "default" | "delay" | "name";

/** groups the core lets the API pin a node on; LoadBalance / Relay answer HTTP 400 */
const SELECTABLE = new Set(["Selector", "URLTest", "Fallback"]);
const selectable = (g: ProxyGroup) => SELECTABLE.has(g.type);

const SORTS: { key: SortBy; label: string }[] = [
  { key: "default", label: "默认" },
  { key: "delay", label: "延迟" },
  { key: "name", label: "名称" },
];

export default function Proxies() {
  const t = useT();
  const { api, phase, mode, notify } = useApp();
  const [groups, setGroups] = useState<ProxyGroup[]>([]);
  const [nodes, setNodes] = useState<Record<string, ProxyNode>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [sortBy, setSortBy] = useState<SortBy>("default");
  const [hideUnavail, setHideUnavail] = useState(false);

  const visibleNodes = useCallback(
    (g: ProxyGroup): string[] => {
      let names = [...g.all];
      if (hideUnavail) {
        // hide only nodes whose last test failed; untested nodes stay
        names = names.filter((name) => {
          if (name === g.now) return true;
          const h = nodes[name]?.history;
          if (!h || h.length === 0) return true;
          return h[h.length - 1].delay > 0;
        });
      }
      if (sortBy === "delay") {
        names.sort((a, b) => {
          const da = lastDelay(nodes[a]);
          const db = lastDelay(nodes[b]);
          return (da && da > 0 ? da : Infinity) - (db && db > 0 ? db : Infinity);
        });
      } else if (sortBy === "name") {
        names.sort((a, b) => a.localeCompare(b));
      }
      return names;
    },
    [nodes, sortBy, hideUnavail],
  );

  const refresh = useCallback(async () => {
    if (!api) return;
    const g = await apiGet<{ proxies: ProxyGroup[] }>(api, "/group");
    const p = await apiGet<{ proxies: Record<string, ProxyNode> }>(api, "/proxies");
    const list = g.proxies.filter((x) => x.name !== "GLOBAL");
    setGroups(list);
    setNodes(p.proxies);
    setExpanded((cur) => cur ?? list[0]?.name ?? null);
  }, [api]);

  useEffect(() => {
    if (phase === "running") void refresh().catch((e) => notify(String(e)));
    else {
      setGroups([]);
      setNodes({});
    }
  }, [phase, refresh, notify]);

  const select = async (g: ProxyGroup, node: string) => {
    if (!api) return;
    if (!selectable(g)) {
      notify(t("{type} 组由内核自动选择，不能手动切换", { type: g.type }));
      return;
    }
    try {
      await apiPut(api, `/proxies/${encodeURIComponent(g.name)}`, { name: node });
    } catch (e) {
      const msg = String(e);
      notify(t("切换节点失败: {msg}", { msg: msg.replace(/^Error:\s*/, "") }));
      return;
    }
    await refresh().catch((e) => notify(String(e)));
  };

  const testGroup = async (group: string) => {
    if (!api) return;
    setTesting(group);
    try {
      await apiGet(
        api,
        `/group/${encodeURIComponent(group)}/delay?url=${encodeURIComponent(TEST_URL)}&timeout=5000`,
      );
    } catch {
      /* individual timeouts are expected */
    }
    setTesting(null);
    await refresh().catch((e) => notify(String(e)));
  };

  return (
    <div className="proxy-page flex h-full min-w-0 flex-col px-4 md:px-14 pb-4 pt-5 md:pt-13">
      <div className="flex flex-col items-start gap-3 md:flex-row md:items-center md:justify-between">
        <div><h1 className="text-xl font-semibold md:hidden">{t("代理")}</h1><span className="cap hidden md:inline">{t("Mfork · 代理 Proxies")}</span></div>
        <div className="flex w-full items-center gap-5 overflow-x-auto whitespace-nowrap pb-1 [scrollbar-width:none] md:w-auto md:overflow-visible md:pb-0">
          <div className="flex shrink-0 items-center gap-3">
            <span className="cap2">{t("排序")}</span>
            {SORTS.map((s) => (
              <button
                key={s.key}
                onClick={() => setSortBy(s.key)}
                className={`text-[13px] transition-colors ${
                  sortBy === s.key ? "text-fg" : "text-dim hover:text-mut"
                }`}
              >
                {t(s.label)}
              </button>
            ))}
          </div>
          <div className="h-3.5 w-px bg-line" />
          <button
            onClick={() => setHideUnavail((v) => !v)}
            className={`text-[13px] transition-colors ${
              hideUnavail ? "text-fg" : "text-dim hover:text-mut"
            }`}
          >
            {t("隐藏不可用")}
          </button>
          <div className="h-3.5 w-px bg-line" />
          <button
            onClick={() => void refresh().catch((e) => notify(String(e)))}
            className="text-[13px] text-mut hover:text-fg"
            disabled={phase !== "running"}
          >
            {t("刷新")}
          </button>
        </div>
      </div>

      {phase === "running" && mode === "direct" ? (
        <div role="status" className="mt-4 rounded-xl border border-line px-4 py-3 text-sm text-mut">
          {t("直连模式 · 不经过节点")}
        </div>
      ) : null}

      <div className="mt-6 min-h-0 flex-1 overflow-y-auto">
        {phase !== "running" ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">{t("内核未运行")}</span>
          </div>
        ) : groups.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2">
            <span className="cap">{t("当前配置无策略组")}</span>
            <span className="cap2">{t("导入订阅后此处显示策略组与节点")}</span>
          </div>
        ) : (
          groups.map((g) => {
            const open = expanded === g.name;
            return (
              <div key={g.name} className="proxy-group border-b border-line-soft">
                <div
                  className="proxy-group-header flex cursor-pointer items-center justify-between gap-3 py-4"
                  onClick={() => setExpanded(open ? null : g.name)}
                >
                  {/* phones: name on one line, meta + current node beneath;
                      desktop: everything on one baseline */}
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-baseline gap-3.5">
                      <span className="[overflow-wrap:anywhere] text-[15px] font-semibold text-fg-2 md:truncate md:text-[16px]">
                        {g.name}
                      </span>
                      <span className="cap2 hidden md:inline">{g.type}</span>
                      <span className="cap2 hidden md:inline">{t("{n} nodes", { n: g.all.length })}</span>
                    </div>
                    <div className="mt-1 flex min-w-0 items-center gap-2 text-[12.5px] text-mut md:hidden">
                      <span className="cap2 shrink-0">{g.type}</span>
                      <span className="shrink-0">{t("{n} nodes", { n: g.all.length })}</span>
                      {g.now ? (
                        <>
                          <span className="shrink-0 text-dim">·</span>
                          <span className="truncate text-fg-3">{g.now}</span>
                        </>
                      ) : null}
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-3 md:gap-4">
                    <span className="hidden max-w-[260px] truncate text-[13px] text-mut md:inline">{g.now}</span>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void testGroup(g.name);
                      }}
                      className="shrink-0 whitespace-nowrap text-[13px] text-accent hover:text-fg disabled:opacity-50"
                      disabled={testing === g.name}
                    >
                      {testing === g.name ? t("测速中…") : t("测速")}
                    </button>
                    <svg
                      width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                      strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"
                      className={`text-dim transition-transform ${open ? "rotate-90" : ""}`}
                    >
                      <path d="m9 6 6 6-6 6" />
                    </svg>
                  </div>
                </div>
                {open ? (
                  <div className="proxy-nodes grid grid-cols-1 gap-x-4 pb-4 md:grid-cols-2 xl:grid-cols-3">
                    {!selectable(g) ? (
                      <div className="cap2 col-span-full pb-1">
                        {t("{type} 组由内核自动选择，不能手动切换", { type: g.type })}
                      </div>
                    ) : null}
                    {visibleNodes(g).map((name) => {
                      const n = nodes[name];
                      const d = lastDelay(n);
                      const selected = g.now === name;
                      return (
                        <button
                          key={name}
                          onClick={() => void select(g, name)}
                          className={`proxy-node min-w-0 flex items-center justify-between gap-3 border-b border-line-faint px-3 py-3 text-left ${selected ? "bg-accent/10 rounded-xl" : ""} ${
                            selectable(g) ? "" : "cursor-default"
                          }`}
                        >
                          <span className="flex min-w-0 flex-1 items-center gap-2.5">
                            {selected ? (
                              <span className="block h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
                            ) : null}
                            <span className="min-w-0 flex-1">
                            <span className={`block [overflow-wrap:anywhere] text-[14px] leading-6 ${selected ? "text-fg" : "text-fg-3"}`}>
                              {name}
                            </span>
                            <span className="block text-xs leading-5 text-dim">{n?.type ?? ""}</span>
                            </span>
                          </span>
                          <span className={`font-sg shrink-0 text-[15px] ${delayColor(d)}`}>
                            {d && d > 0 ? `${d} ms` : "—"}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                ) : null}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
