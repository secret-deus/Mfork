import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import {
  ApiInfo,
  apiGet,
  apiPatch,
  coreStart,
  coreStatus,
  coreStop,
  wsUrl,
} from "./lib/core";
import { ConnectionsSnapshot, ConnItem } from "./lib/types";

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
  start: () => Promise<void>;
  stop: () => Promise<void>;
  setMode: (mode: ProxyMode) => Promise<void>;
  syncConfig: () => Promise<void>;
  init: () => Promise<void>;
}

let sockets: WebSocket[] = [];

function closeStreams() {
  for (const ws of sockets) ws.close();
  sockets = [];
}

function openStreams(
  api: ApiInfo,
  set: (partial: Partial<AppState>) => void,
) {
  closeStreams();
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
  sockets = [traffic, memory, conns];
}

let initialized = false;

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

  async start() {
    set({ phase: "starting", error: null });
    try {
      const api = await coreStart();
      const version = await apiGet<{ version: string }>(api, "/version");
      const configs = await apiGet<{ mode: string }>(api, "/configs");
      openStreams(api, set);
      set({
        phase: "running",
        api,
        version: version.version,
        mode: configs.mode.toLowerCase() as ProxyMode,
      });
    } catch (e) {
      set({ phase: "error", error: String(e), api: null });
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
  },

  async setMode(mode) {
    const { api } = get();
    if (!api) return;
    await apiPatch(api, "/configs", { mode });
    set({ mode });
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
    await listen<number | null>("core-exited", (e) => {
      closeStreams();
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
      openStreams(status.api, set);
      set({
        phase: "running",
        api: status.api,
        version: version.version,
        mode: configs.mode.toLowerCase() as ProxyMode,
      });
    } else {
      // auto-start on launch
      await get().start();
    }
  },
}));
