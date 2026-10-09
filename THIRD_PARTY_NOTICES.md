# Third-Party Notices

AgentVault 当前直接包含下列上游项目的源代码。本文件只记录已实际复用的代码，不代表开发方案中提到的候选项目已经被引入。

## cc-sessions

- 上游仓库：https://github.com/ccpopy/cc-sessions
- 锁定 commit：`1c912b2bb35e328881f543dfef1eeb5ff510f2bf`
- 许可证：MIT License
- 上游版权：Copyright (c) 2026 ccpopy
- 使用范围：AgentVault 的当前应用源码、构建配置、测试和资源均以该 commit 为 fork 基线，后续修改由本仓库 Git 历史记录。

上游 MIT License 全文及版权声明按原样保留在仓库根目录的 [`LICENSE`](LICENSE) 中。

## kage — Qoder CLI 测试夹具

- 上游仓库：https://github.com/farmcan/kage
- 锁定 commit：`729873ec59f39847fa0ea87e3c7b6fb8a312d423`
- 许可证：MIT License；版权原文：`Copyright (c) 2026`
- 使用范围：`src-tauri/tests/fixtures/qoder/sample-qoder-session.jsonl`，来自上游同名 fixture，用于 Qoder CLI 0.1.29 消息格式回归；未引入其 JavaScript 运行环境。
- 完整许可证保留在 [`src-tauri/tests/fixtures/qoder/LICENSE`](src-tauri/tests/fixtures/qoder/LICENSE)。

## Grok Build CLI

- 上游仓库：https://github.com/xai-org/grok-build
- 锁定 commit：`4247f661689354b831191f11eeeac8424993fe3d`
- 许可证：Apache-2.0；Copyright 2023-2026 SpaceXAI。
- 使用范围：`src-tauri/src/grok_sessions.rs` 的用户轮次与 rewind 过滤算法，以及 `src-tauri/tests/fixtures/grok` 中依据上游构造器改编的合成夹具。仅移植小模块，不引入上游运行环境。修改说明在模块头部和夹具 README；完整许可证见 [LICENSE](src-tauri/tests/fixtures/grok/LICENSE)。

## workbuddy-exporter

- 上游仓库：https://github.com/lisaallen0823-art/workbuddy-exporter
- 锁定 commit：`52189e6ae48cb1acb90afaa92b729d99d1f1b1bb`
- 许可证：MIT；Copyright (c) 2026 lisaallen0823。
- 使用范围：`src-tauri/src/workbuddy_sessions.rs` 参考其 `jsonl2md.py` 的 user_query、cb_summary 与 output_text 提取规则改写为 Rust；夹具由 AgentVault 合成，不含真实会话。不引入 Python、图片复制或原生写入。完整许可证见 [LICENSE](src-tauri/tests/fixtures/workbuddy/LICENSE)。

## agent-sessions：会话读取规则与测试夹具

- 仓库：https://github.com/jazzyalex/agent-sessions
- 锁定 commit：ab439f13211e56b809f4a917d5c38e80d2bb5258
- MIT；Copyright (c) 2026 Alexander Malakhov。
- 使用范围：src-tauri/src/hermes_sessions.rs 与 dsh_sessions.rs 的读取和正文投影规则，改编为 Rust；不引入 Swift 应用。原许可证与修改说明见 src-tauri/tests/fixtures/hermes 和 dsh。Hermes/DSH 测试 sample 为合成数据。
- alpha.11 扩充：qwen_sessions.rs、cline_sessions.rs、copilot_sessions.rs、antigravity_sessions.rs 移植对应 Qwen、Cline、Copilot 与 Antigravity Discovery/Parser 的路径与角色投影规则。Cline 和 Antigravity 复用上游 stage0 小夹具；Copilot 仅保留上游 small.jsonl 前 15 条完整事件，排除后续占位 schema 示例；Qwen 夹具为 AgentVault 合成。分别在 src-tauri/tests/fixtures/{qwen,cline,copilot,antigravity} 保存 MIT 原文及 README 修改说明；不引入上游 Swift 应用或运行环境。

## DeepSeek Harness：v4 存储合同

- 仓库：https://github.com/deepseek-ai/deepseek-harness
- 锁定 commit：00102833dfaee1da9f48a3a8eae9d34005a75218
- MIT；原始许可证见 src-tauri/tests/fixtures/dsh/LICENSE.deepseek。
- 使用范围：dsh_sessions.rs 文件名/事件投影规则，known-events.txt 从官方 persistence-schema.json 提取事件名。当前仅读 v3/v4，不移植历史迁移器或 Agent 运行环境。

