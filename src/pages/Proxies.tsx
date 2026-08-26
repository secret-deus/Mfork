import { useCallback, useEffect, useState } from "react";
import { useApp } from "../store";
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

export default function Proxies() {
  const { api, phase } = useApp();
  const [groups, setGroups] = useState<ProxyGroup[]>([]);
  const [nodes, setNodes] = useState<Record<string, ProxyNode>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [testing, setTesting] = useState<string | null>(null);

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
    await apiPut(api, `/proxies/${encodeURIComponent(group)}`, { name: node });
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
        <div className="cap">Mfork · 代理 Proxies</div>
        <button
          onClick={() => void refresh()}
          className="text-xs text-mut hover:text-fg"
          disabled={phase !== "running"}
        >
          刷新
        </button>
      </div>

      <div className="mt-6 min-h-0 flex-1 overflow-y-auto">
        {phase !== "running" ? (
          <div className="flex h-full items-center justify-center">
            <span className="cap">内核未运行</span>
          </div>
        ) : groups.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2">
            <span className="cap">当前配置无策略组</span>
            <span className="cap2">导入订阅后（M2）此处显示策略组与节点</span>
          </div>
        ) : (
          groups.map((g) => {
            const open = expanded === g.name;
            return (
              <div key={g.name} className="border-b border-[#16181d]">
                <div
                  className="flex cursor-pointer items-center justify-between py-4"
                  onClick={() => setExpanded(open ? null : g.name)}
                >
                  <div className="flex items-baseline gap-3.5">
                    <span className="text-[15px] font-semibold text-[#d6d9de]">{g.name}</span>
                    <span className="cap2">{g.type}</span>
                    <span className="cap2">{g.all.length} nodes</span>
                  </div>
                  <div className="flex items-center gap-4">
                    <span className="text-xs text-mut">{g.now}</span>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        void testGroup(g.name);
                      }}
                      className="text-xs text-accent hover:text-fg disabled:opacity-50"
                      disabled={testing === g.name}
                    >
                      {testing === g.name ? "测速中…" : "测速"}
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
                    {g.all.map((name) => {
                      const n = nodes[name];
                      const d = lastDelay(n);
                      const selected = g.now === name;
                      return (
                        <button
                          key={name}
                          onClick={() => void select(g.name, name)}
                          className="flex items-center justify-between border-b border-[#131519] py-3 text-left"
                        >
                          <span className="flex min-w-0 items-center gap-2.5">
                            {selected ? (
                              <span className="block h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
                            ) : null}
                            <span className={`truncate text-[13px] ${selected ? "text-fg" : "text-[#b6bac2]"}`}>
                              {name}
                            </span>
                            <span className="cap2 shrink-0">{n?.type ?? ""}</span>
                          </span>
                          <span className={`font-sg shrink-0 text-sm ${delayColor(d)}`}>
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
