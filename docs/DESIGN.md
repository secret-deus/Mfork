# Mfork 設計文檔

> **Mfork** — 一個對標 Sparkle 功能範圍的 mihomo 桌面客戶端。Windows 優先落地，macOS / Linux 共用同一套代碼（含提權服務）。
> 品牌：logo 為「分流 Fork」單色幾何標（一條流量分成兩路）；視覺定稿為「雜誌排版」方向——Space Grotesk 大字號數字、留白、髮絲線代替卡片、64px 圖標欄導航、中性近黑底 #0b0c0f。配色在 macOS 實機驗證後重定（2026-09）：正文 Inter Variable 500 字重、數字 Space Grotesk；主文字 #f5f6f8、說明文字 #c4cbd5（不再使用中灰）、微標籤金黃 #ffd166、動作/單位/選中態純藍 #4f9cff（不用紫調）、狀態薄荷綠 #62e39a、錯誤 #ff6b6b。完整設計稿見 design/ 目錄與 Artifact 畫布。

## 1. 技術棧

| 層 | 選型 | 說明 |
|---|---|---|
| 應用框架 | Tauri 2 | Rust 後端 + WebView2（Windows 自帶），安裝包小、內存佔用低 |
| 前端 | React 19 + TypeScript + Vite | |
| UI 組件 | shadcn/ui (Radix) + Tailwind CSS 4 | 簡約優雅路線：中性色、細邊框、克制的彩色；用戶定稿於原型第三版 |
| 圖表 | Canvas 2D + d3-geo | 概覽頁節點地球；速率以大字號數字呈現，未用圖表庫 |
| 編輯器 | Monaco Editor | 配置/覆寫的 YAML/JS 編輯 |
| 狀態管理 | Zustand | 全局狀態、WS 推送與設置；頁面級數據由各頁自行 fetch |
| Rust 關鍵庫 | serde_yaml, reqwest(blocking), sysproxy, boa_engine(JS 覆寫), zip, tauri-plugin-{dialog,autostart,single-instance,updater,process} | 同步 API + 線程，未引入 tokio |

## 2. 總體架構

```
┌─────────────────────────────────────────────────┐
│                  React 前端 (WebView2)            │
│  頁面 UI ── Tauri IPC ──► Rust 後端              │
│     │                                            │
│     └── HTTP / WebSocket ──► mihomo REST API     │
│                              (127.0.0.1:9090)    │
├─────────────────────────────────────────────────┤
│                  Tauri Rust 後端                  │
│  ├─ core:    mihomo 進程守護    updater: 內核自升級│
│  ├─ profile: 訂閱下載、覆寫合併、生成 config.yaml │
│  ├─ system:  系統代理、自啟   tray: 托盤  backup   │
│  └─ service: TUN 提權服務（SCM/launchd/systemd）通信│
├─────────────────────────────────────────────────┤
│  mihomo（隨包捆綁一份；自升級版放 app-data/core-bin）│
└─────────────────────────────────────────────────┘
```

**關鍵設計決策：運行時數據走「前端 → mihomo API」直連**（節點列表、延遲測試、連接、日誌、流量、模式切換），不經過 Rust 轉發——減少 IPC 樣板代碼，且 metacubexd 已驗證此模式。Rust 只負責 WebView 做不了的事：進程、文件、註冊表、提權。

### 2.1 內核進程管理（Rust `core` 模塊）

- 隨包捆綁一份 mihomo（`resources/mihomo/`，由 `scripts/fetch-core.mjs` 拉取）；設置頁可按 stable / alpha 通道從 GitHub Releases 自升級，新內核放 `<app-data>/core-bin/` 並優先使用，可一鍵恢復自帶版本（見 `updater` 模塊）
- 啟動流程：生成最終配置 → spawn 子進程（TUN 開啟時改由提權服務代啟）→ 輪詢 API 端口就緒（≤8s）→ 返回 ApiInfo 給前端 → 重新指向系統代理
- 守護：watcher 線程偵測子進程退出，清掉系統代理並向前端發 `core-exited`（前端顯示異常，不自動重啟）；stdout/stderr 落盤 `core/core.log`、`core.err.log`
- external-controller 隨機端口 + 隨機 secret；混合端口默認隨機，設置頁可固定
- 退出 GUI 時殺內核（含服務代啟的），不留孤兒進程

