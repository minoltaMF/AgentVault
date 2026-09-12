# AgentVault 后续开发计划：从核心库到完整产品流程

评估日期：2026-09-08。代码基线：`d1d4c3a`，`0.1.0-alpha.2`。本文件是下一阶段建议，不表示以下功能已经实现，也不授权执行提交、发布、原生数据写入或路径迁移。

## 1. 结论

2026-09-12 进度补记：Codex 删除运行态保护、可验证删除前快照与隔离恢复界面已实现。P1 第一项「Codex/Claude 统一只读会话工作台」现已接通，范围与后续缺口见 [统一工作台](unified-session-workbench.md)。下文保留 2026-09-08 评估基线，不把当时的缺口当作最新状态。

AgentVault 已有较丰富的上游会话管理功能和一组独立的新核心库，当前最需要补齐的是两者之间的真实用户流程。继续照旧方案第 32 节逐个增加模块，收益已经低于接通、验证和打磨现有能力。

下一阶段围绕四个结果推进：**保住、找到、原生续聊、跨 Agent 接棒**。其中“保住”必须以可验证且能回读的不可变快照为依据；“找到”不能只依赖仍然存在的原生文件；“续聊”不能把复制命令、启动进程和成功恢复混为一谈；“接棒”必须保留证据、缺失信息与来源。

建议先补已确认的 Codex 删除运行态保护，再交付统一只读列表；随后尽早完成手动 checkpoint 与源文件消失后的回读。Doctor 可随来源管理接入，但不要让一个漂亮的诊断页成为继续推迟真正备份能力的理由。

## 2. 评估范围与现场

- 重新通读根目录 AGENTS.md、原开发方案全文、README、上游基线、两份 alpha 发布说明和安全测试矩阵；核对 Cargo workspace、应用依赖、npm 脚本和锁文件相关内容、Tauri 配置与权限、CI 和 release workflow。
- 审阅核心 crates、其契约测试、前端路由和主要业务入口，以及 Tauri、CLI、WebUI 的连接关系。核心与产品接入分别进行只读交叉审阅。本轮不是逐行穷尽整个继承代码库的安全审计，也没有进行真实桌面视觉验收。
- 初始工作区干净；分支 `main`，本地 `origin/main` 指向同一提交。未 fetch，因此这不代表远端实时状态。
- origin 为 `https://github.com/minoltaMF/AgentVault.git`；upstream 为 `https://github.com/ccpopy/cc-sessions.git`。
- 实际 fork 基线为 `1c912b2bb35e328881f543dfef1eeb5ff510f2bf`。其后有 18 个提交，包含原方案第一轮的 17 项提交及 alpha.2 打包修复。
- 实际复用登记仍只有 cc-sessions，根许可证为 MIT。候选开源项目不等于已经移植；不因为原方案推荐双许可证就更改当前许可证。
- GitHub 网页和连接器读取 AgentVault 都返回 404。仓库是否私有、权限是否不足及最新 Actions/Release 状态未核实，不据此判断仓库不存在或未发布。

## 3. 真实完成度

不采用一个总百分比；分别看库实现、应用接入和用户验收。

| 能力 | 当前事实 | 下一阶段缺口 |
| --- | --- | --- |
| 旧四 Provider 会话管理 | Codex、Claude、OpenCode、Cursor 已有列表、预览、备份、导出、整理、统计及部分修复入口 | 新旧策略统一，跨来源视图，真实平台和大数据体验验收 |
| Provider SDK | 描述、能力、发现、分支图与 ResumePlan 契约存在；Claude/Codex 发现已有局部复用 | Codex/Claude canonical 解析尚未迁完；SDK 尚无统一 parse 入口 |
| Pi | v1/v2/v3 解析、分支图、未知记录保留及恢复计划 | 不在现有桌面/CLI Provider 列表；全文读取需要后续流式化 |
| Registry / FTS | 复合身份、source cursor、事务投影、ProjectLocation、unicode/trigram 搜索可独立使用 | 应用层扫描与解析供给、持久化位置、列表查询、筛选排序和命中定位 |
| Vault | 不可变 manifest、对象存储、去重与只读 verification | 原生采集、活动 JSONL 分块、快照提交编排、持久化状态、回读和 restore |
| Resume | 按 Provider 生成参数数组，保守选择同项目/同机器 cwd | 实际 preflight、终端适配、启动结果和失败反馈 |
| Recovery Capsule | 对调用方给定数据生成确定性 Markdown | 实际会话/快照证据输入、Git 采集绑定、预览、复制和导出 |
| Git context | 只读 HEAD、分支、dirty 文件及 diff stat | checkpoint 绑定与持久化；当前没有保存源码内容 |
| Doctor / Recovery Inbox | 有界 CLI 探测和诊断模型；内存态风险排序去重 | 事实采集、状态更新、UI/CLI 接入；未知状态不能伪装成健康或故障 |
| WorkBuddy | 复合 native identity 上的确定性 overlay | 公开协议采集、持久化和产品展示；当前不会自动读取 WorkBuddy |
| 发布 | 版本检查、多平台 workflow、alpha.2 Windows NSIS 修复已配置 | 当前远端结果、安装运行、升级回退、签名/公证仍需对应证据 |

