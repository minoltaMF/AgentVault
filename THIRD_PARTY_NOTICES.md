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

## agent-sessions：Hermes 与 DSH 读取规则

- 仓库：https://github.com/jazzyalex/agent-sessions
- 锁定 commit：ab439f13211e56b809f4a917d5c38e80d2bb5258
- MIT；Copyright (c) 2026 Alexander Malakhov。
- 使用范围：src-tauri/src/hermes_sessions.rs 与 dsh_sessions.rs 的读取和正文投影规则，改编为 Rust；不引入 Swift 应用。原许可证与修改说明见 src-tauri/tests/fixtures/hermes 和 dsh。测试 sample 为合成数据。

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