### 2.2 配置/訂閱系統（Rust `profile` 模塊）

數據目錄（Windows `%APPDATA%/app.mfork.desktop/`，macOS `~/Library/Application Support/app.mfork.desktop/`）：

```
profiles/          # 每個訂閱一個 yaml + index.json 元數據(url, interval, 上次更新, 流量信息)
overrides/         # 覆寫文件 (yaml 合併 / js 腳本) + index.json 順序與啟用狀態
core/config.yaml   # 生成的最終配置（勿手改）；core.log / core.err.log
core-bin/          # 自升級後的內核（可選）
app-config.json    # GUI 自身設置（AppSettings）
```

- 訂閱下載：UA 用 `clash.meta`（服務商據此下發完整協議），解析 `subscription-userinfo` 頭顯示流量/到期
- 生成管線：`選中的 profile → 依序應用 overrides（yaml 深合併 → js 函數變換）→ 注入 GUI 托管字段（端口、external-controller、TUN 開關等）→ config.yaml`
- 定時更新：Rust 側後台線程每 10 分鐘掃一次，按 profile 各自的 interval 判斷是否到期；更新後發 `profiles-auto-updated` 讓前端熱重載
- GUI 托管字段與用戶配置分離：用戶在設置頁改的端口/DNS/TUN 等最後注入，優先級最高

### 2.3 系統集成（Rust `system` 模塊）

- **系統代理**：`sysproxy` crate（Windows 註冊表 + 刷新 / macOS networksetup / Linux gsettings）；繞過列表可編輯；內核每次啟動端口變化時自動重新指向，停止或退出時清除。未做 PAC
- **托盤**：常駐托盤，菜單含顯示窗口、模式三選、系統代理開關、退出（標籤隨語言切換）；關窗口=隱藏到托盤
- **開機自啟**：tauri-plugin-autostart；「靜默啟動」選項（啟動只進托盤）
- **單實例**：tauri-plugin-single-instance，二次啟動喚起已有窗口

### 2.4 TUN 提權（Rust `service` 模塊 + `service/` crate）

採用 Clash Verge Rev 的成熟方案：安裝一個開機自啟的特權服務（一次性系統授權），服務以 SYSTEM/root 權限代為啟動內核，GUI 通過 `127.0.0.1:33217` 的 JSON-line 協議下發啟停指令，token 文件僅管理員可寫。內核路徑在安裝時釘死（`core-path.txt`），請求只能指定配置與數據目錄。未裝服務時 TUN 開關引導安裝。

| 平台 | 服務形態 | 提權方式 | 安裝位置 |
|---|---|---|---|
| Windows | SCM 服務 `MforkService`（SYSTEM） | PowerShell `Start-Process -Verb RunAs`（UAC） | 隨包 `resources/service/`，原地運行 |
| macOS | launchd daemon `app.mfork.service` | `osascript … with administrator privileges` | 複製到 `/Library/Application Support/Mfork/` |
| Linux | systemd 單元 `mfork-service.service` | `pkexec`（polkit） | 複製到 `/usr/local/lib/mfork/` |

macOS / Linux 的 daemon 收到 SIGTERM 時先殺內核子進程再退出；內核自升級後 GUI 會以同一提權流程重新釘定新路徑。

## 3. 頁面設計（對標 Sparkle）

