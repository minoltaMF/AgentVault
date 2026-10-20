import assert from "node:assert/strict";
import test from "node:test";
import { previewSearchContext } from "./preview-search-context";
import { selectPreviewSearchHit, stepPreviewSearchHit } from "./preview-search-navigation";

test("context uses filtered chronological neighbors with non-contiguous source identities", () => {
  assert.deepEqual(previewSearchContext([3, 9, 42, 80, 91, 120], 42, 1, 2), {
    indices: [9, 42, 80, 91], before: 1, after: 2, remainingBefore: 1, remainingAfter: 1,
  });
});
test("context clamps to the loaded window and handles absent or removed hits", () => {
  assert.deepEqual(previewSearchContext([9, 42, 80], 9, 3, 3), {
    indices: [9, 42, 80], before: 0, after: 2, remainingBefore: 0, remainingAfter: 0,
  });
  for (const active of [null, 100]) {
    assert.deepEqual(previewSearchContext([9, 42], active, 3, 3).indices, []);
  }
  assert.deepEqual(previewSearchContext([], null, 3, 3).indices, []);
  assert.deepEqual(previewSearchContext([9], 9, 3, 3).indices, [9]);
});
test("expanding context neither adds hits nor changes their selection and navigation order", () => {
  const source = [2, 5, 9, 20, 30, 50, 80];
  const hits = [9, 50];
  const context = new Set(previewSearchContext(source, 50, 3, 3).indices);
  const visible = source.filter(index => hits.includes(index) || context.has(index));
  assert.deepEqual(visible, [9, 20, 30, 50, 80]);
  assert.equal(selectPreviewSearchHit(hits, 50, null), 50);
  assert.equal(stepPreviewSearchHit(hits, 9, 1), 50);
  assert.equal(stepPreviewSearchHit([...hits].reverse(), 50, 1), 9);
  assert.deepEqual(previewSearchContext(source, 9, 0, 0).indices, [9]);
});
