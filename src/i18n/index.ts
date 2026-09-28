// Lightweight i18n: the source language is zh-CN and the *source string is
// the key*. `t("启动内核")` looks the string up in the active dictionary and
// falls back to the key itself, so untranslated strings stay readable.
// Placeholders use `{name}`: t("已运行 {h} 小时", { h }).
import { useCallback } from "react";
import { create } from "zustand";
import en from "./en";
import zhCN from "./zh-CN";
import zhTW from "./zh-TW";

export type Locale = "zh-CN" | "zh-TW" | "en";

export const LOCALES: { value: Locale; label: string }[] = [
  { value: "zh-CN", label: "简体中文" },
  { value: "zh-TW", label: "繁體中文" },
  { value: "en", label: "English" },
];

const dicts: Record<Locale, Record<string, string>> = {
  "zh-CN": zhCN,
  "zh-TW": zhTW,
  en,
};

interface LocaleState {
  locale: Locale;
  setLocale: (l: Locale) => void;
}

export const useLocale = create<LocaleState>((set) => ({
  locale: "zh-CN",
  setLocale(locale) {
    document.documentElement.lang = locale;
    set({ locale });
  },
}));

export type Params = Record<string, string | number>;

export function t(key: string, params?: Params): string {
  const dict = dicts[useLocale.getState().locale];
  let s = dict[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      s = s.split(`{${k}}`).join(String(v));
    }
  }
  return s;
}

/** Component hook: re-renders on locale change. */
export function useT() {
  const locale = useLocale((s) => s.locale);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useCallback((key: string, params?: Params) => t(key, params), [locale]);
}

export function isLocale(v: string): v is Locale {
  return v === "zh-CN" || v === "zh-TW" || v === "en";
}
