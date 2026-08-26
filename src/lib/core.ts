import { invoke } from "@tauri-apps/api/core";

export interface ApiInfo {
  port: number;
  secret: string;
  mixed_port: number;
  started_at_ms: number;
}

export interface CoreStatus {
  running: boolean;
  api: ApiInfo | null;
}

export const coreStart = () => invoke<ApiInfo>("core_start");
export const coreStop = () => invoke<boolean>("core_stop");
export const coreStatus = () => invoke<CoreStatus>("core_status");

export const apiUrl = (api: ApiInfo, path: string) =>
  `http://127.0.0.1:${api.port}${path}`;

export const wsUrl = (api: ApiInfo, path: string) =>
  `ws://127.0.0.1:${api.port}${path}?token=${encodeURIComponent(api.secret)}`;

async function request<T>(
  api: ApiInfo,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const res = await fetch(apiUrl(api, path), {
    ...init,
    headers: {
      Authorization: `Bearer ${api.secret}`,
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
  });
  if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

export const apiGet = <T>(api: ApiInfo, path: string) => request<T>(api, path);

export const apiPatch = <T>(api: ApiInfo, path: string, body: unknown) =>
  request<T>(api, path, { method: "PATCH", body: JSON.stringify(body) });

export const apiPut = <T>(api: ApiInfo, path: string, body?: unknown) =>
  request<T>(api, path, {
    method: "PUT",
    body: body === undefined ? undefined : JSON.stringify(body),
  });

export const apiDelete = <T>(api: ApiInfo, path: string) =>
  request<T>(api, path, { method: "DELETE" });

export function formatDuration(sinceIso: string): string {
  const s = Math.max(0, Math.floor((Date.now() - Date.parse(sinceIso)) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60)
    .toString()
    .padStart(2, "0");
  const ss = (s % 60).toString().padStart(2, "0");
  return h > 0 ? `${h}:${m}:${ss}` : `${m}:${ss}`;
}

export interface SubInfo {
  upload: number;
  download: number;
  total: number;
  expire: number;
}

export interface ProfileMeta {
  id: string;
  name: string;
  url: string | null;
  updated_at: number;
  sub: SubInfo | null;
}

export interface ProfilesIndex {
  active: string | null;
  items: ProfileMeta[];
}

export const profilesList = () => invoke<ProfilesIndex>("profiles_list");
export const profileImport = (url: string) =>
  invoke<ProfileMeta>("profile_import", { url });
export const profileUpdate = (id: string) =>
  invoke<ProfileMeta>("profile_update", { id });
export const profileDelete = (id: string) =>
  invoke<void>("profile_delete", { id });
export const profileSelect = (id: string) =>
  invoke<string | null>("profile_select", { id });

export function relativeTime(unixSecs: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000 - unixSecs));
  if (s < 60) return "刚刚";
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}

export function formatRate(bytesPerSec: number): { value: string; unit: string } {
  if (bytesPerSec < 1024) return { value: bytesPerSec.toFixed(0), unit: "B/s" };
  const kb = bytesPerSec / 1024;
  if (kb < 1024) return { value: kb.toFixed(kb < 10 ? 1 : 0), unit: "KB/s" };
  const mb = kb / 1024;
  if (mb < 1024) return { value: mb.toFixed(1), unit: "MB/s" };
  return { value: (mb / 1024).toFixed(2), unit: "GB/s" };
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(0)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(0)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}
