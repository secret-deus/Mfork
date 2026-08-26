import Editor, { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/editor/editor.worker.js?worker";
import tsWorker from "monaco-editor/language/typescript/ts.worker.js?worker";

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

export default function CodeEditor({
  language,
  value,
  onChange,
}: {
  language: "yaml" | "javascript";
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <Editor
      language={language}
      theme="mfork-dark"
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
