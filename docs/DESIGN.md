# Mfork 設計文檔

> **Mfork** — 一個對標 Sparkle 功能範圍的 mihomo 桌面客戶端。Windows 優先，架構預留跨平台。
> 品牌：logo 為「分流 Fork」單色幾何標（一條流量分成兩路）；視覺定稿為「雜誌排版」方向——Space Grotesk 大字號數字、留白、髮絲線代替卡片、64px 圖標欄導航、中性近黑底 #0b0c0f + accent #7c9cff 克制使用。完整設計稿見 design/ 目錄與 Artifact 畫布。

## 1. 技術棧

| 層 | 選型 | 說明 |
|---|---|---|
| 應用框架 | Tauri 2 | Rust 後端 + WebView2（Windows 自帶），安裝包小、內存佔用低 |
| 前端 | React 19 + TypeScript + Vite | |
| UI 組件 | shadcn/ui (Radix) + Tailwind CSS 4 | 簡約優雅路線：中性色、細邊框、克制的彩色；用戶定稿於原型第三版 |
| 圖表 | Recharts | 流量實時曲線 |
| 編輯器 | Monaco Editor | 配置/覆寫的 YAML/JS 編輯 |
| 狀態管理 | Zustand + TanStack Query | Query 管 REST 輪詢，Zustand 管 WS 推送與全局狀態 |
| Rust 關鍵庫 | tokio, serde_yaml, reqwest, sysproxy, tauri-plugin-* | |

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
│  ├─ core:    mihomo.exe 進程守護、版本管理        │
│  ├─ profile: 訂閱下載、覆寫合併、生成 config.yaml │
│  ├─ system:  系統代理、托盤、開機自啟             │
│  └─ service: TUN 提權服務（Windows Service）通信  │
├─────────────────────────────────────────────────┤
│  mihomo.exe (捆綁: stable / alpha 雙版本)         │
└─────────────────────────────────────────────────┘
```

**關鍵設計決策：運行時數據走「前端 → mihomo API」直連**（節點列表、延遲測試、連接、日誌、流量、模式切換），不經過 Rust 轉發——減少 IPC 樣板代碼，且 metacubexd 已驗證此模式。Rust 只負責 WebView 做不了的事：進程、文件、註冊表、提權。

### 2.1 內核進程管理（Rust `core` 模塊）

- 捆綁 mihomo stable + alpha 兩個可執行檔於 resources，設置頁可切換、可通過 `/upgrade` API 或下載 release 自升級
- 啟動流程：生成最終配置 → 校驗（`mihomo -t`）→ spawn 子進程 → 輪詢 `/version` 確認就緒 → 通知前端
- 守護：子進程意外退出時自動重啟（帶退避），日誌落盤
- external-controller 隨機端口 + 隨機 secret，寫入運行時配置，通過 IPC 交給前端

### 2.2 配置/訂閱系統（Rust `profile` 模塊）

數據目錄（`%APPDATA%/mihomo-gui/`）：

```
profiles/          # 每個訂閱一個 yaml + 元數據(url, ua, interval, 上次更新, 流量信息)
overrides/         # 覆寫文件 (yaml 合併 / js 腳本)
config.yaml        # 生成的最終配置（勿手改）
app-config.yaml    # GUI 自身設置
service.log / core.log
```

- 訂閱下載：UA 用 `clash.meta`（服務商據此下發完整協議），解析 `subscription-userinfo` 頭顯示流量/到期
- 生成管線：`選中的 profile → 依序應用 overrides（yaml 深合併 → js 函數變換）→ 注入 GUI 托管字段（端口、external-controller、TUN 開關等）→ config.yaml`
- 定時更新：Rust 側 tokio 定時器，按 profile 各自的 interval
- GUI 托管字段與用戶配置分離：用戶在設置頁改的端口/DNS/TUN 等最後注入，優先級最高

### 2.3 系統集成（Rust `system` 模塊）

- **系統代理**：Windows 註冊表 `Internet Settings` + `InternetSetOption` 刷新；支持 PAC 與繞過列表
- **托盤**：常駐托盤，菜單含模式切換、系統代理/TUN 開關、退出；關窗口=隱藏到托盤
- **開機自啟**：tauri-plugin-autostart；「靜默啟動」選項（啟動只進托盤）
- **單實例**：tauri-plugin-single-instance，二次啟動喚起已有窗口

### 2.4 TUN 提權（Rust `service` 模塊，Windows）

採用 Clash Verge Rev 的成熟方案：安裝一個開機自啟的 Windows 服務（一次性 UAC），服務以 SYSTEM 權限代為啟動內核，GUI 通過本地 IPC（命名管道/HTTP）下發啟停指令。未裝服務時 TUN 開關引導安裝。

## 3. 頁面設計（對標 Sparkle）

| 頁面 | 內容 |
|---|---|
| **概覽** | 實時流量曲線（WS `/traffic`）、上/下行速率、內存佔用、模式三選（規則/全局/直連）、系統代理與 TUN 快捷開關、當前訂閱卡片 |
| **代理** | 策略組列表 → 組內節點網格；節點顯示延遲色階；一鍵組內測速（`/group/{name}/delay`）；隱藏不可用節點、排序選項 |
| **訂閱** | 訂閱卡片（流量條、到期、上次更新）；新增（URL/本地文件）、更新、編輯、刪除、拖拽排序；右鍵進 Monaco 編輯原始 yaml |
| **覆寫** | 覆寫文件列表（yaml/js），Monaco 編輯，可綁定到指定訂閱或全局 |
| **連接** | 實時連接表（WS `/connections`）：host、規則、鏈路、上下行、時長；搜索過濾；單條/全部斷開 |
| **規則** | 規則列表（`/rules`）+ 規則集 provider 列表與手動更新 |
| **日誌** | WS `/logs` 實時滾動，等級過濾，暫停/清空 |
| **設置** | 端口、允許局域網、IPv6、DNS/fake-ip、TUN 詳細參數、內核版本切換與升級、GeoData 更新、主題（亮/暗/跟隨系統）、WebDAV 備份恢復、開機自啟、熱鍵 |

## 4. 里程碑

- **M1 骨架與內核閉環**：Tauri 腳手架、內核啟停守護、API 連通、概覽頁（流量+模式切換）
- **M2 訂閱系統**：訂閱增刪改查/更新/流量信息、覆寫合併管線、Monaco 編輯
- **M3 代理與系統集成**：代理頁全功能、系統代理開關、托盤、開機自啟、單實例
- **M4 監控**：連接頁、日誌頁、規則頁
- **M5 TUN 與服務**：Windows 提權服務、TUN 參數設置、DNS 設置
- **M6 打磨**：內核升級、GeoData 更新、WebDAV 備份、主題、自動更新（tauri-updater）、多語言（zh-TW/zh-CN/en）

## 5. 風險與預案

- **WebView2 缺失**（極老 Win10）：安裝器附帶 Evergreen Bootstrapper
- **殺軟誤報**：TUN 驅動（wintun.dll）與代理行為易被誤報，發佈時代碼簽名可緩解
- **訂閱格式差異**：僅支持 Clash/mihomo 格式訂閱；其他格式後續靠 Sub-Store 集成解決（M6 之後）
