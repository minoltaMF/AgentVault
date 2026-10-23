import assert from "node:assert/strict";
import test from "node:test";
import { clearPreviewMemories, PREVIEW_MEMORY_KEY, readMemoryPolicy, setMemoryEnabled } from "./preview-memory";
import { READING_POSITION_KEY, readPositions, saveReadingPosition } from "./preview-reading-position";
import { SEARCH_STATE_KEY, readSearchStates, saveSearchState } from "./preview-search-state";

function storage() {
  const values = new Map<string, string>();
  return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
}
function seeded() {
  const s = storage();
  for (const key of ['["codex","/a"]', '["claude","/a"]', '["codex","/A"]']) {
    const position = { key, eventIndex: 2, eventOffset: 2, signature: "1:2", newestFirst: false, onlyMessages: true, savedAt: 1 };
    assert.ok(saveReadingPosition(s, position));
    assert.ok(saveSearchState(s, { key, query: "needle", position, occurrenceIndex: 1, savedAt: 1 }));
  }
  return s;
}

test("memory defaults to enabled and disabling persists without deleting saved entries", () => {
  const s = seeded();
  assert.deepEqual(readMemoryPolicy(s), { enabled: true, revision: 0 });
  const before = s.getItem(READING_POSITION_KEY);
  assert.equal(setMemoryEnabled(s, false), true);
  assert.deepEqual(readMemoryPolicy(s), { enabled: false, revision: 1 });
  assert.equal(s.getItem(READING_POSITION_KEY), before);
  assert.equal(setMemoryEnabled(s, true), true);
  assert.deepEqual(readMemoryPolicy(s), { enabled: true, revision: 2 });
});

test("current-session clearing removes both memories with exact provider and path isolation", () => {
  const s = seeded();
  assert.equal(clearPreviewMemories(s, '["codex","/a"]'), true);
  assert.deepEqual(readPositions(s).map((item) => item.key), ['["codex","/A"]', '["claude","/a"]']);
  assert.deepEqual(readSearchStates(s).map((item) => item.key), ['["codex","/A"]', '["claude","/a"]']);
  assert.equal(readMemoryPolicy(s).revision, 1);
});

test("clear-all repairs damaged memory stores while preserving disabled policy", () => {
  const s = seeded();
  setMemoryEnabled(s, false);
  s.setItem(READING_POSITION_KEY, "broken");
  s.setItem(SEARCH_STATE_KEY, "{}");
  assert.equal(clearPreviewMemories(s), true);
  assert.equal(s.getItem(READING_POSITION_KEY), "[]");
  assert.equal(s.getItem(SEARCH_STATE_KEY), "[]");
  assert.deepEqual(readMemoryPolicy(s), { enabled: false, revision: 2 });
});

test("every mutation invalidates an old view token before record writes", () => {
  const s = seeded();
  const oldToken = readMemoryPolicy(s).revision;
  const writes: string[] = [];
  const tracking = { getItem: s.getItem, setItem(key: string, value: string) { writes.push(key); s.setItem(key, value); } };
  assert.equal(clearPreviewMemories(tracking, "missing"), true);
  assert.notEqual(readMemoryPolicy(s).revision, oldToken);
  assert.deepEqual(writes, [PREVIEW_MEMORY_KEY, READING_POSITION_KEY, SEARCH_STATE_KEY]);
  const nextToken = readMemoryPolicy(s).revision;
  setMemoryEnabled(s, true);
  assert.notEqual(readMemoryPolicy(s).revision, nextToken);
});

test("unreadable or invalid policy fails closed and refuses mutations", () => {
  const s = storage();
  for (const raw of ["", "bad", "null", "{}", '{"enabled":true,"revision":-1}', '{"enabled":true,"revision":1.5}']) {
    s.setItem(PREVIEW_MEMORY_KEY, raw);
    assert.deepEqual(readMemoryPolicy(s), { enabled: false, revision: 0 });
    assert.equal(setMemoryEnabled(s, true), false);
    assert.equal(clearPreviewMemories(s), false);
    assert.equal(s.getItem(PREVIEW_MEMORY_KEY), raw);
  }
  const blocked = { getItem(): string | null { throw new Error("read blocked"); }, setItem() { assert.fail("write after read failure"); } };
  assert.deepEqual(readMemoryPolicy(blocked), { enabled: false, revision: 0 });
  assert.equal(setMemoryEnabled(blocked, false), false);
  assert.equal(clearPreviewMemories(blocked), false);
});

test("current-session clearing refuses unreadable or malformed records without erasing other sessions", () => {
  for (const raw of ["broken", "{}", "[{}]", " ".repeat(4_000_001)]) {
    const s = seeded();
    const positions = s.getItem(READING_POSITION_KEY);
    s.setItem(SEARCH_STATE_KEY, raw);
    assert.equal(clearPreviewMemories(s, '["codex","/a"]'), false);
    assert.equal(s.getItem(READING_POSITION_KEY), positions);
    assert.equal(s.getItem(SEARCH_STATE_KEY), raw);
    assert.equal(readMemoryPolicy(s).revision, 1);
  }
  const s = seeded();
  const original = s.getItem(READING_POSITION_KEY);
  assert.equal(clearPreviewMemories({ ...s, getItem(key) { if (key === SEARCH_STATE_KEY) throw new Error("blocked"); return s.getItem(key); } }, "a"), false);
  assert.equal(s.getItem(READING_POSITION_KEY), original);
});

test("policy write failure prevents all deletion and partial record failure returns false with an advanced revision", () => {
  const s = seeded();
  const before = s.getItem(READING_POSITION_KEY);
  const blocked = { getItem: s.getItem, setItem() { throw new Error("quota"); } };
  assert.equal(setMemoryEnabled(blocked, false), false);
  assert.equal(clearPreviewMemories(blocked), false);
  assert.equal(s.getItem(READING_POSITION_KEY), before);
  assert.equal(readMemoryPolicy(s).revision, 0);
  const partial = { getItem: s.getItem, setItem(key: string, value: string) { if (key === SEARCH_STATE_KEY) throw new Error("quota"); s.setItem(key, value); } };
  assert.equal(clearPreviewMemories(partial), false);
  assert.equal(s.getItem(READING_POSITION_KEY), "[]");
  assert.equal(readSearchStates(s).length, 3);
  assert.equal(readMemoryPolicy(s).revision, 1);
});

test("revision overflow refuses writes rather than reusing an old token", () => {
  const s = seeded();
  s.setItem(PREVIEW_MEMORY_KEY, JSON.stringify({ enabled: true, revision: Number.MAX_SAFE_INTEGER }));
  assert.equal(setMemoryEnabled(s, false), false);
  assert.equal(clearPreviewMemories(s), false);
  assert.equal(readPositions(s).length, 3);
  assert.equal(readMemoryPolicy(s).revision, Number.MAX_SAFE_INTEGER);
});
