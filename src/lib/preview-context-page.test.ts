import assert from "node:assert/strict";
import test from "node:test";
import { contextPageRequest, previewTailReached } from "./preview-context-page";

test("only request one adjacent page when fewer than three loaded neighbors remain", () => {
  assert.equal(contextPageRequest("before", 3, 250, 450, false, 200), null);
  assert.deepEqual(contextPageRequest("before", 2, 250, 450, false, 200), { start: 50, count: 200 });
  assert.deepEqual(contextPageRequest("before", 0, 80, 280, true, 200), { start: 0, count: 80 });
  assert.deepEqual(contextPageRequest("after", 0, 250, 450, false, 200), { start: 450, count: 200 });
});
test("first and last boundaries are independent of sort order and filtered neighbors", () => {
  assert.equal(contextPageRequest("before", 0, 0, 200, false, 200), null);
  assert.equal(contextPageRequest("after", 0, 1000, 1200, true, 200), null);
  assert.deepEqual(contextPageRequest("after", 0, 1000, 1200, false, 200), { start: 1200, count: 200 });
});
test("prepending preserves a known tail; replacement and append establish their own tail", () => {
  assert.equal(previewTailReached(true, false, true, 200, 200), true);
  assert.equal(previewTailReached(false, false, true, 200, 200), false);
  assert.equal(previewTailReached(true, true, false, 200, 200), false);
  assert.equal(previewTailReached(false, false, false, 0, 200), true);
  assert.equal(previewTailReached(false, true, false, 80, 200), true);
});
