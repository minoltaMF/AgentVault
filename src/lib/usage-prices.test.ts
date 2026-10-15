import assert from "node:assert/strict";
import test from "node:test";
import { applyPriceDrafts, maxPriceImportBytes, mergePriceImport, parsePriceImport, previewPriceImport, serializePriceBook, usagePriceCatalog } from "./usage-prices";
import { priceKey, readPriceBook, type UsageSession } from "./session-usage";

const key = priceKey("codex", "m");
const other = priceKey("claude", "m");
const encode = (prices: unknown) => JSON.stringify({ version: 1, prices });
const session = (provider = "codex", path = "a"): UsageSession => ({ provider, rollout_path: path, id: path, title: path, cwd: "", warnings: [], models: [{ model: "m", tokens: { input: 100, output: 50, cache_read: 0, cache_write: 10, cache_write_1h: 10, reasoning: 50 } }] });

test("catalog unions saved models, deduplicates sessions and isolates provider pricing", () => {
  const saved = priceKey("claude", "saved");
  const rows = usagePriceCatalog([session(), session(), session("claude")], { [key]: { input: 0 }, [saved]: { output: 2 } });
  assert.equal(rows.length, 3);
  const codex = rows.find(row => row.key === key)!;
  assert.equal(codex.sessionCount, 1);
  assert.deepEqual(codex.usedFields, ["input", "output", "cache_write_1h"]);
  assert.deepEqual(codex.missingFields, ["output", "cache_write_1h"]);
  assert.deepEqual(rows.find(row => row.key === other)!.missingFields, ["input", "output", "cache_write_1h"]);
  assert.equal(rows.at(-1)!.key, saved);
  assert.equal(rows.at(-1)!.sessionCount, 0);
});

test("catalog does not invent missing prices for zero or invalid usage", () => {
  const zero = session(); Object.keys(zero.models[0].tokens).forEach(field => { zero.models[0].tokens[field as keyof typeof zero.models[0]['tokens']] = 0; });
  assert.deepEqual(usagePriceCatalog([zero], {})[0].missingFields, []);
  const invalid = session(); invalid.models[0].tokens.input = -1;
  const row = usagePriceCatalog([invalid], {})[0];
  assert.equal(row.invalidUsage, true); assert.deepEqual(row.missingFields, []);
  assert.deepEqual(usagePriceCatalog([session("unsupported")], { invalid: {} }), []);
});

test("batch updates only supplied fields, empty clears, zero is free, and no mutation occurs", () => {
  const prices = { [key]: { input: 1, output: 2 }, [other]: { input: 3 } };
  const before = JSON.stringify(prices);
  const updated = applyPriceDrafts(prices, { [key]: { input: "0", output: " " } });
  assert.deepEqual(updated[key], { input: 0 }); assert.deepEqual(updated[other], { input: 3 });
  assert.equal(JSON.stringify(prices), before);
  assert.throws(() => applyPriceDrafts(prices, { [key]: { input: "5" }, [other]: { output: "-1" } }));
  assert.equal(JSON.stringify(prices), before);
  for (const bad of ["NaN", "Infinity", "1e3", "0x10", ".", "-0", "1,2"]) assert.throws(() => applyPriceDrafts(prices, { [key]: { input: bad } }));
});

test("import and export round-trip localStorage v1 including zero and unicode", () => {
  const prices = { [key]: { input: 0, output: 1.25 }, [priceKey("claude", "中文模型")]: {} };
  const text = serializePriceBook(prices);
  assert.deepEqual(parsePriceImport(text), prices);
  assert.deepEqual(parsePriceImport("\uFEFF" + text), prices);
  assert.deepEqual(readPriceBook(text), prices);
});

