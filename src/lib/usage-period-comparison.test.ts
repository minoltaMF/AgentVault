import assert from "node:assert/strict";
import test from "node:test";
import { compareUsagePeriods } from "./usage-period-comparison";
import { priceKey, type UsageSession, type UsageTokens } from "./session-usage";

const now = new Date("2026-10-12T12:00:00Z");
const prices = { [priceKey("codex", "m")]: { input: 1, output: 1, cache_read: 1, cache_write_5m: 1, cache_write_1h: 1 } };
const model = (input: number, extra: Partial<UsageTokens> = {}) => ({ model: "m", tokens: { input, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, reasoning: 0, ...extra } });
function session(date: string, input: number, path = date): UsageSession {
  const models = [model(input)];
  return { provider: "codex", rollout_path: path, id: path, title: path, cwd: "/project", models, warnings: [], daily: [{ date, models }], undated_models: [] };
}
const pair = () => [session("2026-10-12", 200), session("2026-10-05", 100)];

test("adjacent UTC periods include today and have exact 7 or 30-day inclusive boundaries", () => {
  for (const days of [7, 30] as const) {
    for (const date of [new Date("2026-01-02T00:30:00+08:00"), new Date("2024-03-01T23:59:59Z")]) {
      const result = compareUsagePeriods([], {}, days, date, true);
      const stamp = (value: string) => Date.parse(value + "T00:00:00Z");
      assert.equal(stamp(result.current.range.to) - stamp(result.current.range.from), (days - 1) * 86400000);
      assert.equal(stamp(result.previous.range.to) - stamp(result.previous.range.from), (days - 1) * 86400000);
      assert.equal(stamp(result.current.range.from) - stamp(result.previous.range.to), 86400000);
      assert.equal(result.current.range.to, date.toISOString().slice(0, 10));
    }
  }
  const leap = compareUsagePeriods([], {}, 7, new Date("2024-03-01Z"), true);
  assert.deepEqual(leap.current.range, { from: "2024-02-24", to: "2024-03-01" });
  assert.deepEqual(leap.previous.range, { from: "2024-02-17", to: "2024-02-23" });
});

test("period totals honor all inclusive endpoints and exclude outside dates", () => {
  const rows = ["2026-09-28", "2026-09-29", "2026-10-05", "2026-10-06", "2026-10-12", "2026-10-13"].map(date => session(date, 100));
  const result = compareUsagePeriods(rows, prices, 7, now, true);
  assert.equal(result.current.tokens, 200); assert.equal(result.previous.tokens, 200);
  assert.equal(result.current.recordedDays, 2); assert.equal(result.previous.sessionCount, 2);
  assert.deepEqual(result.tokensChange, { delta: 0, percent: 0, reason: null });
  const month = compareUsagePeriods(["2026-08-13", "2026-08-14", "2026-09-12", "2026-09-13", "2026-10-12", "2026-10-13"].map(date => session(date, 100)), prices, 30, now, true);
  assert.equal(month.current.tokens, 200); assert.equal(month.previous.tokens, 200);
  assert.equal(month.current.recordedDays, 2); assert.equal(month.previous.recordedDays, 2);
});

test("deduplicates sessions and isolates provider rates without counting token subsets twice", () => {
  const rows = pair();
  rows[0].models = [model(200, { output: 40, reasoning: 30, cache_write: 50, cache_write_1h: 20 })];
  rows[0].daily![0].models = rows[0].models;
  const claude = { ...session("2026-10-12", 100), provider: "claude" };
  const result = compareUsagePeriods([...rows, rows[0], claude], { ...prices, [priceKey("claude", "m")]: { input: 2 } }, 7, now, true);
  assert.equal(result.current.tokens, 390); assert.equal(result.current.sessionCount, 2);
  assert.equal(result.current.recordedDays, 1);
  assert.ok(Math.abs(result.current.known - 490 / 1e6) < 1e-12);
  assert.equal(result.tokensChange.delta, 290); assert.equal(result.tokensChange.percent, 290);
});

