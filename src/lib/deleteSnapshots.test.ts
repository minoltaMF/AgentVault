import assert from "node:assert/strict";
import test from "node:test";
import { deleteSnapshotLink, deleteSnapshotStatus, recoveryFolderError, readSnapshotActivities, saveSnapshotActivity } from "./deleteSnapshots";

test("snapshot result links preserve a single encoded folder name across host paths", () => {
  assert.equal(deleteSnapshotLink("C:\\backups\\delete-20260909T000000Z-abc"), "/codex/delete-snapshots?snapshot=delete-20260909T000000Z-abc");
  assert.equal(deleteSnapshotLink("/backups/a #中文/"), "/codex/delete-snapshots?snapshot=a%20%23%E4%B8%AD%E6%96%87");
  assert.equal(deleteSnapshotLink(), "/codex/delete-snapshots");
});
test("readable metadata is never labelled as verified", () => {
  assert.equal(deleteSnapshotStatus("unverified"), "尚未校验");
  assert.equal(deleteSnapshotStatus("incomplete"), "文件不完整");
  assert.equal(deleteSnapshotStatus("unreadable"), "无法读取");
});
test("isolated recovery name cannot escape its selected parent or use Windows aliases", () => {
  for (const name of ["", " ", "../old", "a/b", "a\\b", "C:other", ".", "..", "COM1", "nul.txt", "data.", " data", "data "]) {
    assert.ok(recoveryFolderError(name), name);
  }
  assert.equal(recoveryFolderError("Codex 恢复-2026"), null);
});

test("operation history survives reload and stays scoped without granting verification", () => {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
  saveSnapshotActivity(storage, "/a/snapshot", "restore", { ok: true, time: "2026-09-09T00:00:00Z", output: "/new" });
  saveSnapshotActivity(storage, "/a/snapshot", "verify", { ok: false, time: "2026-09-09T01:00:00Z", error: "corrupt" });
  assert.equal(readSnapshotActivities(storage, "/a/snapshot").restore?.output, "/new");
  assert.equal(readSnapshotActivities(storage, "/a/snapshot").verify?.ok, false);
  assert.deepEqual(readSnapshotActivities(storage, "/b/snapshot"), {});
  assert.deepEqual(readSnapshotActivities({ getItem: () => "not json" }, "/a"), {});
  assert.deepEqual(readSnapshotActivities({ getItem: () => '{"verify":{"ok":"true","time":"invalid"}}' }, "/a"), {});
});
