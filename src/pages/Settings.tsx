import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { open, save } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { useApp } from "../store";
import {
  AppSettings,
  AppUpdateInfo,
  BackupEntry,
  CoreRelease,
  CoreVersionInfo,
  ServiceStatus,
  apiPost,
  appUpdateCheck,
  appUpdateInstall,
  backupExportLocal,
  backupImportLocal,
  coreCheckUpdate,
  coreRevert,
  coreUpgrade,
  coreVersionInfo,
  formatBytes,
  serviceInstall,
  serviceStatus,
  serviceUninstall,
  webdavBackup,
  webdavCheck,
  webdavDelete,
  webdavList,
  webdavRestore,
  netDiagnose,
  type DiagItem,
} from "../lib/core";
import { LOCALES, useT } from "../i18n";
import { THEMES } from "../theme";
import { IS_DESKTOP } from "../lib/platform";

// ---------------------------------------------------------------- atoms

function Toggle({
  on,
  onChange,
  disabled,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={() => onChange(!on)}
      disabled={disabled}
      className={`relative h-[18px] w-[34px] shrink-0 rounded-full transition-colors ${
        on ? "bg-accent" : "bg-toggle-off"
      } disabled:opacity-40`}
    >
      <span
        className={`absolute top-[2px] block h-[14px] w-[14px] rounded-full bg-knob transition-all ${
          on ? "left-[18px]" : "left-[2px]"
        }`}
      />
    </button>
  );
}

function Row({
  label,
  sub,
  right,
  last,
}: {
  label: string;
  sub?: React.ReactNode;
  right: React.ReactNode;
  last?: boolean;
}) {
  return (
    <div
      className={`flex items-center justify-between gap-6 py-3.5 ${last ? "" : "border-b border-line-soft"}`}
    >
      <div className="min-w-0 shrink">
        <div className="text-[13px] leading-5 text-fg-2">{label}</div>
        {sub ? <div className="mt-0.5 text-[12px] leading-relaxed text-fg-3 select-text">{sub}</div> : null}
      </div>
      {right}
    </div>
  );
}

function Sect({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="border-b border-line-strong pb-2.5 text-[19px] font-medium leading-tight">{title}</div>
      {children}
    </div>
  );
}