关键代码依据：

- `src-tauri/Cargo.toml` 只依赖新核心中的 provider-claude、provider-codex、provider-sdk、vault-io；未依赖 registry、vault、resume、recovery、health、app-service、Pi 和 WorkBuddy。
- `src/App.tsx` 和 `src/lib/api.ts` 仍是旧四 Provider 的路由与类型；`src/hooks/useSessions.ts` 按 Provider 整批拉取并在前端筛选。
- `crates/provider-sdk/src/provider.rs` 尚只有发现及恢复规划相关接口；Codex/Claude Provider 未声明 PARSE。
- `crates/vault/src/lib.rs` 明确把原生采集和恢复留给后续层；`crates/app-service/src/lib.rs` 当前只是 Recovery Inbox 投影。
- `docs/safety-test-matrix.md` 明确列出尚未实现的 JSONL capture、SQLite online backup、snapshot DB reconcile、watcher reconcile 和新 Vault restore。

## 4. 先处理的实际问题

### 4.1 原生写入边界未收口

`src/routes/sessions.tsx:1203` 提示删除“不可撤销，也不会自动备份”，`:1206` 明确允许 Codex/ChatGPT Desktop 运行时执行删除。`src-tauri/src/sessions/codex_delete.rs:69` 的实现只推迟 Desktop 私有项目状态清理，仍继续处理 Core 数据。

这与当前 AGENTS 的“无法确认写入者已停止时拒绝操作”不一致。已有事务、CAS、补偿测试有价值，但不能据此宣称运行中的原生 Session 可以安全删除。也不能用隐藏按钮代替后端保护。

先做一个有限的 Codex 删除保护修复，随后建立全入口写操作清单：重命名、删除、编辑、归档、转换、导入、恢复、cwd 修改、索引修复和清理。区分原生数据、AgentVault 自有元数据、导出文件和纯读取；默认限制应落在共享服务边界。旧功能保留实现和回归覆盖，尚未满足保护条件的操作在产品中解释不可用原因，不能靠一个“高级模式”绕过硬性检查。

### 4.2 搜索得到与受到保护仍是两回事

现有正文搜索有任务状态和取消能力，不能重复建设成另一套扫描弹窗。但它没有使用新 FTS，也没有形成从 Vault 找回已消失源文件的统一入口。应分别显示：来源可读、索引更新时间、快照存在、最近验证时间、原生续聊条件。

### 4.3 文档已经出现状态漂移

README 同时存在“更新检查仍会访问上游”的旧表述和 internal alpha 禁用更新源的新说明；末尾还称 fork 只有 upstream，与本地 origin 已配置不一致。原方案中的首批提交顺序已经走完，却不能视为 Alpha 用户验收完成。

保留原方案作为设计资料；另维护当前支持矩阵、里程碑状态和证据。历史基线中的历史事实保留日期，避免把历史记录改成现在状态。

## 5. 分阶段开发顺序

各阶段是多个可独立评审的提交，不是一条要求 Agent 一次完成的大提示。时间只作为粗略容量规划，按验收结果调整，不沿用原方案 5–8 天或 18–25 天的承诺。

