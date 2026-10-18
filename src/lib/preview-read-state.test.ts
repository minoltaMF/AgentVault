import assert from "node:assert/strict";
import test from "node:test";
import { previewReadFailure, previewEmptyMessage } from "./preview-read-state";

test("only an explicit backend prefix identifies unavailable files", () => {
  assert.deepEqual(previewReadFailure(new Error("[PREVIEW_FILE_UNAVAILABLE] moved")), { kind: "unavailable", detail: "moved" });
  assert.deepEqual(previewReadFailure("[PREVIEW_READ_FAILED] permission denied"), { kind: "failed", detail: "permission denied" });
  assert.equal(previewReadFailure("network request failed").kind, "failed");
  assert.equal(previewReadFailure("parse error containing [PREVIEW_FILE_UNAVAILABLE]").kind, "failed");
});
test("empty transcript is distinct from an empty page or filtered messages", () => {
  assert.match(previewEmptyMessage(0, 0, false), /^空会话/);
  assert.match(previewEmptyMessage(0, 200, false), /^当前页/);
  assert.match(previewEmptyMessage(1, 0, false), /^当前筛选/);
  assert.match(previewEmptyMessage(0, 0, true), /^当前筛选/);
});
