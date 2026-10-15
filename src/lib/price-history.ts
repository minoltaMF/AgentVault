import { priceStorageKey, rateFields, readPriceBook, type PriceBook } from "./session-usage";

export const priceHistoryStorageKey = "agentvault.session-usage-price-history.v1";
export const maxPriceHistoryEntries = 20;
export const maxPriceHistoryBytes = 2 * 1024 * 1024;
export type PriceChangeReason = "edit" | "import" | "restore";
export type PriceHistoryEntry = { id: string; time: string; reason: PriceChangeReason; beforeRaw: string | null; afterRaw: string | null; before: PriceBook; after: PriceBook };
type StoredEntry = Omit<PriceHistoryEntry, "before" | "after">;
type Journal = { version: 1; entries: StoredEntry[]; pending: StoredEntry | null };
export type PriceHistoryStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
export type PriceHistoryState = { raw: string | null; prices: PriceBook; history: PriceHistoryEntry[] };
const bytes = (raw: string) => new TextEncoder().encode(raw).byteLength;
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const damaged = () => new Error("单价配置或变更历史损坏，已拒绝修改并保留原数据");

// Preserve legacy model keys and exact raw snapshots; never silently sanitize data on restore.
function pricesFromRaw(raw: string | null): PriceBook {
  if (raw === null) return {};
  let data: unknown;
  try { data = JSON.parse(raw); } catch { throw damaged(); }
  if (!object(data) || data.version !== 1 || !object(data.prices) || Object.keys(data).some(key => key !== "version" && key !== "prices")) throw damaged();
  for (const rates of Object.values(data.prices)) {
    if (!object(rates)) throw damaged();
    for (const [field, rate] of Object.entries(rates)) {
      if (!(rateFields as readonly string[]).includes(field) || typeof rate !== "number" || !Number.isFinite(rate) || rate < 0) throw damaged();
    }
  }
  return readPriceBook(raw);
}
function storedEntry(value: unknown): StoredEntry {
  if (!object(value) || Object.keys(value).sort().join() !== "afterRaw,beforeRaw,id,reason,time" || typeof value.id !== "string" || !value.id || typeof value.time !== "string" || !Number.isFinite(Date.parse(value.time)) || !["edit", "import", "restore"].includes(value.reason as string) || !(value.beforeRaw === null || typeof value.beforeRaw === "string") || !(value.afterRaw === null || typeof value.afterRaw === "string") || value.beforeRaw === value.afterRaw) throw damaged();
  pricesFromRaw(value.beforeRaw); pricesFromRaw(value.afterRaw);
  return value as StoredEntry;
}
function load(storage: PriceHistoryStorage): { raw: string | null; entries: StoredEntry[] } {
  const raw = storage.getItem(priceStorageKey);
  pricesFromRaw(raw);
  const encoded = storage.getItem(priceHistoryStorageKey);
  if (encoded === null) return { raw, entries: [] };
  if (bytes(encoded) > maxPriceHistoryBytes) throw damaged();
  let data: unknown;
  try { data = JSON.parse(encoded); } catch { throw damaged(); }
  if (!object(data) || Object.keys(data).sort().join() !== "entries,pending,version" || data.version !== 1 || !Array.isArray(data.entries) || data.entries.length > maxPriceHistoryEntries) throw damaged();
  const entries = data.entries.map(storedEntry);
  const pending = data.pending === null ? null : storedEntry(data.pending);
  const ids = [...entries, ...(pending ? [pending] : [])].map(entry => entry.id);
  if (new Set(ids).size !== ids.length || ids.length > maxPriceHistoryEntries) throw damaged();
  if (pending) {
    if (raw === pending.afterRaw) entries.unshift(pending);
    else if (raw !== pending.beforeRaw) throw new Error("单价与未完成的变更记录不一致，已拒绝修改；请保留配置与历史数据排查");
  }
  return { raw, entries };
}
function state(raw: string | null, entries: StoredEntry[]): PriceHistoryState {
  return { raw, prices: pricesFromRaw(raw), history: entries.map(entry => ({ ...entry, before: pricesFromRaw(entry.beforeRaw), after: pricesFromRaw(entry.afterRaw) })) };
}
/** Read only. Call together with writes under the same cross-window Web Lock. */
export function readPriceHistory(storage: PriceHistoryStorage): PriceHistoryState {
  const current = load(storage);
  return state(current.raw, current.entries);
}
function change(storage: PriceHistoryStorage, expectedRaw: string | null, nextRaw: string | null, reason: PriceChangeReason): PriceHistoryState {
  const current = load(storage);
  if (current.raw !== expectedRaw) throw new Error("单价配置已在其他窗口修改，请重新载入后再编辑");
  pricesFromRaw(nextRaw);
  if (current.raw === nextRaw) return state(current.raw, current.entries);
  const pending: StoredEntry = { id: crypto.randomUUID(), time: new Date().toISOString(), reason, beforeRaw: current.raw, afterRaw: nextRaw };
  const journal: Journal = { version: 1, entries: current.entries.slice(0, maxPriceHistoryEntries - 1), pending };
  let encoded = JSON.stringify(journal);
  while (bytes(encoded) > maxPriceHistoryBytes && journal.entries.length) { journal.entries.pop(); encoded = JSON.stringify(journal); }
  if (bytes(encoded) > maxPriceHistoryBytes) throw new Error("本次变更快照超过 2 MiB，无法安全保存历史，已拒绝修改");
  // Write-ahead journal: if this fails, the active configuration remains untouched.
  // The pending entry is resolved from the exact active raw value on every subsequent read.
  storage.setItem(priceHistoryStorageKey, encoded);
  if (nextRaw === null) storage.removeItem(priceStorageKey); else storage.setItem(priceStorageKey, nextRaw);
  return state(nextRaw, [pending, ...journal.entries]);
}
/** Caller must serialize writes using the application's cross-window Web Lock. */
export function commitPriceChange(storage: PriceHistoryStorage, expectedRaw: string | null, next: PriceBook, reason: "edit" | "import"): PriceHistoryState {
  return change(storage, expectedRaw, JSON.stringify({ version: 1, prices: next }), reason);
}
export function restorePriceChange(storage: PriceHistoryStorage, expectedRaw: string | null, id: string): PriceHistoryState {
  const current = load(storage);
  const entry = current.entries.find(item => item.id === id);
  if (!entry) throw new Error("该历史记录已不存在，请重新载入历史");
  return change(storage, expectedRaw, entry.beforeRaw, "restore");
}
