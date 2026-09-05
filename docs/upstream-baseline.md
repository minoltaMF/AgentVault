# AgentVault 上游基线

本文记录 AgentVault 首次整理 fork 时的可复现基线。记录日期为 2026-09-04。

## 上游来源

- 仓库：https://github.com/ccpopy/cc-sessions
- 分支：`main`
- 锁定 commit：`1c912b2bb35e328881f543dfef1eeb5ff510f2bf`
- 上游提交标题：`feat(export): 增加 Markdown 时间范围筛选`
- 许可证：MIT；根目录 `LICENSE` 保留上游 `Copyright (c) 2026 ccpopy`，未作修改。
- Git 远程：上游只登记为 `upstream`，本次未配置或推送 AgentVault 远程。

## 当前技术栈

- 桌面与本地服务：Tauri 2、Rust 2021 edition。
- 前端：React 18、TypeScript 5、Vite 6、React Router、Zustand、Radix UI、Tailwind CSS。
- 本地数据访问：`rusqlite 0.32`（bundled SQLite）、JSON/JSONL 文件、Tauri 文件与对话框插件。
- CLI 与 Web UI：Rust binary `cc-sessions` 加同一套前端静态资源。
- 包版本：npm package 与 Rust package 均为 `0.6.3`。名称仍为 `cc-session-manager`，本轮不做兼容性迁移。
- Node.js 要求：20 或更高版本；依赖安装以根目录 `package-lock.json` 为准。

## 构建与测试命令

先安装依赖：

```bash
npm ci
```

前端：

```bash
npm run dev
npm run test:frontend
npm run build
```

仓库没有单独的 JavaScript/TypeScript lint 脚本。当前 CI 的格式检查和 Rust 验证为：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

CLI：

```bash
npm run cli:check
npm run cli:build
npm run cli:run -- --help
```

Tauri：

```bash
npm run tauri:dev
npm run tauri:build
```

## 修改前基线结果

本机为 Windows ARM64，预装环境没有 npm、Rust 或 Visual Studio Build Tools。为避免污染系统，基线使用临时目录中的 npm 11.6.0、Rust 1.90/1.98 x86_64 GNU 与 MSYS2 MinGW 工具链；项目文件未因工具准备而改变。

