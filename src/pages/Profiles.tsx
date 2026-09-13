import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useApp } from "../store";
import { t, useT } from "../i18n";
import {
  ProfileMeta,
  ProfilesIndex,
  apiPut,
  formatBytes,
  profileDelete,
  profileImport,
  profileImportLocal,
  profileSelect,
  profileSetInterval,
  profileUpdate,
  profilesList,
  relativeTime,
} from "../lib/core";

const INTERVALS = [
  { v: 0, label: "手动" },
  { v: 6, label: "6h" },
  { v: 12, label: "12h" },
  { v: 24, label: "24h" },
  { v: 72, label: "72h" },
];

function expireText(expire: number): string {
  if (!expire) return t("长期有效");
  return t("到期 {date}", { date: new Date(expire * 1000).toISOString().slice(0, 10) });
}

export default function Profiles() {
  const t = useT();
  const { api, syncConfig } = useApp();
  const [index, setIndex] = useState<ProfilesIndex>({ active: null, items: [] });
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null); // "import" | profile id
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => setIndex(await profilesList());

  useEffect(() => {
    void refresh();
  }, []);

  const run = async (key: string, fn: () => Promise<void>) => {
    setBusy(key);
    setError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(String(e));
      // Import can be persisted even if applying it fails. Show disk state
      // so users do not import duplicates after a controller error.
      await refresh().catch(() => {});
    } finally {
      setBusy(null);
    }
  };

  const applyConfig = async (path: string | null) => {
    if (path && api) {
      await apiPut(api, "/configs?force=true", { path });
      await syncConfig();
    }
  };

  const doImport = () =>
    run("import", async () => {
      const meta = await profileImport(url.trim());
      setUrl("");
      // first import becomes active automatically: apply it
      const idx = await profilesList();
      if (idx.active === meta.id) {
        await applyConfig(await profileSelect(meta.id));
      }
    });

  const doSelect = (p: ProfileMeta) =>
    run(p.id, async () => applyConfig(await profileSelect(p.id)));

  const doUpdate = (p: ProfileMeta) =>
    run(p.id, async () => {
      await profileUpdate(p.id);
      const idx = await profilesList();
      if (idx.active === p.id) {
        await applyConfig(await profileSelect(p.id));
      }
    });

  const doDelete = (p: ProfileMeta) => {
    if (!window.confirm(t("删除订阅「{name}」？", { name: p.name }))) return;
    void run(p.id, async () => profileDelete(p.id));
  };

  return (
    <div className="flex h-full min-w-0 flex-col px-5 md:px-14 pb-8 pt-6 md:pt-13">
      <div className="flex items-center justify-between">
        <div className="cap">{t("Mfork · 订阅 Profiles")}</div>
        <button
          onClick={() =>
            void run("import", async () => {
              const path = await open({
                multiple: false,
                filters: [{ name: t("Clash 配置"), extensions: ["yaml", "yml"] }],
              });
              if (typeof path !== "string") return;
              const meta = await profileImportLocal(path);
              const idx = await profilesList();
              if (idx.active === meta.id) {
                await applyConfig(await profileSelect(meta.id));
              }
            })
          }
          disabled={busy !== null}
          className="text-[13px] text-mut hover:text-fg disabled:opacity-40"
        >
          {t("打开本地文件")}
        </button>
      </div>

      <div className="mt-7 flex items-center gap-3 border-b border-line-strong pb-3">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" className="text-dim">
          <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
          <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
        </svg>
        <input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && url.trim() && busy === null) void doImport();
          }}
          placeholder={t("粘贴订阅链接（https://...）")}
          className="flex-1 bg-transparent text-[14px] outline-none placeholder:text-dim"
        />
        <button
          onClick={() => void doImport()}
          disabled={!url.trim() || busy !== null}
          className="text-[13px] font-medium tracking-[0.5px] text-accent hover:text-fg disabled:opacity-40"
        >
          {busy === "import" ? t("下载中…") : t("导入")}
        </button>
      </div>

      {error ? (
        <div className="mt-3 text-[13px] text-err select-text">{error}</div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {index.items.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2">
            <span className="cap">{t("尚无订阅")}</span>
            <span className="cap2">{t("粘贴订阅链接导入，首个订阅将自动启用")}</span>
          </div>
        ) : (
          index.items.map((p) => {
            const active = index.active === p.id;
            const used = p.sub ? p.sub.upload + p.sub.download : 0;
            const pct = p.sub && p.sub.total > 0 ? Math.min(100, (used / p.sub.total) * 100) : 0;
            return (
              <div key={p.id} className="flex flex-col gap-4 border-b border-line-soft py-6">
                <div className="flex min-w-0 flex-col gap-4 md:flex-row md:items-center md:justify-between">
                  <div className="flex min-w-0 items-center gap-3">
                    {active ? (
                      <span className="block h-[7px] w-[7px] rounded-full bg-accent" />
                    ) : null}
                    <span className={`min-w-0 break-all text-[17px] font-medium leading-snug ${active ? "" : "text-fg-2"}`} title={p.name}>
                      {p.name}
                    </span>
                    {active ? (
                      <span className="cap2 shrink-0 whitespace-nowrap !text-accent">{t("使用中")}</span>
                    ) : (
                      <button
                        onClick={() => void doSelect(p)}
                        disabled={busy !== null}
                        className="cap2 hover:!text-fg disabled:opacity-40"
                      >
                        {t("启用")}
                      </button>
                    )}
                  </div>
                  <div className="flex min-w-0 flex-wrap items-center gap-x-5 gap-y-3">
                    {p.url ? (
                      <div className="flex flex-wrap items-center gap-3.5 whitespace-nowrap" title={t("自动更新间隔")}>
                        <span className="cap2 mr-1">{t("自动更新")}</span>
                        {INTERVALS.map((it) => (
                          <button
                            key={it.v}
                            onClick={() =>
                              void run(p.id, async () => profileSetInterval(p.id, it.v))
                            }
                            className={`text-[12px] tracking-[0.5px] ${
                              p.interval_hours === it.v
                                ? "font-medium text-accent"
                                : "text-dim hover:text-mut"
                            }`}
                          >
                            {t(it.label)}
                          </button>
                        ))}
                      </div>
                    ) : null}
                    {p.url ? <div className="h-3.5 w-px bg-line" /> : null}
                    {p.url ? (
                      <button
                        onClick={() => void doUpdate(p)}
                        disabled={busy !== null}
                        className="text-[13px] text-mut hover:text-fg disabled:opacity-40"
                      >
                        {busy === p.id ? t("处理中…") : t("更新")}
                      </button>
                    ) : null}
                    <button
                      onClick={() => doDelete(p)}
                      disabled={busy !== null}
                      className="text-[13px] text-dim hover:text-err disabled:opacity-40"
                    >
                      {t("删除")}
                    </button>
                  </div>
                </div>
                {p.sub && p.sub.total > 0 ? (
                  <>
                    <div className="flex items-baseline gap-2.5">
                      <span className="font-sg text-[26px] font-normal">
                        {formatBytes(used)}
                      </span>
                      <span className="font-sg text-[14px] text-dim">
                        / {formatBytes(p.sub.total)}
                      </span>
                      <span className="flex-1" />
                      <span className="cap2">{expireText(p.sub.expire)}</span>
                      <span className="cap2">{t("更新于 {time}", { time: relativeTime(p.updated_at) })}</span>
                    </div>
                    <div className="h-0.5 bg-line">
                      <div
                        className={`h-full ${active ? "bg-accent" : "bg-faint"}`}
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  </>
                ) : (
                  <div className="flex gap-5">
                    <span className="cap2">{t("无流量信息")}</span>
                    <span className="cap2">{t("更新于 {time}", { time: relativeTime(p.updated_at) })}</span>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
