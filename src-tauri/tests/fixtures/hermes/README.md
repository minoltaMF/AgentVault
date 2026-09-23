# Hermes 只读来源夹具

本目录的 sample 文件由 AgentVault 合成，不含真实用户会话。

读取规则改编自 jazzyalex/agent-sessions@ab439f13211e56b809f4a917d5c38e80d2bb5258 的 AgentSessions/Services/HermesSessionParser.swift，MIT（LICENSE）。改动为 Rust SQLite 只读事务；按 schema 检查可选列，不引入 Swift/Python。测试刻意省略可选列并验证正文、工具输出分类和原文件不变。
