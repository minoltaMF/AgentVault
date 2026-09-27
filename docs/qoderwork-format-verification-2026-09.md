# QoderWork 本地格式核实

核实日期：2026-09-24。范围：验证能否沿用现有只读来源接入；本次没有安装产品、读取真实会话正文或凭据、运行第三方脚本，也没有修改原生数据。

## 结论

**本批不声明支持 QoderWork。** 已确认它有本地任务历史，但尚未得到可验证的历史文件路径、持久化 schema 和消息夹具。不能把 Qoder CLI 的 JSONL 解析器套到 QoderWork，也不能把登录会话备份器当作聊天记录解析器。该结论不影响 Qwen Code、Cline CLI/Desktop、GitHub Copilot CLI 和 Antigravity 的独立接入。

## 已确认的官方证据

| 事项 | 证据与边界 |
| --- | --- |
| 本地历史 | [Quick Start](https://docs.qoder.com/qoderwork/quick-start) 说明任务历史存储在当前设备，不跨设备同步；支持 macOS 和 Windows。没有提供历史文件名或数据库 schema。 |
| 官方迁移能力 | [Import data](https://docs.qoder.com/qoder/data-import) 明确 Qoder 能导入 QoderWork 用户对话、定时任务等，并只读访问来源数据。说明官方存在读取实现，但文档没有公开实现或磁盘协议，不能据此推定兼容 Qoder CLI。 |
| 配置目录 | [Hooks](https://docs.qoder.com/qoderwork/hooks) 明确用户级配置 `~/.qoderwork/settings.json`；[Skills](https://docs.qoder.com/qoderwork/skills) 明确 `~/.qoderwork/skills/`。这些是配置/扩展路径，并非已验证的历史数据根目录。 |
| 事件字段 | Hooks 文档给出 stdin 公共字段 `session_id`、`cwd`、`hook_event_name`，用户提交含 `prompt`，工具事件含 `tool_name`、`tool_input`、`tool_use_id`，成功工具事件另含 `tool_response`。它们属于运行时 hook 协议，不能证明原生历史使用相同字段、顺序或编码；该页面未提供 transcript 文件定位字段。 |
| 平台范围 | 官方导入文档明确 QoderWork 仅在 macOS/Windows 可用。不能因第三方存在 Linux 路径分支而宣称 Linux 原生支持。 |

## 开源复用核查

1. [`jazzyalex/agent-sessions`](https://github.com/jazzyalex/agent-sessions/tree/ab439f13211e56b809f4a917d5c38e80d2bb5258)，已锁定 `ab439f13211e56b809f4a917d5c38e80d2bb5258`：本次检查该树，没有找到名称含 Qoder 的解析器或测试夹具。因此该基线不能提供 QoderWork 接入依据。
2. [`QoderAI/better-harness` 的 session diagnostics](https://github.com/QoderAI/better-harness/blob/main/references/session-evidence/sessions-diagnostics.md) 将 Qoder 来源定位为 `~/.qoder/projects/.../*.jsonl`。该文档没有独立 QoderWork 平台，不能外推产品兼容。这里只作线索核对，没有移植代码，未将可变 main 作为适配器基线。
3. [`963072676/qoderwork-account-switcher`](https://github.com/963072676/qoderwork-account-switcher/tree/022c1d4191939f9f57af9703cef2e732239a8158)，核实 commit `022c1d4191939f9f57af9703cef2e732239a8158`，MIT，Copyright (c) 2026 王皓晨 (Wang Haochen)。其 [`paths.rs`](https://github.com/963072676/qoderwork-account-switcher/blob/022c1d4191939f9f57af9703cef2e732239a8158/src-tauri/src/core/paths.rs) 提供 CN 版候选目录 `~/.qoderworkcn`、Windows `%APPDATA%/QoderWork CN`、macOS `~/Library/Application Support/QoderWork CN`。但 [`session.rs`](https://github.com/963072676/qoderwork-account-switcher/blob/022c1d4191939f9f57af9703cef2e732239a8158/src-tauri/src/core/session.rs) 处理 Electron 登录状态、认证文件和缓存，不解析用户/助手对话。只登记调研结论，不复用其读写流程，不引入其代码或测试。

GitHub QoderAI 公开仓库清单中可见 changelog、SDK 示例、插件及 harness 项目；本次没有找到公开的 QoderWork 历史持久化实现。此为本次检索范围内的结果，不等于证明任何位置都不存在相关源码。

## 本机有限检查

仅用路径存在性检查，没有递归扫描或打开文件内容：

- Windows 用户目录：`.qoderwork`、`.qoderworkcn`。
- Windows 应用数据目录：`%APPDATA%/QoderWork`、`%APPDATA%/QoderWork CN`。
- Windows 候选安装目录：`%LOCALAPPDATA%/Programs/QoderWork`、`%LOCALAPPDATA%/Programs/QoderWork CN`、`%LOCALAPPDATA%/QoderWork CN`。
- 共享 macOS 用户目录：`.qoderwork`、`.qoderworkcn`、`Library/Application Support/QoderWork`、`Library/Application Support/QoderWork CN`。

本次这些候选路径均不存在，因此没有可核验的本地产品版本或数据 schema。没有检查全部安装位置，不能据此断言机器完全未安装该产品。

## 未验证项和后续验收入口

目前未验证：正式版与 CN 版历史根目录、数据库/文件名称、schema 版本、会话身份及工作目录绑定、用户/助手/推理/工具记录结构、时间与排序语义、附件引用、分支与压缩历史、仍在写入时的读取边界。没有符合这些要求的公开最小历史夹具；hook 示例不足以充当夹具。

后续优先取得官方历史 schema/导出格式或有版本标识的开源读取实现；若仍无公开格式，可在隔离测试账户中生成一段纯合成对话（用户消息、助手回答、一次成功和失败的工具调用），由用户提供脱敏副本及产品版本，再核实文件布局和只读快照。验证通过后才增加来源开关及默认路径，并补齐扫描、取消、损坏文件报告、搜索和命中定位测试。未知格式须明确报告不支持，不能显示为成功读取空列表。

本报告只包含调研与格式门槛，不新增 hooks、不迁移数据、不执行 Qoder 导入，也不将本次未验证内容列入 AgentVault 支持清单。
