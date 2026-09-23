# Agent 来源接入核实与复用基线

初次核实日期：2026-09-23；实现状态更新：2026-09-24；AgentVault 基线：`c7db198` / `0.1.0-alpha.9`。
本轮已实现 DSH/Hermes/ZCode 只读适配器，并把 OpenCode 接入工作台，准备 alpha.10 draft。下列后续候选不代表已经接入；实际移植范围同步至 `THIRD_PARTY_NOTICES.md`、原始许可证与文件级修改说明。

## 已选定范围

- DSH：只读来源发现、列表、正文预览、搜索定位；已支持 v3/v4 canonical JSONL、Zstandard 连续帧；v0–v2、未知版本或必要事件明确拒绝。基于官方 v4 目录补足候选解析器规则，不承诺全部历史版本兼容。
- Hermes Agent：已接入 state.db 中 sessions/messages，只读查询并检测可选字段；不引入 Python 运行环境。
- ZCode：已接入 cli/db/db.sqlite 的 session/message/part，复用官方路径与 sequence 排序规则；不创建或迁移数据库。
- 下一轮 Cline：先覆盖已经有格式证据的 CLI/Desktop；VS Code 扩展历史另行核对，不宣称自动兼容。
- 下一轮 Antigravity：分别识别 transcript 和 Markdown 任务产物；只有产物时标明内容不完整，不充当完整对话。
- OpenCode：复用现有 `opencode_sessions.rs` 与 `content_search.rs`，已补齐统一工作台来源、筛选、扫描和命中定位，不重写已有解析器。
- 下一轮 GitHub Copilot CLI：读取 CLI 会话 events.jsonl；不包含 VS Code Copilot Chat。
- Roo Code、Kilo Code：按用户要求暂不安排。

用户确认批次：本轮 DSH + Hermes + OpenCode + ZCode；下一轮 Qwen Code + Cline CLI/Desktop + Copilot CLI + Antigravity。QoderWork 先验证格式，确认后加入下一轮。VS Code Copilot Chat 与 Cline 扩展版延后。各批继承取消扫描、真实进度、文件级失败、有界预览和只读保护。每批实现完成后按 release-workflow.md 独立验证和生成 draft，本轮实现进入 alpha.10 验证与 draft 交付流程，后续候选调研不单独发版。

## agent-sessions 候选复用基线

- 仓库：https://github.com/jazzyalex/agent-sessions
- 锁定 commit：`ab439f13211e56b809f4a917d5c38e80d2bb5258`
- 许可证：MIT；Copyright (c) 2026 Alexander Malakhov。
- 固定许可证：https://github.com/jazzyalex/agent-sessions/blob/ab439f13211e56b809f4a917d5c38e80d2bb5258/LICENSE
- 优先读取：`AgentSessions/DeepSeekHarness/`、`AgentSessions/Services/HermesSessionParser.swift`、`ClineSessionDiscovery.swift`、`ClineSessionParser.swift`、`AntigravitySessionDiscovery.swift`、`AntigravitySessionParser.swift`、`AntigravityTranscriptParser.swift` 及对应测试。
- DSH 测试入口：`AgentSessionsTests/DeepSeekHarnessFixtureParityTests.swift`、`DeepSeekHarnessSessionParserTests.swift` 与 `Resources/Fixtures/stage0/agents/deepseek-harness/`。
- 复用方式：格式识别、读取算法、最小测试夹具；跨语言移植至现有 Rust 后端，不引入 Swift 应用或运行环境。
- 已知限制：DSH 当前候选仅接受版本 0...3；必须以官方 v4 文档补足。Cline CLI/Desktop 不等同扩展存储。Antigravity 的 brain Markdown 不保证完整聊天。

## 其他产品核实（截至 2026-09-23）

| 产品 | 当前证据与结论 |
| --- | --- |
| Gemini CLI | 官方仍有持续发布，2026-09-22 有 nightly，发布列表包含稳定版 v0.60.0；没有停维护。 |
| Qwen Code | 官方仍发布 CLI、Desktop 与 SDK，发布列表包含 v0.24.3 和 2026-09-22 nightly；没有被 Qoder 替代的依据，按独立来源处理。 |
| Kimi | 旧 `MoonshotAI/kimi-cli` 已归档并声明不再维护；当前 `MoonshotAI/kimi-code` 是维护中的 Kimi Code CLI。名称有沿用，适配时按数据格式/版本区分，不能按名称视为同一个存储协议。 |
| QoderWork | 官方确认任务历史保存在本机，新 Qoder 可只读导入 QoderWork 全部用户对话。存在本地接入依据，但本轮未验证其磁盘 schema 和脱敏样本，不能直接套 Qoder CLI 解析器。 |
| ZCode | 本文指智谱/Z.ai 的 `zai-org/ZCode`。官方已公开 Desktop/Web/CLI 与 Agent 源码，Apache-2.0；默认数据库 `~/.zcode/cli/db/db.sqlite`，有 session/message/part 读取实现。本轮已按锁定 schema 接入只读适配并测试消息顺序；不宣称兼容所有历史版本或远程来源。 |

ZCode 核对基线：`872ad960de7ec172591f7e1952f7849229f94521`。
路径与查询证据：`apps/zcode-cli/packages/adapters/src/storage/session-store/paths.ts`、`repositories/messages.ts`、`repositories/sessions.ts`。本轮已移植路径与排序规则，保留 Apache-2.0 许可证并登记修改范围，见 THIRD_PARTY_NOTICES.md。

## 官方证据

- https://github.com/google-gemini/gemini-cli/releases
- https://github.com/QwenLM/qwen-code/releases
- https://github.com/MoonshotAI/kimi-cli
- https://github.com/MoonshotAI/kimi-code
- https://docs.qoder.com/qoderwork/quick-start
- https://docs.qoder.com/qoder/data-import
- https://github.com/zai-org/ZCode
- https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md
- https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/persistence-changes/2026-09-16-session-format-v4.md

## 每个来源的验收要求

1. 明确产品/版本、默认及自定义目录、跨平台路径和稳定来源身份。
2. 最小夹具覆盖列表、用户/助手消息、工具结果、排序、正文搜索与命中定位。
3. 长会话采用现有有界预览；扫描可取消，失败定位到具体文件或数据库。
4. 识别未知版本、损坏数据和部分记录，不静默显示为空或完整成功。
5. 原生数据仅只读，测试只用临时数据库/合成或已检查的上游夹具；SQLite 不调用上游创建或迁移函数。
6. 原始许可证、来源 commit、移植文件及测试来源随实际实现登记；验证与发布遵循仓库现有门禁。


## alpha.10 本地验证与限制

前端 93/93、Rust lib 548/548、安全矩阵 12/12 与前端构建通过；四来源联动测试覆盖扫描、正文命中及预览偏移。测试使用合成临时数据。最终 DSH 工具事件补充回归 3/3 通过；Rust WebUI + Edge 完成四来源扫描、搜索、命中预览、返回、筛选及设置保存验证，桌面和窄屏无横向溢出或应用错误，测试源文件 SHA-256 不变。标签 CI/draft 制品以工作流结果为准，不把本地 WebUI 测试视为安装运行证明。DSH 为历史事件投影，SQLite 与压缩日志仍可能完整读取单会话后分页；详细读取边界见 [统一工作台](unified-session-workbench.md)。
