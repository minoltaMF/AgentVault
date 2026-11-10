import assert from "node:assert/strict";
import test from "node:test";
import type { PreviewEvent } from "./api";
import { previewEventSearchText, isConversationMessage } from "./previewEvent";
import { isVisibleConversationEvent } from "./conversationDisplay";
import { createPreviewSearchProjection } from "./preview-search-projection";

const event = (index: number, text: string, role = "assistant"): PreviewEvent => ({
  index, role, kind: "message", timestamp: "", text_summary: text,
  raw: { type: role, message: { role, content: text } },
});
const all = () => true;

test("appending and prepending reuse classifications and retain chronological source identities", () => {
  const projection = createPreviewSearchProjection();
  const calls: number[] = [];
  const eligible = (event: PreviewEvent) => { calls.push(event.index); return true; };
  const first = event(8, "Needle"), earlier = event(2, "needle"), later = event(90, "other");
  assert.equal(projection.matches(first, "needle", eligible), true);
  projection.select([first], "needle", eligible);
  const appended = projection.select([first, later], "needle", eligible);
  const prepended = projection.select([earlier, first, later], "needle", eligible);
  assert.deepEqual(appended.matchingEvents, [first]);
  assert.deepEqual(prepended.eligibleIndices, [2, 8, 90]);
  assert.deepEqual(prepended.matchingEvents, [earlier, first]);
  assert.deepEqual(calls, [8, 90, 2]);
  assert.deepEqual(appended.eligibleIndices, [8, 90], "later projections cannot mutate earlier render snapshots");
});

test("changing or clearing the keyword invalidates matches", () => {
  const projection = createPreviewSearchProjection();
  const events = [event(1, "red"), event(2, "blue")];
  assert.deepEqual(projection.select(events, "red", all).matchingEvents, [events[0]]);
  assert.deepEqual(projection.select(events, "blue", all).matchingEvents, [events[1]]);
  assert.deepEqual(projection.select(events, "", all).matchingEvents, events);
  assert.deepEqual(projection.select(events, "red", all).matchingEvents, [events[0]]);
});

test("a newly loaded timeline or changed message filter invalidates eligibility", () => {
  const projection = createPreviewSearchProjection();
  const events = [event(2, "needle", "user"), event(5, "needle", "assistant"), event(7, "needle", "user")];
  const predicate = (indices: Set<number> | null) => (event: PreviewEvent) =>
    isConversationMessage(event) && isVisibleConversationEvent(event, indices);
  assert.deepEqual(projection.select(events, "needle", predicate(null)).matchingEvents, events);
  assert.deepEqual(projection.select(events, "needle", predicate(new Set([7]))).matchingEvents, events.slice(1));
  assert.deepEqual(projection.select(events, "needle", all).matchingEvents, events);
});

test("reread objects with the same source index never reuse a previous match", () => {
  const projection = createPreviewSearchProjection();
  assert.equal(projection.matches(event(5, "needle"), "needle", all), true);
  const replaced = event(5, "changed");
  assert.deepEqual(projection.select([replaced], "needle", all).matchingEvents, []);
});

test("empty queries and excluded events do not serialize hidden data", () => {
  const projection = createPreviewSearchProjection();
  let serialized = 0;
  const hidden = event(1, "");
  hidden.raw = { toJSON() { serialized++; return { tool: "hidden needle" }; } };
  assert.equal(projection.matches(hidden, "needle", () => false), false);
  assert.equal(projection.matches(hidden, "", all), true);
  assert.equal(serialized, 0);
  assert.equal(projection.matches(hidden, "needle", all), true);
  assert.equal(serialized, 1);
});

test("cached projection equals the previous pipeline across pages, filters and raw-only matches", () => {
  const projection = createPreviewSearchProjection();
  const events = Array.from({ length: 601 }, (_, i) => {
    const result = event(i * 3, i % 4 ? "中文 markdown **词**" : "Other", i % 3 ? "assistant" : "user");
    if (i % 7 === 0) result.raw = { type: "tool_result", payload: { nested: ["RAW-NEEDLE"] } };
    return result;
  });
  for (const query of ["raw-needle", "中文", "missing", "", "other"]) {
    for (const predicate of [all, isConversationMessage]) {
      for (const count of [1, 200, 400, 601, 200]) {
        const page = events.slice(0, count);
        const expectedEligible = page.filter(predicate);
        const expectedMatches = expectedEligible.filter(event => !query || previewEventSearchText(event).includes(query));
        const result = projection.select(page, query, predicate);
        assert.deepEqual(result.eligibleEvents, expectedEligible);
        assert.deepEqual(result.eligibleIndices, expectedEligible.map(event => event.index));
        assert.deepEqual(result.matchingEvents, expectedMatches);
      }
    }
  }
});
