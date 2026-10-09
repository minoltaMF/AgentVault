import assert from "node:assert/strict";
import test from "node:test";
import { aggregateUsage, estimateCost, parseRate, priceKey, readPriceBook, summarizeUsage, type UsageSession } from "./session-usage";
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
