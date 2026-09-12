//! Atomic removal of the physical records that make a Codex thread visible.
//!
//! The parent `sessions` module resolves user-facing session/family targets. This module owns the
//! lower-level compensated mutation across SQLite, rollout/index/family files, and Desktop's
//! private project state.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;

use crate::error::{AppError, AppResult};
use crate::family;
use crate::models::{DeleteResult, FamilyStore};
use crate::{paths, state_db};

pub(crate) struct CodexDeleteOutcome {
    pub(crate) result: DeleteResult,
}

pub(crate) fn ensure_delete_allowed(codex_dir: &Path) -> AppResult<()> {
    crate::codex_writer_guard::ensure_local_source(codex_dir)?;
    crate::codex_writer_guard::ensure_codex_writers_stopped()?;
    // Retain Desktop's state-specific check as well as the CLI/app-server process gate.
    crate::codex_projects::ensure_desktop_not_running(codex_dir)
}

struct DeletePreparation {
    id: String,
    rollout_files: Vec<PathBuf>,
    rollout_path: Option<String>,
    history_thread_ids: Vec<String>,
}

#[derive(PartialEq, Eq)]
struct RolloutReference {
    path: PathBuf,
    payload_thread_id: Option<String>,
    file_thread_id: Option<String>,
    history_base_thread_id: Option<String>,
}

#[cfg(test)]
pub(crate) fn delete_codex_artifacts(codex_dir: &Path, id: &str) -> AppResult<CodexDeleteOutcome> {
    let mut outcomes = delete_codex_artifacts_batch(codex_dir, &[id.to_string()])?;
    outcomes
        .pop()
        .ok_or_else(|| AppError::Other("Codex 删除未返回结果".to_string()))
}

/// Delete all Core and Desktop-visible state for several Codex threads as one compensated unit.
#[cfg(test)]
fn delete_codex_artifacts_batch(
    codex_dir: &Path,
    ids: &[String],
) -> AppResult<Vec<CodexDeleteOutcome>> {
    delete_codex_artifacts_batch_with_family_store(codex_dir, ids, None)
}

pub(crate) fn delete_codex_artifacts_batch_with_family_store(
    codex_dir: &Path,
    ids: &[String],
    family_store: Option<&FamilyStore>,
) -> AppResult<Vec<CodexDeleteOutcome>> {
    delete_codex_artifacts_with_backup_root(
        codex_dir,
        ids,
        family_store,
        &crate::codex_delete_snapshot::backup_root(None),
    )
}

