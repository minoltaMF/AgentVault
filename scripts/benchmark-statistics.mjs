import assert from 'node:assert/strict';

// Nearest-rank percentiles retain the measured values (no interpolation).
export function latencyStatistics(samples) {
  assert(samples.length > 0, 'At least one latency sample is required');
  assert(samples.every(value => Number.isFinite(value) && value >= 0), 'Latencies must be finite, nonnegative numbers');
  const sorted = [...samples].sort((a, b) => a - b);
  return {
    samples: sorted.length,
    percentile_method: 'nearest-rank',
    p50_ms: sorted[Math.ceil(sorted.length * 0.50) - 1],
    p95_ms: sorted[Math.ceil(sorted.length * 0.95) - 1],
    max_ms: sorted.at(-1),
    tail_sample_warning: sorted.length < 30 ? 'Fewer than 30 samples; P95 is descriptive only and may equal the maximum.' : null,
  };
}