| 阶段 | 用户得到什么 | 交付边界 | 完成标准 |
| --- | --- | --- | --- |
| P0 安全与事实收口 | 能知道哪些操作可安全执行 | Codex 删除运行态保护；其余写入口清单和门禁分批实施；修正文档现状 | 被拒绝操作零原生改动；桌面、CLI、WebUI 一致；没有“已保护”的虚假状态 |
| P1 统一只读工作台 | 在一处找到不同 Agent 的工作 | 先 Codex/Claude 全部会话列表、来源诊断和详情；Pi 单独增量接入；Doctor 接真实事实 | 复合身份不串选；部分源失败不阻塞全列表；取消/重试/空态清晰；旧四 Provider 入口兼容 |
| P2 持久化发现和精准搜索 | 重启后不必从零扫描，能准确跳到命中消息 | 先独立存储/兼容设计；再 canonical parser、Registry ingest、增量对账、FTS 查询与定位 | 同源重扫不重复，替换/截断重建；坏行降级可见；源不可达保留历史；索引可从证据重建 |
| P3 手动保护闭环 | 明确知道一条会话已保存且能离线回读 | 先一种明确的 JSONL 捕获合同，再分 Provider 接入；manifest 发布与 reconcile；验证、快照列表、回读和导出 | 删除临时源后仍可看完整已捕获内容；损坏对象不能显示 Verified；追加/崩溃不产生虚假成功 |
| P4 原生续聊与接棒 | 找到后可以继续做事 | Resume preflight/终端/cwd；Capsule 输入采集、预览、复制、导出；Git checkpoint 绑定 | 正确 cwd/来源环境；参数不经 shell 拼接；原生 resume 不发 prompt；失败可得到有证据的 Capsule |
| P5 自动保护与恢复箱 | 日常工作自动受保护，异常有可执行建议 | 定时 checkpoint、启动/周期对账，再接 watcher；实际 health events、Inbox 和 Backup Center | 退出重启补漏；取消不留伪快照；磁盘满/根不可达可解释；源缺失不删除唯一副本 |
| P6 可恢复与发布验收 | 能在受支持平台可靠安装、恢复、升级 | 新 Vault restore 先 dry-run/隔离目录；逐 Provider 适配；跨平台真实启动与文档收口 | 临时 home 恢复与 native probe；失败补偿证据；制品同 commit，平台状态独立标注 |

关键依赖与拆分：

- P1 先用显式根和内存态聚合接通实际页面，不为列表第一步迁移应用数据；不要把全文索引、Vault、Pi 分支可视化同时塞进一个补丁。
- P2 的存储设计必须单独写清新 AgentVault 自有数据与继承配置的关系、路径所有权、首次创建、schema 版本、备份、回滚和验证，再实施。原生 Provider 数据目录不动。
- P3 的采集与 P2 的 parser 可在合同确定后独立开发；捕获原始字节不应被解析失败阻断，也不必等待复杂排序和所有筛选完成。
- P3 优先 Codex/Claude 的 JSONL 保护；成员不完整时明确“仅 transcript”，不能声称 sidecar、代码或完整原生环境都已备份。SQLite 捕获作为独立提交使用原生一致性备份，不复制活动裸库冒充快照。
- P4 的健康原生 Resume 可在 P1 后提前做，不能因为尚无 snapshot 而把原本安全的续聊完全阻塞；但源缺失、冲突或 cwd 歧义必须明确处理。
- P6 的 provider-aware restore 属于单独原生写入范围，不能顺手复用旧恢复按钮宣称新 Vault 恢复已完成。Codex 的 state/index/history 依赖与 Claude/Pi 的恢复分别验收。

## 6. 体验打磨应该具体做什么

| 使用环节 | 优先改进 | 验收场景 |
| --- | --- | --- |
| 第一次打开 | 数据源卡片、路径来源说明、缺失/权限/未安装区分、扫描范围预览 | 没装某 Agent 不报全局错误；明确选定数据源后才扫描 |
| 日常找会话 | “全部会话”入口、Provider/项目筛选、标题和正文搜索语义清晰 | 中文短词、错误码、文件路径均能解释命中；可直接定位消息 |
| 列表返回 | 保持筛选、滚动位置、选中项；后台刷新保留已有结果 | 从详情返回不跳顶；切换来源后旧请求不覆盖新结果 |
| 会话详情 | 概览、对话、原始事件、快照和恢复动作逐步组织；工具过程默认折叠 | 未完成助手输出不冒充最终答复；未知事件可查看且不丢弃 |
| 长任务 | 一致的进度、取消、部分成功、失败重试和最终结果 | 取消后可再开始；坏文件不让全来源扫描失败；不要只用一闪而过的 toast |
| 状态解释 | 来源/快照/验证/续聊分别表达，显示检查时间与证据 | “未知”不等于“健康”；Verified 不等于当前原生会话可 Resume |
| 项目移动 | 同一 Project 的位置历史、旧新路径对照、歧义由用户选择 | 同名不同仓库不误合并；没有路径时提供映射动作 |
| 恢复接棒 | 预览目标 cwd、原生恢复条件、Capsule 内容及证据 | 用户知道会启动什么、是否新建会话、是否发送上下文 |
| 日常诊断 | 只有真实可行动异常进入 Inbox，解决后自动消退 | 未实现 watcher 不产生“监控正常”；关闭/忽略有明确语义 |
| 可访问性 | 键盘焦点、弹窗返回、长标题/长路径、窄窗、深浅色、非颜色状态提示 | 键盘完成搜索→预览→返回；所有新状态可读且不只靠颜色 |

