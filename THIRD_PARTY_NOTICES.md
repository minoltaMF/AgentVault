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
