# AGENTS.md — AgentVault 项目约定

本文件适用于整个仓库。系统、开发者和用户的当前明确要求优先；更深目录中的 `AGENTS.override.md` 或 `AGENTS.md` 可补充更具体的约束。

## 数据安全

- 数据安全优先于功能完整性、性能和开发便利性。
- 默认只读 Codex、Claude Code、OpenCode、Cursor 等原生 Agent Session；新增写入能力必须有明确需求、影响范围和恢复路径。
- 不直接覆盖仍可能由原应用写入的 JSONL 或 SQLite。JSONL 写入必须使用快照、指纹或 CAS 检测及原子替换；SQLite 写入必须使用事务、冲突检查和适当的原生备份机制。无法确认写入者已停止时拒绝操作。
- 不修改真实用户 Session 来完成开发或测试；使用临时目录和最小测试夹具。
- 不迁移应用数据目录、配置路径、数据库文件名或 schema，除非单独方案明确覆盖兼容、备份、回滚和验证。

## 变更边界

- 不执行破坏性 Git 操作，包括 `git reset --hard`、`git clean -fd`、强制推送或覆盖用户未提交改动。
- 开发模块或优化完成并通过必要验证后，按用户已授权的发布流程提交当前任务相关文件，推送到 AgentVault 的 origin，并创建新版本注释标签触发 CI draft 制品；具体门禁见 docs/release-workflow.md。不得推送到 upstream、改写历史或移动已发布标签；正式发布 draft 仍须用户另行明确授权。
- 不升级主要依赖或更换包管理器。依赖升级必须作为独立任务评估锁文件、平台兼容和回退方式。
- 保留上游 `LICENSE`、版权信息和 `THIRD_PARTY_NOTICES.md` 中的来源记录；引入上游代码时同步登记仓库、许可证和锁定 commit。
- 除非独立迁移任务明确授权，不修改兼容性标识：bundle identifier、Rust/npm package 名、CLI binary 名、应用数据目录、配置文件路径、SQLite 文件名和 schema。

## 验证

- 修改后必须运行与改动对应的最小测试，再按风险扩大到前端构建、Rust 测试、格式检查、全目标检查或 Tauri 构建。
- 前端至少运行 `npm run test:frontend` 和 `npm run build`；Rust 至少运行相关 `cargo test`，格式改动运行 `cargo fmt --all -- --check`。
- 不得为了通过测试而删除、跳过、放宽或弱化现有测试，也不得硬编码测试结果或绕过安全检查。
- 区分上游已有失败、宿主环境限制和本轮引入的回归；交付时记录实际命令、结果和未覆盖风险。

## 工作方式

- 修改前读取相关代码、配置、锁文件、测试、`git status` 和最近历史，采用最小且完整的补丁。
- 数据写入逻辑优先复用现有原子文件、补偿事务、路径校验和备份机制，不另建绕过保护的快速路径。
- 不顺手实现开发方案后续章节；跨模块重构、Provider SDK、Vault、FTS 和 Recovery Capsule 必须按各自提交边界推进。