test("strict import rejects unsupported envelopes, rates, providers, and malformed keys", () => {
  for (const raw of ["null", "[]", "{}", '{"version":2,"prices":{}}', '{"version":1,"prices":{},"extra":0}', '{"version":1,"prices":null}', '{"version":1,"prices":[]}']) assert.throws(() => parsePriceImport(raw));
  for (const rates of [null, [], 1, { input: -1 }, { input: "1" }, { input: null }, { reasoning: 1 }, { input: true }]) assert.throws(() => parsePriceImport(encode({ [key]: rates })));
  assert.throws(() => parsePriceImport('{"version":1,"prices":{"[\\"codex\\",\\"m\\"]":{"input":1e309}}}'));
  for (const bad of ["m", '[]', '["codex"]', '["grok","m"]', '["codex",1]', '["codex","m",0]', '[null,"m"]']) assert.throws(() => parsePriceImport(encode({ [bad]: {} })));
});

test("canonicalizes JSON key whitespace and rejects alias collisions", () => {
  const alias = '[ "codex", "m" ]';
  assert.deepEqual(parsePriceImport(encode({ [alias]: { input: 2 } })), { [key]: { input: 2 } });
  assert.throws(() => parsePriceImport(encode({ [alias]: {}, [key]: {} })));
  assert.throws(() => applyPriceDrafts({}, { [alias]: { input: "1" }, [key]: { output: "2" } }));
});

test("merge preview counts added and conflicting fields, and import preserves omitted fields", () => {
  const current = { [key]: { input: 1, output: 2 }, [other]: { input: 5 } };
  const fresh = priceKey("claude", "new");
  const incoming = { [key]: { input: 3, output: 2, cache_read: 0 }, [fresh]: { input: 1 } };
  assert.deepEqual(previewPriceImport(current, incoming), { addedModels: 1, addedFields: 2, overwrittenFields: 1, unchangedFields: 1 });
  const result = mergePriceImport(current, incoming);
  assert.deepEqual(result[key], { input: 3, output: 2, cache_read: 0 });
  assert.deepEqual(result[other], { input: 5 }); assert.equal(current[key].input, 1);
  assert.deepEqual(mergePriceImport(current, { [key]: {} }), current);
});

test("import enforces UTF-8 byte and model limits and export refuses non-finite prices", () => {
  assert.throws(() => parsePriceImport(" ".repeat(maxPriceImportBytes + 1)));
  assert.throws(() => parsePriceImport(encode({ [priceKey("codex", "中".repeat(maxPriceImportBytes / 2))]: {} })));
  const prices = Object.fromEntries(Array.from({ length: 1000 }, (_, n) => [priceKey("codex", String(n)), {}]));
  assert.equal(Object.keys(parsePriceImport(encode(prices))).length, 1000);
  prices[priceKey("codex", "extra")] = {};
  assert.throws(() => parsePriceImport(encode(prices)));
  assert.throws(() => serializePriceBook({ [key]: { input: Infinity } }));
});

test("unknown empty model names remain editable and import/export compatible", () => {
  const unknown = priceKey("codex", "");
  const updated = applyPriceDrafts({}, { [unknown]: { input: "0", output: "2" } });
  assert.deepEqual(updated, { [unknown]: { input: 0, output: 2 } });
  assert.deepEqual(parsePriceImport(serializePriceBook(updated)), updated);
  const row = session(); row.models[0].model = "";
  assert.deepEqual(usagePriceCatalog([row], updated)[0].missingFields, ["cache_write_1h"]);
});

test("import merge and preview enforce total model and serialized byte limits before save", () => {
  const current = Object.fromEntries(Array.from({ length: 1000 }, (_, n) => [priceKey("codex", String(n)), {}]));
  const incoming = { [priceKey("claude", "new")]: { input: 1 } };
  assert.throws(() => mergePriceImport(current, incoming), /1000/);
  assert.throws(() => previewPriceImport(current, incoming), /1000/);
  assert.equal(Object.keys(current).length, 1000);
  const largeCurrent = { [priceKey("codex", "中".repeat(180000))]: { input: 1 } };
  const largeIncoming = { [priceKey("claude", "文".repeat(180000))]: { input: 2 } };
  assert.doesNotThrow(() => serializePriceBook(largeCurrent));
  assert.doesNotThrow(() => serializePriceBook(largeIncoming));
  assert.throws(() => mergePriceImport(largeCurrent, largeIncoming), /1 MiB/);
  assert.throws(() => previewPriceImport(largeCurrent, largeIncoming), /1 MiB/);
});
