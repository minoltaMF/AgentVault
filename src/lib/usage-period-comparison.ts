import { filterUsageByDate, priceKey, recentUsageRange, summarizeUsage, undatedUsageSessions, uniqueUsageSessions, validateUsageDateRange, type PriceBook, type UsageDateRange, type UsageSession } from "./session-usage";

export type UsagePeriodSummary = { range: UsageDateRange; sessionCount: number; recordedDays: number; tokens: number; known: number; unknownModels: number; overflow: boolean; hasUsage: boolean };
export type UsagePeriodChange = { delta: number | null; percent: number | null; reason: string | null };

function costOverflow(sessions: UsageSession[], prices: PriceBook): boolean {
  return sessions.some(session => session.models.some(({ model, tokens }) => {
    const rates = prices[priceKey(session.provider, model)] ?? {};
    const billable = { input: tokens.input, output: tokens.output, cache_read: tokens.cache_read, cache_write_5m: Math.max(0, tokens.cache_write - tokens.cache_write_1h), cache_write_1h: tokens.cache_write_1h };
    let total = 0;
    return (Object.keys(billable) as (keyof typeof billable)[]).some(field => {
      const rate = rates[field];
      if (rate === undefined || !Number.isFinite(rate) || rate < 0) return false;
      total += billable[field] / 1_000_000 * rate;
      return !Number.isFinite(total) || total > Number.MAX_SAFE_INTEGER;
    });
  }));
}

function period(sessions: UsageSession[], prices: PriceBook, range: UsageDateRange): UsagePeriodSummary {
  const rows = filterUsageByDate(sessions, range);
  const summary = summarizeUsage(rows, prices);
  // Reasoning and one-hour cache writes are subsets, not additional tokens.
  const tokens = summary.tokens.input + summary.tokens.output + summary.tokens.cache_read + summary.tokens.cache_write;
  const recorded = rows.flatMap(row => (row.daily ?? []).filter(day => day.models.length).map(day => day.date));
  return { range, sessionCount: rows.filter(row => row.models.length).length, recordedDays: new Set(recorded).size,
    tokens, known: summary.known, unknownModels: summary.unknownModels,
    overflow: summary.overflow || !Number.isSafeInteger(tokens) || tokens < 0 || costOverflow(rows, prices), hasUsage: recorded.length > 0 };
}

function change(current: number, previous: number): UsagePeriodChange {
  const delta = current - previous;
  if (previous === 0) return { delta, percent: current === 0 ? 0 : null, reason: current === 0 ? null : "上一周期为零，无法计算百分比" };
  const percent = delta / previous * 100;
  if (!Number.isFinite(percent) || Math.abs(percent) > Number.MAX_SAFE_INTEGER) return { delta, percent: null, reason: "变化百分比超出安全计算范围" };
  return { delta, percent, reason: null };
}

export function compareUsagePeriods(sessions: UsageSession[], prices: PriceBook, days: 7 | 30, now: Date, analysisComplete: boolean) {
  if (days !== 7 && days !== 30) throw new Error("周期仅支持 7 或 30 天");
  const currentRange = recentUsageRange(days, now);
  const previousEnd = new Date(`${currentRange.from}T00:00:00.000Z`);
  previousEnd.setUTCDate(previousEnd.getUTCDate() - 1);
  const previousRange = recentUsageRange(days, previousEnd);
  const unique = uniqueUsageSessions(sessions);
  const current = period(unique, prices, currentRange), previous = period(unique, prices, previousRange);
  const scope = summarizeUsage(unique, prices);
  const undated = new Set(undatedUsageSessions(unique).map(row => priceKey(row.provider, row.rollout_path)));
  for (const row of unique) {
    if ((row.models.length && !row.daily?.some(day => day.models.length)) || row.daily?.some(day => day.models.length && (!day.date || validateUsageDateRange({ from: day.date, to: day.date }))))
      undated.add(priceKey(row.provider, row.rollout_path));
  }
  const undatedSessions = undated.size, missingUsageSessions = scope.missingUsage, warnedSessions = scope.warnedSessions;
  const scopeTokens = scope.tokens.input + scope.tokens.output + scope.tokens.cache_read + scope.tokens.cache_write;
  const reasons: string[] = [];
  if (!analysisComplete) reasons.push("分析尚未完整完成，暂不比较");
  if (warnedSessions) reasons.push(`${warnedSessions} 个会话有解析警告`);
  if (missingUsageSessions) reasons.push(`${missingUsageSessions} 个会话缺少用量记录`);
  if (undatedSessions) reasons.push(`${undatedSessions} 个会话包含无法归入日期的用量`);
  if (current.overflow || previous.overflow || scope.overflow || !Number.isSafeInteger(scopeTokens) || costOverflow(unique, prices)) reasons.push("用量或费用超出安全计算范围");
  if (!current.hasUsage || !previous.hasUsage) reasons.push("有周期没有用量记录，不能视为零消耗");
  const unavailable: UsagePeriodChange = { delta: null, percent: null, reason: reasons.join("；") };
  const tokensChange = reasons.length ? { ...unavailable } : change(current.tokens, previous.tokens);
  const costChange = reasons.length ? { ...unavailable } : change(current.known, previous.known);
  if (!reasons.length && (current.unknownModels || previous.unknownModels)) {
    costChange.percent = null;
    costChange.reason = "存在未配置单价，仅比较已知费用差额，不代表总费用变化";
  }
  return { current, previous, tokensChange, costChange, undatedSessions, missingUsageSessions, warnedSessions };
}
