# 来源与验证

本目录夹具原样复用 jazzyalex/agent-sessions，锁定 commit
`ab439f13211e56b809f4a917d5c38e80d2bb5258`，MIT，版权和许可见 LICENSE。
原路径：Resources/Fixtures/stage0/agents/ 对应 provider 子目录。
Rust 投影移植对应 Services 下 Discovery / Parser 的路径与角色规则。
模块内测试覆盖身份、角色/工具边界、取消、坏文件拒绝以及未扫描预览拒绝。
所有测试使用临时目录，不读写实际用户会话。
Cline官方交叉核对：cline/cline@dcf8c3c33596e3d561a941202297c564a1cbcd49，sdk/packages/core/src/services/session-data.ts 与 session/stores/session-manifest-store.ts。仅CLI/Desktop v1配对产物；不支持VS Code任务目录。不跟随messages_path。
