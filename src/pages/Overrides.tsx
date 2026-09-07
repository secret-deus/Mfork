import { useEffect, useState } from "react";
import { useApp } from "../store";
import {
  OverrideMeta,
  apiPut,
  configRegenerate,
  overrideCreate,
  overrideDelete,
  overrideMove,
  overrideRead,
  overrideToggle,
  overrideWrite,
  overridesList,
} from "../lib/core";
import CodeEditor from "../components/CodeEditor";
import { useT } from "../i18n";

export default function Overrides() {
  const t = useT();
  const { api, syncConfig } = useApp();
  const [list, setList] = useState<OverrideMeta[]>([]);
  const [editing, setEditing] = useState<OverrideMeta | null>(null);
  const [content, setContent] = useState("");
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [newKind, setNewKind] = useState<"yaml" | "js">("yaml");

  const refresh = async () => setList(await overridesList());
  useEffect(() => {
    void refresh();
  }, []);

  const applyToKernel = async () => {
    const path = await configRegenerate();
    if (path && api) {
      await apiPut(api, "/configs?force=true", { path });
      await syncConfig();
    }
  };

  const openEditor = async (o: OverrideMeta) => {
    setContent(await overrideRead(o.id));
    setEditing(o);
    setDirty(false);
    setError(null);
  };

  const save = async () => {
    if (!editing) return;
    setError(null);
    try {
      await overrideWrite(editing.id, content);
      setDirty(false);
      if (editing.enabled) await applyToKernel();
    } catch (e) {
      setError(String(e));
    }
  };

  const act = async (fn: () => Promise<unknown>, reload = true) => {
    setError(null);
    try {
      await fn();
      await refresh();
      if (reload) await applyToKernel();
    } catch (e) {
      setError(String(e));
    }
  };

  if (editing) {
    return (
      <div className="flex h-full min-w-0 flex-col pt-13">
        <div className="flex items-center justify-between px-14">
          <div className="flex items-baseline gap-4">
            <button
              onClick={() => setEditing(null)}
              className="text-[13px] text-mut hover:text-fg"
            >
              {t("← 返回")}
            </button>
            <span className="text-[16px] font-semibold">{editing.name}</span>
            <span className="cap2">{editing.kind}</span>
            {dirty ? <span className="cap2 !text-warn">{t("未保存")}</span> : null}
          </div>
          <button
            onClick={() => void save()}
            disabled={!dirty}
            className="text-[13px] font-semibold tracking-[1px] text-accent hover:text-fg disabled:opacity-40"
          >
            {t("保存并应用")}
          </button>
        </div>
        {error ? (
          <div className="mt-2 px-14 text-[13px] text-err select-text">{error}</div>
        ) : null}
        <div className="mt-4 min-h-0 flex-1 border-t border-line">
          <CodeEditor
            language={editing.kind === "js" ? "javascript" : "yaml"}
            value={content}
            onChange={(v) => {
              setContent(v);
              setDirty(true);
            }}
          />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full min-w-0 flex-col px-14 pb-8 pt-13">
      <div className="flex items-center justify-between">
        <div className="cap">{t("Mfork · 覆写 Overrides")}</div>
        <button
          onClick={() => setCreating(!creating)}
          className="text-[13px] text-accent hover:text-fg"
        >
          {t("新建覆写")}
        </button>
      </div>
      <div className="mt-2 cap2">{t("按列表顺序依次应用到当前订阅；改动保存后即时生效")}</div>

      {creating ? (
        <div className="mt-5 flex items-center gap-4 border-b border-line-strong pb-3">
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder={t("覆写名称")}
            className="flex-1 bg-transparent text-[14px] outline-none placeholder:text-dim"
          />
          <div className="flex gap-3">
            {(["yaml", "js"] as const).map((k) => (
              <button
                key={k}
                onClick={() => setNewKind(k)}
                className={`border-b-2 pb-0.5 text-[13px] uppercase tracking-[1px] ${
                  newKind === k ? "border-accent text-fg" : "border-transparent text-dim"
                }`}
              >
                {k}
              </button>
            ))}
          </div>
          <button
            onClick={() =>
              void act(async () => {
                const meta = await overrideCreate(newName.trim() || t("未命名覆写"), newKind);
                setCreating(false);
                setNewName("");
                await openEditor(meta);
              }, false)
            }
            className="text-[13px] font-semibold text-accent hover:text-fg"
          >
            {t("创建")}
          </button>
        </div>
      ) : null}

      {error ? <div className="mt-3 text-[13px] text-err select-text">{error}</div> : null}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {list.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2">
            <span className="cap">{t("尚无覆写")}</span>
            <span className="cap2">{t("YAML 深合并 或 JS main(config) 变换")}</span>
          </div>
        ) : (
          list.map((o, i) => (
            <div
              key={o.id}
              className="flex items-center justify-between border-b border-line-soft py-4"
            >
              <div className="flex items-center gap-3.5">
                <button
                  onClick={() => void act(() => overrideToggle(o.id, !o.enabled))}
                  className={`h-[17px] w-[30px] rounded-[9px] transition-colors ${
                    o.enabled ? "bg-accent" : "bg-toggle-off"
                  } relative`}
                  title={o.enabled ? t("已启用") : t("已停用")}
                >
                  <span
                    className={`absolute top-[2.5px] block h-3 w-3 rounded-full transition-all ${
                      o.enabled ? "left-[15px] bg-knob-on" : "left-[3px] bg-knob"
                    }`}
                  />
                </button>
                <button
                  onClick={() => void openEditor(o)}
                  className={`text-[15px] font-semibold ${o.enabled ? "" : "text-dim"} hover:text-accent`}
                >
                  {o.name}
                </button>
                <span className="cap2">{o.kind}</span>
              </div>
              <div className="flex items-center gap-5">
                <button
                  onClick={() => void act(() => overrideMove(o.id, true))}
                  disabled={i === 0}
                  className="text-dim hover:text-fg disabled:opacity-30"
                  title={t("上移")}
                >
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m6 15 6-6 6 6" /></svg>
                </button>
                <button
                  onClick={() => void act(() => overrideMove(o.id, false))}
                  disabled={i === list.length - 1}
                  className="text-dim hover:text-fg disabled:opacity-30"
                  title={t("下移")}
                >
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m6 9 6 6 6-6" /></svg>
                </button>
                <button
                  onClick={() => void openEditor(o)}
                  className="text-[13px] text-mut hover:text-fg"
                >
                  {t("编辑")}
                </button>
                <button
                  onClick={() => {
                    if (window.confirm(t("删除覆写「{name}」？", { name: o.name }))) {
                      void act(() => overrideDelete(o.id));
                    }
                  }}
                  className="text-[13px] text-dim hover:text-err"
                >
                  {t("删除")}
                </button>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
