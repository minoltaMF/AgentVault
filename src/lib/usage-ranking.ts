import { aggregateUsage, priceKey, summarizeUsage, uniqueUsageSessions, type PriceBook, type UsageSession } from "./session-usage";

export type UsageRankingGroup = "model" | "project";
export type UsageRankingSort = "tokens" | "cost";
export type UsageRankingEntry = {
  key: string;
  label: string;
  sessions: UsageSession[];
  summary: ReturnType<typeof summarizeUsage>;
  totalTokens: number | null;
};

/** Input sessions have already been filtered to the requested date range. */
export function rankUsage(sessions: UsageSession[], prices: PriceBook, by: UsageRankingGroup, sort: UsageRankingSort): UsageRankingEntry[] {
  const unique = uniqueUsageSessions(sessions);
  // Separate missing paths before using the legacy aggregate's display-label grouping.
  // A real path named "未记录项目路径" must remain a different project.
  const groups = by === "model"
    ? aggregateUsage(unique, by).map(group => ({ ...group, key: group.label, sessions: group.sessions.map(session => ({
      ...session,
      daily: session.daily?.map(day => ({ ...day, models: day.models.filter(model => priceKey(session.provider, model.model) === group.label) })),
      undated_models: session.undated_models?.filter(model => priceKey(session.provider, model.model) === group.label),
    })) }))
    : [
      ...aggregateUsage(unique.filter(session => !!session.cwd), by).map(group => ({ ...group, key: JSON.stringify(["project", group.label]) })),
      ...aggregateUsage(unique.filter(session => !session.cwd), by).map(group => ({ ...group, key: JSON.stringify(["project", null]) })),
    ];
  return groups.map(group => {
    const summary = summarizeUsage(group.sessions, prices);
    // Reasoning and one-hour cache writes are subsets, not additional tokens.
    const total = summary.tokens.input + summary.tokens.output + summary.tokens.cache_read + summary.tokens.cache_write;
    const totalTokens = summary.overflow || summary.missingUsage > 0 || !Number.isSafeInteger(total) || total < 0 ? null : total;
    return { ...group, summary, totalTokens };
  }).sort((a, b) => {
    const aInvalid = a.totalTokens === null;
    const bInvalid = b.totalTokens === null;
    if (aInvalid !== bInvalid) return aInvalid ? 1 : -1;
    const difference = aInvalid ? 0 : sort === "tokens" ? b.totalTokens! - a.totalTokens! : b.summary.known - a.summary.known;
    return difference || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
  });
}
