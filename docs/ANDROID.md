# Mfork Android 設計

桌面版（見 `DESIGN.md`）的 Android 移植。目標是同一個倉庫、同一套 React 界面與 Rust 業務邏輯，加上一層 Kotlin 的 VpnService 與內核宿主，產出可上架側載的 APK。

## 目標與非目標

目標

- 規則 / 全局 / 直連三種模式、訂閱管理、覆寫、節點選擇、連接與日誌頁：與桌面版同一份代碼。
- TUN 即「連線」：開關一次即建立 VPN、啟動內核、接管全部流量。
- 通知欄常駐狀態（模式、實時速率），系統 VPN 被撤銷時正確收尾。
- 訂閱自動更新、備份還原（本地 zip 與 WebDAV）沿用。

非目標（首版不做）

- 內核自升級：Android 10+ 只允許執行 `nativeLibraryDir` 內的文件，內核版本隨 App 一起發。
- 系統代理、托盤、開機自啟、桌面更新器、提權服務：Android 上無此概念或不適用。
- 分應用代理（per-app）：架構上預留（`include-package` / `addDisallowedApplication`），首版不做界面。
- 平板 / 橫屏專門佈局。

## 架構

```
┌──────────────────────────────────────────────┐
│ React UI（src/，與桌面共用）                   │
│   平台差異由 src/lib/platform.ts 判斷          │
├──────────────────────────────────────────────┤
│ Rust（src-tauri/，與桌面共用）                 │
│   profile / override_ / settings / backup     │  ← 原樣復用
│   core.rs：Android 路徑取 fd 後 spawn 內核     │  ← 平台分支
│   system / tray / service：mobile 為空實現     │
├──────────────────────────────────────────────┤
│ Tauri mobile plugin `mfork-vpn`（Kotlin）      │  ← 新增
│   MforkVpnService : VpnService                 │
│   prepare / establish → fd / stop / revoked    │
├──────────────────────────────────────────────┤
│ mihomo android-arm64 官方二進制                 │
│   打包為 jniLibs/arm64-v8a/libmihomo.so        │
└──────────────────────────────────────────────┘
```

## 關鍵決策

### 1. 復用官方 mihomo 二進制，不用 gomobile

ClashMetaForAndroid 把內核編成 Go 共享庫，透過 JNI 呼叫並在 Go 側實現 `protect()`。我們不需要：mihomo 的 `tun` 配置支持 `file-descriptor: <int>`（`config/config.go` `RawTun.FileDescriptor`），內核以普通子進程運行，TUN 由系統建立後把 fd 交給它即可。

代價：二進制必須放在 `nativeLibraryDir` 才能執行（Android 10+ 的 W^X 限制）。做法是把 `mihomo-android-arm64` 改名為 `libmihomo.so` 放進 `jniLibs/arm64-v8a/`，Gradle 開啟 `useLegacyPackaging`（等價於 `extractNativeLibs=true`）讓系統解壓到磁盤；運行時路徑為 `applicationInfo.nativeLibraryDir + "/libmihomo.so"`。

### 2. 內核由 Rust spawn，fd 由 Kotlin 建立後傳給 Rust

Java 的 `ProcessBuilder` 會在子進程關閉所有非標準 fd，無法把 TUN fd 傳下去；Rust 的 `std::process::Command` 只繼承未設 `FD_CLOEXEC` 的 fd。所以：

1. Kotlin：`VpnService.Builder.establish()` 得到 `ParcelFileDescriptor`，`detachFd()` 取原始 fd，經插件返回給 Rust。
2. Rust：`fcntl(fd, F_SETFD, 0)` 清掉 CLOEXEC，生成配置後 spawn 內核，隨後在父進程 `close(fd)`（子進程已持有副本）。
3. 配置：

```yaml
tun:
  enable: true
  file-descriptor: <fd>
  stack: mixed            # 設置頁可選
  auto-route: false       # 路由由 VpnService.Builder 決定
  auto-detect-interface: false
  dns-hijack: [any:53]
  mtu: 9000
```

`core.rs` 現有的 `core_start` 在 Android 上分支為：向插件要 fd → `generate_config`（注入上面的 tun 塊，忽略 `find-process-mode`，Android 無此能力）→ spawn `nativeLibraryDir/libmihomo.so -d <filesDir>/core -f config.yaml` → 等 API 端口。內核退出的 watcher、日誌文件位置、API 端口 / secret 生成全部沿用。

### 3. 用 `addDisallowedApplication(自己)` 代替 `protect()`

內核發出的連線若再進入 TUN 就會迴環。CMFA 用 `protect(fd)` 逐個放行，需要 Go 側配合。我們把 App 自身排除在 VPN 之外（`Builder.addDisallowedApplication(packageName)`），內核作為同一 uid 的子進程，其所有 socket 自動走物理網卡。這是 sing-box / xray 系外掛內核方案的通行做法。

副作用：App 自己的 WebView 請求（訂閱下載、GitHub API）也不走代理。訂閱更新走 Rust 的 reqwest 同樣不走代理——桌面版本來就是直連下載訂閱，行為一致；需要代理下載時可以顯式指向內核的 mixed 端口（`updater::client(via_core = true)` 已有）。

