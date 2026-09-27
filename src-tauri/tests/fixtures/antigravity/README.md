# 来源与验证

本目录夹具原样复用 jazzyalex/agent-sessions，锁定 commit
`ab439f13211e56b809f4a917d5c38e80d2bb5258`，MIT，版权和许可见 LICENSE。
原路径：Resources/Fixtures/stage0/agents/ 对应 provider 子目录。
Rust 投影移植对应 Services 下 Discovery / Parser 的路径与角色规则。
模块内测试覆盖身份、角色/工具边界、取消、坏文件拒绝以及未扫描预览拒绝。
所有测试使用临时目录，不读写实际用户会话。
Antigravity CLI transcript基于上述锁定上游公开夹具；未声称官方稳定公开schema。brain Markdown每文件独立产物，明确标记非完整对话，不访问文件内本地路径。未知事件保留meta，不作为用户正文。


官方目录契约已核对（2026-09-24）：https://www.antigravity.google/docs/hooks 记录 antigravity、antigravity-cli、antigravity-ide 三类 app_data_dir 下 brain/<id>/.system_generated/logs/transcript.jsonl。暂不读取未核验的 transcript_full.jsonl。
