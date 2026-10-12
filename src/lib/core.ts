import { invoke } from "@tauri-apps/api/core";
import { t } from "../i18n";
import type { ProcessRule } from "./process-rules";
import type { ApiInfo, CoreStatus } from "./core-transport";
export { apiGet, apiPatch, apiPut, apiPost, apiDelete, subscribeCoreStream, reconcileCoreSession } from "./core-transport";
export type { ApiInfo, CoreStatus } from "./core-transport";

export const coreStart = () => invoke<ApiInfo>("core_start");
export const coreStop = () => invoke<boolean>("core_stop");
export const coreStatus = () => invoke<CoreStatus>("core_status");
/** Android: system VPN consent dialog; true once granted (always true elsewhere) */
export const vpnPrepare = () => invoke<boolean>("vpn_prepare");
/** last lines of core.err.log / core.log, for diagnosing a failed start */
export const coreLogTail = (lines = 60) => invoke<string>("core_log_tail", { lines });
export type DiagItem = { name: string; ok: boolean; detail: string };
/** probes from the app process and through the core; see core.rs net_diagnose */
export const netDiagnose = () => invoke<DiagItem[]>("net_diagnose");

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
  interval_hours: number;
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
  invoke<void>("profile_select", { id });
export const profileImportLocal = (path: string) =>
  invoke<ProfileMeta>("profile_import_local", { path });
export const profileSetInterval = (id: string, hours: number) =>
  invoke<void>("profile_set_interval", { id, hours });
export const configRegenerate = (mode?: "rule" | "global" | "direct") =>
  invoke<string | null>("config_regenerate", { mode: mode ?? null });

export interface TunSettings {
  enable: boolean;
  stack: "gvisor" | "system" | "mixed";
  auto_route: boolean;
  strict_route: boolean;
  dns_hijack: boolean;
  mtu: number;
}

export interface DnsSettings {
  enable: boolean;
  enhanced_mode: "fake-ip" | "redir-host";
  nameserver: string[];
  fallback: string[];
}

export interface WebdavSettings {
  url: string;
  username: string;
  password: string;
}

export interface AppSettings {
  /** fixed mixed port; 0 = automatic (the last port is reused while free) */
  mixed_port: number;
  /** port the last automatic start used; managed by the backend */
  last_mixed_port: number;
  schema: number;
  allow_lan: boolean;
  ipv6: boolean;
  system_proxy: boolean;
  proxy_bypass: string;
  silent_start: boolean;
  tun: TunSettings;
  dns: DnsSettings;
  core_channel: "stable" | "alpha";
  theme: "dark" | "light" | "system";
  locale: "zh-CN" | "zh-TW" | "en";
  webdav: WebdavSettings;
  /**
   * process-name routing rules (see lib/process-rules.ts). Older settings
   * files and test fixtures may omit the field, so read it as
   * `settings.process_rules ?? []`.
   */
  process_rules: ProcessRule[];
}

// ---- core self-upgrade (M6)

export interface CoreVersionInfo {
  installed: string | null;
  bundled: string | null;
  upgraded: boolean;
}

export interface CoreRelease {
  channel: string;
  version: string;
  asset: string;
  url: string;
  size: number;
}

export const coreVersionInfo = () => invoke<CoreVersionInfo>("core_version_info");
export const coreCheckUpdate = (channel: string) =>
  invoke<CoreRelease>("core_check_update", { channel });
export interface UpgradeResult {
  version: string;
  /** binary replaced, but the elevated service still runs the old one */
  warning: string | null;
}

export const coreUpgrade = (channel: string) =>
  invoke<UpgradeResult>("core_upgrade", { channel });
export const coreRevert = () => invoke<void>("core_revert");

// ---- app self-update (OTA, desktop only)
//
// Routed through the running core when possible (same policy as the core
// updater) so the check/download survives networks where GitHub is only
// reachable through Mfork. Download progress arrives as the
// "app-update-progress" event ({received, total}; total 0 = unknown).

export interface AppUpdateInfo {
  version: string;
  notes: string | null;
  date: string | null;
}

export const appUpdateCheck = () => invoke<AppUpdateInfo | null>("app_update_check");
export const appUpdateInstall = () => invoke<void>("app_update_install");

// ---- backup (M6)

export interface BackupEntry {
  name: string;
  size: number;
  modified: string;
}

export const backupExportLocal = (path: string) =>
  invoke<void>("backup_export_local", { path });
export const backupImportLocal = (path: string) =>
  invoke<void>("backup_import_local", { path });
export const webdavCheck = () => invoke<void>("webdav_check");
export const webdavList = () => invoke<BackupEntry[]>("webdav_list");
export const webdavBackup = () => invoke<string>("webdav_backup");
export const webdavRestore = (name: string) =>
  invoke<void>("webdav_restore", { name });
export const webdavDelete = (name: string) =>
  invoke<void>("webdav_delete", { name });

export interface ServiceStatus {
  installed: boolean;
  reachable: boolean;
  core_running: boolean;
}

export const serviceStatus = () => invoke<ServiceStatus>("service_status");
export const serviceInstall = () => invoke<void>("service_install");
export const serviceUninstall = () => invoke<void>("service_uninstall");

export const settingsGet = () => invoke<AppSettings>("settings_get");
export const settingsSet = (settings: AppSettings) =>
  invoke<void>("settings_set", { settings });
export const systemProxySet = (enable: boolean) =>
  invoke<void>("system_proxy_set", { enable });
export const autostartGet = () => invoke<boolean>("autostart_get");
export const autostartSet = (enable: boolean) =>
  invoke<void>("autostart_set", { enable });
export const traySync = (mode: string | null) =>
  invoke<void>("tray_sync", { mode });

export interface OverrideMeta {
  id: string;
  name: string;
  kind: "yaml" | "js";
  enabled: boolean;
}

export const overridesList = () => invoke<OverrideMeta[]>("overrides_list");
export const overrideCreate = (name: string, kind: "yaml" | "js", enabled = true) =>
  invoke<OverrideMeta>("override_create", { name, kind, enabled });
export const overrideRead = (id: string) => invoke<string>("override_read", { id });
export const overrideWrite = (id: string, content: string) =>
  invoke<void>("override_write", { id, content });
export const overrideDelete = (id: string) => invoke<void>("override_delete", { id });
export const overrideToggle = (id: string, enabled: boolean) =>
  invoke<void>("override_toggle", { id, enabled });
export const overrideMove = (id: string, up: boolean) =>
  invoke<void>("override_move", { id, up });

export function relativeTime(unixSecs: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000 - unixSecs));
  if (s < 60) return t("刚刚");
  if (s < 3600) return t("{n} 分钟前", { n: Math.floor(s / 60) });
  if (s < 86400) return t("{n} 小时前", { n: Math.floor(s / 3600) });
  return t("{n} 天前", { n: Math.floor(s / 86400) });
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
