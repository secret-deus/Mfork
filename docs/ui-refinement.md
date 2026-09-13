# 原版界面精修与 Three.js 地球

## 范围

- 沿用开放式桌面布局和纯图标侧栏，不引入仪表盘卡片。
- 调整正文与元信息字重、字距和分隔线；代理名称与延迟分列对齐，选中项保留淡色背景。
- 手机端代理组、连接列表去掉卡片容器；保留底部导航、长名称换行和触控尺寸。
- 概览保留实时速率、内核指标、模式与系统代理/TUN 控制；收紧左右内容重叠，短窗口可滚动。
- 使用按需加载的 Three.js ES module 替换旧 Canvas 2D 地球。地图继续使用项目已有的 `world-atlas` / Natural Earth 数据，不加载 CDN 或在线纹理。

## 地球的数据与生命周期

`GlobeProps` 保持兼容：`self`、可用 `nodes`、包含有序多跳的 `routes` 和 `active` 均由现有概览逻辑提供。位置仍由原有 `lib/geo.ts` 推断；本次没有新增定位请求，也没有把概念稿示例写入生产代码。

- 真实节点、多跳中继与出口用提升的大圆弧连接；活动连接数影响线路亮度与流动速度。
- 地球、纹理、renderer 每次组件挂载只创建一次。连接数量变化不会重建路线几何；拓扑/坐标变化才重建。
- 深浅主题沿用 CSS 变量；地图栅格缓存最多两份。
- 每个 effect 挂载拥有独立 canvas，避免 React StrictMode/HMR 中旧 context-loss 事件污染新实例。
- 卸载会释放 GPU 资源、context、事件监听、ResizeObserver、IntersectionObserver、主题订阅与 RAF。
- 动画上限约 30 FPS；隐藏、离屏、内核未运行或减少动态效果时不持续绘制。
- 支持指针拖拽、方向键、Home 和重置按钮；WebGL 不可用时仅显示提示，不影响网络功能；context 恢复后重启绘制。

## 验证

### 数学测试

Node.js 22.6+（验证环境 Node.js 24）：

```sh
pnpm test:globe
```

9 项用例：UV 方位、邻近/跨太平洋/跨日期变更线/重合/对跖/极点弧线、多跳接点、非法坐标过滤。

### 浏览器组件回归

```sh
pnpm dev
```

打开 `http://localhost:1420/tests/globe-browser.html`，点击 **Run checks**。独立挂载生产地球组件，不调用 Tauri 或网络内核。

已在 Edge 无头浏览器（SwiftShader WebGL2、原生 RAF）验证 17 项检查：严格模式、实时更新、主题、键盘与重置、停止/恢复动画、隐藏/恢复、context loss/restore、语言标签、卸载释放、减少动态效果和重挂载。后台受限宿主可使用页面内的测试时钟；正式组件没有该 mock。

### 应用布局回归

打开 `http://localhost:1420/tests/app-browser.html`。测试入口使用 Tauri 官方 mock 与内存 fixture，禁止在此页把示例状态当成真实网络状态。

已在 1280×800 和 390×844 下切换概览、代理、订阅、覆写、连接、规则、日志、设置：无页面异常与页面级横向溢出；深色与浅色主题可切换。测试页不属于 Vite 正式构建入口。

### 构建

```sh
pnpm build
pnpm tauri build --no-bundle
```

前端和 Windows release 构建通过。Vite 仍提示较大的 Monaco 主包与 Three.js 分包；地球已单独按需加载，没有隐藏体积警告。`--no-bundle` 不产生安装包，也不验证自动更新签名。

## 未执行的系统操作

没有启动新构建的程序，以免触发原有自动连接；没有切换真实系统代理/TUN、执行真实节点测速或订阅更新。Android 安装包与真机 GPU 表现未在本次 Windows 环境验证。
