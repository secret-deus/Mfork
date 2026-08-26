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
- [ ] M3 代理頁與系統集成：策略組/測速、系統代理、托盤、開機自啟
- [ ] M4 監控：連接、日誌、規則頁；節點地圖
- [ ] M5 TUN 與提權服務
- [ ] M6 打磨：內核升級、WebDAV 備份、自動更新、多語言

## 架構速覽

- **Rust（src-tauri）**：mihomo 進程守護（隨機 API 端口 + 隨機 secret，DIRECT 最小配置起步）、後續的訂閱下載/覆寫合併/系統代理/托盤/TUN 提權
- **前端（src）**：運行時數據直連 mihomo RESTful API（HTTP + WebSocket），不經 Rust 轉發
