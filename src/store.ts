import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import {
  ApiInfo,
  AppSettings,
  apiGet,
  apiPatch,
  apiPut,
  autostartGet,
  configRegenerate,
  autostartSet,
  coreStart,
  coreStatus,
  coreStop,
  settingsGet,
  settingsSet,
  systemProxySet,
  traySync,
  vpnPrepare,
  wsUrl,
} from "./lib/core";
import { IS_ANDROID } from "./lib/platform";
import { ConnectionsSnapshot, ConnItem, LogLine } from "./lib/types";
import { isLocale, t, useLocale } from "./i18n";
import { applyTheme } from "./theme";

export type CorePhase = "stopped" | "starting" | "running" | "error";
export type ProxyMode = "rule" | "global" | "direct";

/** Transient message shown by the global toast (App.tsx). */
export interface Notice {
  id: number;
  text: string;
  err: boolean;
}

interface AppState {
  phase: CorePhase;
  api: ApiInfo | null;
  error: string | null;
  version: string | null;
  mode: ProxyMode | null;
  up: number;
  down: number;
  memory: number;
  connections: number;
  snapshot: ConnectionsSnapshot | null;
  logs: LogLine[];
  logsPaused: boolean;
  settings: AppSettings | null;
  autostart: boolean;
  notice: Notice | null;
  /** show a toast; errors stay a little longer */
  notify: (text: string, err?: boolean) => void;
  dismissNotice: () => void;
  setLogsPaused: (paused: boolean) => void;
  clearLogs: () => void;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  setMode: (mode: ProxyMode) => Promise<void>;
  syncConfig: () => Promise<void>;
  setSystemProxy: (enable: boolean) => Promise<void>;
  updateSettings: (patch: Partial<AppSettings>) => Promise<void>;
  /**
   * Persist a settings patch and make the running core pick it up: "hot"
   * regenerates config.yaml and reloads it in place (PUT /configs), which
   * keeps existing connections; "restart" is for changes the core cannot
   * apply live (mixed port, entering/leaving TUN mode).
   */
  applySettings: (patch: Partial<AppSettings>, how: "hot" | "restart") => Promise<void>;
  /** regenerate config.yaml from the current profile + settings and hot-reload it */
  reloadConfig: () => Promise<void>;
  /** settings changed on disk behind our back (backup restore): reload */
  reloadSettings: () => Promise<void>;
  restart: () => Promise<void>;
  setAutostart: (enable: boolean) => Promise<void>;
  init: () => Promise<void>;
}

let sockets: WebSocket[] = [];

// log buffer: batched into the store every 250ms so chatty debug output
// doesn't trigger a re-render per line
let logSeq = 0;
let logBuf: LogLine[] = [];
let logFlush: number | null = null;
const LOG_CAP = 1000;

function closeStreams() {
  for (const ws of sockets) ws.close();
  sockets = [];
  if (logFlush != null) {
    clearTimeout(logFlush);
    logFlush = null;
  }
  logBuf = [];
}

function openStreams(
  api: ApiInfo,
  set: (partial: Partial<AppState>) => void,
  get: () => AppState,
) {
  closeStreams();
  const logs = new WebSocket(wsUrl(api, "/logs") + "&level=debug");
  logs.onmessage = (e) => {
    if (get().logsPaused) return;
    const d = JSON.parse(e.data);
    logBuf.push({ type: d.type, payload: d.payload, seq: logSeq++, at: Date.now() });
    if (logFlush == null) {
      logFlush = window.setTimeout(() => {
        logFlush = null;
        const merged = [...get().logs, ...logBuf];
        logBuf = [];
        set({
          logs: merged.length > LOG_CAP ? merged.slice(merged.length - LOG_CAP) : merged,
        });
      }, 250);
    }
  };
  sockets.push(logs);
  const traffic = new WebSocket(wsUrl(api, "/traffic"));
  traffic.onmessage = (e) => {
    const d = JSON.parse(e.data);
    set({ up: d.up, down: d.down });
  };
  const memory = new WebSocket(wsUrl(api, "/memory"));
  memory.onmessage = (e) => {
    const d = JSON.parse(e.data);
    set({ memory: d.inuse });
  };
  let prev = new Map<string, { up: number; down: number }>();
  const conns = new WebSocket(wsUrl(api, "/connections"));
  conns.onmessage = (e) => {
    const d = JSON.parse(e.data);
    const raw: Omit<ConnItem, "upSpeed" | "downSpeed">[] = Array.isArray(
      d.connections,
    )
      ? d.connections
      : [];
    const next = new Map<string, { up: number; down: number }>();
    const items: ConnItem[] = raw.map((c) => {
      const p = prev.get(c.id);
      next.set(c.id, { up: c.upload, down: c.download });
      return {
        ...c,
        upSpeed: p ? Math.max(0, c.upload - p.up) : 0,
        downSpeed: p ? Math.max(0, c.download - p.down) : 0,
      };
    });
    prev = next;
    set({
      connections: items.length,
      snapshot: {
        downloadTotal: d.downloadTotal ?? 0,
        uploadTotal: d.uploadTotal ?? 0,
        connections: items,
      },
    });
  };
  sockets.push(traffic, memory, conns);
}

