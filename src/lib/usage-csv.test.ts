import assert from "node:assert/strict";
import test from "node:test";
import { buildUsageCsv } from "./usage-csv";
import { dailyUsageTrend, priceKey, type UsageSession, type UsageStatus } from "./session-usage";

const now = new Date("2026-10-12T12:00:00Z");
const prices = { [priceKey("codex", "m")]: { input: 1 } };
function session(date: string, input: number): UsageSession {
  const models = [{ model: "m", tokens: { input, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, reasoning: 0 } }];
  return { provider: "codex", rollout_path: "/private/" + date, id: date, title: "PRIVATE BODY", cwd: "/secret/project", models, warnings: [], daily: [{ date, models }] };
}
function status(results = [session("2026-10-12", 200), session("2026-10-05", 100)]): UsageStatus {
  return { state: "completed", total_files: results.length, processed_files: results.length, parsed_files: results.length, cache_hits: 0, elapsed_ms: 1, cache_warning: null, results, failures: [], error: null };
}
function csv(state = status(), extra: Partial<Parameters<typeof buildUsageCsv>[0]> = {}) {
  return buildUsageCsv({ status: state, prices, range: { from: "", to: "" }, period: { days: 7, now }, generatedAt: now, ...extra });
}
// Parse quoted fields and embedded newlines independently of the serializer.
function parse(text: string) {
  const rows: string[][] = []; let row: string[] = [], value = "", quoted = false;
  text = text.slice(1);
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') { if (quoted && text[i + 1] === '"') { value += '"'; i++; } else quoted = !quoted; }
    else if (!quoted && c === ",") { row.push(value); value = ""; }
    else if (!quoted && c === "\r" && text[i + 1] === "\n") { row.push(value); rows.push(row); row = []; value = ""; i++; }
    else value += c;
  }
  const [header, ...body] = rows;
  assert.ok(body.every(item => item.length === header.length));
  return body.map(item => Object.fromEntries(header.map((key, i) => [key, item[i]])));
}
test("CSV exports selected range and all days while periods retain independent UTC boundaries", () => {
  const data = Array.from({ length: 60 }, (_, i) => { const date = new Date(now); date.setUTCDate(date.getUTCDate() - i); return session(date.toISOString().slice(0, 10), 100); });
  const all = parse(csv(status(data)));
  assert.equal(all.filter(row => row.record_type === "daily").length, 60);
  const rows = parse(csv(status(data), { range: { from: "2026-10-12", to: "2026-10-12" } }));
  assert.equal(rows.find(row => row.record_type === "summary")?.tokens, "100");
  assert.equal(rows.filter(row => row.record_type === "daily").length, 1);
  assert.equal(rows.find(row => row.record_type === "period_current")?.tokens, "700");
  assert.equal(rows.find(row => row.record_type === "period_previous")?.to, "2026-10-05");
  assert.equal(parse(csv(status(data), { period: { days: 30, now } })).find(row => row.record_type === "period_current")?.tokens, "3000");
});
test("CSV uses BOM CRLF rectangular escaped cells and shields spreadsheet formulas", () => {
  const state = status(); state.state = ' \t=HYPERLINK("x,y")\r\nnext' as UsageStatus["state"];
  const text = csv(state);
  assert.equal(text.charCodeAt(0), 0xfeff);
  assert.ok(text.endsWith("\r\n"));
  assert.equal(parse(text).find(row => row.metric === "analysis_state")?.value, "'" + state.state);
  for (const value of ["+cmd", "-cmd", "@sum", "\ttext", "\rtext", "\ntext"]) {
    state.state = value as UsageStatus["state"];
    assert.equal(parse(csv(state)).find(row => row.metric === "analysis_state")?.value, "'" + value);
  }
});
test("unknown pricing keeps known subtotal but suppresses cost percentage", () => {
  const rows = parse(csv(status(), { prices: {} }));
  const cost = rows.find(row => row.record_type === "period_change" && row.metric === "known_cost_usd")!;
  assert.equal(cost.delta, "0"); assert.equal(cost.percent, ""); assert.match(cost.note, /不代表总费用/);
  assert.equal(rows.find(row => row.record_type === "summary")?.complete, "false");
  assert.equal(rows.find(row => row.record_type === "summary")?.cost_status, "partial_known_only");
});
test("missing records differ from explicit zero, and negative numeric changes remain numeric", () => {
  const missing = parse(csv(status([])));
  assert.equal(missing.find(row => row.record_type === "summary")?.tokens, "");
  const zero = parse(csv(status([session("2026-10-12", 0), session("2026-10-05", 0)])));
  assert.equal(zero.find(row => row.record_type === "summary")?.tokens, "0");
  assert.equal(zero.find(row => row.record_type === "period_change")?.percent, "0");
  const negative = parse(csv(status([session("2026-10-12", 1), session("2026-10-05", 2)])));
  assert.equal(negative.find(row => row.record_type === "period_change")?.delta, "-1");
});
test("partial analysis and quality markers survive without paths or private session text", () => {
  for (const state of ["running", "cancelled", "failed"] as const) {
    const value = status(); value.state = state;
    const rows = parse(csv(value));
    assert.equal(rows.find(row => row.metric === "analysis_complete")?.value, "false");
    assert.equal(rows.find(row => row.record_type === "period_change")?.delta, "");
  }
  const value = status(); value.results[0].warnings = ["PRIVATE WARNING"];
  value.results[0].undated_models = value.results[0].models;
  value.results.push({ ...session("2026-10-01", 1), models: [] });
  value.failures = [{ path: "/private/fail", error: "PRIVATE FAILURE" }];
  value.cache_warning = "PRIVATE CACHE";
  const text = csv(value, { error: "PRIVATE ERROR" }), rows = parse(text);
  for (const metric of ["cache_warning", "analysis_error"]) assert.equal(rows.find(row => row.metric === metric)?.value, "true");
  for (const metric of ["failed_files", "warned_sessions", "undated_sessions", "missing_usage_sessions"]) assert.equal(rows.find(row => row.metric === metric)?.value, "1");
  assert.doesNotMatch(text, /PRIVATE|\/private|\/secret/);
});
test("token and cost overflows export blanks rather than finite-looking partial totals", () => {
  for (const extra of [{ status: status([session("2026-10-12", Number.MAX_SAFE_INTEGER), session("2026-10-11", 10)]) }, { prices: { [priceKey("codex", "m")]: { input: Number.MAX_VALUE } } }]) {
    const rows = parse(csv(status(), extra));
    const summary = rows.find(row => row.record_type === "summary")!;
    assert.equal(summary.tokens, ""); assert.equal(summary.known_cost_usd, "");
    assert.equal(rows.find(row => row.metric === "summary_overflow")?.value, "true");
  }
});
test("duplicates are deduplicated and invalid range rejects without mutating inputs", () => {
  const state = status(); state.results.push(state.results[0]);
  const before = JSON.stringify(state);
  assert.equal(parse(csv(state)).find(row => row.record_type === "summary")?.tokens, "300");
  assert.equal(JSON.stringify(state), before);
  assert.throws(() => csv(state, { range: { from: "2026-02-30", to: "" } }), /日期/);
});

