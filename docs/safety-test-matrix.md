# AgentVault crash、race 与 restore 测试矩阵

本文记录 internal alpha 前的数据安全回归门槛。所有自动化测试只使用临时目录、脱敏夹具和内存数据库，不读取或修改真实 Agent Session。

统一入口：

```bash
npm run test:safety
```

## 已自动化场景

| 分组 | 故障点 | 必须保持的结果 | 自动化覆盖 |
| --- | --- | --- | --- |
| Crash | 同目录 temp 已 fsync、原子 rename 前进程退出 | 原文件保持完整；已同步 temp 可被后续清理或诊断 | `vault-io` 子进程真实退出测试 |
| Crash | content-addressed object 已提交、manifest 未发布 | 原 Session 不变；孤儿 object 可校验和复用；不存在伪 snapshot | `vault/tests/safety_matrix.rs` |
| Race | 读取指纹后原生 JSONL append | CAS 拒绝替换并保留 concurrent bytes | `vault-io` atomic tests |
| Race | 多 writer 写同一内容对象 | 只创建一次，其余复用相同 SHA-256 object | `vault/tests/object_store.rs` |
| Race | 同 snapshot ID 发布不同 manifest | 只有一个 immutable winner；winner 必须可验证 | `vault/tests/safety_matrix.rs` |
| Race | 相同路径的 source file identity 改变 | registry 拒绝增量续扫并要求 full rebuild | `registry/tests/registry_contract.rs` |
| Restore | 多文件补偿期间一个目标被并发改写 | 不覆盖 concurrent bytes；继续补偿其他可确认文件 | `src-tauri/src/backup.rs` |
| Restore | 原文件被删除、新文件被创建后失败 | 重建原文件并删除仅由本次 restore 创建的文件 | `src-tauri/src/backup.rs` |
| Restore | Codex SQLite commit 失败 | 回滚数据库并补偿 rollout、index、history 和 project state | `src-tauri/src/backup.rs` |
| Restore | restore source 与 manifest hash 不同 | 不提交目标文件，不报告成功 | `src-tauri/src/backup.rs` |
| Restore | 写入后无法取得可信 fingerprint | 拒绝盲目补偿并报告最终状态不确定 | `src-tauri/src/backup.rs` |

完整 workspace 测试还覆盖 snapshot/object 损坏、路径穿越、符号链接或 Windows reparse、未知事件保留、搜索索引重建以及多种旧兼容 restore 失败路径。

## Codex 删除运行态保护回归

以下回归由应用 lib 测试覆盖，不改变上面的 11 项 Vault/restore 门禁计数：

- Desktop、CLI/app-server 运行中或探测失败时，单删、批删、family 和历史分支删除拒绝，临时原生目录清单及所有文件字节不变；缺失数据库不会被创建。
- 原有“Desktop 运行时继续删除、推迟私有状态清理”合同已收紧为拒绝。停止后的选择性缓存清理仍有独立回归覆盖，没有删除或跳过原有补偿、CAS 和路径安全检查。
- 提交前观察到新写入者时，回滚 Core 事务并补偿 rollout、index、project state；Core 提交后、独立缓存清理前发现写入者时，保留缓存并报告部分完成。
- WebUI 的单删/批删 dispatch 不能绕过共享保护。CLI 历史分支删除将 family 元数据纳入同一补偿单元，避免提交后再单独写回。
- 本机进程观测不用于证明网络/WSL 来源已停止写入。来源卷未知或不支持时拒绝；运行态探测只识别受支持的原生程序名称，不能当作原生工具配合的独占锁。

针对性命令：`cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib delete_codex`、同一 lib 下的 `codex_writer_guard` 和 `webui_delete_commands` 测试。真实平台进程探测、共享目录、安装器和 Tauri 桌面 UI 仍需按平台验收，不能只用注入探针测试宣称全部通过。

Codex 删除现已接入独立 pre-image 快照包，详见 [删除前快照](codex-delete-snapshots.md)。它复用应用原子文件和补偿机制，不代表通用 Vault capture/restore 或其他 native mutation 已完成。

### 删除前快照与隔离恢复

应用 lib 中的 `delete_snapshot` 回归覆盖：

- 快照目标不可写（路径被普通文件占用）与快照内容损坏时，单删、批删、family、历史分支均失败，源目录清单和文件字节不变。
- 捕获期间源文件变化，或者快照验证后、删除暂存前 rollout 被追加：拒绝删除，保留并发字节并回滚 Core。
- 一个批次共享一份快照，包含全部 family 分支和重复 rollout；移除临时源目录后，从快照恢复并比较原始文件字节、五个 SQLite 数据库的全部表行、BLOB/未知字段、关系边及 WAL 中的内容。
- 损坏快照、目录穿越和伪造恢复完成标记成员在创建输出目录前拒绝；已有恢复目标保持字节不变。
- 原有删除项目状态 CAS 回归收紧为第一次冲突后拒绝，不再重新接受不属于快照的新状态；各分支、index、family 与 Core 的补偿断言保留。通用项目状态写入的重试测试继续保留。

命令：`cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --lib delete_snapshot`，完整删除回归使用同一 lib 的 `delete_` 过滤器。目录 fsync 和 Unix 恢复目录权限需在 macOS/Linux 实机继续验收；未宣称断电硬件测试或原生 Resume 成功。

删除快照管理界面追加回归：`management_tests` 检查正常、损坏和未完成包并列展示，访问范围、原始目录穿越输入、manifest 成员注入及新目录恢复；WebUI dispatch 测试覆盖四个接口与无效恢复不产生输出。前端测试覆盖深链接编码、状态文案、新目录名及浏览器历史记录。真实 WebUI 夹具验证校验/恢复回读、已有目标拒绝、修正重试、重载不沿用历史健康状态和校验后篡改拒绝。目录打开及 Tauri 原生选择器尚未实机验收。

统一工作台扫描的 `workbench_scan` 回归覆盖坏 JSON 单项隔离、目录遍历中取消、读行中取消、取消不计为已处理、任务独立取消、错误详情上限、显式来源要求、缺失 projects 与越界文件拒绝。该入口仅做读取，不替代下列写入/持久性验收。

## 尚未宣称覆盖

下列场景依赖第一轮尚未实现的生产能力，不能因本矩阵存在而视为通过：

| 场景 | 当前缺口 |
| --- | --- |
| Agent append 与固定 4 MiB JSONL checkpoint 同时发生 | append-only chunk capture 尚未实现 |
| manifest 写完、snapshot DB commit 前崩溃 | registry 尚无 snapshot record/reconcile 流程 |
| snapshot DB commit 后、stage 清理前崩溃 | ingest/stage journal 尚未实现 |
| 新 Vault restore 中途断电与 dry-run 后 source 漂移 | 新 Vault restore/dry-run 尚未实现；当前只覆盖继承的兼容 restore |
| 运行中的 Agent SQLite WAL 在线 capture | Codex 删除仅在写入者停止后，从稳定 DB/WAL 副本经 SQLite backup 生成快照；通用在线 capture 尚未实现 |
| watcher 丢事件后的 startup reconcile | watcher/reconcile 尚未实现 |
| Claude、Codex、Pi 从新 Vault 完整恢复并 native CLI probe | provider-aware Vault restore 尚未接线 |

这些缺口必须在对应生产实现提交中先添加失败测试，再实现功能；不得用 mock success 或弱化现有断言把它们标记为已完成。
