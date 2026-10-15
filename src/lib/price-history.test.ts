import assert from "node:assert/strict";
import test from "node:test";
import { commitPriceChange, maxPriceHistoryBytes, maxPriceHistoryEntries, priceHistoryStorageKey, readPriceHistory, restorePriceChange, type PriceHistoryStorage } from "./price-history";
import { priceKey, priceStorageKey } from "./session-usage";
const key = priceKey("codex", "m");
const book = (input: number) => ({ [key]: { input } });
const encode = (input: number) => JSON.stringify({ version: 1, prices: book(input) });
class Memory implements PriceHistoryStorage {
  values = new Map<string, string>();
  failKey: string | null = null;
  writes: string[] = [];
  getItem(key: string) { return this.values.get(key) ?? null; }
  setItem(key: string, value: string) { if (key === this.failKey) throw new Error("quota exceeded"); this.writes.push(key); this.values.set(key, value); }
  removeItem(key: string) { if (key === this.failKey) throw new Error("remove failed"); this.writes.push(key); this.values.delete(key); }
}
test("history records edits and imports, survives reload and restores exact pre-change snapshot", () => {
  const storage = new Memory();
  const legacy = '{ "version": 1, "prices": {"legacy-model": {"input":0}} }';
  storage.values.set(priceStorageKey, legacy);
  const first = commitPriceChange(storage, legacy, book(1), "edit");
  const second = commitPriceChange(storage, first.raw, book(2), "import");
  assert.deepEqual(readPriceHistory(storage), second);
  assert.deepEqual(second.history.map(item => item.reason), ["import", "edit"]);
  assert.deepEqual(second.history[0].before, book(1));
  const restored = restorePriceChange(storage, second.raw, first.history[0].id);
  assert.equal(restored.raw, legacy);
  assert.equal(storage.getItem(priceStorageKey), legacy);
  assert.equal(restored.history[0].reason, "restore");
  assert.deepEqual(restored.history[0].before, book(2));
  assert.equal(restored.history.length, 3);
});
test("restoring initial absence removes configuration and can itself be undone", () => {
  const storage = new Memory();
  const first = commitPriceChange(storage, null, book(0), "edit");
  const undone = restorePriceChange(storage, first.raw, first.history[0].id);
  assert.equal(undone.raw, null); assert.equal(storage.getItem(priceStorageKey), null);
  const redone = restorePriceChange(storage, null, undone.history[0].id);
  assert.deepEqual(redone.prices, book(0));
});
test("journal failure blocks active price write; price write failure resolves aborted pending", () => {
  const storage = new Memory(); storage.values.set(priceStorageKey, encode(1));
  storage.failKey = priceHistoryStorageKey;
  assert.throws(() => commitPriceChange(storage, encode(1), book(2), "edit"), /quota/);
  assert.equal(storage.getItem(priceStorageKey), encode(1)); assert.equal(storage.getItem(priceHistoryStorageKey), null);
  storage.failKey = priceStorageKey;
  assert.throws(() => commitPriceChange(storage, encode(1), book(2), "import"), /quota/);
  assert.equal(readPriceHistory(storage).history.length, 0);
  storage.failKey = null;
  const retry = commitPriceChange(storage, encode(1), book(3), "edit");
  assert.equal(retry.history.length, 1); assert.deepEqual(retry.history[0].before, book(1));
  assert.deepEqual(storage.writes.slice(-2), [priceHistoryStorageKey, priceStorageKey]);
});
test("failed restore removal leaves prior configuration and history intact", () => {
  const storage = new Memory(); const saved = commitPriceChange(storage, null, book(1), "edit");
  storage.failKey = priceStorageKey;
  assert.throws(() => restorePriceChange(storage, saved.raw, saved.history[0].id), /remove failed/);
  assert.deepEqual(readPriceHistory(storage), saved);
});
test("stale writers and ambiguous pending states fail without further writes", () => {
  const storage = new Memory(); const saved = commitPriceChange(storage, null, book(1), "edit");
  const writes = storage.writes.length;
  assert.throws(() => commitPriceChange(storage, null, book(2), "edit"), /其他窗口/);
  assert.equal(storage.writes.length, writes);
  storage.values.set(priceStorageKey, encode(99));
  assert.throws(() => readPriceHistory(storage), /不一致/);
  assert.throws(() => commitPriceChange(storage, encode(99), book(2), "edit"), /不一致/);
  assert.equal(storage.writes.length, writes);
  storage.values.set(priceStorageKey, saved.raw!);
  assert.throws(() => restorePriceChange(storage, saved.raw, "missing"), /已不存在/);
});
test("malformed history and configuration are never cleared or overwritten", () => {
  for (const bad of ["{", '{"version":2,"entries":[],"pending":null}', '{"version":1,"entries":[],"pending":{}}', '{"version":1,"entries":[],"pending":null,"extra":true}']) {
    const storage = new Memory(); storage.values.set(priceHistoryStorageKey, bad);
    assert.throws(() => commitPriceChange(storage, null, book(1), "edit"), /损坏/);
    assert.equal(storage.getItem(priceHistoryStorageKey), bad); assert.equal(storage.writes.length, 0);
  }
  for (const bad of ["{", '{"version":1,"prices":[]}', '{"version":1,"prices":{"m":{"input":-1}}}']) {
    const storage = new Memory(); storage.values.set(priceStorageKey, bad);
    assert.throws(() => readPriceHistory(storage), /损坏/); assert.equal(storage.getItem(priceStorageKey), bad);
  }
});
test("history evicts oldest entries at twenty and no-op save does not add a record", () => {
  const storage = new Memory(); let current = readPriceHistory(storage);
  for (let n = 0; n < 25; n++) current = commitPriceChange(storage, current.raw, book(n), "edit");
  assert.equal(current.history.length, maxPriceHistoryEntries);
  assert.deepEqual(current.history.at(-1)?.before, book(4));
  const writes = storage.writes.length;
  assert.deepEqual(commitPriceChange(storage, current.raw, book(24), "edit"), current);
  assert.equal(storage.writes.length, writes);
});
test("byte limit trims older records but refuses an oversized latest snapshot", () => {
  const storage = new Memory(); const largeKey = priceKey("claude", "中".repeat(50000));
  let current = readPriceHistory(storage);
  for (let n = 0; n < 12; n++) current = commitPriceChange(storage, current.raw, { [largeKey]: { input: n } }, "edit");
  assert.ok(current.history.length < 12); assert.ok(current.history.length > 1);
  assert.ok(new TextEncoder().encode(storage.getItem(priceHistoryStorageKey)!).byteLength <= maxPriceHistoryBytes);
  const snapshot = [...storage.values];
  assert.throws(() => commitPriceChange(storage, current.raw, { ["中".repeat(maxPriceHistoryBytes)]: { input: 1 } }, "edit"), /超过 2 MiB/);
  assert.deepEqual([...storage.values], snapshot);
});
test("read is side-effect free and duplicate record identifiers are rejected", () => {
  const storage = new Memory(); const saved = commitPriceChange(storage, null, book(1), "edit");
  const count = storage.writes.length; assert.deepEqual(readPriceHistory(storage), saved); assert.equal(storage.writes.length, count);
  const journal = JSON.parse(storage.getItem(priceHistoryStorageKey)!); journal.entries.push(journal.pending);
  storage.values.set(priceHistoryStorageKey, JSON.stringify(journal));
  assert.throws(() => readPriceHistory(storage), /损坏/);
});
