import { filterUsageByDate, summarizeUsage, uniqueUsageSessions, validateUsageDateRange, type PriceBook, type UsageDateRange, type UsageSession, type UsageStatus } from "./session-usage";
import { compareUsagePeriods, usageCostOverflow } from "./usage-period-comparison";

type Cell = string | number | boolean | null | undefined;
const columns = ["record_type", "metric", "value", "date", "from", "to", "sessions", "recorded_days", "tokens", "known_cost_usd", "cost_status", "unknown_price_models", "delta", "percent", "complete", "note"] as const;
type Row = Partial<Record<typeof columns[number], Cell>>;

function cell(value: Cell): string {
  if (value === null || value === undefined || (typeof value === "number" && !Number.isFinite(value))) return "";
  let text = String(value);
  // A quoted CSV field can still be interpreted as a spreadsheet formula.
  if (typeof value === "string" && (/^[\s\u0000-\u001f]*[=+@-]/.test(text) || /^[\t\r\n]/.test(text))) text = "'" + text;
  return `"${text.replace(/"/g, '""')}"`;
}

function totals(sessions: UsageSession[], prices: PriceBook) {
  const summary = summarizeUsage(sessions, prices);
  const tokens = summary.tokens.input + summary.tokens.output + summary.tokens.cache_read + summary.tokens.cache_write;
  // estimateCost treats arithmetic overflow as unknown pricing. Distinguish it
  // here so a truncated known subtotal never looks like a trustworthy amount.
  const costOverflow = usageCostOverflow(sessions, prices);
  return { ...summary, total: tokens, overflow: summary.overflow || !Number.isSafeInteger(tokens) || tokens < 0 || costOverflow, hasUsage: sessions.some(session => session.models.length) };
}

function groupDays(sessions: UsageSession[], range: UsageDateRange) {
  const groups = new Map<string, UsageSession[]>();
  for (const session of sessions) {
    const buckets = new Map<string, NonNullable<UsageSession["daily"]>>();
    for (const day of session.daily ?? []) {
      if (!day.date || validateUsageDateRange({ from: day.date, to: day.date }) || (range.from && day.date < range.from) || (range.to && day.date > range.to)) continue;
      const bucket = buckets.get(day.date) ?? [];
      bucket.push(day); buckets.set(day.date, bucket);
    }
    for (const [date, daily] of buckets) {
      const group = groups.get(date) ?? [];
      // Reuse the shared model merge on just this day's buckets, never rescan
      // every session for every distinct date in a long history.
      group.push(...filterUsageByDate([{ ...session, daily }], { from: date, to: date }));
      groups.set(date, group);
    }
  }
  return [...groups].sort(([a], [b]) => a.localeCompare(b));
}

