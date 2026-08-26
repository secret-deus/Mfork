import { useApp } from "../store";

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
        on ? "bg-accent" : "bg-[#23262d]"
      } disabled:opacity-40`}
    >
      <span
        className={`absolute top-[2px] block h-[14px] w-[14px] rounded-full bg-[#e7e9ec] transition-all ${
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
  sub?: string;
  right: React.ReactNode;
  last?: boolean;
}) {
  return (
    <div
      className={`flex items-center justify-between gap-4 py-3.5 ${last ? "" : "border-b border-[#16181d]"}`}
    >
      <div>
        <div className="text-[12.5px] text-[#c3c8d0]">{label}</div>
        {sub ? <div className="mt-0.5 text-[10.5px] text-dim">{sub}</div> : null}
      </div>
      {right}
    </div>
  );
}

function Sect({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="border-b border-line-strong pb-2.5 text-[15px] font-semibold">{title}</div>
      {children}
    </div>
  );
}

const Val = ({ children }: { children: React.ReactNode }) => (
  <span className="font-sg text-[12.5px] text-[#9aa0a8]">{children}</span>
);

const Soon = ({ m }: { m: string }) => <span className="cap2">{m}</span>;

export default function Settings() {
  const {
    phase,
    api,
    version,
    start,
    stop,
    settings,
    autostart,
    setSystemProxy,
    updateSettings,
    setAutostart,
  } = useApp();

  return (
    <div className="flex h-full min-w-0 flex-col overflow-y-auto px-14 pb-8 pt-13">
      <div className="cap">Mfork · 设置 Settings</div>
      <div className="mt-7 grid grid-cols-2 content-start gap-x-16 gap-y-8">
        <Sect title="内核">
          <Row
            label="当前版本"
            sub={phase === "running" ? "运行中" : "未运行"}
            right={<Val>{version ?? "—"}</Val>}
          />
          <Row
            label="内核控制"
            right={
              <div className="flex gap-5">
                {phase === "running" ? (
                  <>
                    <button
                      onClick={async () => {
                        await stop();
                        await start();
                      }}
                      className="text-xs text-mut hover:text-fg"
                    >
                      重启
                    </button>
                    <button onClick={() => void stop()} className="text-xs text-mut hover:text-fg">
                      停止
                    </button>
                  </>
                ) : (
                  <button onClick={() => void start()} className="text-xs text-accent hover:text-fg">
                    启动
                  </button>
                )}
              </div>
            }
          />
          <Row label="版本通道" right={<Val>Stable</Val>} />
          <Row label="内核升级 / GeoData 更新" right={<Soon m="M6" />} last />
        </Sect>

        <Sect title="网络">
          <Row
            label="混合端口"
            sub="HTTP 与 SOCKS5 共用（当前为随机端口）"
            right={<Val>{api?.mixed_port ?? "—"}</Val>}
          />
          <Row
            label="外部控制 API"
            right={<Val>{api ? `127.0.0.1:${api.port}` : "—"}</Val>}
          />
          <Row label="固定端口 / 允许局域网 / IPv6" right={<Soon m="M2" />} last />
        </Sect>

        <Sect title="系统">
          <Row
            label="系统代理"
            sub={
              settings?.system_proxy && api
                ? `已指向 127.0.0.1:${api.mixed_port}`
                : "接管系统 HTTP/SOCKS 代理设置"
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
            label="代理绕过"
            sub="不经过代理的地址列表"
            right={
              <input
                key={settings?.proxy_bypass ?? ""}
                defaultValue={settings?.proxy_bypass ?? ""}
                onBlur={(e) => {
                  if (settings && e.target.value !== settings.proxy_bypass)
                    void updateSettings({ proxy_bypass: e.target.value });
                }}
                disabled={!settings}
                spellCheck={false}
                className="w-56 border-b border-[#23262d] bg-transparent pb-0.5 text-right font-sg text-[11.5px] text-[#9aa0a8] outline-none focus:border-accent"
              />
            }
          />
          <Row
            label="开机自启"
            right={<Toggle on={autostart} onChange={(v) => void setAutostart(v)} />}
          />
          <Row
            label="静默启动"
            sub="启动时不显示窗口，仅驻留托盘"
            right={
              <Toggle
                on={settings?.silent_start ?? false}
                onChange={(v) => void updateSettings({ silent_start: v })}
                disabled={!settings}
              />
            }
          />
          <Row
            label="关闭主窗口"
            sub="隐藏到托盘，从托盘菜单退出"
            right={<Val>托盘</Val>}
            last
          />
        </Sect>

        <Sect title="TUN 模式">
          <Row label="启用 TUN" sub="虚拟网卡接管全局流量，需系统服务" right={<Soon m="M5" />} last />
        </Sect>

        <Sect title="外观">
          <Row label="主题" right={<Val>深色</Val>} />
          <Row label="语言" right={<Val>简体中文</Val>} last />
        </Sect>

        <Sect title="备份">
          <Row label="WebDAV 备份与恢复" right={<Soon m="M6" />} last />
        </Sect>
      </div>
    </div>
  );
}
