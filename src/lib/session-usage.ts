export type UsageTokens = { input: number; output: number; cache_read: number; cache_write: number; cache_write_1h: number; reasoning: number };
export type UsageModel = { model: string; tokens: UsageTokens };
export type UsageDay = { date: string; models: UsageModel[] };
export type UsageSession = { provider: string; rollout_path: string; id: string; title: string; cwd: string; models: UsageModel[]; warnings: string[]; daily?: UsageDay[]; undated_models?: UsageModel[] };
export type UsageScope = Pick<UsageSession, "provider" | "rollout_path" | "id" | "title" | "cwd">;
export type UsageStatus = { state: "running" | "completed" | "cancelled" | "failed"; total_files: number; processed_files: number; cache_hits: number; parsed_files: number; elapsed_ms: number; cache_warning: string | null; results: UsageSession[]; failures: { path: string; error: string }[]; error: string | null };
export const rateFields = ["input", "output", "cache_read", "cache_write_5m", "cache_write_1h"] as const;
export type RateField = typeof rateFields[number];
export type Rates = Partial<Record<RateField, number>>;
export type PriceBook = Record<string, Rates>;
export const priceKey = (provider: string, model: string) => JSON.stringify([provider, model]);
export const priceStorageKey = "agentvault.session-usage-prices.v1";
export function parseRate(value: string): number | undefined {
  if (!value.trim()) return undefined;
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0 || !/^\d*(\.\d*)?$/.test(value.trim())) throw new Error("单价必须为空或非负有限数字");
  return number;
}
export function readPriceBook(raw: string | null): PriceBook {
  try {
    const data = JSON.parse(raw ?? "null");
    if (data?.version !== 1 || !data.prices || typeof data.prices !== "object") return {};
    const prices: PriceBook = {};
    for (const [key, rates] of Object.entries(data.prices)) {
      if (!rates || typeof rates !== "object") continue;
      const clean: Rates = {};
      for (const field of rateFields) {
        const value = (rates as Rates)[field];
        if (typeof value === "number" && Number.isFinite(value) && value >= 0) clean[field] = value;
      }
      prices[key] = clean;
    }
    return prices;
  } catch { return {}; }
}
export function estimateCost(tokens: UsageTokens, rates: Rates = {}) {
  // Input is uncached; reasoning is already part of output. The one-hour cache is a subset.
  const billable = { input: tokens.input, output: tokens.output, cache_read: tokens.cache_read, cache_write_5m: Math.max(0, tokens.cache_write - tokens.cache_write_1h), cache_write_1h: tokens.cache_write_1h };
  let known = 0; const unknown: RateField[] = [];
  const invalid = Object.values(tokens).some(value => !Number.isSafeInteger(value) || value < 0) || tokens.cache_write_1h > tokens.cache_write || tokens.reasoning > tokens.output;
  if (invalid) return { known: 0, unknown: [...rateFields], invalid: true };
  for (const field of rateFields) {
    if (!billable[field]) continue;
    const rate = rates[field];
    if (rate === undefined || !Number.isFinite(rate) || rate < 0) unknown.push(field);
    else {
      const value = billable[field] / 1_000_000 * rate;
      if (!Number.isFinite(value) || value > Number.MAX_SAFE_INTEGER || known + value > Number.MAX_SAFE_INTEGER) unknown.push(field);
      else known += value;
    }
  }
  return { known, unknown, invalid: false };
}
export function uniqueUsageSessions(sessions: UsageSession[]): UsageSession[] {
  return [...new Map(sessions.map(session => [priceKey(session.provider, session.rollout_path), session])).values()];
}
export function aggregateUsage(sessions: UsageSession[], by: "model" | "project") {
  const groups = new Map<string, { label: string; sessions: UsageSession[] }>();
  for (const session of uniqueUsageSessions(sessions)) {
    const labels = by === "project" ? [session.cwd || "未记录项目路径"] : [...new Set(session.models.map(model => priceKey(session.provider, model.model)))];
    for (const label of labels) {
      const group = groups.get(label) ?? { label, sessions: [] };
      group.sessions.push(by === "model" ? { ...session, models: session.models.filter(model => priceKey(session.provider, model.model) === label) } : session);
      groups.set(label, group);
    }
  }
  return [...groups.values()];
}
export function summarizeUsage(sessions: UsageSession[], prices: PriceBook) {
  const tokens: UsageTokens = { input: 0, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, reasoning: 0 };
  let known = 0, unknownModels = 0, missingUsage = 0, warnedSessions = 0;
  let overflow = false;
  for (const session of uniqueUsageSessions(sessions)) {
    if (!session.models.length) missingUsage++;
    if (session.warnings.length) warnedSessions++;
    for (const model of session.models) {
      for (const key of Object.keys(tokens) as (keyof UsageTokens)[]) {
        const sum = tokens[key] + model.tokens[key];
        if (!Number.isSafeInteger(sum) || sum < 0) overflow = true;
        tokens[key] = sum;
      }
      const cost = estimateCost(model.tokens, prices[priceKey(session.provider, model.model)]);
      if (!Number.isFinite(known + cost.known) || known + cost.known > Number.MAX_SAFE_INTEGER || cost.invalid) overflow = true;
      else known += cost.known;
      if (cost.unknown.length) unknownModels++;
    }
  }
  return { tokens, known, unknownModels, missingUsage, warnedSessions, overflow };
}

