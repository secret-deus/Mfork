import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import {
  ApiInfo,
  AppSettings,
  apiGet,
  apiPatch,
  autostartGet,
  autostartSet,
  coreStart,
  coreStatus,
  coreStop,
  settingsGet,
  settingsSet,
  systemProxySet,
  traySync,
  wsUrl,
} from "./lib/core";
import { ConnectionsSnapshot, ConnItem, LogLine } from "./lib/types";

export type CorePhase = "stopped" | "starting" | "running" | "error";
export type ProxyMode = "rule" | "global" | "direct";

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
  setLogsPaused: (paused: boolean) => void;
  clearLogs: () => void;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  setMode: (mode: ProxyMode) => Promise<void>;
  syncConfig: () => Promise<void>;
  setSystemProxy: (enable: boolean) => Promise<void>;
  updateSettings: (patch: Partial<AppSettings>) => Promise<void>;
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

async function apiPatchlessReload(api: ApiInfo, path: string) {
  const { apiPut } = await import("./lib/core");
  await apiPut(api, "/configs?force=true", { path });
}

export const useApp = create<AppState>((set, get) => ({
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

  setLogsPaused(paused) {
    set({ logsPaused: paused });
  },

  clearLogs() {
    logBuf = [];
    set({ logs: [] });
  },

  async start() {
    set({ phase: "starting", error: null });
    try {
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
  },

  async stop() {
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
      set({ error: String(e) });
    }
  },

  async updateSettings(patch) {
    const cur = get().settings ?? (await settingsGet());
    const next = { ...cur, ...patch };
    set({ settings: next });
    await settingsSet(next);
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
    void settingsGet().then((s) => set({ settings: s }));
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
      set({ error: e.payload });
    });
    // auto-updated subscriptions: hot-reload the regenerated config
    await listen<string | null>("profiles-auto-updated", (e) => {
      const { api } = get();
      if (e.payload && api) {
        void apiPatchlessReload(api, e.payload).then(() => get().syncConfig());
      }
    });
    await listen<number | null>("core-exited", (e) => {
      closeStreams();
      void traySync(null);
      set({
        phase: "error",
        api: null,
        error: `内核进程意外退出（code ${e.payload ?? "?"}）`,
        up: 0,
        down: 0,
        memory: 0,
        connections: 0,
        snapshot: null,
      });
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
}));
