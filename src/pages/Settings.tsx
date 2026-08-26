import { useApp } from "../store";

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
  const { phase, api, version, start, stop } = useApp();

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
          <Row label="系统代理" sub="接管系统 HTTP/SOCKS 代理设置" right={<Soon m="M3" />} />
          <Row label="开机自启 · 静默启动 · 托盘" right={<Soon m="M3" />} last />
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
