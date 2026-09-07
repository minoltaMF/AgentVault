import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const cargo = process.env.CARGO || "cargo";

const matrix = [
  {
    group: "crash",
    name: "fsync 后、rename 前进程退出仍保留原文件",
    args: [
      "test",
      "-p",
      "vault-io",
      "atomic::tests::crash_after_temp_sync_before_rename_preserves_the_original",
      "--",
      "--exact",
    ],
  },
  {
    group: "crash",
    name: "object 已提交、manifest 未发布时对象和源文件可恢复",
    args: [
      "test",
      "-p",
      "vault",
      "--test",
      "safety_matrix",
      "object_survives_interruption_before_manifest_publish",
      "--",
      "--exact",
    ],
  },
  {
    group: "race",
    name: "原生 append 后 CAS 拒绝覆盖",
    args: [
      "test",
      "-p",
      "vault-io",
      "atomic::tests::refuses_to_replace_a_file_changed_after_snapshot",
      "--",
      "--exact",
    ],
  },
  {
    group: "race",
    name: "多 writer 写入同一 content-addressed object",
    args: [
      "test",
      "-p",
      "vault",
      "--test",
      "object_store",
      "concurrent_puts_create_once_and_reuse_the_same_object",
      "--",
      "--exact",
    ],
  },
  {
    group: "race",
    name: "同 snapshot ID 的冲突 manifest 只能有一个 verified winner",
    args: [
      "test",
      "-p",
      "vault",
      "--test",
      "safety_matrix",
      "concurrent_conflicting_manifest_publish_keeps_one_verified_winner",
      "--",
      "--exact",
    ],
  },
  {
    group: "race",
    name: "source file replacement 强制 registry cursor 全量重建",
    args: [
      "test",
      "-p",
      "registry",
      "--test",
      "registry_contract",
      "append_advances_events_and_cursor_but_rejects_a_replaced_file",
      "--",
      "--exact",
    ],
  },
  {
    group: "restore",
    name: "并发目标改写不被补偿覆盖，其他已提交文件仍回滚",
    args: [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--no-default-features",
      "--lib",
      "backup::tests::restore_compensation_continues_after_a_concurrent_target_change",
      "--",
      "--exact",
    ],
  },
  {
    group: "restore",
    name: "补偿覆盖 present→absent 与 absent→present",
    args: [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--no-default-features",
      "--lib",
      "backup::tests::restore_compensation_recreates_removed_files_and_removes_new_files",
      "--",
      "--exact",
    ],
  },
  {
    group: "restore",
    name: "SQLite commit 失败补偿已写文件和 project state",
    args: [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--no-default-features",
      "--lib",
      "backup::tests::codex_restore_commit_failure_compensates_desktop_project_state",
      "--",
      "--exact",
    ],
  },
  {
    group: "restore",
    name: "restore source hash 漂移不会覆盖目标",
    args: [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--no-default-features",
      "--lib",
      "backup::tests::atomic_restore_copy_rejects_unverified_source_bytes",
      "--",
      "--exact",
    ],
  },
  {
    group: "restore",
    name: "无法确认 post-write fingerprint 时拒绝盲目补偿",
    args: [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--no-default-features",
      "--lib",
      "backup::tests::restore_snapshot_never_blindly_compensates_without_a_post_write_fingerprint",
      "--",
      "--exact",
    ],
  },
];

for (const testCase of matrix) {
  console.log(`\n[${testCase.group}] ${testCase.name}`);
  const result = spawnSync(cargo, testCase.args, {
    cwd: repoRoot,
    env: process.env,
    stdio: "inherit",
    shell: false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${cargo} ${testCase.args.join(" ")} failed with exit code ${result.status}`,
    );
  }
}

console.log(`\nSafety matrix passed: ${matrix.length}/${matrix.length}`);
