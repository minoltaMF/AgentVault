import test from "node:test";
import assert from "node:assert/strict";
import { scanProgressText, workbenchSessions, requestGeneration } from "./allSessions.ts";
import { sessionIdentity } from "./sessionIdentity.ts";
import type { SessionSummary, WorkbenchScanStatus } from "./api.ts";

const row = (provider: "codex" | "claude", path: string, updated_at: number, extra = {}) => ({
  provider, id: "same-id", rollout_path: path, updated_at, title: "中文任务", first_user_message: "排查 ERROR_42", cwd: "/projects/demo", archived: false, ...extra,
} as SessionSummary);
const empty = { query: "", provider: "", project: "", archive: "" };

test("workbench preserves cross-provider and same-ID physical sessions, newest first", () => {
  const input = [row("codex", "/one", 1), row("claude", "/two", 3), row("claude", "/three", 2)];
  const output = workbenchSessions(input, empty);
  assert.deepEqual(output.map((s) => s.updated_at), [3, 2, 1]);
  assert.equal(new Set(output.map(sessionIdentity)).size, 3);
  assert.equal(input[0].updated_at, 1);
});

test("workbench combines exact project/provider/archive filters and metadata search", () => {
  const input = [row("codex", "/one", 1), row("claude", "/two", 2, { archived: true }), row("claude", "/three", 3, { cwd: "/another/demo" })];
  assert.equal(workbenchSessions(input, { ...empty, query: "中文" }).length, 3);
  assert.equal(workbenchSessions(input, { ...empty, query: "error_42", provider: "claude", project: "/projects/demo", archive: "archived" }).length, 1);
  assert.equal(workbenchSessions(input, { ...empty, project: "demo" }).length, 0);
  assert.equal(workbenchSessions(input, { ...empty, query: "unknown body content" }).length, 0);
});

test("request generation rejects unmounted and superseded responses", () => {
  const requests = requestGeneration();
  const first = requests.next();
  requests.invalidate();
  assert.equal(requests.current(first), false);
  const retry = requests.next();
  assert.equal(requests.current(retry), true);
  assert.equal(requests.current(first), false);
});

test("scan progress uses observed counts without assuming a final total", () => {
  const status = { phase: "discovering", discovered_files: 2, processed_files: 0, failed_files: 3 } as WorkbenchScanStatus;
  assert.equal(scanProgressText(status), "发现文件：已发现 2 个文件，已处理 0 个，失败 3 项");
  assert.equal(scanProgressText({ ...status, phase: "reading", processed_files: 1 }), "读取文件：已发现 2 个文件，已处理 1 个，失败 3 项");
});