### 4. VpnService.Builder 參數

| 項 | 值 | 說明 |
|---|---|---|
| `addAddress` | `172.19.0.1/30`，IPv6 開啟時加 `fdfe:dcba:9876::1/126` | TUN 自身地址，與 fake-ip 段錯開 |
| `addRoute` | `0.0.0.0/0`；IPv6 開啟時 `::/0` | 全接管，分流由內核規則決定 |
| `addDnsServer` | `198.18.0.2` | 落在 fake-ip 段內，任何 DNS 查詢都會進 TUN 被 `dns-hijack` 攔下 |
| `setMtu` | 與設置頁 `tun.mtu` 同步（默認 9000） | |
| `addDisallowedApplication` | 自己 | 見決策 3 |
| `setBlocking` | `false` | fd 交給內核自行讀寫 |
| `setMetered` | `false` | |
| `setSession` | 當前訂閱名 | 顯示在系統 VPN 設置裡 |

`onRevoke()`（用戶在系統設置斷開或另一 VPN 搶佔）→ 停內核、發 `vpn-revoked` 事件、前端回到已停止態。

### 5. 前台服務與通知

`MforkVpnService` 在 `establish` 成功後 `startForeground`，通知顯示模式與實時速率（速率由前端經插件 `updateNotification` 推送，每秒一次，App 進背景時由 Rust 側輪詢 `/traffic` 接管——待定，看 WebView 在背景是否被凍結）。Android 14 需要聲明 `foregroundServiceType`，VPN 場景用 `specialUse` 並在 manifest 的 `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` 說明用途（待驗證）。

服務只在 Rust 明確請求（`ACTION_START` 且有回呼等 fd）時才 `establish()`，且回傳 `START_NOT_STICKY`：系統自行拉起服務（「始終開啟 VPN」、或進程被殺後的重啟）時若直接建 TUN，後面沒有內核，所有流量進黑洞而系統設定卻顯示「已連線」。這種啟動改為通知頁面走正常連線流程；App 不在時發一條「點擊打開應用完成連線」的通知。

首次啟動需 `VpnService.prepare()` 的系統授權對話框，以及 Android 13+ 的通知權限；引導放在概覽頁的連線按鈕流程裡。

### 6. TLS：reqwest 切到 rustls

reqwest 默認 `native-tls` 在 Android 上要交叉編譯 OpenSSL，`rustls-tls` 純 Rust、帶 webpki 根證書，桌面行為不變。這一步已隨 M0 落地。

## 目錄與構建

```
src-tauri/
  gen/android/                 # `tauri android init` 生成的 Gradle 工程（提交）
    app/src/main/jniLibs/arm64-v8a/libmihomo.so   # 由 scripts/fetch-core.mjs --android 放置，不提交
  src/core.rs                  # cfg(desktop) / cfg(target_os = "android") 分支
  src/{system,tray,service}.rs # cfg(desktop)
  src/{system,tray,service}_stub.rs  # cfg(mobile) 空實現
plugins/mfork-vpn/             # Tauri mobile plugin（Rust 殼 + android/ Kotlin）
scripts/fetch-core.mjs         # 增加 --android：下載 mihomo-android-arm64 → libmihomo.so
```

構建命令：`pnpm tauri android init`（一次）、`pnpm tauri android dev`（連真機 / 模擬器）、`pnpm tauri android build --target aarch64`（產出 APK / AAB）。CI 加一個 `ubuntu` job 裝 SDK/NDK 打 APK。

工具鏈（Windows 開發機，2026-09-11 實際裝法，全部在用戶目錄、不動系統設置）：

| 組件 | 位置 | 來源 |
|---|---|---|
| JDK 17 (Temurin 17.0.20.1) | `%LOCALAPPDATA%\Programs\jdk-17` | adoptium.net |
| platform-tools r37、platforms android-35/36、build-tools 35.0.0、NDK 27.2.12479018 (r27c) | `%LOCALAPPDATA%\Android\Sdk` | 騰訊鏡像 `mirrors.cloud.tencent.com/AndroidSDK`，zip 手動鋪目錄（dl.google.com 在本機 TLS 握手失敗，sdkmanager 用不了）；`licenses/` 手寫 |
| Rust targets | `rustup target add aarch64-linux-android`（`tauri android init` 又補了 armv7 / i686 / x86_64） | |

環境變量只在構建命令前 export：`JAVA_HOME`、`ANDROID_HOME`、`NDK_HOME`。Gradle 不讀 `HTTPS_PROXY`，本機代理寫在 `~/.gradle/gradle.properties` 的 `systemProp.https.proxy*`；`gen/android` 的 `build.gradle.kts` 與 `buildSrc/build.gradle.kts` 在 `google()` 前加了阿里雲鏡像。AGP 8.11 生成的工程 `compileSdk = 36`，所以 platforms 需要 android-36。