| 命令 | 修改前结果 |
| --- | --- |
| `npm ci` | 通过；安装 420 个包。npm audit 报告 1 个 low、1 个 high，未运行 `audit fix`。 |
| `npm run build` | 通过；Vite 报告 `vendor-charts` 538.37 kB，超过默认 500 kB 提示阈值。 |
| `npm run test:frontend` | 通过；67/67。 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 通过。 |
| `cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | 通过。 |
| `cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib` | 通过；499/499。该命令覆盖可在当前宿主链接运行的 Rust 单元测试。 |
| `cargo check --manifest-path src-tauri/Cargo.toml --no-default-features --bin cc-sessions` | 通过。 |
| `cargo test --manifest-path src-tauri/Cargo.toml --all-targets` | 当前宿主未完成：Windows GNU 链接 Tauri `cdylib` 时出现 PE 导出序号/MinGW 运行库错误；测试代码本身随后用上面的纯 Rust lib 命令全部通过。 |
| `npm run tauri:build -- --no-bundle` | 前端预构建通过；Windows GNU release LTO 长时间运行后按停止条件终止。未得到可执行产物，需在受支持的原生 MSVC、macOS 或 Linux 构建环境复验。 |

上表中两项未完成均是当前宿主缺少受支持原生工具链造成的覆盖缺口，不是修改前发现的源码回归。本轮改动后必须复跑其余可执行命令并与此基线对照。

### 本轮改后对照

同日将完整 tracked diff 应用到本地临时镜像后，前端测试仍为 67/67、Rust lib 测试仍为 499/499，前端生产构建、Rust 格式检查、全目标 `cargo check`、CLI `cargo check`、JSON 配置解析和发布脚本语法检查均通过。生产构建仍只有同一条 538.37 kB chunk 警告，没有本轮新增失败。实际工作区直接运行前端测试也为 67/67；其 Vite 构建因 Windows 把同一 UNC 共享解析为多个盘符而失败，本地镜像中的同一源码和命令已通过。

## 当前已有功能

当前代码继承 cc-sessions 的完整功能，并已开始以独立 workspace crate 落地 AgentVault 新架构：

- 浏览、筛选、搜索和预览 Codex、Claude Code、OpenCode、Cursor 会话。
- 会话重命名、归档、删除、项目分组和续聊命令复制。
- Codex、Claude Code、OpenCode 的文本编辑、删除事件、撤销与快照恢复。
- 四类 Provider 的选择性备份、恢复、会话包导入导出和 Markdown 导出。
- Codex、Claude Code、OpenCode 的目录移动；Cursor 只读路径解析与数据库内会话管理。
- 支持范围内的跨 Provider 会话转换、Codex 分支/模型服务同步、Claude Memory 管理。
- 使用统计、Codex/Claude 可见性修复、Codex 索引修复和 Cursor 残留诊断/清理。
- Tauri 桌面界面、`cc-sessions` CLI 和本地 Web UI。
- Provider SDK、Claude/Codex 只读发现、Pi v1/v2/v3 只读解析与分支图。
- 可重建的 canonical SQLite registry、跨 machine/source 隔离的 native identity，以及逐 source file 的增量游标与事务投影提交。

这些是兼容基线，不代表后续 AgentVault 的默认策略；后续新增流程必须遵循原生 Session 默认只读的边界。

## 应用数据与数据库位置

本轮不迁移任何路径或 schema。AgentVault 仍沿用上游位置：

| 用途 | 当前默认位置 |
| --- | --- |
| Codex 根目录 | `~/.codex` |
| Codex 会话 | `~/.codex/sessions/`、`~/.codex/archived_sessions/` |
| Codex 索引与数据库 | `session_index.jsonl`、`history.jsonl`、`state_5.sqlite`、`logs_2.sqlite`、`thread_history_1.sqlite`（均相对 Codex 根目录） |
| 上游自有 Codex 辅助元数据 | `session_family.json`、`archive_ledger.json`、`session_provenance.json`（均相对 Codex 根目录） |
| Claude Code 根目录 | `~/.claude`，会话位于 `projects/` |
| OpenCode 根目录与数据库 | `~/.local/share/opencode/opencode.db` |
| Cursor IDE | 平台配置目录下的 `Cursor/User/globalStorage/state.vscdb`，并读取 `workspaceStorage/` |
| cursor-agent | `~/.cursor/chats/` |
| 默认备份目录 | `~/cc-backups` |
| Tauri 桌面设置 | Tauri `app_config_dir()/settings.json`；目录继续由 bundle identifier `dev.cc.session-manager` 决定 |
| CLI Web UI 设置 | `CC_SESSIONS_WEBUI_SETTINGS` 指定位置；便携模式为 executable 旁 `cc-sessions-webui-settings.json`，否则为系统配置目录下 `cc-sessions/cc-sessions-webui-settings.json` |

`crates/registry` 已提供独立 canonical registry schema，但本轮不选择默认文件位置，也不接入 Tauri/CLI，因此不会创建或迁移任何实际应用数据。当前仍没有 AgentVault Vault 或 FTS 数据库。

## 当前已知风险

- 继承代码包含编辑、删除、恢复、移动、修复和数据库清理等写操作；它们不能被视为 AgentVault 后续架构的默认只读行为。
- 多个辅助元数据文件仍写在 Codex 根目录，尚未迁移到独立 Vault；路径和所有权需要后续兼容设计。
- Cursor 将大量状态放在共享 `state.vscdb`，且运行时会回写缓存；任何写操作都必须确认 Cursor 已退出并保持事务/冲突保护。
- 更新检查、发布页链接和制品文件名仍指向或沿用 cc-sessions。当前没有 AgentVault 发布源，不能把上游 release 当作 AgentVault release。
- Provider SDK 和 canonical registry 尚未接入旧 Tauri/CLI 查询路径；当前没有不可变 Vault、FTS、Recovery Capsule 或新的默认数据目录。
- npm 基线审计存在 1 个 low、1 个 high；本次不升级依赖，需在独立依赖维护任务中确认可利用性和兼容性。
- `vendor-charts` 生产 chunk 超过 Vite 默认提示阈值；当前只是体积警告。
- 本机 Windows ARM64 未覆盖原生 Tauri 桌面链接/打包；CI 或受支持的原生环境仍是发布前必需验证。

## 后续重构边界

- 下一步只在 canonical registry 之上增加 unicode/trigram 搜索，不同时实现 resume、Vault 或 Recovery Capsule。
- Provider SDK、Pi、registry 已作为独立 workspace crate 建立，但尚未替换旧 Tauri 业务路径；接入必须保持兼容且不得触碰原生 Session。
- 在有显式迁移方案前，保持 bundle identifier、Rust/npm package 名、CLI binary 名、应用数据目录、配置路径、SQLite 文件名和 schema 不变。
- 原生 Agent Session 默认只读；任何写回必须复用或加强现有原子写入、事务、快照、CAS、路径安全和补偿机制。
- 不为重构删除现有功能或弱化现有测试；每一步先建立等价回归覆盖，再移动实现。
