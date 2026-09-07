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

## 尚未宣称覆盖

下列场景依赖第一轮尚未实现的生产能力，不能因本矩阵存在而视为通过：

| 场景 | 当前缺口 |
| --- | --- |
| Agent append 与固定 4 MiB JSONL checkpoint 同时发生 | append-only chunk capture 尚未实现 |
| manifest 写完、snapshot DB commit 前崩溃 | registry 尚无 snapshot record/reconcile 流程 |
| snapshot DB commit 后、stage 清理前崩溃 | ingest/stage journal 尚未实现 |
| 新 Vault restore 中途断电与 dry-run 后 source 漂移 | 新 Vault restore/dry-run 尚未实现；当前只覆盖继承的兼容 restore |
| SQLite WAL 在线 checkpoint/backup | SQLite online backup 尚未实现 |
| watcher 丢事件后的 startup reconcile | watcher/reconcile 尚未实现 |
| Claude、Codex、Pi 从新 Vault 完整恢复并 native CLI probe | provider-aware Vault restore 尚未接线 |

这些缺口必须在对应生产实现提交中先添加失败测试，再实现功能；不得用 mock success 或弱化现有断言把它们标记为已完成。
