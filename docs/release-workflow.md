# AgentVault 开发交付与 draft 发布

按用户授权，每次模块或优化完成后，默认交付到代码仓库和可下载的 CI draft 制品。仅修改草案、尚未完成实现或验证失败时，不创建版本标签。

## 固定目标与边界

- `origin`：`https://github.com/minoltaMF/AgentVault.git`，承载开发提交与 Release。
- `upstream`：`https://github.com/ccpopy/cc-sessions.git`，仅追踪来源，不推送。
- 所有版本的 Release workflow 均创建 draft；预发布版本同时标记 prerelease。正式发布 draft 需用户另行明确授权。
- 每个版本使用新的注释标签，不覆盖历史标签，不强制推送。部分平台失败时不能把已有资产视为完整交付。
- Git 远程不等于应用内更新源。仓库已公开，alpha draft 制品仍需有权限的 GitHub 登录访问；不启用应用内更新，不回退上游更新。

## 完成交付的顺序

1. 核对适用约定、工作区改动、分支与远程，只提交本次任务文件；保护其他未提交成果。
2. 更新 `package.json`、`package-lock.json` 的根与应用条目、`src-tauri/Cargo.toml`、根 `Cargo.lock` 的应用条目、`src-tauri/tauri.conf.json`，保持同一版本；新增 `docs/releases/<version>.md`，记录范围、验证与限制。
3. 运行 `npm run release:check -- --tag v<version>`、`npm run test:frontend`、`npm run build`、`npm run test:safety`、对应 Rust 测试和 `cargo fmt --all -- --check`。宿主限制必须记录，完整桌面目标由 CI 验证。
4. 复核 diff 与待提交文件，提交后核对远程 main 未发生冲突，正常推送 main；创建并推送指向该提交的注释标签 `v<version>`。不得移动已推送标签来修补失败版本。
5. 核验同一提交的 CI（Linux、Windows）及 Release（Windows、Linux、macOS arm64、macOS Intel）全部成功。Windows alpha 使用 NSIS，避免 MSI 对预发布版本的限制。
6. 核验 draft 状态、标签对应提交以及安装包、CLI 和 Windows portable 资产的版本与架构。CI 构建成功不等于签名、公证或目标机器运行验收。
7. 向用户交付提交 SHA、标签、CI 与 draft 链接、资产情况和未覆盖事项；不点击 Publish release。

CI 失败时先定位原因：环境问题可重试失败任务；需要代码修复时提交修复并使用新版本标签，不改写历史。最终状态以 GitHub 上的实际任务和资产为准。