pub(crate) fn delete_codex_artifacts_with_backup_root(
    codex_dir: &Path,
    ids: &[String],
    family_store: Option<&FamilyStore>,
    backup_root: &Path,
) -> AppResult<Vec<CodexDeleteOutcome>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut unique_ids = Vec::with_capacity(ids.len());
    let mut seen_ids = HashSet::with_capacity(ids.len());
    for id in ids {
        super::validate_delete_id(id)?;
        if seen_ids.insert(id.clone()) {
            unique_ids.push(id.clone());
        }
    }

    // No native write (including WAL setup) is allowed before both process and state preflight.
    ensure_delete_allowed(codex_dir)?;
    crate::codex_projects::preflight_thread_project_state_cleanup(codex_dir, &unique_ids)?;
    let rollout_references = scan_rollout_references(codex_dir)?;

    // Read the deletion plan from a native backup, not a writable source connection. This also
    // avoids source -shm creation when snapshot creation/verification subsequently fails.
    let snapshot_preparation =
        crate::codex_delete_snapshot::Preparation::new(codex_dir, backup_root)
            .map_err(|error| AppError::Other(format!("删除前快照创建失败，已拒绝删除: {error}")))?;
    let planning_root = snapshot_preparation.files_root();
    preflight_thread_history_database_for_delete(&planning_root)?;
    let planning_state = state_db::open_ro(&planning_root)?;
    let mut preparations = Vec::with_capacity(unique_ids.len());
    for id in &unique_ids {
        let rollout_path: Option<String> = planning_state
            .query_row(
                "SELECT rollout_path FROM threads WHERE id = ?",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let mut rollout_files = rollout_references
            .iter()
            .filter(|reference| {
                reference.payload_thread_id.as_deref() == Some(id.as_str())
                    || reference.file_thread_id.as_deref() == Some(id.as_str())
            })
            .map(|reference| reference.path.clone())
            .collect::<Vec<_>>();
        if let Some(raw_path) = rollout_path.as_deref() {
            let db_path = PathBuf::from(paths::strip_verbatim(
                &paths::host_path_string_from_codex_record(codex_dir, raw_path),
            ));
            if db_path.is_file() {
                validate_codex_rollout_path_for_delete(codex_dir, &db_path, id)?;
                rollout_files.push(db_path);
            }
        }
        let mut canonical_files = Vec::with_capacity(rollout_files.len());
        for path in rollout_files {
            validate_codex_rollout_path_for_delete(codex_dir, &path, id)?;
            let canonical = path.canonicalize()?;
            if !canonical_files.contains(&canonical) {
                canonical_files.push(canonical);
            }
        }
        let mut history_thread_ids = vec![id.clone()];
        for rollout in &canonical_files {
            if let Some(file_thread_id) = rollout_filename_thread_id(rollout) {
                if !history_thread_ids.contains(&file_thread_id) {
                    history_thread_ids.push(file_thread_id);
                }
            }
        }
        preparations.push(DeletePreparation {
            id: id.clone(),
            rollout_path: canonical_files
                .first()
                .map(|path| path.to_string_lossy().into_owned()),
            rollout_files: canonical_files,
            history_thread_ids,
        });
    }

    preflight_history_base_references(&preparations, &rollout_references)?;
    drop(planning_state);
    let all_rollouts = preparations
        .iter()
        .flat_map(|prepared| prepared.rollout_files.iter().cloned())
        .collect::<Vec<_>>();
    let snapshot = snapshot_preparation
        .finish(&unique_ids, &all_rollouts)
        .map_err(|error| AppError::Other(format!("删除前快照校验失败，已拒绝删除: {error}")))?;
    let deletion = (|| -> AppResult<Vec<CodexDeleteOutcome>> {
        if scan_rollout_references(codex_dir)? != rollout_references {
            return Err(AppError::Other(
                "快照期间 rollout 集合变化，已拒绝删除".into(),
            ));
        }
        snapshot.ensure_source_unchanged()?;
        // Scanning/capture can be slow; do not rely on the observation made before that work.
        ensure_delete_allowed(codex_dir)?;
        // Do not change journal mode before comparing the source to its verified pre-image.
        let state = rusqlite::Connection::open_with_flags(
            paths::state_db_path(codex_dir),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        state.pragma_update(None, "foreign_keys", "ON")?;
        let spawn_edges_attached: bool = state.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='thread_spawn_edges')",
        [], |row| row.get(0),
    )?;
        let transaction =
            rusqlite::Transaction::new_unchecked(&state, rusqlite::TransactionBehavior::Immediate)?;
        let logs_attached = attach_logs_database_for_delete(codex_dir, &transaction)?;
        let thread_history_attached =
            attach_thread_history_database_for_delete(codex_dir, &transaction)?;
        snapshot.ensure_after_sqlite_open()?;
        let index_path = paths::session_index_path(codex_dir);
        let mut journal = crate::mutation_journal::MutationJournal::default();
        let operation = (|| -> AppResult<Vec<CodexDeleteOutcome>> {
            let mut outcomes = Vec::with_capacity(preparations.len());
            for prepared in &preparations {
                ensure_delete_allowed(codex_dir)?;
                let rows =
                    transaction.execute("DELETE FROM threads WHERE id = ?", [&prepared.id])?;
                if spawn_edges_attached {
                    // 删除会话作为 child 的入边；若它仍有未删除的子代理，则保留出边，
                    // 让孤儿诊断/清理仍能发现这些后代。批量删除后代时，其入边会随之删除。
                    transaction.execute(
                        "DELETE FROM thread_spawn_edges WHERE child_thread_id = ?1",
                        [&prepared.id],
                    )?;
                }
                let rows_logs = if logs_attached {
                    transaction.execute(
                        "DELETE FROM delete_logs.logs WHERE thread_id = ?",
                        [&prepared.id],
                    )?
                } else {
                    0
                };
                let rows_history = if thread_history_attached {
                    delete_thread_history_rows(&transaction, &prepared.history_thread_ids)?
                } else {
                    0
                };

                for rollout in &prepared.rollout_files {
                    let expected = snapshot
                        .expected_file(rollout)?
                        .ok_or_else(|| AppError::Other("删除 rollout 缺少快照预图".into()))?;
                    #[cfg(test)]
                    crate::codex_delete_snapshot::inject_before_removal(rollout)?;
                    journal.remove_file_if_unchanged(rollout, &expected)?;
                }

                outcomes.push(CodexDeleteOutcome {
                    result: DeleteResult {
                        id: prepared.id.clone(),
                        rollout_path: prepared.rollout_path.clone(),
                        snapshot_path: Some(snapshot.path.to_string_lossy().into_owned()),
                        threads_rows_deleted: rows as u32,
                        logs_rows_deleted: rows_logs as u32,
                        history_rows_deleted: rows_history.min(u32::MAX as usize) as u32,
                        rollout_deleted: !prepared.rollout_files.is_empty(),
                        rollout_missing: prepared.rollout_files.is_empty(),
                        sidecar_deleted: false,
                        tasks_deleted: false,
                        file_history_deleted: false,
                        shared_data_preserved: false,
                        desktop_restart_required: false,
                        ok: true,
                        error: None,
                    },
                });
            }

            // Keep this after every Core file mutation so late Desktop/CAS failures exercise the same
            // compensation path as other write failures.
            mutate_preimage_file(&snapshot, &mut journal, &index_path, |bytes| {
                let Some(bytes) = bytes else {
                    return Ok(None);
                };
                let content = String::from_utf8(bytes.to_vec())
                    .map_err(|error| AppError::Other(error.to_string()))?;
                let mut changed = false;
                let mut output = String::new();
                for line in content.lines() {
                    let remove = serde_json::from_str::<serde_json::Value>(line)
                        .ok()
                        .is_some_and(|value| {
                            ["id", "session_id"].iter().any(|key| {
                                value
                                    .get(key)
                                    .and_then(|v| v.as_str())
                                    .is_some_and(|id| unique_ids.iter().any(|target| target == id))
                            })
                        });
                    if remove {
                        changed = true;
                    } else if !line.is_empty() {
                        output.push_str(line);
                        output.push('\n');
                    }
                }
                Ok(changed.then(|| output.into_bytes()))
            })?;
            mutate_preimage_file(
                &snapshot,
                &mut journal,
                &paths::archive_ledger_path(codex_dir),
                |bytes| {
                    let Some(bytes) = bytes else {
                        return Ok(None);
                    };
                    let Ok(mut ledger) =
                        serde_json::from_slice::<crate::models::ArchiveLedger>(bytes)
                    else {
                        return Ok(None);
                    };
                    let mut changed = false;
                    for id in &unique_ids {
                        changed |= ledger.entries.remove(id).is_some();
                    }
                    if changed {
                        Ok(Some(serde_json::to_vec_pretty(&ledger)?))
                    } else {
                        Ok(None)
                    }
                },
            )?;
            ensure_delete_allowed(codex_dir)?;
            let expected_global =
                snapshot.expected_file(&paths::codex_global_state_json_path(codex_dir))?;
            if let Some(receipt) = crate::codex_projects::clear_thread_project_states_from_snapshot(
                codex_dir,
                &unique_ids,
                expected_global.as_ref(),
            )? {
                journal.register_project_state_receipt(receipt);
            }
            if let Some(family_store) = family_store {
                let family_path = paths::family_store_path(codex_dir);
                mutate_preimage_file(&snapshot, &mut journal, &family_path, |_| {
                    Ok(Some(serde_json::to_vec_pretty(family_store)?))
                })?;
            }
            // A late writer start follows the existing transaction/file compensation path.
            ensure_delete_allowed(codex_dir)?;
            Ok(outcomes)
        })();

        let mut outcomes = match operation {
            Ok(outcomes) => {
                crate::mutation_journal::commit_transaction_with_compensation(
                    transaction,
                    journal,
                )?;
                outcomes
            }
            Err(error) => {
                return Err(
                    crate::mutation_journal::rollback_transaction_with_compensation(
                        transaction,
                        journal,
                        error,
                    ),
                );
            }
        };

        // Desktop's catalog and summaries live outside Core's transaction. Each store rechecks
        // writers before opening for write and before commit; late failure is explicitly partial.
        // Keep the cleanup after the compensated Core commit: a Core failure must never hide a thread
        // that still exists. If this separate cleanup fails, report an explicit partial deletion so a
        // missing rollout is not mistaken for a fully successful operation.
        if let Err(error) =
            crate::codex_projects::clear_deleted_thread_cache_rows(codex_dir, &unique_ids)
        {
            let message = format!("会话主体已删除，但 Codex Desktop 目录缓存清理失败: {error}");
            for outcome in &mut outcomes {
                outcome.result.ok = false;
                outcome.result.error = Some(message.clone());
            }
            return Ok(outcomes);
        }

        // Empty date directories are cosmetic and deliberately outside the transaction. Failure to
        // remove one cannot make the conversation visible again, so it must not downgrade deletion.
        for prepared in &preparations {
            for rollout in &prepared.rollout_files {
                cleanup_empty_rollout_ancestors_best_effort(codex_dir, rollout);
            }
        }

        Ok(outcomes)
    })();
    deletion.map_err(|error| {
        AppError::Other(format!("{error}；删除前快照: {}", snapshot.path.display()))
    })
}

