import assert from "node:assert/strict";
import test from "node:test";
import { gapExpansionIndices, previewSearchGaps } from "./preview-search-gaps";

test("gaps count only eligible loaded events between visible messages", () => {
  const gaps = previewSearchGaps([2, 9, 30, 50, 80, 100, 140], [9, 80, 100], false);
  assert.deepEqual([...gaps], [[80, { indices: [30, 50] }]]);
  assert.equal(gaps.has(9), false);
  assert.equal(gaps.has(140), false);
});

test("newest-first gaps attach to the following displayed message and expand in display order", () => {
  const gaps = previewSearchGaps([2, 9, 30, 50, 80, 100], [100, 80, 9], true);
  assert.deepEqual([...gaps], [[9, { indices: [50, 30] }]]);
  assert.deepEqual(gapExpansionIndices(gaps.get(9)!, 1), [50]);
});

test("expanding a gap leaves only its remaining hidden messages", () => {
  const eligible = Array.from({ length: 25 }, (_, index) => index * 3);
  const initial = previewSearchGaps(eligible, [0, 72], false).get(72)!;
  const expanded = gapExpansionIndices(initial);
  assert.equal(expanded.length, 20);
  assert.deepEqual([...previewSearchGaps(eligible, [0, ...expanded, 72], false)], [
    [72, { indices: [63, 66, 69] }],
  ]);
  const reverse = previewSearchGaps(eligible, [72, 0], true).get(0)!;
  assert.deepEqual([...previewSearchGaps(eligible, [72, ...gapExpansionIndices(reverse), 0], true)], [
    [0, { indices: [9, 6, 3] }],
  ]);
  assert.equal(initial.indices.length, 23);
});

test("empty, single, adjacent and fully visible results do not create gaps", () => {
  for (const visible of [[], [9], [9, 30], [2, 9, 30, 80]]) {
    assert.equal(previewSearchGaps([2, 9, 30, 80], visible, false).size, 0);
  }
  assert.equal(previewSearchGaps([], [], false).size, 0);
  assert.deepEqual(gapExpansionIndices({ indices: [] }), []);
  assert.deepEqual(gapExpansionIndices({ indices: [2, 9] }, 0), []);
  assert.deepEqual(gapExpansionIndices({ indices: [2, 9] }, -1), []);
});
