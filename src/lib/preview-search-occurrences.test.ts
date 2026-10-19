import assert from "node:assert/strict";
import test from "node:test";
import { groupSearchMarks, clampOccurrenceIndex, stepOccurrenceIndex } from "./preview-search-occurrences";

test("split inline marks form one occurrence, independent renderers remain distinct", () => {
  const mark = (id: string | null) => ({getAttribute: () => id});
  const marks = [mark("a:0"), mark("a:0"), mark("a:1"), mark("b:0"), mark(null), mark(null)];
  const groups = groupSearchMarks(marks);
  assert.deepEqual(groups.map(group => group.length), [2, 1, 1, 1, 1]);
  assert.equal(groups[0][0], marks[0]);
  assert.equal(groups[0][1], marks[1]);
  assert.deepEqual(groupSearchMarks([]), []);
});

test("message-local navigation wraps and handles empty, single and shrinking content", () => {
  assert.equal(stepOccurrenceIndex(0, 3, -1), 2);
  assert.equal(stepOccurrenceIndex(2, 3, 1), 0);
  assert.equal(stepOccurrenceIndex(0, 3, 1), 1);
  assert.equal(stepOccurrenceIndex(0, 0, 1), 0);
  assert.equal(stepOccurrenceIndex(0, 1, -1), 0);
  assert.equal(clampOccurrenceIndex(5, 2), 1);
  assert.equal(clampOccurrenceIndex(-1, 2), 0);
});