fn mutate_preimage_file(
    snapshot: &crate::codex_delete_snapshot::VerifiedSnapshot,
    journal: &mut crate::mutation_journal::MutationJournal,
    path: &Path,
    mutation: impl FnOnce(Option<&[u8]>) -> AppResult<Option<Vec<u8>>>,
) -> AppResult<()> {
    let expected = snapshot.expected_file(path)?;
    let bytes = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if bytes.as_deref().map(crate::atomic_file::fingerprint_bytes) != expected {
        return Err(AppError::AtomicWriteConflict(format!(
            "文件在删除快照后发生变化: {}",
            path.display()
        )));
    }
    if let Some(output) = mutation(bytes.as_deref())? {
        journal.mutate_file(path, || {
            use std::io::Write;
            let writer = |file: &mut fs::File| -> AppResult<()> {
                file.write_all(&output)?;
                Ok(())
            };
            if let Some(expected) = &expected {
                crate::atomic_file::replace_with_writer_if_unchanged(path, expected, writer)
            } else {
                crate::atomic_file::create_with_writer_if_absent(path, writer)
            }
        })?;
    }
    Ok(())
}

fn rollout_filename_thread_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let rest = stem.strip_prefix("rollout-")?;
    if rest.len() < 36 {
        return None;
    }
    let candidate = &rest[rest.len() - 36..];
    let bytes = candidate.as_bytes();
    let valid = bytes.iter().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            *byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    });
    valid.then(|| candidate.to_string())
}

