import type { SessionSummary } from "./api";
import type { UsageScope } from "./session-usage";

/** Resolve only within the analyzed scope; IDs alone are not physical identities. */
export function resolveUsageSession(sessions: readonly SessionSummary[], usage: UsageScope): SessionSummary | undefined {
  if (!usage.rollout_path) return undefined;
  const matches = sessions.filter(session => session.provider === usage.provider && session.rollout_path === usage.rollout_path);
  return matches.length === 1 ? matches[0] : undefined;
}
