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
  profileSelect,
  settingsGet,
  settingsSet,
  systemProxySet,
  traySync,
  vpnPrepare,
  subscribeCoreStream,
  reconcileCoreSession,
} from "./lib/core";
import { IS_ANDROID } from "./lib/platform";
import { ConnectionsSnapshot, ConnItem, LogLine } from "./lib/types";
import { isLocale, t, useLocale } from "./i18n";
import { applyTheme } from "./theme";
import { applyPreservingMode, confirmedMode, createSerialQueue } from "./lib/core-queue";

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
  /** Returns true only when the same running core accepted the regenerated config. */
  reloadConfig: () => Promise<boolean>;
  selectProfile: (id: string) => Promise<void>;
  configPending: boolean;
  markConfigPending: () => void;
  /** settings changed on disk behind our back (backup restore): reload */
  reloadSettings: () => Promise<void>;
  restart: () => Promise<void>;
  setAutostart: (enable: boolean) => Promise<void>;
  init: () => Promise<void>;
}

let streams: (() => void)[] = [];

// log buffer: batched into the store every 250ms so chatty debug output
// doesn't trigger a re-render per line
let logSeq = 0;
let logBuf: LogLine[] = [];
let logFlush: number | null = null;
const LOG_CAP = 1000;

function closeStreams() {
  for (const close of streams) close();
  streams = [];
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
  const onError = (message: string) => get().notify(message);
  const logs = subscribeCoreStream<{ type: string; payload: string }>(api, "logs", (d) => {
    if (get().logsPaused) return;
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
  }, onError);
  const traffic = subscribeCoreStream<{ up: number; down: number }>(api, "traffic", (d) => {
    set({ up: d.up, down: d.down });
  }, onError);
  const memory = subscribeCoreStream<{ inuse: number }>(api, "memory", (d) => {
    set({ memory: d.inuse });
  }, onError);
  let prev = new Map<string, { up: number; down: number }>();
  const conns = subscribeCoreStream<ConnectionsSnapshot>(api, "connections", (d) => {
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
  }, onError);
  streams = [logs, traffic, memory, conns];
}

let initialized = false;

// start / stop / restart are serialised: two overlapping restarts used to
// spawn two cores and then kill both (core_start releases its lock before
// the API port is up). A restart requested while one is still *queued* is
// coalesced into it; one requested while a restart is already running is
// queued behind it so a settings change made mid-restart is not lost.
const serialize = createSerialQueue();
let queuedRestart: Promise<void> | null = null;
let configRevision = 0;

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
    const revision = configRevision;
    set({ phase: "starting", error: null });
    let attempted: ApiInfo | null = get().api;
    try {
      if (IS_ANDROID) {
        const s = get().settings ?? (await settingsGet());
        if (s.tun.enable && !(await vpnPrepare())) {
          throw new Error(t("未获得 VPN 授权，无法启用 TUN"));
        }
      }
      const api = await coreStart();
      attempted = api;
      const version = await apiGet<{ version: string }>(api, "/version");
      const configs = await apiGet<{ mode: string }>(api, "/configs");
      openStreams(api, set, get);
      const mode = confirmedMode(configs.mode);
      set({ phase: "running", api, version: version.version, mode });
      void traySync(mode);
      set({ configPending: revision !== configRevision });
    } catch (e) {
      closeStreams();
      const api = await reconcileCoreSession(attempted);
      set({ phase: "error", error: String(e), api });
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

  const reloadImpl = async () => {
    const { api, phase } = get();
    if (!api || phase !== "running") return false;
    const revision = configRevision;
    const accepted = await applyPreservingMode({
      isCurrent: () => get().api === api && get().phase === "running",
      readMode: async () => (await apiGet<{ mode: unknown }>(api, "/configs")).mode,
      generate: configRegenerate,
      apply: (path) => hotReload(api, path),
      observed: (mode) => { set({ mode }); void traySync(mode); },
    });
    if (accepted && revision === configRevision) set({ configPending: false });
    return accepted && revision === configRevision;
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
  configPending: false,
  markConfigPending() { configRevision++; set({ configPending: true }); },

  notify(text, err = true) {
    const id = Date.now() + Math.random();
    set({ notice: { id, text, err } });
    if (err) {
      const line: LogLine = { type: "error", payload: `[App] ${text}`, seq: logSeq++, at: Date.now() };
      set({ logs: [...get().logs, line].slice(-LOG_CAP) });
    }
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
      if (get().api || get().phase === "running" || get().phase === "error") await stopImpl();
      await startImpl();
    });
    queuedRestart = p;
    return p;
  },

  setMode(mode) {
    return serialize(async () => {
      const { api, phase } = get();
      if (!api || phase !== "running") return;
      await apiPatch(api, "/configs", { mode: confirmedMode(mode) });
      const actual = confirmedMode((await apiGet<{ mode: unknown }>(api, "/configs")).mode);
      if (get().api !== api || get().phase !== "running") return;
      set({ mode: actual });
      void traySync(actual);
      if (actual !== mode) throw new Error("Proxy mode change was not accepted");
    }).catch((e) => { get().notify(String(e)); });
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
    get().markConfigPending();
    if (!get().api && get().phase !== "running" && get().phase !== "starting") return;
    try {
      if (how === "restart" || get().phase === "error") await get().restart();
      else if (!(await get().reloadConfig())) throw new Error(t("配置尚未应用，请重试"));
    } catch (e) {
      get().notify(t("设置已保存，但应用到内核失败: {msg}", { msg: String(e) }));
    }
  },

  reloadConfig: () => serialize(reloadImpl),

  selectProfile(id) {
    return serialize(async () => {
      await profileSelect(id);
      get().markConfigPending();
      if (get().phase === "running" && !(await reloadImpl())) throw new Error(t("配置尚未应用，请重试"));
    });
  },

  async setAutostart(enable) {
    await autostartSet(enable);
    set({ autostart: enable });
  },

  async syncConfig() {
    const { api } = get();
    if (!api) return;
    const configs = await apiGet<{ mode: unknown }>(api, "/configs");
    if (get().api === api && get().phase === "running") set({ mode: confirmedMode(configs.mode) });
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
    // Android: the VPN went away without us asking — revoked by the
    // system / another VPN app, or stopped from the notification or tile
    if (IS_ANDROID) {
      window.addEventListener("mfork-vpn-stopped", (e) => {
        const reason = (e as CustomEvent<string>).detail;
        if (reason === "revoked") get().notify(t("VPN 已被系统断开，内核已停止"));
        void get().stop();
      });
      // the system started our VPN service itself (always-on VPN): connect
      // through the normal path so a core sits behind the tunnel
      window.addEventListener("mfork-vpn-start-requested", () => {
        const phase = get().phase;
        if (phase !== "running" && phase !== "starting") void get().start();
      });
    }
    // app-config.json failed to parse: the backend moved it aside
    await listen<string>("settings-corrupt", (e) => {
      get().notify(t("设置文件损坏，已恢复默认：{msg}", { msg: e.payload }));
      void get().reloadSettings();
    });
    // Backend notifications request fresh generation inside the same queue, never a stale path.
    await listen<boolean>("profiles-auto-updated", (e) => {
      if (e.payload) {
        get().markConfigPending();
        void get().reloadConfig().then((ok) => {
          if (!ok && get().phase === "running") get().notify(t("配置尚未应用，请重试"));
        }).catch((err) => get().notify(String(err)));
      }
    });
    await listen<string>("profile-update-failed", (e) => get().notify(e.payload));
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
      set({ api: status.api });
    }
    // The guarded start path also reattaches to an existing service core.
    await get().start();
  },
  };
});