test("missing periods are not zero while explicitly recorded zeros compare correctly", () => {
  const empty = compareUsagePeriods([session("2026-10-12", 100)], prices, 7, now, true);
  assert.equal(empty.previous.hasUsage, false); assert.equal(empty.tokensChange.delta, null);
  const zeros = compareUsagePeriods([session("2026-10-12", 0), session("2026-10-05", 0)], prices, 7, now, true);
  assert.equal(zeros.previous.hasUsage, true);
  assert.deepEqual(zeros.tokensChange, { delta: 0, percent: 0, reason: null });
  const baselineZero = compareUsagePeriods([session("2026-10-12", 100), session("2026-10-05", 0)], prices, 7, now, true);
  assert.equal(baselineZero.tokensChange.delta, 100); assert.equal(baselineZero.tokensChange.percent, null);
  const dropped = compareUsagePeriods([session("2026-10-12", 0), session("2026-10-05", 100)], prices, 7, now, true);
  assert.equal(dropped.tokensChange.percent, -100);
});

test("scope-wide warnings, undated and missing usage block misleading decreases", () => {
  const variants: UsageSession[] = [
    { ...session("2020-01-01", 10), warnings: ["partial parse"] },
    { ...session("2020-01-01", 10), daily: undefined },
    { ...session("2020-01-01", 10), undated_models: [model(5)] },
    { ...session("2020-01-01", 10), models: [], daily: [] },
    { ...session("2020-01-01", 10), daily: [] },
    session("2026-02-30", 10),
    { ...session("2020-01-01", 10), daily: [{ date: "2020-01-01", models: [model(5)] }, { date: "", models: [model(5)] }] },
  ];
  for (const extra of variants) {
    const result = compareUsagePeriods([...pair(), extra], prices, 7, now, true);
    assert.equal(result.tokensChange.delta, null); assert.equal(result.costChange.delta, null);
    assert.ok(result.tokensChange.reason);
  }
  const unfinished = compareUsagePeriods(pair(), prices, 7, now, false);
  assert.equal(unfinished.tokensChange.percent, null); assert.equal(unfinished.costChange.delta, null);
});

test("unknown rates allow known cost delta but never cost percentage, with independent token change", () => {
  const result = compareUsagePeriods(pair(), {}, 7, now, true);
  assert.deepEqual(result.tokensChange, { delta: 100, percent: 100, reason: null });
  assert.equal(result.costChange.delta, 0); assert.equal(result.costChange.percent, null);
  assert.match(result.costChange.reason!, /仅比较已知费用差额/);
  const free = compareUsagePeriods(pair(), { [priceKey("codex", "m")]: { input: 0 } }, 7, now, true);
  assert.deepEqual(free.costChange, { delta: 0, percent: 0, reason: null });
});

test("token totals, malformed subset counts and extreme costs cannot masquerade as valid deltas", () => {
  for (const tokens of [model(Number.MAX_SAFE_INTEGER, { output: 1 }), model(10, { reasoning: 1 }), model(10, { cache_write_1h: 1 })]) {
    const rows = pair(); rows[0].models = [tokens]; rows[0].daily![0].models = [tokens];
    const result = compareUsagePeriods(rows, prices, 7, now, true);
    assert.equal(result.current.overflow, true); assert.equal(result.tokensChange.delta, null); assert.equal(result.costChange.delta, null);
  }
  const expensive = compareUsagePeriods(pair(), { [priceKey("codex", "m")]: { input: Number.MAX_VALUE } }, 7, now, true);
  assert.equal(expensive.current.overflow, true); assert.equal(expensive.costChange.delta, null);
});

test("unsafe percentages are omitted without losing a finite difference", () => {
  const result = compareUsagePeriods([session("2026-10-12", Number.MAX_SAFE_INTEGER - 1), session("2026-10-05", 1)], prices, 7, now, true);
  assert.equal(result.tokensChange.delta, Number.MAX_SAFE_INTEGER - 2);
  assert.equal(result.tokensChange.percent, null); assert.match(result.tokensChange.reason!, /安全计算范围/);
});

test("comparison does not mutate inputs and validates unsupported dates or intervals", () => {
  const rows = pair(), original = structuredClone(rows), originalPrices = structuredClone(prices), stamp = now.getTime();
  compareUsagePeriods(rows, prices, 7, now, true);
  assert.deepEqual(rows, original); assert.deepEqual(prices, originalPrices); assert.equal(now.getTime(), stamp);
  assert.throws(() => compareUsagePeriods(rows, prices, 7, new Date("invalid"), true));
  assert.throws(() => compareUsagePeriods(rows, prices, 8 as 7, now, true));
});
