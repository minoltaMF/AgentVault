import assert from "node:assert/strict";
import test from "node:test";
import type { PreviewEvent } from "./api";
import { MAX_READING_POSITIONS, matchesReadingPosition, readingEventSignature, readingPositionKey, readPositions, saveReadingPosition, type ReadingPosition } from "./preview-reading-position";
const event = { index: 700, timestamp: "2026-10-18", role: "user", kind: "message", raw: { text: "private transcript" } } as PreviewEvent;
const position: ReadingPosition = { key: readingPositionKey("codex", "/a"), eventIndex: 700, eventOffset: 400, signature: readingEventSignature(event), newestFirst: true, onlyMessages: true, savedAt: 1 };
function storage() { let value: string | null = null; return { getItem: () => value, setItem: (_: string, next: string) => { value = next; } }; }
test("positions persist without transcript content and isolate provider and exact path", () => {
  const s = storage();
  for (const [provider, path] of [["codex", "/a"], ["claude", "/a"], ["codex", "/A"]]) saveReadingPosition(s, { ...position, key: readingPositionKey(provider, path) });
  assert.equal(readPositions(s).length, 3);
  assert.ok(!s.getItem()?.includes("private transcript"));
  saveReadingPosition(s, { ...position, eventOffset: 402, savedAt: 2 });
  assert.equal(readPositions(s).length, 3);
  assert.equal(readPositions(s)[0].eventOffset, 402);
});
test("restoration distinguishes source index from offset and rejects shortened or rewritten targets", () => {
  assert.ok(matchesReadingPosition([event], 400, position));
  assert.ok(!matchesReadingPosition([event], 399, position));
  assert.ok(!matchesReadingPosition([], 400, position));
  assert.ok(!matchesReadingPosition([{ ...event, index: 701 }], 400, position));
  assert.ok(!matchesReadingPosition([{ ...event, raw: { text: "changed" } }], 400, position));
  assert.ok(matchesReadingPosition([event, { ...event, index: 702 }], 400, position));
});
test("bounded storage evicts old sessions and tolerates corrupt, invalid, and unavailable storage", () => {
  const s = storage();
  for (let i = 0; i < 110; i++) saveReadingPosition(s, { ...position, key: String(i), savedAt: i });
  assert.equal(readPositions(s).length, MAX_READING_POSITIONS);
  assert.equal(readPositions(s)[0].key, "109");
  s.setItem("", "bad json"); assert.deepEqual(readPositions(s), []);
  s.setItem("", JSON.stringify([{ ...position, eventOffset: -1 }, position])); assert.equal(readPositions(s).length, 1);
  const unavailable = { getItem() { throw new Error("blocked"); }, setItem() { throw new Error("quota"); } };
  assert.deepEqual(readPositions(unavailable), []);
  assert.equal(saveReadingPosition(unavailable, position), false);
});
