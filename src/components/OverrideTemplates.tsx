import { useRef, useState } from "react";
import { useT } from "../i18n";
import { overrideCreate, overrideDelete, overrideWrite, type OverrideMeta } from "../lib/core";
import { overrideTemplates, templateYaml } from "../lib/override-templates";

export default function OverrideTemplates({ onCreated, onClose, onBusyChange }: {
  onCreated: (meta: OverrideMeta, content: string) => void;
  onClose: () => void;
  onBusyChange?: (busy: boolean) => void;
}) {
  const t = useT();
  const [selected, setSelected] = useState<string>("suffix");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const template = overrideTemplates.find(item => item.id === selected)!;
  const yaml = templateYaml(template.rule);
  const create = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    onBusyChange?.(true);
    setError(null);
    let meta: OverrideMeta | undefined;
    try {
      meta = await overrideCreate(name.trim() || t(template.name), "yaml", false);
      await overrideWrite(meta.id, yaml);
    } catch (e) {
      let message = String(e);
      if (meta) {
        try { await overrideDelete(meta.id); }
        catch (cleanupError) { message += `\n${t("清理失败，请在列表中删除本次停用的覆写。")} ${String(cleanupError)}`; }
      }
      setError(message);
      busyRef.current = false;
      setBusy(false);
      onBusyChange?.(false);
      return;
    }
    busyRef.current = false;
    setBusy(false);
    onBusyChange?.(false);
    onCreated(meta!, yaml);
  };
  return (
    <section aria-label={t("常用模板")} className="mt-5 min-w-0 shrink-0 border-y border-line py-4">
      <div className="flex items-center justify-between gap-4">
        <h2 className="text-[14px] font-medium">{t("常用模板")}</h2>
        <button disabled={busy} onClick={onClose} className="text-[13px] text-mut disabled:opacity-40">{t("取消")}</button>
      </div>
      <div className="mt-3 flex flex-col gap-3 md:flex-row">
        <label className="flex min-w-0 flex-1 flex-col gap-1 text-[12px] text-mut">
          {t("规则类型")}
          <select disabled={busy} value={selected} onChange={e => setSelected(e.target.value)} className="min-w-0 rounded border border-line bg-bg p-2 text-[13px] text-fg">
            {overrideTemplates.map(item => <option key={item.id} value={item.id}>{t(item.name)}</option>)}
          </select>
        </label>
        <label className="flex min-w-0 flex-1 flex-col gap-1 text-[12px] text-mut">
          {t("覆写名称")}
          <input disabled={busy} value={name} onChange={e => setName(e.target.value)} placeholder={t(template.name)} className="min-w-0 rounded border border-line bg-transparent p-2 text-[13px] text-fg outline-none" />
        </label>
      </div>
      <p className="mt-3 text-[13px] text-mut">{t(template.description)}</p>
      <pre className="mt-3 overflow-x-auto border-l-2 border-accent py-2 pl-3 text-[12px] text-fg select-text">{yaml}</pre>
      <p className="mt-3 text-[12px] leading-relaxed text-mut">{t("规则前置且保留订阅规则；仅规则模式生效。DIRECT 为直连，可在创建后改为 REJECT 或实际策略组名称。")}</p>
      <p className="mt-1 text-[12px] leading-relaxed text-mut">{t("模板创建后默认停用；请在编辑器修改示例并保存，再返回列表手动启用。")}</p>
      {error && <p role="alert" className="mt-3 whitespace-pre-wrap break-words text-[13px] text-err select-text">{error}</p>}
      <button disabled={busy} onClick={() => void create()} className="mt-4 text-[13px] font-medium text-accent disabled:opacity-40">{busy ? t("创建中…") : t("创建模板")}</button>
    </section>
  );
}
