import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { setTimeout as delay } from 'node:timers/promises';
import { latencyStatistics } from './benchmark-statistics.mjs';

let child;
async function stop() {
  if (!child) return;
  const owned = child; child = undefined;
  // Failed spawn emits error/close but never exit; there is no process to stop.
  if (!owned.pid || owned.exitCode !== null || owned.signalCode !== null) return;
  const exited = once(owned, 'exit');
  owned.kill();
  await Promise.race([exited, delay(10000, undefined, { ref: false }).then(() => { if (owned.exitCode === null && owned.signalCode === null) owned.kill('SIGKILL'); })]);
  if (owned.exitCode === null && owned.signalCode === null) await exited;
}
let interrupted;
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { interrupted = signal; void stop(); });
async function start(cli, directory, settings) {
  assert(!interrupted, `Interrupted: ${interrupted}`);
  const listener = net.createServer(); listener.listen(0, '127.0.0.1'); await once(listener, 'listening');
  const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
  const origin = `http://127.0.0.1:${port}`;
  child = spawn(cli, ['webui', '--host', '127.0.0.1', '--port', String(port)], {
    cwd: directory, windowsHide: true,
    env: { ...process.env, AGENTVAULT_SEARCH_DIAGNOSTICS: '1', CC_SESSIONS_WEBUI_SETTINGS: settings, CC_SESSIONS_WEBUI_DIST: path.join(root, 'dist') },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let startupError; child.on('error', error => { startupError = error; });
  const logs = []; child.stdout.on('data', data => logs.push(data)); child.stderr.on('data', data => logs.push(data));
  child.on('close', () => { void fs.appendFile(path.join(directory, 'server.log'), Buffer.concat(logs)); });
  const deadline = performance.now() + 30000;
  while (performance.now() < deadline) {
    assert(!interrupted, `Interrupted: ${interrupted}`);
    if (startupError) throw startupError;
    if (child.exitCode !== null) throw new Error('WebUI exited during startup');
    try {
      const html = await (await fetch(origin, { signal: AbortSignal.timeout(1000) })).text();
      const config = html.match(/window\.__CC_SESSIONS_WEBUI__ = (\{.*?\});/);
      if (config) {
        const token = JSON.parse(config[1]).apiToken;
        return async (command, args = {}) => {
          const response = await fetch(`${origin}/api/invoke/${command}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-CC-Sessions-Webui-Token': token }, body: JSON.stringify(args), signal: AbortSignal.timeout(timeout) });
          const value = await response.json();
          assert(response.ok, `${command}: ${JSON.stringify(value)}`); return value;
        };
      }
    } catch (error) { if (startupError) throw startupError; }
    await delay(50);
  }
  throw new Error('WebUI startup timed out');
}
async function poll(api, command, jobId) {
  const deadline = performance.now() + timeout;
  while (performance.now() < deadline) {
    assert(!interrupted, `Interrupted: ${interrupted}`);
    const status = await api(command, { jobId });
    if (status.state !== 'running') {
      assert.equal(status.state, 'completed', JSON.stringify(status));
      assert.equal(status.failed_files, 0, JSON.stringify(status)); assert.equal(status.error, null);
      return status;
    }
    await delay(25);
  }
  throw new Error(`${command} timed out`);
}
async function inventory(directory) {
  const files = [];
  for (const item of await fs.readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, item.name);
    if (item.isDirectory()) files.push(...await inventory(file));
    else files.push({ path: file, bytes: (await fs.stat(file)).size, sha256: await hash(file) });
  }
  return files.sort((a, b) => a.path.localeCompare(b.path));
}

// Same immutable Pi sources and derived index for both executables; no native data.
const options = {};
const accepted = ['baseline', 'candidate', 'output', 'rounds', 'warmups', 'sessions', 'messages', 'body-length', 'timeout-ms', 'baseline-revision', 'candidate-revision'];
for (let i = 2; i < process.argv.length; i += 2) {
  const key = process.argv[i].replace(/^--/, '');
  assert(process.argv[i].startsWith('--') && accepted.includes(key), `Unknown option: ${process.argv[i]}`);
  assert(process.argv[i + 1], `Missing value for ${key}`); options[key] = process.argv[i + 1];
}
assert(options.baseline && options.candidate && options.output, '--baseline, --candidate and --output required');
const output = path.resolve(options.output);
const count = Number(options.sessions ?? 1000), messages = Number(options.messages ?? 100), bodyLength = Number(options['body-length'] ?? 1024);
const rounds = Number(options.rounds ?? 30), warmups = Number(options.warmups ?? 1), timeout = Number(options['timeout-ms'] ?? 1800000);
for (const [name, value] of Object.entries({ count, messages, bodyLength, rounds, warmups, timeout })) assert(Number.isSafeInteger(value) && value > 0, `${name} must be positive integer`);
assert(messages >= 2 && messages % 2 === 0 && bodyLength >= 64, 'Use even messages >= 2 and body-length >= 64');
const root = await fs.mkdtemp(path.join(os.tmpdir(), 'agentvault-pi-tail-'));
const directory = path.join(root, 'pi'), source = path.join(directory, 'source');
const hash = async file => { const digest = createHash('sha256'); for await (const chunk of createReadStream(file)) digest.update(chunk); return digest.digest('hex'); };
const report = {
  schema_version: 1, started_at: new Date().toISOString(), fixture_root: root,
  machine: { platform: process.platform, arch: process.arch, os_release: os.release(), cpu: os.cpus()[0]?.model, logical_cpus: os.cpus().length, total_memory_bytes: os.totalmem(), node: process.version },
  methodology: 'One generated Pi source tree and one derived index. AB then BA rounds form ABBA blocks. Each visit restarts the binary and warms all queries before measuring. Startup and preview verification excluded. OS cache not flushed. No outliers removed. Timestamps and PIDs allow external I/O/scheduling correlation. Revision labels supplied by caller; executable SHA-256 measured. Process I/O counters do not imply physical disk I/O; CPU wall-time gaps cannot alone separate storage waits from scheduler delays. External system samples must be aligned by UTC; no ETW context-switch trace is collected by this script.',
  sample: { sessions: count, messages_per_session: messages, body_characters: bodyLength, rounds_per_version: rounds, warmups_per_query_per_visit: warmups, poll_interval_ms: 25 },
  binaries: {}, visits: [], phases: [], pass: false,
};
const queries = [
  { name: 'sparse', query: 'AVBENCH_SPARSE_MATCH', hits: 1, role: 'assistant' },
  { name: 'absent', query: 'AVBENCH_ABSENT_QUERY', hits: 0, role: 'assistant' },
  { name: 'user', query: 'AVBENCH_USER_MATCH', hits: 1, role: 'user' },
];
const paths = [];
async function search(api, version, round, kind, query, indexed, reused) {
  const measurement = { version, round, kind, query_class: query.name, started_at: new Date().toISOString(), pid: child.pid };
  report.phases.push(measurement);
  const began = performance.now();
  try {
    const job = await api('start_workbench_content_search', { codexDir: '', claudeDir: '', piDir: source, query: query.query, scopes: [{ provider: 'pi', rollout_paths: paths }] });
    const status = await poll(api, 'content_search_status', job.job_id);
    measurement.elapsed_ms = performance.now() - began; measurement.finished_at = new Date().toISOString();
    Object.assign(measurement, { indexed_files: status.indexed_files, reused_files: status.reused_files, matching_sessions: status.results.length, scanned_files: status.scanned_files, scanned_bytes: status.scanned_bytes, diagnostics: status.diagnostics });
    assert.equal(status.indexed_files, indexed); assert.equal(status.reused_files, reused);
    assert.equal(status.results.length, query.hits); assert.equal(status.skipped_files, 0); assert.equal(status.truncated, false);
    if (query.hits) {
      assert.equal(path.resolve(status.results[0].session.rollout_path), path.resolve(paths[0]));
      const hit = status.results[0].matches.find(match => match.snippet.includes(query.query));
      assert(hit); assert.equal(hit.role, query.role);
      const preview = await api('preview_session_range', { provider: 'pi', rolloutPath: paths[0], offset: hit.event_offset, limit: 1 });
      assert.equal(preview.length, 1); assert.equal(preview[0].role, query.role); assert(preview[0].text_summary.includes(query.query));
      measurement.preview_offset_verified = true;
    }
    measurement.pass = true;
  } catch (error) {
    measurement.elapsed_ms ??= performance.now() - began; measurement.finished_at ??= new Date().toISOString();
    measurement.error = String(error.stack ?? error); measurement.pass = false; throw error;
  }
}
try {
  for (const version of ['baseline', 'candidate']) {
    const binary = path.resolve(options[version]);
    report.binaries[version] = { path: binary, sha256: await hash(binary), revision_label: options[`${version}-revision`] ?? null };
  }
  assert.notEqual(report.binaries.baseline.sha256, report.binaries.candidate.sha256, 'Baseline and candidate binaries are identical; rebuild the intended comparison');
  await fs.mkdir(path.join(root, 'dist')); await fs.writeFile(path.join(root, 'dist/index.html'), '<html><head></head><body>Pi tail benchmark</body></html>');
  await fs.mkdir(path.join(source, 'sessions/project'), { recursive: true });
  for (let n = 0; n < count; n++) {
    const id = `s${String(n).padStart(6, '0')}`, timestamp = '2026-09-30T00:00:00Z';
    const rows = [{ type: 'session', version: 3, id, cwd: '/benchmark', timestamp }];
    for (let m = 0; m < messages; m++) {
      const role = m % 2 ? 'assistant' : 'user';
      const prefix = n === 0 && m === 0 ? queries[2].query : n === 0 && m === messages - 1 ? queries[0].query : `message-${m}`;
      const text = prefix + ' x'.repeat(Math.ceil(bodyLength / 2)).slice(0, bodyLength - prefix.length);
      rows.push({ type: 'message', id: `${id}-${m}`, parentId: m ? `${id}-${m - 1}` : null, timestamp, message: { role, content: text } });
    }
    const file = path.join(source, 'sessions/project', `${id}.jsonl`); paths.push(file);
    await fs.writeFile(file, rows.map(row => JSON.stringify(row)).join('\n') + '\n');
  }
  report.source_hashes_before = await inventory(source);
  const settings = Object.fromEntries(['codex', 'claude', 'opencode', 'cursor', 'qoder', 'workbuddy', 'grok', 'pi', 'dsh', 'hermes', 'zcode', 'qwen', 'cline', 'copilot', 'antigravity'].map(name => [`${name}_dir`, '']));
  Object.assign(settings, { pi_dir: source, backup_dir: path.join(directory, 'backup'), open_command: 'auto', refresh_interval_ms: 5000, preview_only_messages: true, preview_collapse_process: true });
  const settingsFile = path.join(directory, 'settings.json'); await fs.writeFile(settingsFile, JSON.stringify(settings));
  const api = await start(report.binaries.baseline.path, directory, settingsFile);
  const scan = await api('start_workbench_scan', { codexDir: '', claudeDir: '', piDir: source, provider: 'pi' });
  assert.equal((await poll(api, 'workbench_scan_status', scan.job_id)).results.length, count);
  await search(api, 'baseline', 0, 'seed_index', queries[0], count, 0); await stop();
  for (let round = 1; round <= rounds; round++) {
    const order = round % 2 ? ['baseline', 'candidate'] : ['candidate', 'baseline'];
    for (const version of order) {
      console.log(`Pi round ${round}/${rounds}: ${version}`);
      const visit = { order: report.visits.length + 1, round, version, started_at: new Date().toISOString() }; report.visits.push(visit);
      const api = await start(report.binaries[version].path, directory, settingsFile);
      visit.pid = child.pid; report.binaries[version].app_version = await api('app_version');
      const rotated = queries.map((_, position) => queries[(round - 1 + position) % queries.length]);
      for (let warmup = 0; warmup < warmups; warmup++) for (const query of rotated) await search(api, version, round, 'warmup', query, 0, count);
      for (const query of rotated) await search(api, version, round, 'measured', query, 0, count);
      await stop(); visit.finished_at = new Date().toISOString();
      await fs.writeFile(output, JSON.stringify(report, null, 2));
    }
  }
  report.latency = Object.fromEntries(['baseline', 'candidate'].map(version => [version, Object.fromEntries(queries.map(query => [query.name, latencyStatistics(report.phases.filter(sample => sample.version === version && sample.kind === 'measured' && sample.query_class === query.name).map(sample => sample.elapsed_ms))]))]));
  report.pass = true;
} catch (error) { report.error = String(error.stack ?? error); process.exitCode = 1; }
finally {
  await stop();
  if (report.source_hashes_before) {
    try {
      report.source_hashes_after = await inventory(source);
      assert.deepEqual(report.source_hashes_after, report.source_hashes_before); report.source_hashes_unchanged = true;
    } catch (error) { report.source_hashes_unchanged = false; report.pass = false; report.error = `${report.error ?? ''}\n${error.stack ?? error}`; process.exitCode = 1; }
  }
  report.finished_at = new Date().toISOString();
  await fs.writeFile(output, JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ pass: report.pass, output, fixture_root: root, error: report.error }));
}
