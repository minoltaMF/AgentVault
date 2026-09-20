import assert from "node:assert/strict";
import test from "node:test";
import type { PreviewEvent } from "./api";
import type { ConversationPreviewRow } from "./conversationDisplay";
import { boundPreviewProcessRows, findPreviewRowIndex, previewWindowStart } from "./previewWindow.ts";

test("distant search offsets use a bounded window without loading the entire prefix", () => {
  for (const offset of [0, 99, 200, 99999]) {
    const start = previewWindowStart(offset, 200);
    assert.ok(start >= 0 && start <= offset && offset < start + 200);
  }
  assert.equal(previewWindowStart(99999, 200), 99899);
});

test("jump targets retain source indices through reverse order and grouped process rows", () => {
  const event = (index: number) => ({ index } as PreviewEvent);
  const rows: ConversationPreviewRow[] = [
    { type: "event", event: event(41) },
    { type: "process", key: 85, events: [event(85), event(102)], hasFinalResponse: true },
    { type: "event", event: event(130) },
  ];
  assert.equal(findPreviewRowIndex(rows, 41), 0);
  assert.equal(findPreviewRowIndex([...rows].reverse(), 41), 2);
  assert.equal(findPreviewRowIndex([...rows].reverse(), 102), 1);
  assert.equal(findPreviewRowIndex(rows, 1), -1);
});

test("large process turns remain bounded while every original jump target is reachable", () => {
  const events = Array.from({ length: 1001 }, (_, index) => ({ index: index * 3 + 7 } as PreviewEvent));
  const rows = boundPreviewProcessRows([{ type: "process", key: 7, events, hasFinalResponse: true }]);
  assert.equal(rows.length, 26);
  const recovered = rows.flatMap((row) => row.type === "process" ? row.events : []);
  assert.deepEqual(recovered, events);
  assert.equal(new Set(rows.map((row) => row.type === "process" && row.key)).size, rows.length);
  for (const row of rows) assert.ok(row.type === "process" && row.events.length <= 40);
  assert.equal(findPreviewRowIndex(rows, 3007), 25);
  assert.equal(findPreviewRowIndex([...rows].reverse(), 3007), 0);
});
