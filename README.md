# Mfork

一個基於 [mihomo](https://github.com/MetaCubeX/mihomo) 內核的桌面代理客戶端。Tauri 2 + React 19 構建，支持 Windows / macOS / Linux（系統代理與 TUN 的系統集成 Windows 優先落地）。

> 設計文檔見 [docs/DESIGN.md](docs/DESIGN.md)，UI 設計稿源文件在 [design/](design/)。

## 開發

前置要求：Node.js ≥ 20、pnpm、Rust stable（Windows 為 MSVC 工具鏈；Linux 另需 webkit2gtk 等，見 `.github/workflows/build.yml`）。

```bash
# 1. 安裝依賴
pnpm install

# 2. 下載當前平台的 mihomo 內核（僅首次；自動識別 OS/架構）
node scripts/fetch-core.mjs

# 3. 啟動開發環境
pnpm tauri dev
```

打安裝包：`pnpm tauri build`（內核隨包分發，用戶無需單獨下載）。推送 `v*` tag 或手動觸發 GitHub Actions 可產出 Windows/macOS(x64+arm64)/Linux 全平台安裝包。

## 當前進度

- [x] M1 骨架與內核閉環：Tauri 腳手架、內核啟停守護、REST API 連通、概覽頁（實時流量/內存/連接數、模式切換）；全部頁面骨架（代理/連接/規則/日誌接真實 API）；連接監控雙視圖（socket 級逐條連接 + 應用級按進程聚合，`find-process-mode: always`）
- [x] M2 訂閱系統：訂閱下載（UA clash.meta、subscription-userinfo 流量頭）、增刪改/更新/切換、本地文件導入、按訂閱設定的定時自動更新；覆寫系統（YAML 深合併含 prepend-/append- 語義 + 內嵌 JS 引擎跑 `main(config)` 變換，Monaco 編輯器，保存即熱重載）；配置生成管線（訂閱 → 覆寫鏈 → GUI 托管字段 → config.yaml → PUT /configs 熱重載）
- [x] M3 代理頁與系統集成：策略組/節點測速、排序/隱藏不可用；系統代理（註冊表 + 繞過列表）、托盤菜單、開機自啟、靜默啟動、單實例
- [x] M4 監控：連接頁（socket / 應用雙視圖、搜索、斷開）、日誌頁（等級過濾、暫停）、規則頁（規則集 provider 手動更新）
- [x] M5 TUN 與提權服務：Windows 服務（SYSTEM 權限代啟內核，一次 UAC）、TUN 參數（協議棧/嚴格路由/DNS 劫持）、DNS 接管（fake-ip / 上游）
- [x] M6 打磨：內核自升級（stable / alpha 通道，經內核代理下載）、GeoData 更新、固定端口 / 局域網 / IPv6、主題（深 / 淺 / 跟隨系統）、多語言（简体 / 繁體 / English，含托盤）、備份（本地 zip + WebDAV 備份/恢復）、應用自動更新（tauri-updater + GitHub Releases）

實機測試清單見 [docs/TESTING-M5.md](docs/TESTING-M5.md) 與 [docs/TESTING-M6.md](docs/TESTING-M6.md)。

## 發佈與自動更新

- 推送 `v*` tag 觸發 CI（`tauri-apps/tauri-action`）：全平台打包、簽名 updater 產物並建立 GitHub Release **草稿**（含 `latest.json`），手動 publish 後應用內「檢查更新」即可看到。
- 倉庫 Secrets 需設置 `TAURI_SIGNING_PRIVATE_KEY`（本機 `~/.tauri/mfork.key` 的內容；公鑰已寫在 `src-tauri/tauri.conf.json`）。私鑰丟失則之後的版本無法被舊版本驗證，務必備份。
- 更新源 endpoint 在 `tauri.conf.json → plugins.updater.endpoints`，目前指向 GitHub `secret-deus/Mfork`，換倉庫時同步修改。

## 架構速覽

- **Rust（src-tauri）**：`core` 進程守護（隨機 API 端口 + secret）、`profile` 訂閱與配置生成、`override_` 覆寫管線、`system` 系統代理/自啟、`tray` 托盤、`service` 提權服務客戶端、`updater` 內核自升級、`backup` 本地/WebDAV 備份、`settings` 應用設置
- **i18n（src/i18n）**：源語言 zh-CN，源字串即 key（`t("啟動內核")`），`en.ts` / `zh-TW.ts` 為字典；新增 UI 文字後補齊兩份字典即可
- **主題（src/index.css + src/theme.ts）**：調色板以 CSS 變量掛在 `<html data-theme>`，Tailwind token 經 `@theme inline` 映射；組件只用 token 類名，不寫硬編碼色值
- **前端（src）**：運行時數據直連 mihomo RESTful API（HTTP + WebSocket），不經 Rust 轉發
