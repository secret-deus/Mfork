# Mfork

一個基於 [mihomo](https://github.com/MetaCubeX/mihomo) 內核的桌面代理客戶端。Windows 優先，Tauri 2 + React 19 構建。

> 設計文檔見 [docs/DESIGN.md](docs/DESIGN.md)，UI 設計稿源文件在 [design/](design/)。

## 開發

前置要求：Node.js ≥ 20、pnpm、Rust stable（MSVC 工具鏈）。

```bash
# 1. 安裝依賴
pnpm install

# 2. 下載 mihomo 內核到 src-tauri/resources/mihomo/（僅首次）
powershell -File scripts/fetch-core.ps1

# 3. 啟動開發環境
pnpm tauri dev
```

## 當前進度

- [x] M1 骨架與內核閉環：Tauri 腳手架、內核啟停守護、REST API 連通、概覽頁（實時流量/內存/連接數、模式切換）；全部頁面骨架（代理/連接/規則/日誌接真實 API）；連接監控雙視圖（socket 級逐條連接 + 應用級按進程聚合，`find-process-mode: always`）
- [x] M2 訂閱系統（核心）：訂閱下載（UA clash.meta、subscription-userinfo 流量頭）、增刪改/更新/切換、配置生成管線（訂閱 + GUI 托管字段合併 → config.yaml → PUT /configs 熱重載）
- [ ] M2 餘項：覆寫合併（yaml/js）、定時自動更新、本地文件導入、Monaco 原始編輯
- [ ] M3 代理頁與系統集成：策略組/測速、系統代理、托盤、開機自啟
- [ ] M4 監控：連接、日誌、規則頁；節點地圖
- [ ] M5 TUN 與提權服務
- [ ] M6 打磨：內核升級、WebDAV 備份、自動更新、多語言

## 架構速覽

- **Rust（src-tauri）**：mihomo 進程守護（隨機 API 端口 + 隨機 secret，DIRECT 最小配置起步）、後續的訂閱下載/覆寫合併/系統代理/托盤/TUN 提權
- **前端（src）**：運行時數據直連 mihomo RESTful API（HTTP + WebSocket），不經 Rust 轉發
