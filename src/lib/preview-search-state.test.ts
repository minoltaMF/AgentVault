import assert from "node:assert/strict";
import test from "node:test";
import type { PreviewEvent } from "./api";
import { matchesReadingPosition, readingEventSignature, readingPositionKey } from "./preview-reading-position";
import { MAX_SEARCH_STATES, SEARCH_STATE_KEY, readSearchStates, removeSearchState, saveSearchState, type SearchState } from "./preview-search-state";

const event = { index: 700, timestamp: "2026-10-22", role: "user", kind: "message", raw: { text: "private transcript" } } as PreviewEvent;
const key = readingPositionKey("codex", "/a");
const state: SearchState = { key, query: "  中文 🔎 café  ", position: { key, eventIndex: 700, eventOffset: 400, signature: readingEventSignature(event), newestFirst: true, onlyMessages: true, savedAt: 1 }, occurrenceIndex: 7, savedAt: 1 };
function storage(initial: string | null = null) {
  const values = new Map<string, string>();
  if (initial !== null) values.set(SEARCH_STATE_KEY, initial);
  return { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
}

test("search state isolates exact provider/path and preserves unicode query and occurrence", () => {
  const s = storage();
  for (const [provider, path] of [["codex", "/a"], ["claude", "/a"], ["codex", "/A"]]) {
    const nextKey = readingPositionKey(provider, path);
    assert.equal(saveSearchState(s, { ...state, key: nextKey, position: { ...state.position!, key: nextKey } }), true);
  }
  assert.equal(readSearchStates(s).length, 3);
  assert.deepEqual(readSearchStates(s).find((item) => item.key === state.key), state);
  assert.equal(saveSearchState(s, { ...state, occurrenceIndex: 2, savedAt: 2 }), true);
  assert.equal(readSearchStates(s).length, 3);
  assert.equal(readSearchStates(s)[0].occurrenceIndex, 2);
  assert.equal(readSearchStates(s)[0].query, state.query);
});

test("no-hit query persists with null position and removal preserves other sessions", () => {
  const s = storage();
  saveSearchState(s, state);
  const noHit = { ...state, key: "other", position: null, occurrenceIndex: 0 };
  assert.equal(saveSearchState(s, noHit), true);
  assert.equal(removeSearchState(s, state.key), true);
  assert.deepEqual(readSearchStates(s), [noHit]);
  assert.equal(removeSearchState(s, "missing"), true);
  assert.deepEqual(readSearchStates(s), [noHit]);
});

test("stored positions reuse signature checks and never persist transcript extras", () => {
  const s = storage();
  const extra = { ...state, transcript: "secret-extra", position: { ...state.position!, text: "secret-extra" } };
  assert.equal(saveSearchState(s, extra), true);
  const restored = readSearchStates(s)[0].position!;
  assert.equal(matchesReadingPosition([event], 400, restored), true);
  assert.equal(matchesReadingPosition([{ ...event, raw: { text: "rewritten" } }], 400, restored), false);
  assert.equal(matchesReadingPosition([event], 399, restored), false);
  assert.ok(!s.getItem(SEARCH_STATE_KEY)?.includes("private transcript"));
  assert.ok(!s.getItem(SEARCH_STATE_KEY)?.includes("secret-extra"));
  assert.deepEqual(restored, state.position);
});

test("invalid entries, old shapes, corrupt and oversized storage safely degrade", () => {
  const invalid: unknown[] = [null, {}, { ...state, key: "" }, { ...state, key: "x".repeat(32769) },
    { ...state, query: " \n " }, { ...state, query: "x".repeat(4097) }, { ...state, occurrenceIndex: -1 },
    { ...state, occurrenceIndex: 1.5 }, { ...state, occurrenceIndex: Number.MAX_SAFE_INTEGER + 1 },
    { ...state, savedAt: -1 }, { ...state, savedAt: Infinity }, { ...state, position: undefined },
    { ...state, position: { ...state.position, key: "wrong" } },
    { ...state, position: { ...state.position, signature: "transcript" } },
    { ...state, position: { ...state.position, eventOffset: -1 } }];
  for (const value of invalid) assert.equal(saveSearchState(storage(), value as SearchState), false);
  assert.deepEqual(readSearchStates(storage(JSON.stringify([...invalid, state]))), [state]);
  for (const raw of ["bad json", "{}", "null", " ".repeat(4_000_001)]) assert.deepEqual(readSearchStates(storage(raw)), []);
  const accepted = { ...state, query: "字".repeat(4096) };
  const s = storage(); assert.equal(saveSearchState(s, accepted), true);
  assert.equal(readSearchStates(s)[0].query.length, 4096);
});

test("latest 100 unique sessions survive deduplication and eviction", () => {
  const states = Array.from({ length: 110 }, (_, i) => ({ ...state, key: String(i), position: null, savedAt: i }));
  const s = storage(JSON.stringify([...states, { ...states[109], query: "newest duplicate", savedAt: 200 }]));
  const found = readSearchStates(s);
  assert.equal(found.length, MAX_SEARCH_STATES);
  assert.equal(found[0].query, "newest duplicate");
  assert.equal(found.at(-1)?.key, "10");
  assert.equal(new Set(found.map((item) => item.key)).size, 100);
  saveSearchState(s, { ...state, key: "new", position: null, savedAt: 201 });
  assert.equal(readSearchStates(s)[0].key, "new");
  assert.equal(readSearchStates(s).at(-1)?.key, "11");
});

test("blocked reads, quota failures and oversized writes fail without throwing", () => {
  const blocked = { getItem(): string | null { throw new Error("blocked"); }, setItem() { assert.fail("must not overwrite unreadable storage"); } };
  assert.deepEqual(readSearchStates(blocked), []);
  assert.equal(saveSearchState(blocked, state), false);
  assert.equal(removeSearchState(blocked, key), false);
  const quota = { getItem: () => null, setItem() { throw new Error("quota"); } };
  assert.equal(saveSearchState(quota, state), false);
  assert.equal(removeSearchState(quota, key), false);
  const longKey = "k".repeat(32760);
  const oversized: SearchState[] = [];
  for (let i = 0; i < 100; i++) {
    const nextKey = longKey + i;
    oversized.push({ ...state, key: nextKey, query: "q".repeat(4096), position: { ...state.position!, key: nextKey }, savedAt: i });
    if (JSON.stringify(oversized).length > 4_000_000) {
      const candidate = oversized.pop()!;
      const original = JSON.stringify(oversized);
      const s = storage(original);
      assert.equal(saveSearchState(s, candidate), false);
      assert.equal(s.getItem(SEARCH_STATE_KEY), original);
      return;
    }
  }
  assert.fail("oversized serialized state should be refused");
});
