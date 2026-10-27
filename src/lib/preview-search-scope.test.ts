import assert from "node:assert/strict";
import test from "node:test";
import { previewSearchScope } from "./preview-search-scope";

test("full pages still need EOF confirmation and only request one adjacent page", () => {
  const scope = previewSearchScope(0, 200, false, false, false, 0, 200);
  assert.equal(scope.complete, false);
  assert.equal(scope.before, null);
  assert.deepEqual(scope.after, { start: 200, count: 200 });
  assert.equal(scope.range, "已搜索原始事件 1–200");
  assert.match(scope.result, /已加载范围无命中/);
});
test("middle and tail windows never imply a complete search", () => {
  const middle = previewSearchScope(400, 600, false, false, false, 2, 200);
  assert.deepEqual(middle.before, { start: 200, count: 200 });
  assert.deepEqual(middle.after, { start: 600, count: 200 });
  assert.equal(middle.complete, false);
  assert.equal(middle.range, "已搜索原始事件 401–600");
  const tail = previewSearchScope(100, 250, true, false, false, 0, 200);
  assert.deepEqual(tail.before, { start: 0, count: 100 });
  assert.equal(tail.after, null);
  assert.equal(tail.complete, false);
});
test("only both boundaries with a successful settled read prove no matches", () => {
  assert.equal(previewSearchScope(0, 250, true, false, false, 0, 200).result, "整个会话无命中");
  assert.equal(previewSearchScope(0, 250, true, false, false, 1, 200).result, "整个会话已搜索");
  assert.equal(previewSearchScope(0, 0, true, false, false, 0, 200).complete, true);
  for (const [failed, busy] of [[true, false], [false, true], [true, true]]) {
    const scope = previewSearchScope(0, 250, true, failed, busy, 0, 200);
    assert.equal(scope.complete, false);
    assert.doesNotMatch(scope.result, /整个会话无命中/);
  }
});