構建：`node scripts/fetch-core.mjs --android arm64`（放置 `libmihomo.so`）→ `pnpm tauri android build --apk --target aarch64`；產物在 `src-tauri/gen/android/app/build/outputs/apk/arm64/release/`。

Windows 上未開「開發者模式」時，tauri-cli 在 cargo 編譯完成後用符號連結把 `libmfork_lib.so` 放進 jniLibs 會失敗（非管理員不能建符號連結）。繞法：cargo 那一步已經完成，手動複製後直接跑 Gradle 並跳過 rust 任務：

```bash
cp src-tauri/target/aarch64-linux-android/release/libmfork_lib.so src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a/
cd src-tauri/gen/android && ./gradlew assembleArm64Release -x rustBuildArm64Release -PtargetList=aarch64 -ParchList=arm64 -PabiList=arm64-v8a
```

`[profile.release]` 開了 `lto` / `codegen-units = 1` / `strip`，Rust 庫從 29 MB 降到 17.7 MB，APK 28 MB（其中 mihomo 佔大頭）。簽名：`app/build.gradle.kts` 從 `keystore.properties` 讀取（本機在 `%LOCALAPPDATA%\Mfork\`，自簽，可換）。

## 前端差異

`src/lib/platform.ts` 導出 `IS_ANDROID`（由 UA 判斷），據此：

- 隱藏：窗口控制按鈕、系統代理行、開機自啟、靜默啟動、提權服務行、內核升級 / 恢復、關於頁的桌面更新器。
- 概覽頁的「啟動內核」即「連線」，會先走 `vpn_prepare`（系統授權）再 `core_start`。
- VPN 模式（`tun.enable`）在 Android 上預設開啟；設定裡叫「服務模式」，關掉即「僅代理模式」（只開本地混合端口，需用戶在 Wi-Fi 詳情手動指代理）。0.1.3 之前的安裝在磁碟上存的是關，`settings.rs` 的 `schema` 遷移（v1）會在升級後首次載入時自動打開，否則連線不會彈系統授權。概覽頁運行時標示當前是 VPN 還是僅代理。
- 佈局：桌面版 `minWidth 1024`，側欄固定 64px。手機上改為底部導航（5 個主頁 + 更多），各頁去掉固定寬度與多列網格，表格改卡片。這是 M1 最大的工作量，需要逐頁過。
- 觸控：地球組件的 pointer 事件已可用；hover 顯示的操作（連接頁的關閉按鈕）改為常顯或滑動。

## 里程碑

| 階段 | 內容 | 驗收 |
|---|---|---|
| **M0 基礎** | 桌面專屬代碼按 `cfg(desktop)` 隔離、reqwest 切 rustls、capabilities 分平台、`platform.ts`；`cargo check --target aarch64-linux-android` 通過 | 桌面構建與測試不變；Android target 可 check |
| **M1 殼** | `tauri android init`、Gradle 工程提交、jniLibs 打包 mihomo、界面響應式改造（第一版：底部導航、單列設置、窄屏內距） | 真機裝上 APK，界面可用；內核以子進程從 nativeLibraryDir 啟動，混合端口可用（Wi-Fi 手動代理），TUN 待 M2 |
| **M2 VPN** | `mfork-vpn` 插件（prepare / establish / stop / revoked）、Rust 側 fd 傳遞與 tun 配置、前台通知 | 真機開關連線，瀏覽器流量按規則分流，`onRevoke` 收尾正確 |
| **M3 打磨** | 通知速率、電池優化引導、Android 13 通知權限、訂閱 / 覆寫 / 備份在手機上驗證、IPv6 開關 | 一天日常使用不掉線 |
| **M4 發佈** | CI 出 APK、簽名、版本號與桌面對齊、README | tag 觸發產出 arm64 APK |

## 風險與待驗證

1. **fd 繼承**：`establish()` 返回的 fd 是否帶 CLOEXEC，Rust 端清位後子進程能否使用。M2 第一件事，寫一個最小驗證：spawn `libmihomo.so -v` 前後對比 `/proc/self/fd`。
2. **`file-descriptor` 模式的完整性**：mihomo 拿到 fd 後是否還嘗試建 device / 設路由（應為否，`auto-route: false` 保險）；`stack: system` 與 `gvisor` 在 Android 上的表現與功耗，需實測選默認值。
3. **fake-ip 與系統 DNS**：`addDnsServer(198.18.0.2)` 依賴 `dns-hijack` 攔截；若用戶切到 `redir-host`，改為把上游 DNS 直接下發。
4. **WebView 背景凍結**：App 進背景後前端的 WS 與計時器可能被凍結，通知速率需由原生或 Rust 側維持。
5. **boa_engine 在 arm64-android 的編譯**：純 Rust，預期可過，但編譯時間長，CI 需緩存。
6. **16 KB 頁對齊**：Android 15 起要求 `.so` 16 KB 對齊，Rust cdylib 需 `-C link-arg=-Wl,-z,max-page-size=16384`；`libmihomo.so` 是靜態 Go 二進制、由我們 exec 而非 dlopen，不受此限。
7. **Google Play 政策**：VPN 類應用需申報 `VpnService` 用途；首版側載發佈不涉及。
