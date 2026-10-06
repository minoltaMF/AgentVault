import assert from 'node:assert/strict';
import test from 'node:test';
import { latencyStatistics } from './benchmark-statistics.mjs';

test('nearest-rank percentiles use measured values without changing input order', () => {
  const samples = Array.from({ length: 40 }, (_, i) => 40 - i);
  const result = latencyStatistics(samples);
  assert.equal(result.samples, 40);
  assert.equal(result.p50_ms, 20);
  assert.equal(result.p95_ms, 38);
  assert.equal(result.max_ms, 40);
  assert.equal(result.tail_sample_warning, null);
  assert.equal(samples[0], 40);
});

test('one and three samples retain a visible small-sample limitation', () => {
  const single = latencyStatistics([12.5]);
  assert.equal(single.p50_ms, 12.5);
  assert.equal(single.p95_ms, 12.5);
  assert.match(single.tail_sample_warning, /Fewer than 30/);
  const three = latencyStatistics([30, 10, 20]);
  assert.equal(three.p50_ms, 20);
  assert.equal(three.p95_ms, 30);
});

test('invalid and empty measurements cannot produce a summary', () => {
  for (const samples of [[], [-1], [NaN], [Infinity]]) assert.throws(() => latencyStatistics(samples));
  assert.equal(latencyStatistics([0]).max_ms, 0);
});
