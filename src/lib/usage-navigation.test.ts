import assert from "node:assert/strict";
import test from "node:test";
import type { SessionSummary } from "./api";
import { resolveUsageSession } from "./usage-navigation";

const session = (provider: "codex" | "claude", path: string) => ({ provider, rollout_path: path, id: "reused-id", title: "same title", cwd: "/project" }) as SessionSummary;
test("usage navigation resolves the physical transcript within the current scope", () => {
  const a = session("claude", "/a.jsonl"), b = session("claude", "/b.jsonl"), c = session("codex", "/b.jsonl");
  assert.equal(resolveUsageSession([a, b, c], { ...b, title: "changed title" }), b);
  assert.equal(resolveUsageSession([a, b, c], c), c);
  assert.equal(resolveUsageSession([a, c], b), undefined);
});
test("missing or ambiguous paths cannot fall back to a shared session ID", () => {
  const a = session("claude", "/a.jsonl");
  assert.equal(resolveUsageSession([a], { ...a, rollout_path: "" }), undefined);
  assert.equal(resolveUsageSession([a, { ...a }], a), undefined);
  assert.equal(resolveUsageSession([], a), undefined);
});
