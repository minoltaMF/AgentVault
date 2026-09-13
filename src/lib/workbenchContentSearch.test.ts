import test from "node:test";
import assert from "node:assert/strict";
import type { SessionSummary } from "./api.ts";
import { createContentSearchCache, workbenchSearchScopes } from "./workbenchContentSearch.ts";

test("search scope preserves providers, deduplicates paths and keeps empty scope empty", () => {
  const rows = [
    { provider: "codex", rollout_path: "/one" },
    { provider: "claude", rollout_path: "/one" },
    { provider: "codex", rollout_path: "/one" },
  ] as SessionSummary[];
  assert.deepEqual(workbenchSearchScopes(rows), [
    { provider: "codex", rollout_paths: ["/one"] },
    { provider: "claude", rollout_paths: ["/one"] },
  ]);
  assert.deepEqual(workbenchSearchScopes([]), []);
});

test("search cache survives return but isolates new source and late writes", () => {
  const cache = createContentSearchCache();
  const old = cache("old");
  old.query = "needle";
  assert.equal(cache("old").query, "needle");
  const next = cache("new");
  old.query = "late";
  assert.equal(next.query, "");
  assert.equal(cache("old").query, "");
});
