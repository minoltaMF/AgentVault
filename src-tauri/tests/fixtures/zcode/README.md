# ZCode 只读来源夹具

本目录的 sample 文件由 AgentVault 合成，不含真实用户会话。

查询规则改编自 zai-org/ZCode@872ad960de7ec172591f7e1952f7849229f94521 的 apps/zcode-cli/packages/adapters/src/storage/session-store/{paths.ts,repositories/messages.ts,repositories/sessions.ts}，Apache-2.0（LICENSE）。改动为 Rust SQLite 只读事务，不执行上游创建、更新或迁移。夹具故意使物理/时间顺序与 sequence 不同，验证按原生顺序读取。

保留的 NOTICE.md 是锁定版本上游的原始声明，描述 ZCode 而非 AgentVault；本次仅复用上述读取规则，不包含其 Agent 执行或第三方运行环境。
