# DSH 只读来源夹具

本目录的 sample 文件由 AgentVault 合成，不含真实用户会话。

v4 夹具覆盖顶层 `toolCallId` / `isError` 的工具返回，以及 `tool-addition` / `tool-removal` 元数据块；这些内容保留在预览中，不计入用户/助手正文搜索。

解析规则参考 agent-sessions@ab439f13211e56b809f4a917d5c38e80d2bb5258 的 DeepSeekHarnessSessionParser.swift（MIT，LICENSE）；当前格式、文件名和 known-events.txt 来自 deepseek-ai/deepseek-harness@00102833dfaee1da9f48a3a8eae9d34005a75218 的 session-format/filename.ts、persistence-schema.json（MIT，LICENSE.deepseek）。

改动：Rust v3/v4 有界只读投影，Zstandard 拼接帧，拒绝 v0-v2/未知版本、序号断裂和半写入尾部；只读取最新 generation，保留旧文件。失败尝试、系统注入、上下文压缩不作为用户正文。附件只显示占位，不访问附件路径。测试夹具的 assistant/attempt 是刻意最小诊断行，不作为官方全模式验证夹具。