保留已有虚拟列表、搜索取消、请求去重、对话与过程分组、归档来源说明等工作。没有真实界面与数据规模证据前，不做整体视觉重写，也不为了优化首屏顺手更换组件库。

## 7. 开源与维护策略

- 继续以锁定 cc-sessions 为兼容底盘。周期比较上游安全修复和格式兼容改动，选择性移植；不自动合并整个上游，更不覆盖当前原生写入边界。
- Pi Session Manager 仅作为 Pi 浏览/续聊与格式适配的候选参考；当前独立 SDK 已建立，不必为了遵循旧计划再引入一个完整 casr 兼容层。确有缺口时逐模块评估、锁定 commit 和许可证，再迁移。[项目来源](https://github.com/Dwsy/pi-session-manager)
- agent-history 的多环境发现、工作区 alias、导出可作为后续参考，但远程 SSH/WSL 同步延后到本地恢复闭环之后。[项目来源](https://github.com/kvsankar/agent-history)
- Agent Sessions 可用来比较统一浏览、搜索、预览和原生续聊体验，保持 Tauri/Rust 架构。[项目来源](https://github.com/jazzyalex/agent-sessions)
- WorkBuddy 保持公开协议 overlay；当前先用稳定的 native identity，采集适配独立实现，不把同一 Claude/Codex 会话再导入一次。
- 贡献入口补齐 CONTRIBUTING、支持矩阵、脱敏 fixture 规范、问题模板、安全问题反馈方式及最小 ADR。只登记实际引入代码，维护现有 LICENSE 和 THIRD_PARTY_NOTICES。
- 安全修复与依赖维护独立提交；历史 npm audit 的 1 low/1 high 只能作为待复核记录，本轮未重新审计，不能作为实时漏洞结论。
- PR 验收必须附“从哪个入口完成什么动作”的证据。新核心库完成后，其应用接入和用户验收应有后续任务，不把一个 `feat:` 标题当作产品交付。
- 发布说明分清源码候选、CI 构建成功、已安装启动、签名公证和公开发布。兼容 package/binary/bundle identifier 暂不更名；README 的安装链接与实际发布状态同步。

本轮在线核对了 cc-sessions、Pi Session Manager、agent-history、Agent Sessions 的公开项目页面。上述为本项目复用建议，不是对这些项目全部最新代码或许可证的完整重新审计；实际引入时仍需对选定版本复核。[cc-sessions 来源](https://github.com/ccpopy/cc-sessions)

## 8. 暂缓事项与重新启动条件

- 云备份、加密同步、多机 manifest merge：本地快照、验证、回读及保留/恢复规则稳定后再启动。
- 向量检索、Tantivy：先测 FTS5 召回与性能，存在明确瓶颈再引入。
- Flight Recorder、PTY/tmux、常驻 daemon：被动保护稳定且确认“从未落盘”是主要剩余风险后再做。
- 自动 WorkSession 分组：先手动关联和撤销；不在信号不足时自动吞并会话。
- 更多 Provider、插件市场、MCP、手机端：先使首要 Codex/Claude/Pi 完成同一套可验证流程。
- 配额/成本大盘、复杂编排、全面视觉改版：不作为当前主线。

## 9. 验证计划与本轮证据

后续每个相关提交按 AGENTS 运行真实项目命令：前端 `npm run test:frontend`、`npm run build`；相关 Rust crate/应用测试；格式改动 `cargo fmt --all -- --check`。涉及写入/恢复时加入 `npm run test:safety`；发布阶段再跑全 workspace/all-targets 和目标平台打包。

本轮没有修改产品代码、配置或依赖，没有访问或写入真实 Agent Session。宿主可用 Node，但 npm/cargo/rustc 不在 PATH，标准 Cargo 默认位置也没有工具，因此用已有 Node 和仓库依赖执行可运行的等价脚本：

| 实际命令/检查 | 本轮结果 |
| --- | --- |
| `node scripts/check-release.mjs --tag v0.1.0-alpha.2` | 通过，版本和兼容性标识一致 |
| `node node_modules/tsx/dist/cli.mjs --test 'src/lib/*.test.ts'` | 69/69 通过，无跳过；对应 test:frontend 脚本内容 |
| `node node_modules/typescript/bin/tsc -b` | 通过；仅覆盖 build 的 TypeScript 阶段 |
| `node node_modules/vite/bin/vite.js build --config vite.config.ts` | 失败；配置加载时报 vite 与 @vitejs/plugin-react 包入口解析错误 |
| Rust tests / safety matrix | 未重跑，当前宿主工具链不可用 |
| Tauri 真界面、安装器、原生 CLI Resume | 未运行，不属于本轮已通过验收 |

Vite 错误在未修改产品代码的基线发生；可能与 Windows 访问 macOS 共享目录及现有依赖解析有关，根因未确认，不把它当作新回归，也不把完整 build 标成通过。本轮不为规划任务安装全局工具或重新安装依赖。

现有测试的价值与边界：前端主要是纯逻辑测试，不能替代渲染和真实交互；Registry 等契约测试不能证明 ingest 已接通；11 项安全矩阵包含真实 temp-fsync 后退出测试和部分模拟中断场景，不能代替新 Vault restore 或断电耐久性验收。

后续增加固定脱敏数据集：坏头/坏中行/半尾行、重复 ID、多源同 ID、路径移动、20 MB 会话、5 万 turn。性能指标沿用原方案作为目标，首先记录基线，再测 P95 与峰值内存；10 GB 扫描测试在独立受控基准任务中执行，不在日常扫描真实 home 时验证。

## 10. 下一步唯一建议任务

**任务标题：修复 Codex 删除的运行态保护，运行中或无法确认状态时在任何原生写入前拒绝。**

优先理由：这是本轮发现的明确代码行为与项目硬约定冲突，涉及不可撤销操作。范围比新 Vault 或统一 Registry 小，能够快速完成并建立三端共用的保护方式；完成后立即进入 P1 的统一只读列表。

输入与主要位置：

- 根 AGENTS.md 与本文件第 4.1 节。
- `src-tauri/src/sessions/codex_delete.rs`、`src-tauri/src/sessions.rs` 的单删/批删与 family 路径。
- `src-tauri/src/codex_projects/desktop_guard.rs` 及其调用方。
- `src-tauri/src/commands.rs`、`src-tauri/src/webui.rs`、CLI/交互菜单删除入口。
- `src/routes/sessions.tsx` 删除说明及相关结果提示。

实现要求：

1. 先建立单删、批删、family 删除到共同底层写入函数的入口清单，确认检查发生在创建/打开可写 SQLite、改 rollout/index 和辅助元数据之前。
2. 使用可注入的运行态判定；已运行、探测失败、权限不足或无法确认都拒绝。不能把“Desktop 不在运行”直接扩展成“CLI/app-server 等所有写入者均已停止”的结论；无法覆盖的写入来源保持拒绝并说明原因。
3. 在提交前重新校验必要状态，并保留现有 CAS、事务与补偿；一次进程检查不是文件独占锁，也不能替代并发冲突检测。
4. UI 展示明确原因和重试条件；后端强制执行，使绕过 UI 的 CLI/WebUI 调用不能继续写入。不静默结束原生进程，不添加跳过检查的 force 参数。
5. 保留现有删除实现与底层补偿回归，不新增自动快照架构、不改数据库 schema 或数据路径。本任务只关闭运行态缺口；停止后删除的 verified pre-snapshot 要求仍列为 P0 后续门禁，不能因此宣称全部删除流程安全完成。

验收：

- 临时夹具覆盖 Running、Unknown/探测失败、明确停止、检查后状态变化。
- 被拒绝的单删、批删及 family 操作中，原生目录清单、文件字节和数据库逻辑内容不变，不创建缺失原生 DB；无部分删除伪成功。
- Tauri、CLI、WebUI 走相同保护；已存在的冲突/补偿/路径安全测试不被跳过或弱化。
- 已有运行中允许删除的历史测试与新约定矛盾时，先记录合同变更，保留其底层补偿覆盖，并增加更严格的拒绝及零写入断言；不得简单删掉测试以通过。
- 运行相关 Rust 删除/guard 测试、安全矩阵、前端测试与构建；真实页面验证拒绝提示，但所有操作目标均为测试夹具。
- 单独交付补丁与验证记录，不提交推送，不读写真实用户 Session。

可直接交给下一轮的指令：

> 请执行 docs/agentvault-next-development-plan-zh.md 第 10 节的 Codex 删除运行态保护任务。先核对最新 Git 状态、AGENTS 和相关实现，使用临时夹具建立运行中/无法确认时零写入的失败测试，再修复共享后端保护及桌面/CLI/WebUI 的一致反馈。保留现有事务、CAS、补偿与路径安全能力，不顺手实施统一列表、Vault、schema 或路径迁移，不操作真实 Session，不提交或推送。完成相关测试和真实夹具界面验证，明确报告验证范围及剩余 pre-snapshot 缺口。
