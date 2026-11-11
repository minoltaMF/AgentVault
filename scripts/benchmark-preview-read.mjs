// Read-only HTTP measurements. Point --fixture at a synthetic session configured
// in a local WebUI. This script does not start services or modify session files.
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import { createHash } from 'node:crypto';
import { latencyStatistics } from './benchmark-statistics.mjs';

const options = {};
for (let i = 2; i < process.argv.length; i += 2) {
  assert(['--base-url', '--baseline-url', '--provider', '--fixture', '--output', '--rounds', '--offsets'].includes(process.argv[i]), 'Unknown option');
  assert(process.argv[i + 1], `Missing value for ${process.argv[i]}`);
  options[process.argv[i].slice(2)] = process.argv[i + 1];
}
assert(options.fixture && options.output, '--fixture and --output are required');
const targets = [{ mode: 'candidate', url: options['base-url'] ?? 'http://127.0.0.1:43289' }];
if (options['baseline-url']) targets.unshift({ mode: 'baseline', url: options['baseline-url'] });
const provider = options.provider ?? 'claude';
const rounds = Number(options.rounds ?? 9);
const offsets = (options.offsets ?? '0,1000,5000,10000,20000,28000').split(',').map(Number);
assert(Number.isSafeInteger(rounds) && rounds >= 5, 'Use at least five rounds');
assert(offsets.length && offsets.every(n => Number.isSafeInteger(n) && n >= 0), 'Invalid offsets');
for (const target of targets) {
  target.base = new URL(target.url);
  assert(target.base.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(target.base.hostname), 'Use a local synthetic WebUI');
  const htmlResponse = await fetch(target.base, { signal: AbortSignal.timeout(30000), redirect: 'error' });
  assert(htmlResponse.ok, `WebUI HTTP ${htmlResponse.status}`);
  const html = await htmlResponse.text();
  const bootstrap = html.match(/window\.__CC_SESSIONS_WEBUI__ = (\{.*?\});/);
  assert(bootstrap, 'WebUI bootstrap missing');
  target.token = JSON.parse(bootstrap[1]).apiToken;
  assert(typeof target.token === 'string' && target.token.length > 0, 'Local API token missing');
}

async function read(target, offset) {
  const start = performance.now();
  const response = await fetch(new URL('/api/invoke/preview_session_range', target.base), {
    method: 'POST', signal: AbortSignal.timeout(30000), redirect: 'error',
    headers: { 'Content-Type': 'application/json', 'X-CC-Sessions-Webui-Token': target.token },
    body: JSON.stringify({ provider, rolloutPath: options.fixture, offset, limit: 200 }),
  });
  const text = await response.text();
  const completed = performance.now();
  assert(response.ok, `Preview HTTP ${response.status}`);
  const parseStart = performance.now();
  const events = JSON.parse(text);
  const parsed = performance.now();
  assert(Array.isArray(events) && events.length === 200, 'Each measured offset must return a full page');
  return { mode: target.mode, offset, read_ms: completed - start, parse_ms: parsed - parseStart,
    bytes: Buffer.byteLength(text), first_index: events[0].index, last_index: events.at(-1).index,
    sha256: createHash('sha256').update(JSON.stringify(events)).digest('hex') };
}
const expected = new Map();
function verify(sample) {
  if (!expected.has(sample.offset)) expected.set(sample.offset, sample.sha256);
  assert.equal(sample.sha256, expected.get(sample.offset), `Page changed at offset ${sample.offset}`);
}
for (const offset of offsets) for (const target of targets) verify(await read(target, offset));
const samples = [];
for (let round = 0; round < rounds; round++) {
  for (let i = 0; i < offsets.length; i++) {
    const order = round % 2 ? [...targets].reverse() : targets;
    for (const target of order) {
      const sample = await read(target, offsets[(i + round) % offsets.length]);
      verify(sample);
      samples.push({ round: round + 1, ...sample });
    }
  }
}
const summary = offsets.flatMap(offset => targets.map(({ mode }) => {
  const selected = samples.filter(sample => sample.offset === offset && sample.mode === mode);
  return { offset, mode, http_body: latencyStatistics(selected.map(sample => sample.read_ms)),
    response_json_parse: latencyStatistics(selected.map(sample => sample.parse_ms)) };
}));
const result = { schema_version: 2, provider, rounds, offsets, page_size: 200,
  measured_at: new Date().toISOString(), node: process.version,
  scope: 'Warm file cache; rotating offsets and alternating baseline/candidate order after one warmup per target/offset. Full page SHA-256 equality checked outside timing. HTTP body includes backend, serialization and transport; JSON parse is separate. No frontend render or browser click timing.',
  summary, samples };
await writeFile(options.output, JSON.stringify(result, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify({ output: options.output, summary }, null, 2));
