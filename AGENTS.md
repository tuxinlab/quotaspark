# AGENTS.md

额度火花 QuotaSpark：Windows 托盘工具（Tauri 2 · Rust · React 19 · TypeScript），
定时向 Coding Plan 供应商发送 `max_tokens=1` 的最小请求，"点燃"滚动 5 小时额度窗口。
仅面向 Windows（WebView2）。仓库目录名 `cc-activator`，产品/包名 `quotaspark`，Rust crate 名 `cc-activator`。

## 文档索引

- `README.md` —— 功能与使用说明
- `CONTRIBUTING.md` —— 贡献规范（命令、代码规范、提交格式）
- `docs/RELEASING.md` —— 维护者发版与签名流程
- `scripts/e2e.mjs` —— 端到端测试（`data-testid` 锚点以该文件为准）

## 常用命令

```bash
npm install                 # 前置：Node ≥ 20.19、Rust stable（MSVC 工具链）
npm run tauri dev           # 开发调试（前端热更新，改 Rust 自动重编重启）
npm run tauri build         # 打包安装程序（签名需 TAURI_SIGNING_PRIVATE_KEY，见 docs/RELEASING.md）
npx tsc --noEmit            # 前端类型检查（严格模式，提交前须零错误）
cd src-tauri && cargo test  # Rust 单测
cd src-tauri && cargo fmt && cargo clippy -- -D warnings   # 须零警告
```

端到端测试真实驱动 UI：先以
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" npm run tauri dev`
启动应用，再另开终端跑 `npm run e2e`（scripts/e2e.mjs，经 CDP 附加 WebView2；截图落 e2e-artifacts/）。

## 结构与分层

- `src/`：React 前端。App.tsx（顶栏 + 供应商卡片）、ProviderForm.tsx（编辑表单）、
  UpdateButton.tsx、api.ts（Tauri invoke 封装）。样式只写在 `src/App.css`。
- `src-tauri/src/`：
  - `commands.rs` — Tauri 命令层（前端 invoke 入口），保持薄，业务逻辑放对应模块
  - `engine.rs` — 激活引擎；模型名会剥掉 cc-switch 的 `[1M]` 后缀
  - `quota.rs` — 14 家供应商额度/余额查询（端点与解析移植自 cc-switch，含 Command Code 与小米 MiMo；识别按域名边界匹配）
  - `scheduler.rs` — 30s tick 调度 + 触发去重（去重键持久化在配置，重启不重发）
  - `cc_sync.rs` — 同步 `~/.cc-switch/cc-switch.db`
  - `store.rs` / `state.rs` — `~/.cc-activator/config.json` 持久化（原子写入）与全局状态
  - `lib.rs` — 插件注册与托盘

## 硬性规则

- **cc-switch 数据库只读**：任何改动不得写入 `~/.cc-switch/`
- **不得新增网络请求**：联网行为仅限激活、额度查询、更新检查三类；要加先开 Issue 讨论
- 配置含 API Key：只存本机、原子写入，不上传、不写日志
- `lib.rs` 中 single-instance 插件必须保持第一个注册（防双实例争写 config.json）
- UI 的 `data-testid` 是 e2e 定位锚点（`provider-card`、`time-chips`、`sched-toggle` 等），
  改 UI 时保留或同步更新 scripts/e2e.mjs
- 主题色用 CSS 变量（`--accent` 等），不写死颜色

## 已知陷阱（前人踩过的坑，勿重蹈）

- **Mutex 不可跨 await**：`state.0.lock()` 的 guard 绝不能跨 `.await` 持有（会卡死全部命令）。
  既定模式：锁内取数据 → 释放 → await → 再锁写回（参考 commands.rs 的 `spawn_activation` /
  `fetch_and_store_quota`）
- **单实例锁**：exe 已在运行时再启动会被拦截，表现为"启动了但没窗口"（第二个进程直接退出）。
  dev 调试前先清旧实例：`taskkill //IM cc-activator.exe //F`
- **Vite 监听已改轮询**（vite.config.ts `usePolling: true`）：若前端改动"没生效"，
  多半是 dev 实例太旧（改 Rust 会重启应用但不会重启前端缓存之外的问题），重启 dev 即可
- **卡片有 hover transform**：卡片内 `position: fixed` 元素的包含块会被 transform 劫持——
  弹层/浮窗**不要用 fixed 全屏遮罩**做点外关闭，用 document 级 mousedown 监听
  （SchedRow 的既定写法）
- **e2e 的 chips/时间断言按内容定位**：chips 排序显示，且用户自己的卡片也有 chips，
  必须用 `testChips(page)` + `filter({ hasText })`，不要按位置取
- **改版本号后 Cargo.lock 会跟着变**（build 时），记得一并提交
- **改应用图标**：改 `public/favicon.svg` 同款设计 → 渲染 1024×1024 的 `app-icon.png` →
  `npx tauri icon app-icon.png` 重新生成全套（打包后生效）

## 发版注意

版本号须三处同步：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`，
并更新根目录 `latest.json`（version/signature/url）。完整流程与签名密钥说明见
`docs/RELEASING.md`——更新私钥（`~/.tauri/quotaspark.key`，不入库）丢失则无法再发更新。

## 其他

- 提交信息用 Conventional Commits，描述可中文（如 `feat(quota): …`）
- 行为改动补测试：Rust 逻辑加单测；UI 流程改动跑 `npm run e2e` 确认不回归
- 运行日志在 `~/.cc-activator/logs/activator.log`
- 改敏感区域（quota.rs、cc_sync.rs、更新器）前先读 CONTRIBUTING.md 与 docs/RELEASING.md
