import assert from "node:assert/strict";
import test from "node:test";
import { filterUsageByDate, priceKey, type UsageModel, type UsageSession } from "./session-usage";
import { rankUsage } from "./usage-ranking";

const model = (name: string, input: number): UsageModel => ({ model: name, tokens: { input, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, reasoning: 0 } });
const session = (id: string, models = [model(id, 100)], cwd = id): UsageSession => ({ id, rollout_path: id, title: id, cwd, provider: "codex", models, warnings: [] });

test("ranks the selected date range rather than lifetime totals", () => {
  const a = session("a", [model("a", 1000)]);
  a.daily = [{ date: "2026-10-01", models: [model("a", 990)] }, { date: "2026-10-02", models: [model("a", 10)] }];
  const b = session("b", [model("b", 50)]);
  b.daily = [{ date: "2026-10-02", models: b.models }];
  const rows = rankUsage(filterUsageByDate([a, b], { from: "2026-10-02", to: "2026-10-02" }), {}, "model", "tokens");
  assert.deepEqual(rows.map(row => row.totalTokens), [50, 10]);
  assert.equal(rows[0].label, priceKey("codex", "b"));
});

test("deduplicates sessions and isolates model usage including dated detail", () => {
  const a = session("a", [model("m", 100), model("n", 500)]);
  a.daily = [{ date: "2026-10-02", models: a.models }]; a.undated_models = a.models;
  const rows = rankUsage([a, a], {}, "model", "tokens");
  assert.deepEqual(rows.map(row => row.totalTokens), [500, 100]);
  assert.equal(rows[0].sessions.length, 1);
  assert.deepEqual(rows[0].sessions[0].models.map(item => item.model), ["n"]);
  assert.deepEqual(rows[0].sessions[0].daily![0].models.map(item => item.model), ["n"]);
  assert.deepEqual(rows[0].sessions[0].undated_models!.map(item => item.model), ["n"]);
  const claude = { ...a, provider: "claude" };
  assert.equal(rankUsage([a, claude], {}, "model", "tokens").length, 4);
});

test("ranks known cost while preserving unknown versus explicitly free cost", () => {
  const partial = session("partial", [model("partial", 1_000_000)]);
  partial.models[0].tokens.output = 100;
  const prices = { [priceKey("codex", "partial")]: { input: 5 }, [priceKey("codex", "free")]: { input: 0 } };
  const rows = rankUsage([session("unknown"), session("free"), partial], prices, "model", "cost");
  assert.equal(rows[0].summary.known, 5); assert.equal(rows[0].summary.unknownModels, 1);
  assert.equal(rows.find(row => row.label === priceKey("codex", "free"))!.summary.unknownModels, 0);
  assert.equal(rows.find(row => row.label === priceKey("codex", "unknown"))!.summary.unknownModels, 1);
});

test("separates missing project from a real path equal to its display label", () => {
  const rows = rankUsage([session("a", [model("m", 1)], ""), session("b", [model("m", 2)], "未记录项目路径")], {}, "project", "tokens");
  assert.equal(rows.length, 2); assert.notEqual(rows[0].key, rows[1].key);
  assert.deepEqual(rows.map(row => row.totalTokens), [2, 1]);
  assert.equal(rows[1].sessions[0].cwd, "");
});

test("missing usage and unsafe totals sort last without becoming zero", () => {
  const huge = session("huge", [model("m", Number.MAX_SAFE_INTEGER)]);
  huge.models[0].tokens.output = 1;
  const invalid = session("invalid", [model("m", -1)]);
  const missing = session("missing", []);
  const valid = session("valid", [model("m", 0)]);
  for (const sort of ["tokens", "cost"] as const) {
    const rows = rankUsage([missing, huge, invalid, valid], {}, "project", sort);
    assert.equal(rows[0].totalTokens, 0);
    assert.equal(rows[0].sessions[0].id, "valid");
    assert.ok(rows.slice(1).every(row => row.totalTokens === null));
    assert.equal(rows.find(row => row.sessions[0].id === "missing")!.summary.missingUsage, 1);
  }
  assert.equal(rankUsage([missing], {}, "model", "tokens").length, 0);
});

test("does not double count subset token fields", () => {
  const a = session("a", [model("m", 10)]);
  Object.assign(a.models[0].tokens, { output: 20, reasoning: 15, cache_read: 30, cache_write: 40, cache_write_1h: 25 });
  assert.equal(rankUsage([a], {}, "project", "tokens")[0].totalTokens, 100);
});

test("ties have stable keys and input objects are unchanged", () => {
  const inputs = [session("z"), session("a")];
  const prices = { [priceKey("codex", "z")]: { input: 1 } };
  const before = JSON.stringify({ inputs, prices });
  for (const group of ["model", "project"] as const) {
    for (const sort of ["tokens", "cost"] as const) {
      assert.deepEqual(rankUsage(inputs, prices, group, sort), rankUsage([...inputs].reverse(), prices, group, sort));
    }
  }
  assert.equal(JSON.stringify({ inputs, prices }), before);
});
