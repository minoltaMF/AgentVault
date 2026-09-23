import test from "node:test";
import assert from "node:assert/strict";
import { createWorkbenchCache } from "./workbenchCache.ts";
import type { WorkbenchScanStatus } from "./api.ts";

const completed = { progress: { state: "completed", results: [] } as unknown as WorkbenchScanStatus, checkedAt: "2026-09-13T00:00:00Z" };

test("completed empty sources and view survive page return without disk persistence", () => {
  const cache = createWorkbenchCache();
  cache.source("codex", "/codex", "/codex").completed = completed;
  Object.assign(cache.view("/codex", "/claude"), { search: "q=hello", page: 2, scrollTop: 912 });
  assert.equal(cache.source("codex", "/codex", "/codex").completed, completed);
  assert.deepEqual(cache.view("/codex", "/claude"), { search: "q=hello", page: 2, scrollTop: 912 });
  assert.equal(createWorkbenchCache().source("codex", "/codex", "/codex").completed, null);
});

test("changed source discards its cache and late writes cannot contaminate replacement", () => {
  const cache = createWorkbenchCache();
  const old = cache.source("codex", "/old", "/old");
  old.completed = completed;
  const current = cache.source("codex", "/new", "/new");
  old.completed = { ...completed, checkedAt: "late response" };
  assert.equal(current.completed, null);
  assert.equal(cache.source("codex", "/old", "/old").completed, null);
});

test("provider caches are separate and changed context resets only affected data", () => {
  const cache = createWorkbenchCache();
  cache.source("codex", "/same", "/same").completed = completed;
  assert.equal(cache.source("claude", "/same", "/same").completed, null);
  cache.source("claude", "/same", "/same").completed = completed;
  assert.equal(cache.source("claude", "/same", "/different-codex").completed, null);
  assert.equal(cache.source("codex", "/same", "/same").completed, completed);
  cache.view("/same", "/same").page = 5;
  assert.deepEqual(cache.view("/same", "/new"), { search: "", page: 0, scrollTop: 0 });
});

test("Qoder source and view are isolated when its configured root changes", () => {
  const cache = createWorkbenchCache();
  const old = cache.source("qoder", "/qoder-a", "/codex");
  old.completed = completed;
  cache.view("/codex", "/claude", "/qoder-a").page = 2;
  const next = cache.source("qoder", "/qoder-b", "/codex");
  old.completed = { ...completed, checkedAt: "late" };
  assert.equal(next.completed, null);
  assert.equal(cache.source("claude", "/qoder-b", "/codex").completed, null);
  assert.deepEqual(cache.view("/codex", "/claude", "/qoder-b"), { search: "", page: 0, scrollTop: 0 });
});

for (const [provider, rootIndex] of [["workbuddy", 3], ["grok", 4], ["pi", 5], ["dsh", 6], ["hermes", 7], ["opencode", 8], ["zcode", 9]] as const) {
  test(provider + " source changes clear retained view and reject late results", () => {
    const cache = createWorkbenchCache();
    const roots: [string, string, string, string, string, string, string, string, string, string] = ["/codex", "/claude", "/qoder", "/workbuddy", "/grok", "/pi", "/dsh", "/hermes", "/opencode", "/zcode"];
    const old = cache.source(provider, roots[rootIndex], roots[0]);
    old.completed = completed;
    cache.view(...roots).page = 5;
    roots[rootIndex] += "-new";
    const current = cache.source(provider, roots[rootIndex], roots[0]);
    old.completed = { ...completed, checkedAt: "late" };
    assert.equal(current.completed, null);
    assert.deepEqual(cache.view(...roots), { search: "", page: 0, scrollTop: 0 });
  });
}