## ZCode：SQLite 会话读取

- 仓库：https://github.com/zai-org/ZCode
- 锁定 commit：872ad960de7ec172591f7e1952f7849229f94521
- Apache-2.0；Copyright 2026 Z.AI Co., Ltd。
- 使用范围：zcode_sessions.rs 的默认数据位置、session/message/part 读取和 sequence 排序，改编为 Rust；不包含原生写入、迁移或 Node 运行环境。原始许可证和修改说明见 src-tauri/tests/fixtures/zcode。

## Qwen Code：路径与 transcript 合同

- 仓库：https://github.com/QwenLM/qwen-code
- 锁定 commit：`085e98c00cac2f8dd29eb39c760409bc6da889a9`。
- 许可证：Apache-2.0；完整原文见 [LICENSE.qwen-code](src-tauri/tests/fixtures/qwen/LICENSE.qwen-code)。锁定 commit 的仓库根目录未提供 NOTICE 文件。
- 使用范围：qwen_sessions.rs 根据 packages/core/src/config/storage.ts、services/session-transcript-reader.ts 与 utils/transcript-records.ts 核对文件布局、父链及消息结构，改编为 Rust 只读投影；夹具由本项目合成，不复制 Agent 运行时，不引入 Node 运行环境。

## GitHub Copilot SDK：事件字段合同

- 仓库：https://github.com/github/copilot-sdk
- 锁定 commit：`075f027363fc3b1e904d09370763731c3ecd2d88`。
- MIT；Copyright GitHub, Inc.；原文见 [LICENSE.copilot-sdk](src-tauri/tests/fixtures/copilot/LICENSE.copilot-sdk)。
- 使用范围：copilot_sessions.rs 依据 nodejs/src/generated/session-events.ts 核对事件信封和正文、工具字段；Rust 投影及测试来源另见上述 agent-sessions 条目。
- github/copilot-cli 的文档/分发仓库核对 commit 为 `57dd2440141be0b7d6d628472890f861e3b3ca55`。该仓库不提供 CLI 运行时实现；此接入不声称 Copilot CLI 本身开源，没有复制其专有运行时代码。

## Wake：持久搜索索引查询规则

- 仓库：https://github.com/iAmCorey/Wake
- 锁定 commit：`71aeca67ec80f8645d1f9d5199290c2c732036ce`。
- 许可证：MIT；Copyright (c) 2026 Corey Chiu。完整许可证见 `docs/licenses/Wake-MIT.txt`。
- 移植范围：`crates/registry/src/lib.rs` 的 `body_matches` 改编自 `crates/wake-core/src/db.rs` 的 `fts_match_expr` / `needs_like_fallback`：复用 FTS5 字面量引号转义及短查询降级规则，改为本项目既有的完整子串语义和指定 session 主键范围。
- `src-tauri/src/workbench_index.rs` 参考 Wake scanner 的文件大小、修改时间和解析器版本失效设计；复用 AgentVault 既有 Registry schema、消息分类器与原子事务，不复制 Wake Store/schema、GPUI、MCP 或整套扫描器。新增夹具由 AgentVault 合成。

## Magpie：Codex / Claude 会话用量归一化

- 仓库：https://github.com/yetone/magpie
- 锁定 commit：`d1d4b7ed2a34cd3aac280ce0e69dfce2989264e9`。
- 许可证：MIT；Copyright (c) 2026 yetone。完整原文见 [Magpie-MIT.txt](docs/licenses/Magpie-MIT.txt)。
- 改编范围：`src-tauri/src/session_usage/codex.rs`、`claude.rs` 参考并移植 `internal/sessions/codex_usage.go`、`claude_usage.go`、`claude.go` 的响应身份、累计差值、修订去重及缓存 TTL 规则；将部分算法改写为 Rust 按次流式读取，不引入 Go、网关或上游缓存数据库。
- 回归场景改编自 `codex_usage_test.go`、`claude_usage_test.go`、`claude_cache_ttl_test.go`；`tests/fixtures/usage/codex-compaction-counter-domains.jsonl` 复制自该提交同名 testdata 合成夹具。其他测试仅使用合成数据。
- 不声称完整移植上游全部分页恢复、跨计数域推断或价格服务；歧义和缺失记录明确标注。配置写入、认证和模型调用未移植。
