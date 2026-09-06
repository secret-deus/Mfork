import Editor, { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/editor/editor.worker.js?worker";
import tsWorker from "monaco-editor/language/typescript/ts.worker.js?worker";
import { useEffect, useState } from "react";
import { onThemeChange, resolvedTheme } from "../theme";

// bundle monaco locally (no CDN — the app must work offline)
self.MonacoEnvironment = {
  getWorker: (_: unknown, label: string) =>
    label === "typescript" || label === "javascript" ? new tsWorker() : new editorWorker(),
};
loader.config({ monaco });

monaco.editor.defineTheme("mfork-dark", {
  base: "vs-dark",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": "#0b0c0f",
    "editor.lineHighlightBackground": "#121317",
    "editorLineNumber.foreground": "#3a3f47",
    "editorLineNumber.activeForeground": "#8b8f97",
    "editorGutter.background": "#0b0c0f",
    "editorWidget.background": "#121317",
    "editorIndentGuide.background1": "#1c1f24",
  },
});

monaco.editor.defineTheme("mfork-light", {
  base: "vs",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": "#f7f7f8",
    "editor.lineHighlightBackground": "#eeeff2",
    "editorLineNumber.foreground": "#c8cbd1",
    "editorLineNumber.activeForeground": "#6b7079",
    "editorGutter.background": "#f7f7f8",
    "editorWidget.background": "#ffffff",
    "editorIndentGuide.background1": "#dcdee2",
  },
});

export default function CodeEditor({
  language,
  value,
  onChange,
}: {
  language: "yaml" | "javascript";
  value: string;
  onChange: (v: string) => void;
}) {
  const [theme, setTheme] = useState(resolvedTheme());
  useEffect(() => onThemeChange(setTheme), []);
  return (
    <Editor
      language={language}
      theme={theme === "light" ? "mfork-light" : "mfork-dark"}
      value={value}
      onChange={(v) => onChange(v ?? "")}
      options={{
        minimap: { enabled: false },
        fontSize: 12.5,
        fontFamily: '"Cascadia Code", Consolas, monospace',
        scrollBeyondLastLine: false,
        renderLineHighlight: "all",
        padding: { top: 12 },
        automaticLayout: true,
        tabSize: 2,
      }}
    />
  );
}
