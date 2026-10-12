import assert from "node:assert/strict";
import test from "node:test";
import { aggregateUsage, dailyUsageTrend, estimateCost, filterUsageByDate, parseRate, priceKey, readPriceBook, recentUsageRange, summarizeUsage, undatedUsageSessions, validateUsageDateRange, type UsageSession } from "./session-usage";
const tokens = { input: 100, output: 20, cache_read: 50, cache_write: 40, cache_write_1h: 10, reasoning: 15 };
const session: UsageSession = { provider: "codex", rollout_path: "/a", id: "a", title: "a", cwd: "/project", models: [{ model: "m", tokens }], warnings: [] };
test("unknown prices are distinct from legitimate zero prices", () => {
  assert.equal(estimateCost(tokens).unknown.length, 5);
  assert.equal(estimateCost(tokens, { input: 0 }).unknown.length, 4);
  assert.equal(parseRate(""), undefined); assert.equal(parseRate("0"), 0);
  for (const invalid of ["-1", "NaN", "Infinity", "1e999", "0x10"]) assert.throws(() => parseRate(invalid));
});
test("cost categories do not double count reasoning or one-hour cache writes", () => {
  const result = estimateCost(tokens, { input: 1, output: 2, cache_read: 3, cache_write_5m: 4, cache_write_1h: 5 });
  assert.deepEqual(result.unknown, []); assert.ok(Math.abs(result.known - (100 + 40 + 150 + 120 + 50) / 1e6) < 1e-12);
});
test("aggregation deduplicates physical sessions and isolates provider price identities", () => {
  const prices = { [priceKey("codex", "m")]: { input: 1 } };
  assert.equal(summarizeUsage([session, session], prices).tokens.input, 100);
  assert.equal(aggregateUsage([session, session], "project")[0].sessions.length, 1);
  assert.equal(aggregateUsage([session, { ...session, provider: "claude" }], "model").length, 2);
  assert.equal(summarizeUsage([{ ...session, models: [] }], prices).missingUsage, 1);
});
test("stored prices require supported version and reject invalid rates", () => {
  assert.deepEqual(readPriceBook('{"version":2,"prices":{"a":{"input":1}}}'), {});
  assert.deepEqual(readPriceBook('{"version":1,"prices":{"a":{"input":0,"output":-1,"cache_read":"1"}}}'), { a: { input: 0 } });
  assert.deepEqual(readPriceBook("bad"), {});
});


test("unsafe arithmetic and parser warnings never imply a complete estimate", () => {
  assert.equal(estimateCost({ ...tokens, input: Number.MAX_SAFE_INTEGER + 1 }).invalid, true);
  assert.ok(estimateCost(tokens, { input: Number.MAX_VALUE }).unknown.includes("input"));
  assert.equal(summarizeUsage([{ ...session, warnings: ["ambiguous"] }], {}).warnedSessions, 1);
  assert.equal(summarizeUsage([{ ...session, models: [{ model: "m", tokens: { ...tokens, input: Number.MAX_SAFE_INTEGER + 1 } }] }], {}).overflow, true);
});