export function buildUsageCsv({ status, prices, range, period, error, generatedAt = new Date() }: {
  status: UsageStatus; prices: PriceBook; range: UsageDateRange;
  period: { days: 7 | 30; now: Date }; error?: string; generatedAt?: Date;
}): string {
  const invalidRange = validateUsageDateRange(range);
  if (invalidRange) throw new Error(invalidRange);
  if (!Number.isFinite(generatedAt.getTime())) throw new Error("导出时间无效");
  const sessions = uniqueUsageSessions(status.results);
  const analysisComplete = status.state === "completed" && !status.error && !error && !status.failures.length;
  const comparison = compareUsagePeriods(sessions, prices, period.days, period.now, analysisComplete);
  const scoped = filterUsageByDate(sessions, range), summary = totals(scoped, prices);
  const globallyIncomplete = !analysisComplete || comparison.undatedSessions > 0 || comparison.warnedSessions > 0 || comparison.missingUsageSessions > 0;
  const rows: Row[] = [];
  const meta = (metric: string, value: Cell, note?: string) => rows.push({ record_type: "metadata", metric, value, note });
  const quality = (metric: string, value: Cell, note: string) => rows.push({ record_type: "quality", metric, value, note });
  meta("format_version", 1);
  meta("generated_at_utc", generatedAt.toISOString());
  meta("date_timezone", "UTC", "日期边界均包含；最近周期包含尚未结束的今天，不外推全天用量。");
  meta("analysis_state", status.state);
  meta("scope_sessions", sessions.length, "当前分析范围；不包含未选入分析的会话。");
  meta("processed_files", status.processed_files);
  meta("total_files", status.total_files);
  meta("comparison_days", period.days, "相邻等长周期；独立于当前日期筛选。");
  meta("comparison_anchor_utc", period.now.toISOString());
  meta("pricing", "current_local_prices_usd_per_million_tokens", "本期与上期使用同一份当前本地单价；已知费用不是完整账单，缺单价不等于免费。");
  meta("missing_values", "blank", "空数值表示缺少记录或无法安全计算；有明确记录的零保留为 0。缺失日期不补零。");
  rows.push({ record_type: "summary", from: range.from, to: range.to, sessions: scoped.length,
    tokens: summary.hasUsage && !summary.overflow ? summary.total : null,
    known_cost_usd: summary.hasUsage && !summary.overflow ? summary.known : null,
    cost_status: !summary.hasUsage ? "no_usage" : summary.overflow ? "overflow" : summary.unknownModels ? "partial_known_only" : "known", unknown_price_models: summary.unknownModels, complete: !globallyIncomplete && summary.hasUsage && !summary.overflow && !summary.unknownModels,
    note: "当前日期范围汇总；空起止表示不限日期，未限定日期时包含无日期用量。Token 不重复计入 reasoning 和一小时缓存子集。" });
  for (const [date, daySessions] of groupDays(sessions, range)) {
    const daySummary = totals(daySessions, prices);
    rows.push({ record_type: "daily", date, sessions: daySessions.length,
      tokens: daySummary.hasUsage && !daySummary.overflow ? daySummary.total : null,
      known_cost_usd: daySummary.hasUsage && !daySummary.overflow ? daySummary.known : null,
      cost_status: !daySummary.hasUsage ? "no_usage" : daySummary.overflow ? "overflow" : daySummary.unknownModels ? "partial_known_only" : "known", unknown_price_models: daySummary.unknownModels, complete: !globallyIncomplete && daySummary.hasUsage && !daySummary.overflow && !daySummary.unknownModels && !daySummary.warnedSessions && !daySummary.missingUsage,
      note: daySummary.overflow ? "超出安全计算范围" : !daySummary.hasUsage ? "未记录用量" : "已记录日期；不代表全天或全部会话均已记录。" });
  }
  for (const [record_type, result] of [["period_current", comparison.current], ["period_previous", comparison.previous]] as const) {
    rows.push({ record_type, from: result.range.from, to: result.range.to, sessions: result.sessionCount, recorded_days: result.recordedDays,
      tokens: result.hasUsage && !result.overflow ? result.tokens : null,
      known_cost_usd: result.hasUsage && !result.overflow ? result.known : null,
      cost_status: !result.hasUsage ? "no_usage" : result.overflow ? "overflow" : result.unknownModels ? "partial_known_only" : "known", unknown_price_models: result.unknownModels, complete: !globallyIncomplete && result.hasUsage && !result.overflow && !result.unknownModels,
      note: result.overflow ? "超出安全计算范围" : result.hasUsage ? "已记录用量；本期包含未结束的今天。" : "没有用量记录，不能视为零消耗" });
  }
  for (const [metric, change] of [["tokens", comparison.tokensChange], ["known_cost_usd", comparison.costChange]] as const) {
    rows.push({ record_type: "period_change", metric, delta: change.delta, percent: change.percent,
      complete: change.delta !== null && change.percent !== null, note: change.reason ?? "本期减上期；percent 为百分数，不是小数比例。" });
  }
  quality("analysis_complete", analysisComplete, "只有分析完成、无分析错误且无文件失败时为 true；运行中、取消和失败的导出均为部分结果。");
  quality("analysis_error", Boolean(error || status.error), "错误详情请查看应用；CSV 不包含来源路径或原始错误文本。");
  quality("failed_files", status.failures.length, "失败文件的用量未纳入；不输出文件路径。");
  quality("cache_warning", Boolean(status.cache_warning), "缓存警告不必然意味着分析缺失；具体原因请查看应用。");
  quality("undated_sessions", comparison.undatedSessions, "无日期用量不能归入按日或周期统计；周期变化因此不可用。");
  quality("warned_sessions", comparison.warnedSessions, "存在解析警告时周期变化不可用；不导出原始会话内容或警告中的路径。");
  quality("missing_usage_sessions", comparison.missingUsageSessions, "缺少用量记录不等于零消耗。");
  quality("unknown_price_models", summary.unknownModels, "当前日期范围中缺少适用单价的模型记录数；已知费用不代表总费用。");
  quality("summary_overflow", summary.overflow, "超出安全计算范围时汇总数值留空。");
  const csv = "\uFEFF" + [columns.map(cell).join(","), ...rows.map(row => columns.map(column => cell(row[column])).join(","))].join("\r\n") + "\r\n";
  if (new TextEncoder().encode(csv).byteLength > 32 * 1024 * 1024) throw new Error("CSV 超过 32 MiB，请缩小日期范围后重试");
  return csv;
}
