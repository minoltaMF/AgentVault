// Synthetic frontend CPU benchmark. No native session files or browser state are read.
// node node_modules/tsx/dist/cli.mjs scripts/benchmark-preview-search.ts --output result.json
// Add --baseline-only to measure alpha.49 before the optimized module exists.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import os from "node:os";
import { performance } from "node:perf_hooks";
import type { PreviewEvent } from "../src/lib/api";
import { isConversationMessage, previewEventSearchText } from "../src/lib/previewEvent";
import { isVisibleConversationEvent } from "../src/lib/conversationDisplay";
import { continuePreviewSearch } from "../src/lib/preview-continuous-search";
import { latencyStatistics } from "./benchmark-statistics.mjs";

type Eligible = (event: PreviewEvent) => boolean;
type Projection = { eligibleEvents: PreviewEvent[]; matchingEvents: PreviewEvent[] };
type Selector = {
  select: (events: readonly PreviewEvent[], query: string, eligible: Eligible) => Projection;
  matches: (event: PreviewEvent, query: string, eligible: Eligible) => boolean;
};
type Mode = "baseline" | "optimized";
const options: Record<string, string> = {};
let baselineOnly = false;
for (let i = 2; i < process.argv.length; i++) {
  const option = process.argv[i];
  if (option === "--baseline-only") { baselineOnly = true; continue; }
  assert(["--output", "--rounds", "--sizes", "--body-length"].includes(option), `Unknown option: ${option}`);
  assert(process.argv[i + 1], `Missing value for ${option}`);
  options[option.slice(2)] = process.argv[++i];
}
const rounds = Number(options.rounds ?? 5);
const sizes = (options.sizes ?? "3000,10000,30000").split(",").map(Number);
const bodyLength = Number(options["body-length"] ?? 2048);
const startedAt = new Date().toISOString();
assert(Number.isSafeInteger(rounds) && rounds >= 5, "Use at least five measured rounds");
assert(sizes.length > 0 && sizes.every(size => Number.isSafeInteger(size) && size >= 600 && size % 200 === 0), "Sizes must be multiples of 200 and >= 600");
assert(Number.isSafeInteger(bodyLength) && bodyLength >= 64, "Body length must be an integer >= 64");
const pageSize = 200;
const query = "benchmark-final-needle";
const missingQuery = "benchmark-absent-needle";
const body = "长会话性能测试 MixedCase text with code `let value = 42;` and context. ".repeat(Math.ceil(bodyLength / 60)).slice(0, bodyLength);

function fixture(size: number): PreviewEvent[] {
  return Array.from({ length: size }, (_, index) => {
    const hidden = index % 5 === 0;
    const role = hidden ? "tool_result" : index % 2 ? "assistant" : "user";
    const text = `${body}\nmessage ${index}${index === size - 1 ? ` ${query}` : ""}`;
    return {
      index: index * 2, timestamp: "2026-01-01T00:00:00Z", role,
      kind: hidden ? "tool_result" : "message", text_summary: text.slice(0, 100),
      raw: hidden ? { type: "tool_result", content: text, metadata: { hidden: true } }
        : { type: role, message: { role, content: [{ type: "text", text }] } },
    };
  });
}

function predicate(events: readonly PreviewEvent[]): Eligible {
  // Mimic the current conversation filter and timeline visibility restriction.
  const prompts = new Set(events.filter(event => event.role === "user" && event.index % 6 !== 0).map(event => event.index));
  return event => isConversationMessage(event) && isVisibleConversationEvent(event, prompts);
}

// Preserve alpha.49's existing WeakMap text cache and map/filter order exactly.
function baseline(): Selector {
  return {
    select(events, normalizedQuery, eligible) {
      const searchable = events.map(event => ({ event, searchText: previewEventSearchText(event) }));
      const selected = searchable.filter(({ event }) => eligible(event));
      const matched = selected.filter(({ searchText }) => !normalizedQuery || searchText.includes(normalizedQuery)).map(({ event }) => event);
      // The old UI retained wrappers. Unwrap only on inspection, outside timed selection.
      return { get eligibleEvents() { return selected.map(({ event }) => event); }, matchingEvents: matched };
    },
    matches: (event, normalizedQuery, eligible) => eligible(event) && previewEventSearchText(event).includes(normalizedQuery),
  };
}
let optimized: (() => Selector) | undefined;
if (!baselineOnly) ({ createPreviewSearchProjection: optimized } = await import("../src/lib/preview-search-projection"));
const factory = (mode: Mode) => mode === "baseline" ? baseline() : optimized!();

