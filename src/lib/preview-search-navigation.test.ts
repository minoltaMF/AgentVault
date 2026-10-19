import assert from "node:assert/strict";
import test from "node:test";
import { selectPreviewSearchHit, stepPreviewSearchHit, previewSearchScope } from "./preview-search-navigation";

test("navigation follows visible order, wraps, and preserves non-contiguous source indices", () => {
  const hits = [8, 42, 900];
  assert.equal(stepPreviewSearchHit(hits, 8, 1), 42);
  assert.equal(stepPreviewSearchHit(hits, 900, 1), 8);
  assert.equal(stepPreviewSearchHit(hits, 8, -1), 900);
  assert.equal(stepPreviewSearchHit([...hits].reverse(), 900, 1), 42);
});
test("selection preserves the active hit through added results and honors explicit search targets", () => {
  assert.equal(selectPreviewSearchHit([8, 42, 900], 42, 900), 42);
  assert.equal(selectPreviewSearchHit([8, 42, 900], null, 900), 900);
  assert.equal(selectPreviewSearchHit([8, 42], 900, null), 8);
  assert.equal(selectPreviewSearchHit([], 42, null), null);
  assert.equal(stepPreviewSearchHit([], null, 1), null);
  assert.equal(stepPreviewSearchHit([42], 42, -1), 42);
  assert.equal(stepPreviewSearchHit([8, 42], null, -1), 42);
});
test("counts never describe partial, reverse-tail or failed reads as a complete conversation", () => {
  assert.equal(previewSearchScope(0, 1200, 1200, true, false, false), "完整会话");
  assert.equal(previewSearchScope(1000, 1200, 1200, true, false, false), "已加载范围");
  assert.equal(previewSearchScope(0, 200, 1200, true, true, false), "已加载范围");
  assert.equal(previewSearchScope(0, 200, 0, false, true, false), "完整会话");
  assert.equal(previewSearchScope(0, 200, 1200, false, true, true), "部分结果 · 读取中断");
});