function NetDiagRow() {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const [items, setItems] = useState<DiagItem[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      setItems(await netDiagnose());
    } catch (e) {
      setErr(String(e));
    }
    setBusy(false);
  };
  return (
    <Row
      label={t("网络诊断")}
      sub={
        <div>
          <div>{t("从应用进程与内核两侧分别探测，用于定位“已连接但无流量”")}</div>
          {err ? <div className="mt-2 text-err">{err}</div> : null}
          {items ? (
            <div className="mt-2.5 space-y-1.5">
              {items.map((it) => (
                <div key={it.name} className="flex gap-2 text-[12.5px] leading-relaxed">
                  <span className={`shrink-0 ${it.ok ? "text-ok" : "text-err"}`}>{it.ok ? "✓" : "✗"}</span>
                  <span className="min-w-0 break-all">
                    <span className="text-fg-2">{it.name}</span>
                    <span className="text-fg-3"> · {it.detail}</span>
                  </span>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      }
      right={
        <Btn onClick={() => void run()} disabled={busy}>
          {busy ? t("诊断中…") : t("运行诊断")}
        </Btn>
      }
    />
  );
}

const Val = ({ children }: { children: React.ReactNode }) => (
  <span className="page-num font-sg text-[13px] font-medium text-fg">{children}</span>
);

function Btn({
  onClick,
  disabled,
  accent,
  danger,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  accent?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`text-[13px] disabled:opacity-40 ${
        accent
          ? "text-accent hover:text-fg"
          : danger
            ? "text-mut hover:text-err"
            : "text-mut hover:text-fg"
      }`}
    >
      {children}
    </button>
  );
}

function Sel({
  value,
  options,
  onChange,
  disabled,
}: {
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled}
      className="border-b border-line-strong bg-transparent pb-0.5 text-right font-sg text-[13px] text-fg-3 outline-none focus:border-accent disabled:opacity-40"
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

const inputCls =
  "border-b border-line-strong bg-transparent pb-0.5 text-right font-sg text-[13px] text-fg-3 outline-none focus:border-accent disabled:opacity-40";

/** Text field that commits on blur / Enter. */
function Field({
  value,
  onCommit,
  disabled,
  width = "w-56",
  type = "text",
  placeholder,
}: {
  value: string;
  onCommit: (v: string) => void;
  disabled?: boolean;
  width?: string;
  type?: string;
  placeholder?: string;
}) {
  return (
    <input
      key={value}
      type={type}
      defaultValue={value}
      placeholder={placeholder}
      onBlur={(e) => {
        if (e.target.value !== value) onCommit(e.target.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
      }}
      disabled={disabled}
      spellCheck={false}
      className={`${width} ${inputCls}`}
    />
  );
}

// ---------------------------------------------------------------- sections

function CoreSection() {
  const t = useT();
  const { phase, api, version, start, stop, restart, settings, updateSettings } = useApp();
  const [info, setInfo] = useState<CoreVersionInfo | null>(null);
  const [latest, setLatest] = useState<CoreRelease | null>(null);
  const [busy, setBusy] = useState<"check" | "upgrade" | "revert" | "geo" | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [msg, setMsg] = useState<{ text: string; err?: boolean } | null>(null);

  const refreshInfo = () => void coreVersionInfo().then(setInfo);
  useEffect(refreshInfo, [phase]);
  useEffect(() => {
    const un = listen<{ received: number; total: number }>("core-upgrade-progress", (e) => {
      const { received, total } = e.payload;
      setProgress(total > 0 ? Math.min(100, Math.round((received / total) * 100)) : null);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const channel = settings?.core_channel ?? "stable";
  const current = version ?? info?.installed ?? null;

  const doCheck = async () => {
    setBusy("check");
    setMsg(null);
    setLatest(null);
    try {
      const rel = await coreCheckUpdate(channel);
      setLatest(rel);
      if (rel.version === current) setMsg({ text: t("已是最新版本") });
    } catch (e) {
      setMsg({ text: String(e), err: true });
    } finally {
      setBusy(null);
    }
  };

  const doUpgrade = async () => {
    setBusy("upgrade");
    setMsg(null);
    setProgress(0);
    try {
      const r = await coreUpgrade(channel);
      setLatest(null);
      setMsg(
        r.warning
          ? { text: r.warning, err: true }
          : { text: t("内核已更新到 {v}", { v: r.version }) },
      );
      await restart();
    } catch (e) {
      setMsg({ text: String(e), err: true });
      void restart();
    } finally {
      setBusy(null);
      setProgress(null);
      refreshInfo();
    }
  };

  const doRevert = async () => {
    if (!window.confirm(t("恢复为安装包自带的内核版本？"))) return;
    setBusy("revert");
    setMsg(null);
    try {
      await coreRevert();
      setLatest(null);
      await restart();
    } catch (e) {
      setMsg({ text: String(e), err: true });
      // the core was stopped before the failing step: bring it back
      void restart();
    } finally {
      setBusy(null);
      refreshInfo();
    }
  };

  const doGeo = async () => {
    if (!api) return;
    setBusy("geo");
    setMsg(null);
    try {
      await apiPost(api, "/upgrade/geo");
      setMsg({ text: t("GeoData 已更新") });
    } catch (e) {
      setMsg({ text: String(e), err: true });
    } finally {
      setBusy(null);
    }
  };

  const upgradeLabel =
    busy === "upgrade"
      ? progress != null && progress < 100
        ? t("下载中 {p}%", { p: progress })
        : t("安装中…")
      : t("升级到 {v}", { v: latest?.version ?? "" });

  return (
    <Sect title={t("内核")}>
      <Row
        label={t("当前版本")}
        sub={
          phase === "running"
            ? info?.upgraded
              ? t("运行中 · 自升级版本")
              : t("运行中 · 安装包自带")
            : t("未运行")
        }
        right={<Val>{current ?? "—"}</Val>}
      />
      <Row
        label={t("内核控制")}
        right={
          <div className="flex gap-5">
            {phase === "running" ? (
              <>
                <Btn onClick={() => void restart()}>{t("重启")}</Btn>
                <Btn onClick={() => void stop()}>{t("停止")}</Btn>
              </>
            ) : (
              <Btn accent onClick={() => void start()} disabled={phase === "starting"}>
                {t("启动")}
              </Btn>
            )}
          </div>
        }
      />
      {IS_DESKTOP ? (
        <>
      <Row
        label={t("版本通道")}
        sub={t("Alpha 为每日构建，功能最新但可能不稳定")}
        right={
          <Sel
            value={channel}
            options={[
              { value: "stable", label: "Stable" },
              { value: "alpha", label: "Alpha" },
            ]}
            onChange={(v) => {
              setLatest(null);
              setMsg(null);
              void updateSettings({ core_channel: v as AppSettings["core_channel"] });
            }}
            disabled={!settings || busy !== null}
          />
        }
      />
      <Row
        label={t("内核升级")}
        sub={
          msg ? (
            <span className={msg.err ? "text-err" : "text-ok"}>{msg.text}</span>
          ) : latest ? (
            t("{channel} 最新 {v} · {size}", {
              channel: channel === "alpha" ? "Alpha" : "Stable",
              v: latest.version,
              size: formatBytes(latest.size),
            })
          ) : (
            t("从 GitHub 下载最新内核（经内核代理）")
          )
        }
        right={
          <div className="flex gap-4">
            {latest && latest.version !== current ? (
              <Btn accent onClick={() => void doUpgrade()} disabled={busy !== null}>
                {upgradeLabel}
              </Btn>
            ) : (
              <Btn accent onClick={() => void doCheck()} disabled={busy !== null}>
                {busy === "check" ? t("检查中…") : t("检查更新")}
              </Btn>
            )}
            {info?.upgraded ? (
              <Btn onClick={() => void doRevert()} disabled={busy !== null}>
                {busy === "revert" ? t("处理中…") : t("恢复自带版本")}
              </Btn>
            ) : null}
          </div>
        }
      />
        </>
      ) : null}
      <Row
        label={t("GeoData 更新")}
        sub={t("内置 Geo 数据，保留手动更新 · 更新需要内核运行")}
        right={
          <Btn accent onClick={() => void doGeo()} disabled={phase !== "running" || busy !== null}>
            {busy === "geo" ? t("更新中…") : t("立即更新")}
          </Btn>
        }
        last
      />
    </Sect>
  );
}

function NetworkSection() {
  const t = useT();
  const { api, settings, applySettings } = useApp();
  return (
    <Sect title={t("网络")}>
      <Row
        label={t("混合端口")}
        sub={
          settings?.mixed_port
            ? t("HTTP 与 SOCKS5 共用 · 固定端口")
            : t("HTTP 与 SOCKS5 共用 · 留空为自动（沿用上次端口）")
        }
        right={
          <div className="flex items-center gap-3">
            {api ? <Val>{api.mixed_port}</Val> : null}
            <Field
              width="w-20"
              placeholder={t("自动")}
              value={settings?.mixed_port ? String(settings.mixed_port) : ""}
              onCommit={(v) => {
                const n = parseInt(v, 10);
                void applySettings(
                  { mixed_port: Number.isFinite(n) && n > 0 && n < 65536 ? n : 0 },
                  "restart",
                );
              }}
              disabled={!settings}
            />
          </div>
        }
      />
      {!IS_DESKTOP ? (
        <Row
          label={t("在手机上使用")}
          sub={t("Wi-Fi 详情 → 代理 → 手动，主机 127.0.0.1，端口填上方混合端口")}
          right={<Val>{api ? `127.0.0.1:${api.mixed_port}` : "—"}</Val>}
        />
      ) : null}
      <Row
        label={t("外部控制 API")}
        right={<Val>{api ? api.transport === "unix" ? "Unix socket" : `127.0.0.1:${api.port}` : "—"}</Val>}
      />
      <Row
        label={t("允许局域网")}
        sub={t("监听 0.0.0.0，同网段设备可使用本机代理")}
        right={
          <Toggle
            on={settings?.allow_lan ?? false}
            onChange={(v) => void applySettings({ allow_lan: v }, "hot")}
            disabled={!settings}
          />
        }
      />
      <Row
        label="IPv6"
        right={
          <Toggle
            on={settings?.ipv6 ?? false}
            onChange={(v) => void applySettings({ ipv6: v }, "hot")}
            disabled={!settings}
          />
        }
        last
      />
    </Sect>
  );
}

function SystemSection() {
  const t = useT();
  const { api, settings, autostart, setSystemProxy, updateSettings, setAutostart } = useApp();
  return (
    <Sect title={t("系统")}>
      <Row
        label={t("系统代理")}
        sub={
          settings?.system_proxy && api
            ? t("已指向 127.0.0.1:{port}", { port: api.mixed_port })
            : t("接管系统 HTTP/SOCKS 代理设置")
        }
        right={
          <Toggle
            on={settings?.system_proxy ?? false}
            onChange={(v) => void setSystemProxy(v)}
            disabled={!settings}
          />
        }
      />
      <Row
        label={t("代理绕过")}
        sub={t("不经过代理的地址列表")}
        right={
          <Field
            value={settings?.proxy_bypass ?? ""}
            onCommit={(v) => void updateSettings({ proxy_bypass: v })}
            disabled={!settings}
          />
        }
      />
      <Row
        label={t("开机自启")}
        right={<Toggle on={autostart} onChange={(v) => void setAutostart(v)} />}
      />
      <Row
        label={t("静默启动")}
        sub={t("启动时不显示窗口，仅驻留托盘")}
        right={
          <Toggle
            on={settings?.silent_start ?? false}
            onChange={(v) => void updateSettings({ silent_start: v })}
            disabled={!settings}
          />
        }
      />
      <Row
        label={t("关闭主窗口")}
        sub={t("隐藏到托盘，从托盘菜单退出")}
        right={<Val>{t("托盘")}</Val>}
        last
      />
    </Sect>
  );
}

function TunSection() {
  const t = useT();
  const { settings, applySettings } = useApp();
  const [svc, setSvc] = useState<ServiceStatus | null>(null);
  const [svcBusy, setSvcBusy] = useState(false);
  const [svcError, setSvcError] = useState<string | null>(null);

  useEffect(() => {
    void serviceStatus().then(setSvc);
  }, []);

  // entering / leaving TUN mode switches who runs the core (service vs.
  // child process) and needs a restart; the other knobs reload in place on
  // desktop. On Android the VPN interface is re-established per start, so
  // every TUN change restarts.
  const applyAndRestart = (patch: Partial<AppSettings>) => applySettings(patch, "restart");
  const applyHot = (patch: Partial<AppSettings>) => applySettings(patch, IS_DESKTOP ? "hot" : "restart");

  const svcAction = async (action: () => Promise<void>) => {
    setSvcBusy(true);
    setSvcError(null);
    try {
      await action();
    } catch (e) {
      setSvcError(String(e));
    } finally {
      setSvcBusy(false);
      setSvc(await serviceStatus());
    }
  };

  const tun = settings?.tun;
  return (
    <Sect title={IS_DESKTOP ? t("TUN 模式") : t("服务模式")}>
      {IS_DESKTOP ? <Row
        label={t("系统服务")}
        sub={
          svcError ??
          (svc?.reachable
            ? t("运行中 · 内核可以 SYSTEM 权限启动")
            : svc?.installed
              ? t("已安装但未响应")
              : t("未安装 · TUN 需要提权服务（安装时弹出一次系统授权）"))
        }
        right={
          <div className="flex gap-4">
            {svc?.installed ? (
              <>
                <Btn onClick={() => void svcAction(serviceInstall)} disabled={svcBusy}>
                  {t("重新指向内核")}
                </Btn>
                <Btn danger onClick={() => void svcAction(serviceUninstall)} disabled={svcBusy}>
                  {svcBusy ? t("处理中…") : t("卸载")}
                </Btn>
              </>
            ) : (
              <Btn accent onClick={() => void svcAction(serviceInstall)} disabled={svcBusy}>
                {svcBusy ? t("等待授权…") : t("安装服务")}
              </Btn>
            )}
          </div>
        }
      /> : null}
      <Row
        label={IS_DESKTOP ? t("启用 TUN") : t("VPN 模式")}
        sub={
          !IS_DESKTOP
            ? tun?.enable
              ? t("接管全部应用的流量（系统 VPN）")
              : t("仅代理模式：在 Wi-Fi 详情把代理指向 127.0.0.1 与混合端口")
            : svc?.reachable
              ? t("虚拟网卡接管全局流量；与其他代理客户端的 TUN/系统代理互斥")
              : t("需先安装系统服务")
        }
        right={
          <Toggle
            on={tun?.enable ?? false}
            onChange={(v) => {
              if (tun) void applyAndRestart({ tun: { ...tun, enable: v } });
            }}
            disabled={!tun || (IS_DESKTOP && !svc?.reachable && !tun.enable)}
          />
        }
      />
      <Row
        label={t("协议栈")}
        right={
          <Sel
            value={IS_DESKTOP ? (tun?.stack ?? "mixed") : "gvisor"}
            options={IS_DESKTOP ? [
              { value: "mixed", label: "Mixed" },
              { value: "gvisor", label: "gVisor" },
              { value: "system", label: "System" },
            ] : [{ value: "gvisor", label: "gVisor" }]}
            onChange={(v) => {
              if (tun) void applyHot({ tun: { ...tun, stack: v as typeof tun.stack } });
            }}
            disabled={!tun || !IS_DESKTOP}
          />
        }
      />
      {!IS_DESKTOP ? <NetDiagRow /> : null}
      {IS_DESKTOP ? <Row
        label={t("严格路由")}
        right={
          <Toggle
            on={tun?.strict_route ?? true}
            onChange={(v) => {
              if (tun) void applyHot({ tun: { ...tun, strict_route: v } });
            }}
            disabled={!tun}
          />
        }
      /> : null}
      <Row
        label={t("DNS 劫持")}
        sub={t("接管发往任意 53 端口的查询")}
        right={
          <Toggle
            on={IS_DESKTOP ? (tun?.dns_hijack ?? true) : true}
            onChange={(v) => {
              if (tun) void applyHot({ tun: { ...tun, dns_hijack: v } });
            }}
            disabled={!tun || !IS_DESKTOP}
          />
        }
        last
      />
    </Sect>
  );
}

function DnsSection() {
  const t = useT();
  const { settings, applySettings } = useApp();
  const applyHot = (patch: Partial<AppSettings>) => applySettings(patch, "hot");
  const dns = settings?.dns;
  return (
    <Sect title="DNS">
      <Row
        label={t("接管 DNS")}
        sub={t("关闭时沿用订阅自带的 dns 配置")}
        right={
          <Toggle
            on={dns?.enable ?? false}
            onChange={(v) => {
              if (dns) void applyHot({ dns: { ...dns, enable: v } });
            }}
            disabled={!dns}
          />
        }
      />
      <Row
        label={t("模式")}
        right={
          <Sel
            value={dns?.enhanced_mode ?? "fake-ip"}
            options={[
              { value: "fake-ip", label: "Fake-IP" },
              { value: "redir-host", label: "Redir-Host" },
            ]}
            onChange={(v) => {
              if (dns)
                void applyHot({
                  dns: { ...dns, enhanced_mode: v as typeof dns.enhanced_mode },
                });
            }}
            disabled={!dns || !dns.enable}
          />
        }
      />
      <Row
        label={t("上游 DNS")}
        sub={t("逗号分隔，支持 DoH/DoT/IP")}
        right={
          <Field
            width="w-64"
            value={dns?.nameserver.join(", ") ?? ""}
            onCommit={(v) => {
              if (!dns) return;
              const list = v
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean);
              if (list.join() !== dns.nameserver.join())
                void applyHot({ dns: { ...dns, nameserver: list } });
            }}
            disabled={!dns || !dns.enable}
          />
        }
        last
      />
    </Sect>
  );
}

function AppearanceSection() {
  const t = useT();
  const { settings, updateSettings } = useApp();
  return (
    <Sect title={t("外观")}>
      <Row
        label={t("主题")}
        right={
          <Sel
            value={settings?.theme ?? "dark"}
            options={THEMES.map((th) => ({ value: th.value, label: t(th.label) }))}
            onChange={(v) => void updateSettings({ theme: v as AppSettings["theme"] })}
            disabled={!settings}
          />
        }
      />
      <Row
        label={t("语言")}
        right={
          <Sel
            value={settings?.locale ?? "zh-CN"}
            options={LOCALES}
            onChange={(v) => void updateSettings({ locale: v as AppSettings["locale"] })}
            disabled={!settings}
          />
        }
        last
      />
    </Sect>
  );
}

function BackupSection() {
  const t = useT();
  const { settings, updateSettings, reloadSettings, reloadConfig, markConfigPending } = useApp();
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<{ text: string; err?: boolean } | null>(null);
  const [list, setList] = useState<BackupEntry[] | null>(null);

  const wd = settings?.webdav;
  const setWd = (patch: Partial<NonNullable<typeof wd>>) => {
    if (wd) void updateSettings({ webdav: { ...wd, ...patch } });
  };

  const run = async (key: string, fn: () => Promise<string | void>) => {
    setBusy(key);
    setMsg(null);
    try {
      const r = await fn();
      if (r) setMsg({ text: r });
    } catch (e) {
      setMsg({ text: String(e), err: true });
    } finally {
      setBusy(null);
    }
  };

  /** After a restore: settings + profiles changed on disk → reload + hot-reload core. */
  const afterRestore = async () => {
    await reloadSettings();
    markConfigPending();
    if (useApp.getState().phase === "running" && !(await reloadConfig())) throw new Error(t("配置尚未应用，请重试"));
    return t("已恢复，订阅与设置已重新加载");
  };

  const exportLocal = () =>
    run("export", async () => {
      const path = await save({
        defaultPath: "mfork-backup.zip",
        filters: [{ name: "Zip", extensions: ["zip"] }],
      });
      if (!path) return;
      await backupExportLocal(path);
      return t("已导出到 {path}", { path });
    });

  const importLocal = () =>
    run("import", async () => {
      const path = await open({ multiple: false, filters: [{ name: "Zip", extensions: ["zip"] }] });
      if (typeof path !== "string") return;
      if (!window.confirm(t("从备份恢复会覆盖当前订阅、覆写与设置，继续？"))) return;
      await backupImportLocal(path);
      return afterRestore();
    });

  const refreshList = () =>
    run("list", async () => {
      setList(await webdavList());
    });

  return (
    <Sect title={t("备份")}>
      <Row
        label={t("本地备份")}
        sub={t("订阅、覆写与应用设置打包为 zip")}
        right={
          <div className="flex gap-4">
            <Btn accent onClick={() => void exportLocal()} disabled={busy !== null}>
              {t("导出")}
            </Btn>
            <Btn onClick={() => void importLocal()} disabled={busy !== null}>
              {t("导入恢复")}
            </Btn>
          </div>
        }
      />
      <Row
        label={t("WebDAV 地址")}
        sub={t("备份存放在该地址下的 Mfork/ 目录")}
        right={
          <Field
            width="w-64"
            placeholder="https://dav.example.com/dav"
            value={wd?.url ?? ""}
            onCommit={(v) => setWd({ url: v.trim() })}
            disabled={!wd}
          />
        }
      />
      <Row
        label={t("用户名")}
        right={
          <Field
            width="w-40"
            value={wd?.username ?? ""}
            onCommit={(v) => setWd({ username: v })}
            disabled={!wd}
          />
        }
      />
      <Row
        label={t("密码")}
        right={
          <Field
            width="w-40"
            type="password"
            value={wd?.password ?? ""}
            onCommit={(v) => setWd({ password: v })}
            disabled={!wd}
          />
        }
      />
      <Row
        label={t("WebDAV 备份")}
        sub={
          msg ? (
            <span className={msg.err ? "text-err" : "text-ok"}>{msg.text}</span>
          ) : (
            t("先测试连接，再备份或查看已有备份")
          )
        }
        right={
          <div className="flex gap-4">
            <Btn
              onClick={() => void run("check", async () => (await webdavCheck(), t("连接正常")))}
              disabled={busy !== null || !wd?.url}
            >
              {busy === "check" ? t("测试中…") : t("测试连接")}
            </Btn>
            <Btn
              accent
              onClick={() =>
                void run("backup", async () => {
                  const name = await webdavBackup();
                  setList(null);
                  return t("已上传 {name}", { name });
                })
              }
              disabled={busy !== null || !wd?.url}
            >
              {busy === "backup" ? t("上传中…") : t("立即备份")}
            </Btn>
            <Btn onClick={() => void refreshList()} disabled={busy !== null || !wd?.url}>
              {busy === "list" ? t("加载中…") : t("查看备份")}
            </Btn>
          </div>
        }
        last={list === null}
      />
      {list !== null ? (
        <div className="py-2">
          {list.length === 0 ? (
            <div className="cap2 py-2">{t("云端暂无备份")}</div>
          ) : (
            list.map((b, i) => (
              <div
                key={b.name}
                className={`flex items-center justify-between gap-4 py-2 ${
                  i === list.length - 1 ? "" : "border-b border-line-faint"
                }`}
              >
                <div className="min-w-0">
                  <div className="truncate font-sg text-[12.5px] text-fg-3">{b.name}</div>
                  <div className="text-[12px] text-mut">
                    {formatBytes(b.size)}
                    {b.modified ? ` · ${b.modified}` : ""}
                  </div>
                </div>
                <div className="flex shrink-0 gap-4">
                  <Btn
                    accent
                    onClick={() =>
                      void run(`restore:${b.name}`, async () => {
                        if (!window.confirm(t("从备份恢复会覆盖当前订阅、覆写与设置，继续？")))
                          return;
                        await webdavRestore(b.name);
                        return afterRestore();
                      })
                    }
                    disabled={busy !== null}
                  >
                    {busy === `restore:${b.name}` ? t("恢复中…") : t("恢复")}
                  </Btn>
                  <Btn
                    danger
                    onClick={() =>
                      void run(`delete:${b.name}`, async () => {
                        if (!window.confirm(t("删除云端备份「{name}」？", { name: b.name }))) return;
                        await webdavDelete(b.name);
                        setList((l) => l?.filter((x) => x.name !== b.name) ?? null);
                      })
                    }
                    disabled={busy !== null}
                  >
                    {t("删除")}
                  </Btn>
                </div>
              </div>
            ))
          )}
        </div>
      ) : null}
    </Sect>
  );
}

function AboutSection() {
  const t = useT();
  const [appVersion, setAppVersion] = useState("");
  const [update, setUpdate] = useState<AppUpdateInfo | null>(null);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [msg, setMsg] = useState<{ text: string; err?: boolean } | null>(null);

  useEffect(() => {
    void getVersion().then(setAppVersion);
  }, []);
  useEffect(() => {
    const un = listen<{ received: number; total: number }>("app-update-progress", (e) => {
      const { received, total } = e.payload;
      setProgress(total > 0 ? Math.min(100, Math.round((received / total) * 100)) : null);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const doCheck = async () => {
    setBusy("check");
    setMsg(null);
    try {
      const u = await appUpdateCheck();
      if (u) {
        setUpdate(u);
        setMsg({ text: t("发现新版本 {v}", { v: u.version }) });
      } else {
        setUpdate(null);
        setMsg({ text: t("已是最新版本") });
      }
    } catch (e) {
      setMsg({ text: t("检查更新失败：{e}", { e: String(e) }), err: true });
    } finally {
      setBusy(null);
    }
  };

  const doInstall = async () => {
    if (!update) return;
    setBusy("install");
    setMsg(null);
    try {
      await appUpdateInstall();
      setMsg({ text: t("安装完成，正在重启…") });
      await relaunch();
    } catch (e) {
      setMsg({ text: String(e), err: true });
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  return (
    <Sect title={t("关于")}>
      <Row label={t("Mfork 版本")} right={<Val>{appVersion || "—"}</Val>} last={!IS_DESKTOP} />
      {IS_DESKTOP ? <Row
        label={t("应用更新")}
        sub={
          msg ? (
            <span className={msg.err ? "text-err" : "text-ok"}>{msg.text}</span>
          ) : (
            t("从 GitHub Releases 检查新版本")
          )
        }
        right={
          update ? (
            <Btn accent onClick={() => void doInstall()} disabled={busy !== null}>
              {busy === "install"
                ? progress != null && progress < 100
                  ? t("下载中 {p}%", { p: progress })
                  : t("安装中…")
                : t("下载并安装")}
            </Btn>
          ) : (
            <Btn accent onClick={() => void doCheck()} disabled={busy !== null}>
              {busy === "check" ? t("检查中…") : t("检查更新")}
            </Btn>
          )
        }
        last
      /> : null}
    </Sect>
  );
}

// ---------------------------------------------------------------- page

export default function Settings() {
  const t = useT();
  return (
    <div className="flex h-full min-w-0 flex-col pt-8">
      {/* the scroll area sits under the drag strip so the scrollbar never runs into the window buttons */}
      <div className="min-h-0 flex-1 overflow-y-auto px-5 md:px-14 pt-5 pb-10">
        <div className="cap">{t("Mfork · 设置 Settings")}</div>
        <div className="mt-8 grid grid-cols-1 content-start gap-x-20 gap-y-10 md:grid-cols-2">
          <CoreSection />
          <NetworkSection />
          {IS_DESKTOP ? <SystemSection /> : null}
          <TunSection />
          <DnsSection />
          <AppearanceSection />
          <BackupSection />
          <AboutSection />
        </div>
      </div>
    </div>
  );
}