fn scan_rollout_references(codex_dir: &Path) -> AppResult<Vec<RolloutReference>> {
    let mut rollouts = family::scan_rollouts(codex_dir)?;
    rollouts.extend(family::scan_archived_rollouts(codex_dir)?);
    rollouts.sort();
    rollouts.dedup();

    Ok(rollouts
        .into_iter()
        .map(|path| {
            let (payload_thread_id, history_base_thread_id) = family::read_session_meta(&path)
                .ok()
                .map(|meta| {
                    let payload = meta.get("payload").unwrap_or(&meta);
                    (
                        payload
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        payload
                            .get("history_base")
                            .and_then(|base| base.get("thread_id"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    )
                })
                .unwrap_or_default();
            let file_thread_id = rollout_filename_thread_id(&path);
            RolloutReference {
                path,
                payload_thread_id,
                file_thread_id,
                history_base_thread_id,
            }
        })
        .collect())
}

fn validate_codex_rollout_path_for_delete(
    codex_dir: &Path,
    path: &Path,
    logical_thread_id: &str,
) -> AppResult<()> {
    if super::validate_codex_rollout_path(codex_dir, path, logical_thread_id).is_ok() {
        return Ok(());
    }
    let file_thread_id = rollout_filename_thread_id(path).ok_or_else(|| {
        AppError::Path(format!(
            "Codex rollout 文件名既不匹配逻辑会话 ID，也不包含有效 UUID: {}",
            path.to_string_lossy()
        ))
    })?;
    for root in [
        paths::sessions_dir(codex_dir),
        paths::archived_sessions_dir(codex_dir),
    ] {
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() || crate::path_safety::metadata_is_link_or_reparse(&metadata) {
            return Err(AppError::Path(format!(
                "Codex rollout 根路径不是普通目录或属于链接/junction: {}",
                root.to_string_lossy()
            )));
        }
        let clean_root = PathBuf::from(paths::strip_verbatim(&root.to_string_lossy()));
        let clean_path = PathBuf::from(paths::strip_verbatim(&path.to_string_lossy()));
        if clean_path.strip_prefix(&clean_root).is_err() {
            continue;
        }
        crate::path_safety::validate_descendant(
            &root,
            path,
            crate::path_safety::EntryKind::File,
            false,
            "Codex rollout 删除目标",
        )?;
        let meta = family::read_session_meta(path)?;
        let actual_id = meta
            .get("payload")
            .and_then(|payload| payload.get("id"))
            .and_then(serde_json::Value::as_str);
        if actual_id == Some(logical_thread_id) || actual_id == Some(file_thread_id.as_str()) {
            return Ok(());
        }
        return Err(AppError::Other(format!(
            "Codex rollout 内容 ID 既不匹配逻辑会话 {logical_thread_id}，也不匹配文件 UUID {file_thread_id}: {}",
            path.to_string_lossy()
        )));
    }
    Err(AppError::Path(format!(
        "Codex rollout 不在 sessions 或 archived_sessions 内，拒绝删除: {}",
        path.to_string_lossy()
    )))
}

fn preflight_history_base_references(
    preparations: &[DeletePreparation],
    rollout_references: &[RolloutReference],
) -> AppResult<()> {
    let deletion_ids = preparations
        .iter()
        .flat_map(|prepared| prepared.history_thread_ids.iter().cloned())
        .collect::<HashSet<_>>();
    let deletion_paths = preparations
        .iter()
        .flat_map(|prepared| prepared.rollout_files.iter().cloned())
        .collect::<HashSet<_>>();
    for reference in rollout_references {
        let is_selected = reference
            .path
            .canonicalize()
            .ok()
            .is_some_and(|canonical| deletion_paths.contains(&canonical));
        if is_selected {
            continue;
        }
        let Some(base_id) = reference.history_base_thread_id.as_deref() else {
            continue;
        };
        if deletion_ids.contains(base_id) {
            let child_id = reference.payload_thread_id.as_deref().unwrap_or("未知会话");
            return Err(AppError::Other(format!(
                "无法删除会话：未选中的派生会话 {child_id} 仍通过 history_base 引用 {base_id}"
            )));
        }
    }
    Ok(())
}

fn attach_thread_history_database_for_delete(
    codex_dir: &Path,
    transaction: &rusqlite::Transaction<'_>,
) -> AppResult<bool> {
    let history_path = codex_dir.join("thread_history_1.sqlite");
    let metadata = match fs::symlink_metadata(&history_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || crate::path_safety::metadata_is_link_or_reparse(&metadata) {
        return Err(AppError::Path(format!(
            "Codex thread history 数据库不是普通文件: {}",
            history_path.to_string_lossy()
        )));
    }
    transaction.execute(
        "ATTACH DATABASE ?1 AS delete_history",
        [history_path.to_string_lossy().into_owned()],
    )?;
    const TABLES: [&str; 4] = [
        "thread_turns",
        "thread_items",
        "thread_history_projection_state",
        "thread_realtime_items",
    ];
    for table in TABLES {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM delete_history.sqlite_schema WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(AppError::Other(format!(
                "Codex thread history 数据库缺少 {table} 表: {}",
                history_path.to_string_lossy()
            )));
        }
    }
    Ok(true)
}

fn preflight_thread_history_database_for_delete(codex_dir: &Path) -> AppResult<()> {
    let history_path = codex_dir.join("thread_history_1.sqlite");
    let metadata = match fs::symlink_metadata(&history_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || crate::path_safety::metadata_is_link_or_reparse(&metadata) {
        return Err(AppError::Path(format!(
            "Codex thread history 数据库不是普通文件: {}",
            history_path.to_string_lossy()
        )));
    }
    let connection = rusqlite::Connection::open_with_flags(
        &history_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    for table in [
        "thread_turns",
        "thread_items",
        "thread_history_projection_state",
        "thread_realtime_items",
    ] {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(AppError::Other(format!(
                "Codex thread history 数据库缺少 {table} 表: {}",
                history_path.to_string_lossy()
            )));
        }
    }
    Ok(())
}

fn delete_thread_history_rows(
    transaction: &rusqlite::Transaction<'_>,
    thread_ids: &[String],
) -> AppResult<usize> {
    let mut deleted = 0usize;
    for thread_id in thread_ids {
        deleted += transaction.execute(
            "DELETE FROM delete_history.thread_items WHERE thread_id = ?1",
            [thread_id],
        )?;
        deleted += transaction.execute(
            "DELETE FROM delete_history.thread_realtime_items WHERE thread_id = ?1",
            [thread_id],
        )?;
        deleted += transaction.execute(
            "DELETE FROM delete_history.thread_turns WHERE thread_id = ?1",
            [thread_id],
        )?;
        deleted += transaction.execute(
            "DELETE FROM delete_history.thread_history_projection_state WHERE thread_id = ?1",
            [thread_id],
        )?;
    }
    Ok(deleted)
}

fn attach_logs_database_for_delete(
    codex_dir: &Path,
    transaction: &rusqlite::Transaction<'_>,
) -> AppResult<bool> {
    let logs_path = codex_dir.join("logs_2.sqlite");
    let metadata = match fs::symlink_metadata(&logs_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || crate::path_safety::metadata_is_link_or_reparse(&metadata) {
        return Err(AppError::Path(format!(
            "Codex logs 数据库不是普通文件: {}",
            logs_path.to_string_lossy()
        )));
    }
    transaction.execute(
        "ATTACH DATABASE ?1 AS delete_logs",
        [logs_path.to_string_lossy().into_owned()],
    )?;
    let has_logs_table: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM delete_logs.sqlite_schema WHERE type='table' AND name='logs')",
        [],
        |row| row.get(0),
    )?;
    if !has_logs_table {
        return Err(AppError::Other(format!(
            "Codex logs 数据库缺少 logs 表: {}",
            logs_path.to_string_lossy()
        )));
    }
    Ok(true)
}

fn cleanup_empty_rollout_ancestors_best_effort(codex_dir: &Path, rollout: &Path) {
    let sessions_root = paths::sessions_dir(codex_dir);
    let mut current = rollout.parent();
    while let Some(dir) = current {
        if dir == sessions_root || !dir.starts_with(&sessions_root) {
            break;
        }
        match fs::remove_dir(dir) {
            Ok(()) => current = dir.parent(),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                ) =>
            {
                break;
            }
            Err(_) => break,
        }
    }
}
