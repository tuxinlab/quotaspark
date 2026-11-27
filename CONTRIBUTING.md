# 贡献指南

感谢你考虑为额度火花 QuotaSpark 贡献代码！这份文档说明如何搭建环境、遵守哪些规范、以及提交改动的方法。

## 行为准则

参与本项目（Issue、PR、评论区）请遵守 [行为准则](CODE_OF_CONDUCT.md)。

## 开发环境

| 依赖 | 要求 |
| --- | --- |
| Node.js | ≥ 20.19（Vite 8 要求） |
| Rust | stable，MSVC 工具链（`rustup default stable-x86_64-pc-windows-msvc`） |
| WebView2 | Windows 10/11 一般已内置 |
| GitHub 账号 | 提 Issue / PR 用 |

```bash
git clone https://github.com/tuxinlab/quotaspark.git
cd quotaspark
npm install
npm run tauri dev     # 跑起来看看
```

## 常用命令

```bash
npm run tauri dev      # 开发模式（改前端热更新，改 Rust 自动重编重启）
npm run tauri build    # 打包安装程序
npx tsc --noEmit       # 前端类型检查
cd src-tauri && cargo test        # Rust 单测
cargo fmt && cargo clippy -- -D warnings   # Rust 格式化与静态检查
npm run e2e            # 端到端测试（启动方式见下）
```

端到端测试会真实驱动 UI，需要先以调试端口启动应用：

```bash
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" npm run tauri dev
# 另开一个终端：
npm run e2e
```

## 改动规范

- **前端**：TypeScript 严格模式，提交前 `npx tsc --noEmit` 必须零错误；样式写在 `src/App.css`，主题色用 CSS 变量（`--accent` 等），不要写死颜色
- **后端**：`cargo fmt` 格式化，`cargo clippy -- -D warnings` 零警告；命令层（`commands.rs`）保持薄，业务逻辑放对应模块
- **涉及行为的改动请补测试**：Rust 逻辑加单测（`src-tauri/src/**/tests`），UI 流程改动跑一遍 `npm run e2e` 确认不回归
- **对 cc-switch 数据库只读**：任何改动都不得写入 `~/.cc-switch/`
- **不要引入新的网络请求**：本工具的联网行为只有激活、额度查询、更新检查三类，新增请求请先开 Issue 讨论

## 提交信息

使用 [Conventional Commits](https://www.conventionalcommits.org/zh-hans/)，描述可中文：

```text
feat: 新增 XX 功能
fix(proxy): 修复 XX 报错
docs: 更新 README
refactor(quota): 重构额度解析
```

常用类型：`feat`（功能）、`fix`（修复）、`docs`（文档）、`refactor`（重构）、`build`（构建/依赖）、`test`（测试）。

## 提交 Pull Request

1. 大改动（新功能、行为变更）请**先开 Issue 讨论方案**，避免做完不被接受
2. 从 `main` 拉功能分支（如 `feat/xxx`），或 Fork 后在自己仓库改
3. 提交前跑一遍上面的检查命令，确保全绿
4. 开 PR 时按模板填写：改了什么、为什么、怎么测的
5. 等维护者 review；有意见就继续在同一分支提交，PR 会自动更新

## 项目结构

```
src/                  # React 前端（App.tsx 顶栏与卡片，ProviderForm 编辑表单）
src-tauri/src/
  commands.rs         # Tauri 命令层（前端 invoke 的入口，保持薄）
  engine.rs           # 激活引擎（最小请求点燃窗口）
  quota.rs            # 15 家供应商的额度/余额查询
  net_err.rs          # 网络错误分类与提示文案
  scheduler.rs        # 定时调度（30s tick + 去重）
  cc_sync.rs          # cc-switch 数据库只读同步
  store.rs / state.rs # 配置持久化与全局状态
  proxy/              # （预留）本地路由转发
docs/RELEASING.md     # 维护者发版流程
```

## 报告问题

Bug 请用 [Issue 模板](https://github.com/tuxinlab/quotaspark/issues/new?template=bug_report.yml) 提交，
尽量附上 `~/.cc-activator/logs/activator.log` 中的相关片段（**注意先抹掉其中的 API Key 等敏感信息**）。
