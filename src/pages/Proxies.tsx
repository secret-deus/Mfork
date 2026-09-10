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

const SORTS: { key: SortBy; label: string }[] = [
  { key: "default", label: "默认" },
  { key: "delay", label: "延迟" },
  { key: "name", label: "名称" },
];

export default function Proxies() {
  const t = useT();
  const { api, phase, notify } = useApp();
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
    if (phase === "running") void refresh();
    else {
      setGroups([]);
      setNodes({});
    }
  }, [phase, refresh]);

  const select = async (group: string, node: string) => {
    if (!api) return;
    try {
      await apiPut(api, `/proxies/${encodeURIComponent(group)}`, { name: node });
    } catch (e) {
      notify(t("切换节点失败: {msg}", { msg: String(e) }));
    }
    await refresh();
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
    await refresh();
  };

  return (
    <div className="flex h-full min-w-0 flex-col px-14 pb-8 pt-13">
      <div className="flex items-center justify-between">
        <div className="cap">{t("Mfork · 代理 Proxies")}</div>
        <div className="flex items-center gap-5">
          <div className="flex items-center gap-3">
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
            onClick={() => void refresh()}
            className="text-[13px] text-mut hover:text-fg"
            disabled={phase !== "running"}
          >
            {t("刷新")}
          </button>
        </div>
      </div>

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
              <div key={g.name} className="border-b border-line-soft">
                <div
                  className="flex cursor-pointer items-center justify-between py-4"
                  onClick={() => setExpanded(open ? null : g.name)}
                >
                  <div className="flex items-baseline gap-3.5">
                    <span className="text-[16px] font-semibold text-fg-2">{g.name}</span>
                    <span className="cap2">{g.type}</span>
                    <span className="cap2">{t("{n} nodes", { n: g.all.length })}</span>
                  </div>
                  <div className="flex items-center gap-4">
                    <span className="text-[13px] text-mut">{g.now}</span>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void testGroup(g.name);
                      }}
                      className="text-[13px] text-accent hover:text-fg disabled:opacity-50"
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
                  <div className="grid grid-cols-3 gap-x-12 pb-4">
                    {visibleNodes(g).map((name) => {
                      const n = nodes[name];
                      const d = lastDelay(n);
                      const selected = g.now === name;
                      return (
                        <button
                          key={name}
                          onClick={() => void select(g.name, name)}
                          className="flex items-center justify-between border-b border-line-faint py-3 text-left"
                        >
                          <span className="flex min-w-0 items-center gap-2.5">
                            {selected ? (
                              <span className="block h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
                            ) : null}
                            <span className={`truncate text-[14px] ${selected ? "text-fg" : "text-fg-3"}`}>
                              {name}
                            </span>
                            <span className="cap2 shrink-0">{n?.type ?? ""}</span>
                          </span>
                          <span className={`font-sg shrink-0 text-[15px] ${delayColor(d)}`}>
                            {d && d > 0 ? d : "—"}
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