const dated: UsageSession = { ...session, daily: [
  { date: "2026-10-10", models: session.models },
  { date: "2026-10-11", models: session.models },
], models: [{ model: "m", tokens: { ...tokens, input: 300 } }], undated_models: session.models };
test("date ranges validate calendar dates and UTC inclusive recent boundaries", () => {
  for (const value of ["", "2024-02-29", "2026-10-12"]) assert.equal(validateUsageDateRange({ from: value, to: "" }), "");
  for (const value of ["2025-02-29", "2026-04-31", "2026-13-01", "2026-1-01", "2026-10-12T00:00:00Z"])
    assert.notEqual(validateUsageDateRange({ from: value, to: "" }), "");
  assert.notEqual(validateUsageDateRange({ from: "2026-10-12", to: "2026-10-11" }), "");
  const now = new Date("2026-10-12T00:30:00+08:00");
  assert.deepEqual(recentUsageRange(7, now), { from: "2026-10-05", to: "2026-10-11" });
  assert.deepEqual(recentUsageRange(30, now), { from: "2026-09-12", to: "2026-10-11" });
  assert.deepEqual(recentUsageRange(7, new Date("2024-03-01T23:59:59Z")), { from: "2024-02-24", to: "2024-03-01" });
  assert.throws(() => recentUsageRange(0, now));
});
test("date filtering recomputes matching model totals without including undated usage", () => {
  const range = { from: "2026-10-10", to: "2026-10-11" };
  const filtered = filterUsageByDate([dated, dated, session], range);
  assert.equal(filtered.length, 0); // The last duplicate is the authoritative session snapshot.
  const both = filterUsageByDate([dated, dated], range);
  assert.equal(both.length, 1);
  assert.equal(both[0].models.length, 1);
  assert.equal(both[0].models[0].tokens.input, 200);
  assert.equal(filterUsageByDate([dated], { from: "2026-10-11", to: "" })[0].models[0].tokens.input, 100);
  assert.deepEqual(filterUsageByDate([dated], { from: "bad", to: "" }), []);
  assert.deepEqual(filterUsageByDate([dated], { from: "", to: "" }), [dated]);
  assert.equal(dated.daily![0].models[0].tokens.input, 100);
  assert.equal(undatedUsageSessions([dated, dated])[0].models[0].tokens.input, 100);
  assert.equal(undatedUsageSessions([session])[0].models[0].tokens.input, 100);
  assert.deepEqual(undatedUsageSessions([{ ...dated, undated_models: [] }]), []);
});
test("daily trend isolates provider prices, deduplicates paths and sorts populated dates", () => {
  const rates = { input: 1, output: 1, cache_read: 1, cache_write_5m: 1, cache_write_1h: 1 };
  const prices = { [priceKey("codex", "m")]: rates, [priceKey("claude", "m")]: { ...rates, input: 2 } };
  const claude = { ...dated, provider: "claude", daily: [{ date: "2026-10-11", models: session.models }] };
  const result = dailyUsageTrend([dated, claude, dated], prices);
  assert.deepEqual(result.map(row => row.date), ["2026-10-10", "2026-10-11"]);
  assert.equal(result[0].sessions, 1);
  assert.equal(result[1].sessions, 2);
  assert.equal(result[1].tokens, 420); // Reasoning and the one-hour cache are subsets.
  assert.ok(Math.abs(result[1].known - 520 / 1e6) < 1e-12);
  assert.equal(result[1].incomplete, false);
  assert.equal(dailyUsageTrend([dated], prices, { from: "", to: "2026-10-10" }).length, 1);
  assert.deepEqual(dailyUsageTrend([session], prices), []);
  assert.deepEqual(dailyUsageTrend([dated], prices, { from: "2026-02-30", to: "" }), []);
});
test("daily trend keeps partial estimates and arithmetic overflow explicit", () => {
  assert.equal(dailyUsageTrend([dated], {})[0].incomplete, true);
  assert.equal(dailyUsageTrend([dated], {})[0].unknownModels, 1);
  const huge = { ...dated, daily: [{ date: "2026-10-10", models: [{ model: "m", tokens: { ...tokens, input: Number.MAX_SAFE_INTEGER } }] }] };
  assert.equal(dailyUsageTrend([huge], {})[0].overflow, true);
  assert.equal(dailyUsageTrend([{ ...dated, warnings: ["partial"] }], {})[0].warnedSessions, 1);
  const duplicateDays = { ...dated, daily: [dated.daily![0], dated.daily![0]] };
  const result = dailyUsageTrend([duplicateDays], {});
  assert.equal(result.length, 1);
  assert.equal(result[0].sessions, 1);
  assert.equal(result[0].tokens, 420);
});
