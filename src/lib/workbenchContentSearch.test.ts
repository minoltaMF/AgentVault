import test from "node:test";
import assert from "node:assert/strict";
import type { SessionSummary } from "./api.ts";
import { createContentSearchCache, mergeIndexedSessions, workbenchSearchScopes } from "./workbenchContentSearch.ts";

test("fresh source results supersede cached metadata including empty scans", () => {
  const cached = [
    { provider: "codex", rollout_path: "/old", title: "old" },
    { provider: "claude", rollout_path: "/claude" },
  ] as SessionSummary[];
  assert.deepEqual(mergeIndexedSessions([], cached, []), cached);
  assert.deepEqual(mergeIndexedSessions([], cached, ["codex"]), [cached[1]]);
  const live = [{ ...cached[0], title: "fresh" }];
  assert.deepEqual(mergeIndexedSessions(live, cached, []), [live[0], cached[1]]);
  assert.deepEqual(workbenchSearchScopes(mergeIndexedSessions([], cached, ["codex", "claude"])), []);
});

test("search scope preserves providers, deduplicates paths and keeps empty scope empty", () => {
  const rows = [
    { provider: "codex", rollout_path: "/one" },
    { provider: "claude", rollout_path: "/one" },
    { provider: "qoder", rollout_path: "/one" },
    { provider: "workbuddy", rollout_path: "/one" },
    { provider: "grok", rollout_path: "/one" },
    { provider: "pi", rollout_path: "/one" },
    { provider: "dsh", rollout_path: "/one" },
    { provider: "hermes", rollout_path: "/one" },
    { provider: "opencode", rollout_path: "/one" },
    { provider: "zcode", rollout_path: "/one" },
    { provider: "qwen", rollout_path: "/one" },
    { provider: "cline", rollout_path: "/one" },
    { provider: "copilot", rollout_path: "/one" },
    { provider: "antigravity", rollout_path: "/one" },
    { provider: "codex", rollout_path: "/one" },
  ] as SessionSummary[];
  assert.deepEqual(workbenchSearchScopes(rows), [
    { provider: "codex", rollout_paths: ["/one"] },
    { provider: "claude", rollout_paths: ["/one"] },
    { provider: "qoder", rollout_paths: ["/one"] },
    { provider: "workbuddy", rollout_paths: ["/one"] },
    { provider: "grok", rollout_paths: ["/one"] },
    { provider: "pi", rollout_paths: ["/one"] },
    { provider: "dsh", rollout_paths: ["/one"] },
    { provider: "hermes", rollout_paths: ["/one"] },
    { provider: "opencode", rollout_paths: ["/one"] },
    { provider: "zcode", rollout_paths: ["/one"] },
    { provider: "qwen", rollout_paths: ["/one"] },
    { provider: "cline", rollout_paths: ["/one"] },
    { provider: "copilot", rollout_paths: ["/one"] },
    { provider: "antigravity", rollout_paths: ["/one"] },
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

test("all indexed providers replace historical rows after an empty native refresh", () => {
  for (const provider of ["codex", "claude", "qoder", "workbuddy", "grok", "pi", "qwen", "copilot"]) {
    const historical = [{ provider, rollout_path: "/fixture" }] as SessionSummary[];
    assert.deepEqual(mergeIndexedSessions([], historical, []), historical);
    assert.deepEqual(mergeIndexedSessions([], historical, [provider]), []);
    assert.deepEqual(workbenchSearchScopes(mergeIndexedSessions([], historical, [])), [{ provider, rollout_paths: ["/fixture"] }]);
  }
});