export type UsageCacheStatus = { state: "ready" | "missing" | "degraded" | "disabled" | "unchecked"; disk_bytes: number | null; memory_entries: number; memory_bytes: number; path: string | null; warning: string | null; backup_path: string | null };

export type UsageDateRange = { from: string; to: string };
const allUsageDates: UsageDateRange = { from: "", to: "" };
function validUsageDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const date = new Date(`${value}T00:00:00.000Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}
export function validateUsageDateRange(range: UsageDateRange): string {
  if ((range.from && !validUsageDate(range.from)) || (range.to && !validUsageDate(range.to))) return "日期必须是有效的 YYYY-MM-DD 日期";
  if (range.from && range.to && range.from > range.to) return "开始日期不能晚于结束日期";
  return "";
}
export function recentUsageRange(days: number, now = new Date()): UsageDateRange {
  if (!Number.isSafeInteger(days) || days < 1 || !Number.isFinite(now.getTime())) throw new Error("天数和当前日期无效");
  const to = now.toISOString().slice(0, 10);
  const start = new Date(`${to}T00:00:00.000Z`);
  start.setUTCDate(start.getUTCDate() - days + 1);
  const from = Number.isFinite(start.getTime()) ? start.toISOString().slice(0, 10) : "";
  if (!validUsageDate(from) || !validUsageDate(to)) throw new Error("日期范围超出支持范围");
  return { from, to };
}
function matchesUsageDate(date: string, range: UsageDateRange): boolean {
  return validUsageDate(date) && (!range.from || date >= range.from) && (!range.to || date <= range.to);
}
function mergeUsageModels(models: UsageModel[]): UsageModel[] {
  const merged = new Map<string, UsageModel>();
  for (const model of models) {
    const previous = merged.get(model.model);
    if (!previous) merged.set(model.model, { model: model.model, tokens: { ...model.tokens } });
    else for (const key of Object.keys(previous.tokens) as (keyof UsageTokens)[]) previous.tokens[key] += model.tokens[key];
  }
  return [...merged.values()];
}
export function filterUsageByDate(sessions: UsageSession[], range: UsageDateRange): UsageSession[] {
  if (validateUsageDateRange(range)) return [];
  const unique = uniqueUsageSessions(sessions);
  if (!range.from && !range.to) return unique;
  return unique.flatMap(session => {
    const daily = (session.daily ?? []).filter(day => matchesUsageDate(day.date, range));
    if (!daily.length) return [];
    return [{ ...session, daily, models: mergeUsageModels(daily.flatMap(day => day.models)), undated_models: [] }];
  });
}
export function undatedUsageSessions(sessions: UsageSession[]): UsageSession[] {
  return uniqueUsageSessions(sessions).flatMap(session => {
    const models = session.daily === undefined ? session.models : session.undated_models ?? [];
    return models.length ? [{ ...session, models, daily: [], undated_models: models }] : [];
  });
}
export type UsageDailyTrend = { date: string; sessions: number; tokens: number; known: number; incomplete: boolean; overflow: boolean; unknownModels: number; warnedSessions: number; missingUsage: number };
export function dailyUsageTrend(sessions: UsageSession[], prices: PriceBook, range: UsageDateRange = allUsageDates): UsageDailyTrend[] {
  if (validateUsageDateRange(range)) return [];
  const dates = new Map<string, UsageSession[]>();
  for (const session of uniqueUsageSessions(sessions)) {
    const days = new Map<string, UsageModel[]>();
    for (const day of session.daily ?? []) {
      if (matchesUsageDate(day.date, range)) days.set(day.date, [...days.get(day.date) ?? [], ...day.models]);
    }
    for (const [date, models] of days) {
      const group = dates.get(date) ?? [];
      group.push({ ...session, models: mergeUsageModels(models) });
      dates.set(date, group);
    }
  }
  return [...dates].sort(([a], [b]) => a.localeCompare(b)).map(([date, rows]) => {
    const summary = summarizeUsage(rows, prices);
    const tokens = summary.tokens.input + summary.tokens.output + summary.tokens.cache_read + summary.tokens.cache_write;
    const overflow = summary.overflow || !Number.isSafeInteger(tokens) || tokens < 0;
    return { date, sessions: rows.length, tokens, known: summary.known, overflow,
      incomplete: overflow || summary.unknownModels > 0 || summary.warnedSessions > 0 || summary.missingUsage > 0,
      unknownModels: summary.unknownModels, warnedSessions: summary.warnedSessions, missingUsage: summary.missingUsage };
  });
}
