// Theme switching: dark (default) / light / follow system. The palette
// lives in index.css as CSS variables keyed off `data-theme` on <html>.
export type Theme = "dark" | "light" | "system";

export const THEMES: { value: Theme; label: string }[] = [
  { value: "dark", label: "深色" },
  { value: "light", label: "浅色" },
  { value: "system", label: "跟随系统" },
];

const media = window.matchMedia("(prefers-color-scheme: light)");
let current: Theme = "dark";
const listeners = new Set<(resolved: "dark" | "light") => void>();

export function resolvedTheme(theme: Theme = current): "dark" | "light" {
  if (theme === "system") return media.matches ? "light" : "dark";
  return theme;
}

function paint() {
  const r = resolvedTheme();
  document.documentElement.dataset.theme = r;
  document.documentElement.style.colorScheme = r;
  for (const l of listeners) l(r);
}

export function applyTheme(theme: string) {
  current = theme === "light" || theme === "system" ? theme : "dark";
  paint();
}

/** Subscribe to the resolved (dark/light) theme; returns an unsubscribe. */
export function onThemeChange(fn: (resolved: "dark" | "light") => void) {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

media.addEventListener("change", () => {
  if (current === "system") paint();
});

export function isTheme(v: string): v is Theme {
  return v === "dark" || v === "light" || v === "system";
}