function sample(size: number, mode: Mode) {
  // Fresh, value-identical object graphs per mode prevent one run warming another's WeakMap.
  const all = fixture(size), eligible = predicate(all), selector = factory(mode);
  const pageSamples: { end: number; select_ms: number; new_page_match_ms: number }[] = [];
  let result: Projection = { eligibleEvents: [], matchingEvents: [] };
  for (let end = pageSize; end <= size + pageSize - 1; end += pageSize) {
    const actualEnd = Math.min(end, size), events = all.slice(0, actualEnd);
    // Allocation of accumulated events models React state setup but is not part of projection CPU.
    const selectStart = performance.now();
    result = selector.select(events, query, eligible);
    const selectMs = performance.now() - selectStart;
    const matchStart = performance.now();
    for (const event of all.slice(Math.max(0, end - pageSize), actualEnd)) selector.matches(event, query, eligible);
    pageSamples.push({ end: actualEnd, select_ms: selectMs, new_page_match_ms: performance.now() - matchStart });
  }
  const actualEligible = result.eligibleEvents.map(event => event.index);
  const expectedEligible = all.filter(eligible).map(event => event.index);
  assert.deepEqual(actualEligible, expectedEligible);
  assert.deepEqual(result.matchingEvents.map(event => event.index), [all.at(-1)!.index]);
  return {
    mode, events: size, cumulative_select_ms: pageSamples.reduce((sum, page) => sum + page.select_ms, 0),
    cumulative_new_page_match_ms: pageSamples.reduce((sum, page) => sum + page.new_page_match_ms, 0),
    page_select: latencyStatistics(pageSamples.map(page => page.select_ms)),
    first_page_select_ms: pageSamples[0].select_ms,
    final_page_select_ms: pageSamples.at(-1)!.select_ms,
    page_samples: pageSamples,
    eligible_count: actualEligible.length, hit_indices: result.matchingEvents.map(event => event.index),
  };
}

async function cancellationSample(size: number, mode: Mode) {
  const all = fixture(size), eligible = predicate(all), selector = factory(mode);
  let end = size - 400;
  selector.select(all.slice(0, end), missingQuery, eligible);
  let cancelled = false, reads = 0, readsAfterCancellation = 0;
  let scheduledAt = 0, callbackAt = 0;
  const result = await continuePreviewSearch<PreviewEvent>({
    direction: "after", nextPage: () => end < all.length ? { start: end, count: pageSize } : null,
    readPage: async page => {
      if (cancelled) readsAfterCancellation++;
      reads++;
      if (reads === 1) {
        scheduledAt = performance.now();
        setTimeout(() => { callbackAt = performance.now(); cancelled = true; }, 0);
      }
      const batch = all.slice(page.start, page.start + page.count);
      end += batch.length;
      selector.select(all.slice(0, end), missingQuery, eligible);
      return batch;
    },
    matches: event => selector.matches(event, missingQuery, eligible),
    isCancelled: () => cancelled, onProgress: () => {},
  });
  const settledAt = performance.now();
  assert.equal(result.status, "cancelled");
  assert.equal(readsAfterCancellation, 0);
  assert.equal(reads, 1);
  return { mode, events: size, timer_dispatch_ms: callbackAt - scheduledAt,
    callback_to_settlement_ms: settledAt - callbackAt, reads, reads_after_cancel: readsAfterCancellation };
}

const modes: Mode[] = baselineOnly ? ["baseline"] : ["baseline", "optimized"];
const samples: (ReturnType<typeof sample> & { round: number })[] = [];
const cancellation: (Awaited<ReturnType<typeof cancellationSample>> & { round: number })[] = [];
// One unreported warm-up for each mode, using independent objects, to avoid JIT-first-run claims.
for (const mode of modes) { sample(3000, mode); await cancellationSample(3000, mode); }
for (const size of sizes) {
  for (let round = 0; round < rounds; round++) {
    const ordered = round % 2 ? [...modes].reverse() : modes;
    for (const mode of ordered) {
      const measured = sample(size, mode);
      samples.push({ ...measured, round: round + 1 });
      cancellation.push({ ...await cancellationSample(size, mode), round: round + 1 });
      process.stderr.write(`${size} ${mode} round ${round + 1}: projection ${measured.cumulative_select_ms.toFixed(2)} ms\n`);
    }
  }
}
const summary = sizes.flatMap(size => modes.map(mode => {
  const selected = samples.filter(sample => sample.events === size && sample.mode === mode);
  const cancelled = cancellation.filter(sample => sample.events === size && sample.mode === mode);
  return { events: size, mode,
    cumulative_select: latencyStatistics(selected.map(sample => sample.cumulative_select_ms)),
    first_page_select: latencyStatistics(selected.map(sample => sample.first_page_select_ms)),
    final_page_select: latencyStatistics(selected.map(sample => sample.final_page_select_ms)),
    new_page_match: latencyStatistics(selected.map(sample => sample.cumulative_new_page_match_ms)),
    cancellation_timer_dispatch: latencyStatistics(cancelled.map(sample => sample.timer_dispatch_ms)),
    cancellation_callback_to_settlement: latencyStatistics(cancelled.map(sample => sample.callback_to_settlement_ms)),
  };
}));
const report = {
  schema_version: 1, started_at: startedAt, completed_at: new Date().toISOString(),
  machine: { node: process.version, platform: process.platform, arch: process.arch, cpu: os.cpus()[0]?.model },
  configuration: { sizes, rounds, page_size: pageSize, body_characters: bodyLength, baseline_only: baselineOnly,
    warmups_per_mode: 1, order: "alternate baseline/optimized each round", fresh_objects_per_run: true },
  scope: "Node CPU microbenchmark of cumulative projection and new-page matching. No I/O, HTTP, React render, DOM or browser click latency is measured. Cancellation is zero-delay timer dispatch under one-page CPU work, not native request abortion.",
  baseline: "alpha.49 map -> eligibility filter -> keyword filter/map, retaining production previewEventSearchText WeakMap; final wrapper-to-event map excluded from baseline timing",
  summary, samples, cancellation,
};
const json = JSON.stringify(report, null, 2) + "\n";
if (options.output) await writeFile(options.output, json, { flag: "wx" });
process.stdout.write(options.output ? JSON.stringify({ output: options.output, summary }, null, 2) + "\n" : json);
