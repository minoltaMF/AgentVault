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

## 唯一 draft 与制品完整性门禁

Release 工作流按 tag 串行调度，不取消正在运行的同 tag 发布。`prepare` 分页查询同标签 Release，首次创建 draft 并输出唯一 release ID；创建后的列表暂不可见时仅有限重查（最多六次，累计等待 31 秒），不重复创建；重复执行只复用同提交、仍为 draft 的唯一记录。多个匹配 Release、已经正式发布或标签不再指向构建提交时拒绝继续，不自动删除、合并或改写历史。

四平台 `build` 依赖准备结果。Tauri action 仅构建，不接收 tagName/releaseId，也不持有上传凭据；桌面包、CLI 和 portable 均由 `scripts/release-draft.mjs` 按准备阶段的 ID 上传。每次上传前重新检查 draft、唯一性及注释标签解析后的 commit SHA。同名同大小同 SHA-256 制品可以复用，同名不同内容拒绝覆盖。上传响应超时后只重新查询并校验结果，不盲目重复上传或删除已有制品。若重建压缩包因时间戳导致内容不同，也会拒绝覆盖；需人工核对或按修复流程发新标签。

最后的 `verify` 即使矩阵失败也会运行，并要求所有平台成功。随后按 ID 分页获取附件：alpha 精确要求 14 件（四平台 CLI、Windows NSIS 与两件 portable、Linux 三种包、两种 macOS 各 DMG 和 app 压缩包）；稳定版本另要求 Windows MSI，共 15 件。缺失、额外、重名、空文件、未完成上传及缺少 SHA-256 均失败。对每个附件重新下载并验证大小和 SHA-256，结束前再次检查唯一 draft 与 tag 指向。

`npm run test:release` 使用合成附件和模拟 API 覆盖这些拒绝路径，并加入常规 CI 的 `npm test`。摘要记录唯一 release ID、下载校验结果和链接。校验只证明上传内容与 GitHub 提供的摘要一致及清单完整，不代替签名、公证、二进制架构检查或安装运行验收。

历史 alpha.20 的两个 draft 不在本次自动整理范围内，继续保留其原有资产。

最终校验任务继承发布流程已有的 `contents: write` 权限，以便列出未公开 draft；校验程序只发 GET 请求，不修改 Release。仅 `contents: read` 的工作流 token 在本仓库实测看不到 draft。