let initialized = false;

// start / stop / restart are serialised: two overlapping restarts used to
// spawn two cores and then kill both (core_start releases its lock before
// the API port is up). A restart requested while one is still *queued* is
// coalesced into it; one requested while a restart is already running is
// queued behind it so a settings change made mid-restart is not lost.
let lifecycle: Promise<void> = Promise.resolve();
let queuedRestart: Promise<void> | null = null;
function serialize(fn: () => Promise<void>): Promise<void> {
  const next = lifecycle.then(fn, fn);
  lifecycle = next.catch(() => {});
  return next;
}

let noticeTimer: number | null = null;

/** Mirror persisted appearance settings into the DOM / i18n store. */
function applyAppearance(s: AppSettings) {
  applyTheme(s.theme);
  if (isLocale(s.locale)) useLocale.getState().setLocale(s.locale);
}

/** Reload a regenerated config in place: connections survive, listeners are re-created as needed. */
async function hotReload(api: ApiInfo, path: string) {
  await apiPut(api, "/configs?force=true", { path });
}

export const useApp = create<AppState>((set, get) => {
  const startImpl = async () => {
    set({ phase: "starting", error: null });
    try {
      if (IS_ANDROID) {
        const s = get().settings ?? (await settingsGet());
        if (s.tun.enable && !(await vpnPrepare())) {
          throw new Error(t("未获得 VPN 授权，无法启用 TUN"));
        }
      }
      const api = await coreStart();
      const version = await apiGet<{ version: string }>(api, "/version");
      const configs = await apiGet<{ mode: string }>(api, "/configs");
      openStreams(api, set, get);
      const mode = configs.mode.toLowerCase() as ProxyMode;
      set({ phase: "running", api, version: version.version, mode });
      void traySync(mode);
    } catch (e) {
      set({ phase: "error", error: String(e), api: null });
      void traySync(null);
    }
  };

  const stopImpl = async () => {
    closeStreams();
    await coreStop();
    set({
      phase: "stopped",
      api: null,
      up: 0,
      down: 0,
      memory: 0,
      connections: 0,
      snapshot: null,
    });
    void traySync(null);
  };

  return {
  phase: "stopped",
  api: null,
  error: null,
  version: null,
  mode: null,
  up: 0,
  down: 0,
  memory: 0,
  connections: 0,
  snapshot: null,
  logs: [],
  logsPaused: false,
  settings: null,
  autostart: false,
  notice: null,

  notify(text, err = true) {
    const id = Date.now() + Math.random();
    set({ notice: { id, text, err } });
    if (noticeTimer != null) clearTimeout(noticeTimer);
    noticeTimer = window.setTimeout(() => {
      noticeTimer = null;
      if (get().notice?.id === id) set({ notice: null });
    }, err ? 8000 : 4000);
  },

  dismissNotice() {
    set({ notice: null });
  },

  setLogsPaused(paused) {
    set({ logsPaused: paused });
  },

  clearLogs() {
    logBuf = [];
    set({ logs: [] });
  },

  start: () => serialize(startImpl),

  stop: () => serialize(stopImpl),

  restart() {
    if (queuedRestart) return queuedRestart;
    const p = serialize(async () => {
      queuedRestart = null;
      if (get().phase === "running") await stopImpl();
      await startImpl();
    });
    queuedRestart = p;
    return p;
  },

  async setMode(mode) {
    const { api } = get();
    if (!api) return;
    await apiPatch(api, "/configs", { mode });
    set({ mode });
    void traySync(mode);
  },

  async setSystemProxy(enable) {
    try {
      await systemProxySet(enable);
      const s = get().settings;
      if (s) set({ settings: { ...s, system_proxy: enable } });
    } catch (e) {
      get().notify(String(e));
    }
  },

  async updateSettings(patch) {
    const cur = get().settings ?? (await settingsGet());
    const next = { ...cur, ...patch };
    set({ settings: next });
    applyAppearance(next);
    try {
      await settingsSet(next);
    } catch (e) {
      // the file on disk still holds `cur`: put the UI back to match it
      set({ settings: cur });
      applyAppearance(cur);
      get().notify(t("保存设置失败: {msg}", { msg: String(e) }));
      throw e;
    }
  },

  async reloadSettings() {
    const s = await settingsGet();
    set({ settings: s });
    applyAppearance(s);
  },

  async applySettings(patch, how) {
    try {
      await get().updateSettings(patch);
    } catch {
      return; // already rolled back + reported
    }
    if (get().phase !== "running") return;
    try {
      if (how === "restart") await get().restart();
      else await get().reloadConfig();
    } catch (e) {
      get().notify(t("设置已保存，但应用到内核失败: {msg}", { msg: String(e) }));
    }
  },

  async reloadConfig() {
    const { api } = get();
    if (!api) return;
    const path = await configRegenerate();
    if (!path) return;
    await hotReload(api, path);
    await get().syncConfig();
  },

  async setAutostart(enable) {
    await autostartSet(enable);
    set({ autostart: enable });
  },

  async syncConfig() {
    const { api } = get();
    if (!api) return;
    const configs = await apiGet<{ mode: string }>(api, "/configs");
    set({ mode: configs.mode.toLowerCase() as ProxyMode });
  },

  async init() {
    if (initialized) return;
    initialized = true;
    // system settings + autostart state
    void settingsGet().then((s) => {
      set({ settings: s });
      applyAppearance(s);
    });
    void autostartGet().then((a) => set({ autostart: a }));
    // tray menu actions
    await listen<string>("tray-mode", (e) => {
      void get().setMode(e.payload as ProxyMode);
    });
    await listen<boolean>("sys-proxy-changed", (e) => {
      const s = get().settings;
      if (s) set({ settings: { ...s, system_proxy: e.payload } });
    });
    await listen<string>("sys-proxy-error", (e) => {
      get().notify(e.payload);
    });
    // Android: the system (or another VPN app) revoked our VPN
    if (IS_ANDROID) {
      window.addEventListener("mfork-vpn-revoked", () => {
        get().notify(t("VPN 已被系统断开，内核已停止"));
        void get().stop();
      });
    }
    // app-config.json failed to parse: the backend moved it aside
    await listen<string>("settings-corrupt", (e) => {
      get().notify(t("设置文件损坏，已恢复默认：{msg}", { msg: e.payload }));
      void get().reloadSettings();
    });
    // auto-updated subscriptions: hot-reload the regenerated config
    await listen<string | null>("profiles-auto-updated", (e) => {
      const { api } = get();
      if (e.payload && api) {
        void hotReload(api, e.payload)
          .then(() => get().syncConfig())
          .catch((err) => get().notify(String(err)));
      }
    });
    await listen<number | null>("core-exited", (e) => {
      closeStreams();
      void traySync(null);
      const msg = t("内核进程意外退出（code {code}）", { code: e.payload ?? "?" });
      set({
        phase: "error",
        api: null,
        error: msg,
        up: 0,
        down: 0,
        memory: 0,
        connections: 0,
        snapshot: null,
      });
      get().notify(msg);
    });
    const status = await coreStatus();
    if (status.running && status.api) {
      const version = await apiGet<{ version: string }>(status.api, "/version");
      const configs = await apiGet<{ mode: string }>(status.api, "/configs");
      openStreams(status.api, set, get);
      const mode = configs.mode.toLowerCase() as ProxyMode;
      set({
        phase: "running",
        api: status.api,
        version: version.version,
        mode,
      });
      void traySync(mode);
    } else {
      // auto-start on launch
      await get().start();
    }
  },
  };
});
