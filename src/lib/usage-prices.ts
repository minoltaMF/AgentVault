import { estimateCost, parseRate, priceKey, rateFields, uniqueUsageSessions, type PriceBook, type RateField, type Rates, type UsageSession } from "./session-usage";

export const maxPriceImportBytes = 1024 * 1024;
export const maxPriceModels = 1000;
export type PriceDrafts = Record<string, Partial<Record<RateField, string>>>;
export type UsagePriceEntry = { key: string; provider: string; model: string; rates: Rates; usedFields: RateField[]; missingFields: RateField[]; sessionCount: number; invalidUsage: boolean };

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function identity(key: string): [string, string] {
  let parts: unknown;
  try { parts = JSON.parse(key); } catch { throw new Error("模型键必须是 JSON 格式的 [来源, 模型]"); }
  if (!Array.isArray(parts) || parts.length !== 2 || !["codex", "claude"].includes(parts[0]) || typeof parts[1] !== "string") throw new Error("模型键必须包含 codex 或 claude 来源和字符串模型名称");
  return parts as [string, string];
}
function validatePrices(value: unknown): PriceBook {
  if (!object(value)) throw new Error("prices 必须是对象");
  const entries = Object.entries(value);
  if (entries.length > maxPriceModels) throw new Error("单价配置最多支持 1000 个模型");
  const result: PriceBook = {};
  for (const [key, rates] of entries) {
    const canonical = priceKey(...identity(key));
    if (Object.hasOwn(result, canonical)) throw new Error("存在重复的来源与模型配置");
    if (!object(rates)) throw new Error("模型单价必须是对象");
    const clean: Rates = {};
    for (const [field, rate] of Object.entries(rates)) {
      if (!(rateFields as readonly string[]).includes(field)) throw new Error(`不支持的单价字段：${field}`);
      if (typeof rate !== "number" || !Number.isFinite(rate) || rate < 0) throw new Error("单价必须是非负有限数字，未知单价请省略字段");
      clean[field as RateField] = rate;
    }
    result[canonical] = clean;
  }
  return result;
}

export function usagePriceCatalog(sessions: UsageSession[], prices: PriceBook): UsagePriceEntry[] {
  const catalog = new Map<string, UsagePriceEntry>();
  const entry = (provider: string, model: string) => {
    const key = priceKey(provider, model);
    let row = catalog.get(key);
    if (!row) { row = { key, provider, model, rates: { ...prices[key] }, usedFields: [], missingFields: [], sessionCount: 0, invalidUsage: false }; catalog.set(key, row); }
    return row;
  };
  for (const key of Object.keys(prices)) {
    // Existing storage was historically permissive; malformed legacy keys cannot be edited safely.
    try { entry(...identity(key)); } catch { /* Keep unsupported legacy configuration untouched. */ }
  }
  for (const session of uniqueUsageSessions(sessions)) {
    if (session.provider !== "codex" && session.provider !== "claude") continue;
    const seen = new Set<string>();
    for (const model of session.models) {
      const row = entry(session.provider, model.model);
      if (!seen.has(row.key)) { row.sessionCount++; seen.add(row.key); }
      const cost = estimateCost(model.tokens, {});
      if (cost.invalid) { row.invalidUsage = true; continue; }
      for (const field of cost.unknown) if (!row.usedFields.includes(field)) row.usedFields.push(field);
    }
  }
  for (const row of catalog.values()) {
    row.usedFields = rateFields.filter(field => row.usedFields.includes(field));
    row.missingFields = row.usedFields.filter(field => row.rates[field] === undefined || !Number.isFinite(row.rates[field]) || row.rates[field]! < 0);
  }
  return [...catalog.values()].sort((a, b) => Number(b.missingFields.length > 0) - Number(a.missingFields.length > 0) || a.provider.localeCompare(b.provider) || a.model.localeCompare(b.model));
}

export function applyPriceDrafts(prices: PriceBook, drafts: PriceDrafts): PriceBook {
  const updates = new Map<string, Partial<Record<RateField, number | undefined>>>();
  for (const [key, draft] of Object.entries(drafts)) {
    const canonical = priceKey(...identity(key));
    if (updates.has(canonical)) throw new Error("存在重复的来源与模型草稿");
    if (!object(draft)) throw new Error("单价草稿必须是对象");
    const rates: Partial<Record<RateField, number | undefined>> = {};
    for (const [field, value] of Object.entries(draft)) {
      if (!(rateFields as readonly string[]).includes(field) || typeof value !== "string") throw new Error("单价草稿字段无效");
      try { rates[field as RateField] = parseRate(value); } catch { throw new Error(`${identity(key)[1]} / ${field}：单价必须为空或非负有限数字`); }
    }
    updates.set(canonical, rates);
  }
  const result = { ...prices };
  for (const [key, fields] of updates) {
    const rates = { ...result[key] };
    for (const field of rateFields) if (Object.hasOwn(fields, field)) {
      if (fields[field] === undefined) delete rates[field]; else rates[field] = fields[field];
    }
    result[key] = rates;
  }
  return result;
}

export function parsePriceImport(raw: string): PriceBook {
  if (new TextEncoder().encode(raw).byteLength > maxPriceImportBytes) throw new Error("单价配置文件不能超过 1 MiB");
  let value: unknown;
  try { value = JSON.parse(raw.replace(/^\uFEFF/, "")); } catch { throw new Error("单价配置不是有效 JSON"); }
  if (!object(value) || value.version !== 1 || !Object.hasOwn(value, "prices") || Object.keys(value).some(key => key !== "version" && key !== "prices")) throw new Error("配置必须仅包含 version: 1 和 prices");
  return validatePrices(value.prices);
}
export function previewPriceImport(current: PriceBook, incoming: PriceBook) {
  mergePriceImport(current, incoming);
  const clean = validatePrices(incoming);
  let addedModels = 0, addedFields = 0, overwrittenFields = 0, unchangedFields = 0;
  for (const [key, rates] of Object.entries(clean)) {
    if (!Object.hasOwn(current, key)) addedModels++;
    for (const field of rateFields) if (Object.hasOwn(rates, field)) {
      if (current[key]?.[field] === undefined) addedFields++;
      else if (current[key][field] === rates[field]) unchangedFields++;
      else overwrittenFields++;
    }
  }
  return { addedModels, addedFields, overwrittenFields, unchangedFields };
}
export function mergePriceImport(current: PriceBook, incoming: PriceBook): PriceBook {
  const clean = validatePrices(incoming);
  const result = { ...current };
  for (const [key, rates] of Object.entries(clean)) result[key] = { ...current[key], ...rates };
  serializePriceBook(result);
  return result;
}
export function serializePriceBook(prices: PriceBook): string {
  const text = JSON.stringify({ version: 1, prices: validatePrices(prices) }, null, 2) + "\n";
  if (new TextEncoder().encode(text).byteLength > maxPriceImportBytes) throw new Error("单价配置不能超过 1 MiB");
  return text;
}