test("UTF-8 byte limit rejects oversized CSV, including multibyte cells", () => {
  const value = status();
  value.state = "中".repeat(12 * 1024 * 1024) as UsageStatus["state"];
  assert.throws(() => csv(value), /32 MiB/);
});

test("daily export matches shared trends across repeated day buckets, empty records and invalid dates", () => {
  const first = session("2026-10-12", 100);
  first.daily!.push({ date: "2026-10-12", models: first.models }, { date: "2026-10-11", models: [] }, { date: "2026-02-30", models: first.models });
  const second = session("2026-10-12", 50); second.rollout_path += "-second";
  second.daily!.push({ date: "2026-10-10", models: second.models });
  const sessions = [first, second];
  for (const range of [{ from: "", to: "" }, { from: "2026-10-11", to: "2026-10-12" }]) {
    const rows = parse(csv(status(sessions), { range })).filter(row => row.record_type === "daily");
    const expected = dailyUsageTrend(sessions, prices, range);
    assert.deepEqual(rows.map(row => row.date), expected.map(row => row.date));
    for (let i = 0; i < rows.length; i++) {
      assert.equal(rows[i].sessions, String(expected[i].sessions));
      assert.equal(rows[i].unknown_price_models, String(expected[i].unknownModels));
      assert.equal(rows[i].tokens, expected[i].missingUsage === expected[i].sessions ? "" : String(expected[i].tokens));
      assert.equal(rows[i].known_cost_usd, expected[i].missingUsage === expected[i].sessions ? "" : String(expected[i].known));
    }
  }
});
