import type { ContentSearchStatus, SessionSummary } from "./api";

/** A completed native scan, even empty, supersedes historical index metadata. */
export function mergeIndexedSessions(live: readonly SessionSummary[], indexed: readonly SessionSummary[], refreshed: readonly string[]) {
  const keys = new Set(live.map(row => JSON.stringify([row.provider, row.rollout_path])));
  return [...live, ...indexed.filter(row => !refreshed.includes(row.provider)
    && !keys.has(JSON.stringify([row.provider, row.rollout_path])))];
}

export function workbenchSearchScopes(sessions: readonly SessionSummary[]) {
  return (["codex", "claude", "qoder", "workbuddy", "grok", "pi", "dsh", "hermes", "opencode", "zcode", "qwen", "cline", "copilot", "antigravity"] as const).map(provider => ({
    provider,
    rollout_paths: [...new Set(sessions.filter(session => session.provider === provider).map(session => session.rollout_path))].sort(),
  })).filter(scope => scope.rollout_paths.length > 0);
}

export function createContentSearchCache() {
  let saved: { scope: string; value: { query: string; status: ContentSearchStatus | null } } | undefined;
  return (scope: string) => {
    if (saved?.scope !== scope) saved = { scope, value: { query: "", status: null } };
    return saved.value;
  };
}
export const contentSearchCache = createContentSearchCache();
