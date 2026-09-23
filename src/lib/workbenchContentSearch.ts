import type { ContentSearchStatus, SessionSummary } from "./api";

export function workbenchSearchScopes(sessions: readonly SessionSummary[]) {
  return (["codex", "claude", "qoder", "workbuddy", "grok", "pi", "dsh", "hermes", "opencode", "zcode"] as const).map(provider => ({
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
