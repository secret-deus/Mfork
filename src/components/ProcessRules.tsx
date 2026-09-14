import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../store";
import { apiGet } from "../lib/core";
import type { ProxyGroup } from "../lib/types";
import { normalizeProcessRules, type ProcessRule } from "../lib/process-rules";
import { useT } from "../i18n";

interface Draft { originalName: string | null; process_name: string; target: string }
const field = "min-w-0 border-b border-line-strong bg-transparent py-2 text-[13px] text-fg outline-none focus:border-accent disabled:opacity-40";
const action = "shrink-0 py-2 text-[12px] text-mut hover:text-fg disabled:opacity-40";

/** Desktop-only editor. Mounting/reading this section never changes core configuration. */
export default function ProcessRules({ onApplied }: { onApplied?: () => Promise<void> }) {
  const t = useT();
  const { settings, api, phase, mode, snapshot } = useApp();
  const rules = settings?.process_rules ?? [];
  const [open, setOpen] = useState(true);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [groups, setGroups] = useState<string[]>([]);
  const [groupError, setGroupError] = useState("");
  const [error, setError] = useState("");
  const [applyError, setApplyError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  useEffect(() => {
    let cancelled = false;
    setGroups([]); setGroupError("");
    if (phase === "running" && api) {
      void apiGet<{ proxies: ProxyGroup[] }>(api, "/group").then(result => {
        if (!cancelled) setGroups(result.proxies.map(group => group.name).filter(name => name && name.toUpperCase() !== "GLOBAL"));
      }).catch(reason => { if (!cancelled) setGroupError(String(reason)); });
    }
    return () => { cancelled = true; };
  }, [phase, api]);

  const targets = useMemo(() => [...new Set([...groups, "DIRECT", "REJECT", ...rules.map(rule => rule.target)])], [groups, rules]);
  const processes = useMemo(() => [...new Set((snapshot?.connections ?? []).map(connection => connection.metadata.process).filter((name): name is string => !!name))], [snapshot]);
  const latestRules = () => useApp.getState().settings?.process_rules ?? [];
  const failure = (message: string) => { if (alive.current) setError(message); };

  async function applySaved(): Promise<boolean> {
    const state = useApp.getState();
    if (state.phase !== "running" || !state.api) return false;
    if (!await state.reloadConfig()) throw new Error(t("内核状态已变化，请重试应用"));
    // Refresh the read-only list after the core accepted the new configuration.
    await onApplied?.();
    return true;
  }

  /** true means persisted, even if applying to the currently running core failed. */
  async function persist(next: ProcessRule[]): Promise<boolean> {
    if (locked.current || !useApp.getState().settings) return false;
    const validated = normalizeProcessRules(next);
    if (!validated.ok) { failure(t(validated.error.key, validated.error.params)); return false; }
    locked.current = true; setBusy(true); setError(""); setNotice("");
    try {
      try { await useApp.getState().updateSettings({ process_rules: validated.rules }); }
      catch (reason) { failure(t("保存进程规则失败：{msg}", { msg: String(reason) })); return false; }
      if (alive.current) setApplyError("");
      try {
        const applied = await applySaved();
        const message = applied ? t("进程规则已应用，仅影响新连接") : t("进程规则已保存，下次启动内核时生效");
        if (alive.current) setNotice(message);
        useApp.getState().notify(message, false);
      } catch (reason) {
        const message = t("进程规则已保存，但应用失败：{msg}", { msg: String(reason) });
        if (alive.current) setApplyError(message);
        useApp.getState().notify(message);
      }
      return true;
    } finally {
      locked.current = false;
      if (alive.current) setBusy(false);
    }
  }

  async function retry() {
    if (locked.current) return;
    locked.current = true; setBusy(true); setNotice("");
    try {
      const applied = await applySaved();
      if (!applied) throw new Error(t("内核状态已变化，请重试应用"));
      if (alive.current) { setApplyError(""); setNotice(t("进程规则已应用，仅影响新连接")); }
    } catch (reason) {
      if (alive.current) setApplyError(t("进程规则已保存，但应用失败：{msg}", { msg: String(reason) }));
    } finally { locked.current = false; if (alive.current) setBusy(false); }
  }

  async function submit() {
    if (!draft || locked.current) return;
    const current = latestRules();
    let next: ProcessRule[];
    if (draft.originalName === null) next = [...current, { process_name: draft.process_name, target: draft.target, enabled: true }];
    else {
      const index = current.findIndex(rule => rule.process_name === draft.originalName);
      if (index < 0) { failure(t("规则已变化，请取消后重新编辑")); return; }
      next = current.map((rule, i) => i === index ? { ...rule, process_name: draft.process_name, target: draft.target } : rule);
    }
    // Close once saved, including apply failures: do not accidentally add the same rule twice.
    if (await persist(next) && alive.current) setDraft(null);
  }

  async function remove(name: string) {
    if (locked.current || !window.confirm(t("删除进程「{name}」的规则？", { name }))) return;
    const current = latestRules();
    if (!current.some(rule => rule.process_name === name)) { failure(t("规则已变化，请取消后重新编辑")); return; }
    await persist(current.filter(rule => rule.process_name !== name));
  }

  async function toggle(name: string) {
    if (locked.current) return;
    const current = latestRules();
    if (!current.some(rule => rule.process_name === name)) { failure(t("规则已变化，请取消后重新编辑")); return; }
    await persist(current.map(rule => rule.process_name === name ? { ...rule, enabled: !rule.enabled } : rule));
  }

  return (
    <section className="process-rules mt-5 shrink-0 border-y border-line-soft" aria-label={t("进程名分流")}>
      <div className="flex items-center justify-between gap-4 py-2">
        <button type="button" onClick={() => setOpen(value => !value)} aria-expanded={open} className="flex min-w-0 items-baseline gap-3 py-1 text-left">
          <h2 className="text-[14px] font-medium">{t("进程名分流")}</h2>
          <span className="text-[11px] text-dim">{rules.length}　{open ? "−" : "+"}</span>
        </button>
        <button type="button" className={`${action} !text-accent`} disabled={busy || !!draft || !settings} onClick={() => { setOpen(true); setError(""); setDraft({ originalName: null, process_name: "", target: groups[0] ?? "DIRECT" }); }}>
          {t("添加进程规则")}
        </button>
      </div>
      {open ? <div className="max-h-[46vh] overflow-y-auto pb-3">
        <p className="mb-2 text-[11px] leading-relaxed text-mut">{t("按进程名精确匹配，同名进程全部适用；只影响新连接。")}</p>
        {mode !== "rule" ? <p className="mb-2 text-[11px] text-warn" role="status">{t("当前不是规则模式，进程分流不会生效；请切换到规则模式。")}</p> : null}
        {!settings?.tun.enable ? <p className="mb-2 text-[11px] leading-relaxed text-dim">{t("仅处理进入 mihomo 的流量，建议开启 TUN；系统代理无法接管所有程序。")}</p> : null}
        {phase !== "running" ? <p className="mb-2 text-[11px] text-mut">{t("内核未运行，可手动填写策略组名称，保存后将在下次启动生效。")}</p> : null}
        {groupError ? <p className="mb-2 break-all text-[11px] text-warn">{t("读取策略组失败：{msg}", { msg: groupError })}</p> : null}
        {draft ? <form onSubmit={event => { event.preventDefault(); void submit(); }} className="mb-3 flex flex-wrap items-end gap-3 border-b border-line-soft pb-3">
          <label className="flex min-w-0 flex-[1_1_160px] flex-col gap-1 text-[11px] text-mut">
            {t("进程名")}
            <input autoFocus aria-label={t("进程名")} list="process-name-suggestions" placeholder="chrome.exe" className={field} value={draft.process_name} disabled={busy} onChange={event => setDraft({ ...draft, process_name: event.target.value })} />
          </label>
          <label className="flex min-w-0 flex-[1_1_160px] flex-col gap-1 text-[11px] text-mut">
            {t("目标策略")}
            <input aria-label={t("目标策略")} list="process-target-suggestions" className={field} value={draft.target} disabled={busy} onChange={event => setDraft({ ...draft, target: event.target.value })} />
          </label>
          <datalist id="process-name-suggestions">{processes.map(name => <option key={name} value={name} />)}</datalist>
          <datalist id="process-target-suggestions">{targets.map(name => <option key={name} value={name} />)}</datalist>
          <button type="submit" className={`${action} !text-accent`} disabled={busy}>{busy ? t("保存中…") : t("保存规则")}</button>
          <button type="button" className={action} disabled={busy} onClick={() => { setDraft(null); setError(""); }}>{t("取消")}</button>
        </form> : null}
        {error ? <p className="mb-2 break-all text-[12px] text-err" role="alert">{error}</p> : null}
        {applyError ? <div className="mb-2 text-[12px] text-warn" role="alert"><p className="break-all">{applyError}</p><button type="button" className={`${action} !text-accent`} disabled={busy || phase !== "running" || !api} onClick={() => void retry()}>{t("重试应用")}</button></div> : null}
        {notice ? <p className="mb-2 text-[11px] text-ok" role="status">{notice}</p> : null}
        {rules.length === 0 && !draft ? <p className="py-2 text-[12px] text-dim">{t("暂无进程规则，现有订阅规则保持不变。")}</p> : null}
        {rules.map(rule => <div key={rule.process_name} data-process-name={rule.process_name} className="flex flex-wrap items-center gap-3 border-b border-line-faint py-2 last:border-b-0">
          <button type="button" role="switch" aria-checked={rule.enabled} aria-label={t("进程规则开关：{name}", { name: rule.process_name })} disabled={busy || !!draft} onClick={() => void toggle(rule.process_name)} className={`relative h-[18px] w-[34px] shrink-0 rounded-full ${rule.enabled ? "bg-accent" : "bg-toggle-off"} disabled:opacity-40`}>
            <span className={`absolute top-[2px] h-[14px] w-[14px] rounded-full bg-knob ${rule.enabled ? "left-[18px]" : "left-[2px]"}`} />
          </button>
          <span className={`min-w-0 flex-[1_1_140px] truncate font-sg text-[13px] ${rule.enabled ? "text-fg" : "text-dim"}`} title={rule.process_name}>{rule.process_name}</span>
          <span className="min-w-0 flex-[1_1_140px] truncate text-[12px] text-mut" title={rule.target}>→ {rule.target}</span>
          <button type="button" className={action} disabled={busy || !!draft} aria-label={t("编辑进程规则：{name}", { name: rule.process_name })} onClick={() => { setError(""); setDraft({ originalName: rule.process_name, process_name: rule.process_name, target: rule.target }); }}>{t("编辑")}</button>
          <button type="button" className={`${action} hover:!text-err`} disabled={busy || !!draft} aria-label={t("删除进程规则：{name}", { name: rule.process_name })} onClick={() => void remove(rule.process_name)}>{t("删除")}</button>
        </div>)}
      </div> : null}
    </section>
  );
}