| 頁面 | 內容 |
|---|---|
| **概覽** | 實時速率大字（WS `/traffic`）、內存佔用、活動連接數、模式三選（規則/全局/直連）、系統代理快捷開關、內核狀態；右側**節點地球**（`components/Globe.tsx`）：d3-geo 正交投影 + world-atlas land-110m 大陸剪影，出血裁切，只標最近一次測速成功的節點，本機→當前節點大圓流光，內核運行時緩慢自轉。節點座標由名稱中的地區提示推斷（`lib/geo.ts`：國家碼 / 中文地名 / 旗幟 emoji），本機由 IANA 時區推斷；當前節點沿策略組 `now` 鏈解析到真實節點 |
| **代理** | 策略組列表 → 組內節點網格；節點顯示延遲色階；一鍵組內測速（`/group/{name}/delay`）；隱藏不可用節點、排序選項 |
| **訂閱** | 訂閱卡片（流量條、到期、上次更新）；新增（URL/本地文件）、更新、切換、刪除、自動更新間隔（手動/6h/12h/24h/72h）；原始 yaml 由覆寫機制修改，不直接編輯 |
| **覆寫** | 覆寫文件列表（yaml/js）按順序全局應用到當前訂閱；啟用/停用、上下移；Monaco 編輯，保存即熱重載 |
| **連接** | 實時連接表（WS `/connections`）：host、規則、鏈路、上下行、時長；搜索過濾；單條/全部斷開 |
| **規則** | 規則列表（`/rules`）+ 規則集 provider 列表與手動更新 |
| **日誌** | WS `/logs` 實時滾動，等級過濾，暫停/清空 |
| **設置** | 內核（版本、通道、自升級、恢復自帶、GeoData）、網絡（固定混合端口、允許局域網、IPv6）、系統（系統代理、繞過、自啟、靜默啟動）、TUN（服務安裝、協議棧、嚴格路由、DNS 劫持）、DNS（fake-ip / 上游）、外觀（主題、語言）、備份（本地 zip、WebDAV）、關於（應用更新）。未做：熱鍵 |

## 4. 里程碑

- **M1 骨架與內核閉環**：Tauri 腳手架、內核啟停守護、API 連通、概覽頁（流量+模式切換）
- **M2 訂閱系統**：訂閱增刪改查/更新/流量信息、覆寫合併管線、Monaco 編輯
- **M3 代理與系統集成**：代理頁全功能、系統代理開關、托盤、開機自啟、單實例
- **M4 監控**：連接頁、日誌頁、規則頁
- **M5 TUN 與服務**：提權服務（Windows SCM / macOS launchd / Linux systemd）、TUN 參數設置、DNS 設置
- **M6 打磨**：內核升級、GeoData 更新、WebDAV 備份、主題、自動更新（tauri-updater）、多語言（zh-TW/zh-CN/en）

全部里程碑已完成（2026-09）。M6 落地要點：

- 內核自升級：`updater` 模塊從 GitHub Releases 取 stable（`latest`）/ alpha（`Prerelease-Alpha` tag）資源，經運行中的內核混合端口代理下載；新內核放在 `<app-data>/core-bin/`，`core_binary()` 優先取之，「恢復自帶版本」即刪除該目錄。已裝提權服務時升級會重新執行 `service install <新路徑>`（一次系統授權）重新釘定內核路徑。
- GeoData：前端直接 `POST /upgrade/geo`。
- 備份：`backup` 模塊打包 `profiles/`、`overrides/`、`app-config.json` 為 zip；WebDAV 走 MKCOL / PUT / PROPFIND / GET / DELETE，存於 `<url>/Mfork/`。恢復時保留本機 WebDAV 帳號。
- 主題：CSS 變量 + `data-theme`，Monaco 同步切換。
- 多語言：源字串即 key 的輕量 i18n（`src/i18n`），zh-CN / zh-TW / en，托盤菜單同步。
- 應用更新：tauri-plugin-updater，endpoint 指向 GitHub Releases 的 `latest.json`，CI 用 `tauri-action` 產出並簽名。
- 概覽節點地球：見第 3 節概覽行。
- 提權服務跨平台：見 2.4；macOS / Linux 為交叉編譯檢查通過、待實機驗證。
- 多語言：源字串即 key 的輕量 `t()`，托盤菜單由 Rust 側按 locale 重貼標籤。
- 應用更新：`tauri-plugin-updater`，endpoint 為 GitHub Releases `latest.json`；CI 用 `tauri-apps/tauri-action` 簽名並建草稿 Release。

## 5. 風險與預案

- **WebView2 缺失**（極老 Win10）：安裝器附帶 Evergreen Bootstrapper
- **殺軟誤報**：TUN 驅動（wintun.dll）與代理行為易被誤報，發佈時代碼簽名可緩解
- **訂閱格式差異**：僅支持 Clash/mihomo 格式訂閱；其他格式後續靠 Sub-Store 集成解決（M6 之後）
